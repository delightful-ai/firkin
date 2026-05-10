//! In-memory representation of the filesystem we're about to emit, plus
//! the serialization function that turns it into a single ext4 image.
//!
//! Design:
//!   - Callers describe what they want (`add_file`, `add_dir`, `add_symlink`)
//!     against a `FileSystemBuilder`.
//!   - `write(&mut W)` performs the actual byte layout in a single pass:
//!       1. Reserve metadata region: superblock + GDT + bitmaps + inode
//!          table.
//!       2. Write each directory's data blocks + each file's data blocks.
//!       3. Finalize: bitmaps, inode table, group descriptors, superblock.
//!
//! Byte layout matches the Swift reference (EXT4+Formatter.swift) with two
//! intentional simplifications for the spike:
//!   - Multiple block groups. We compute `blocks_per_group = block_size * 8`,
//!     lay out one bitmap/table set per group, and cover the path with
//!     e2fsck-clean tests.
//!   - Linear directories only (no htree). e2fsck accepts these for any
//!     directory size; the kernel mounts them happily.

use bytemuck::Zeroable;
use std::io::{Seek, SeekFrom, Write};

use crate::error::{Ext4Error, Result};
use crate::layout::{
    DirEntryHeader, EXTENT_HEADER_MAGIC, ExtentHeader, ExtentIndex, ExtentLeaf, FIRST_FREE_INODE,
    GROUP_DESC_SIZE, GroupDescriptor, INODE_EXTRA_ISIZE, INODE_INLINE_XATTR_SIZE, INODE_SIZE,
    Inode256, InodeBody, LOST_FOUND_INODE, MAX_BLOCKS_PER_EXTENT, MAX_EXTENTS_PER_INODE,
    ROOT_INODE, SUPERBLOCK_MAGIC, SUPERBLOCK_OFFSET, Superblock, XATTR_IBODY_MAGIC, XattrEntry,
    XattrIbodyHeader, feature_compat, feature_incompat, feature_ro_compat, inode_flag,
};
use crate::types::{BlockNumber, BlockSize, FileMode, InodeNumber};

/// Max file we'll emit. Matches the Swift reference.
const MAX_FILE_SIZE: u64 = 128 * 1024 * 1024 * 1024;

/// Max length of a symlink target we'll accept. 4095 bytes is the Linux
/// PATH_MAX convention; anything longer is invalid.
const MAX_SYMLINK_LEN: usize = 4095;

/// Fast-symlink threshold: target strings < 60 bytes fit inside the
/// inode's `i_block` field with no data block.
const FAST_SYMLINK_MAX: usize = 60;
/// Keep contiguous file runs reasonably coarse while still forcing depth-1
/// extent-tree coverage inside the spike's single-group budget.
const TARGET_BLOCKS_PER_FILE_EXTENT: u32 = 4096;

// ---- Public API -----------------------------------------------------------

/// A single extended attribute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xattr {
    pub name: String,
    pub value: Vec<u8>,
}

/// Build an ext4 image declaratively.
///
/// ```no_run
/// use firkin_ext4::{BlockSize, FileSystemBuilder};
/// let mut fs = FileSystemBuilder::new(BlockSize::DEFAULT);
/// fs.add_file("/hello", b"hi\n", 0o644).unwrap();
/// let mut out = std::fs::File::create("out.img").unwrap();
/// fs.write(&mut out).unwrap();
/// ```
pub struct FileSystemBuilder {
    block_size: BlockSize,
    uuid: [u8; 16],
    fs_timestamp: u32,
    /// Minimum image size in bytes. The writer expands if it needs more.
    min_size: u64,
    /// Tree of paths, indexed by parent → children.
    nodes: Vec<Node>,
    /// `path → node index` lookup. The root path is the empty string; the
    /// root node is always at index 0.
    path_index: std::collections::BTreeMap<String, usize>,
}

#[derive(Debug)]
struct Node {
    name: String,          // basename, "" for root
    parent: Option<usize>, // None for root
    children: Vec<usize>,  // child node indices (directories only)
    content: NodeContent,
    mode: FileMode,
    uid: u32,
    gid: u32,
    xattrs: Vec<Xattr>,
    /// Filled in during `finalize()`. None before.
    inode: Option<InodeNumber>,
    /// For files / non-fast symlinks: first data block (inclusive).
    start_block: BlockNumber,
    /// For files / non-fast symlinks: one-past-last data block (exclusive).
    end_block: BlockNumber,
    /// For directories: link count override (1 dot + 1 parent + 1 per child
    /// directory). Filled in lazily.
    dir_link_count: u16,
    /// For hardlinks: the target's node index (we don't allocate a new
    /// inode in that case).
    hardlink_target: Option<usize>,
    /// xattr block (single-block only), assigned during finalize.
    xattr_block: Option<BlockNumber>,
    /// Serialized file-data extents for regular files.
    file_extents: Vec<ExtentLeaf>,
    /// Optional depth-1 extent-tree leaf block. Present when `file_extents`
    /// overflow the inode's four inline slots.
    extent_leaf_block: Option<BlockNumber>,
    /// Tombstoned by OCI layer application. Nodes stay in the arena so
    /// indices remain stable, but finalization ignores deleted nodes.
    deleted: bool,
}

#[derive(Debug)]
enum NodeContent {
    Directory,
    File(Vec<u8>),
    Symlink(String),
    Whiteout,
    Hardlink, // target is `Node::hardlink_target`
}

impl FileSystemBuilder {
    /// Create a new builder. Timestamps and UUID are zero by default for
    /// reproducible bit-identical output; override with setters if you want
    /// a real mkfs-ish image.
    pub fn new(block_size: BlockSize) -> Self {
        let mut fs = Self {
            block_size,
            uuid: [0; 16],
            fs_timestamp: 0,
            min_size: 1024 * 1024, // 1 MiB floor
            nodes: Vec::new(),
            path_index: std::collections::BTreeMap::new(),
        };
        // Root node.
        fs.nodes.push(Node {
            name: String::new(),
            parent: None,
            children: Vec::new(),
            content: NodeContent::Directory,
            mode: FileMode::dir(0o755),
            uid: 0,
            gid: 0,
            xattrs: Vec::new(),
            inode: None,
            start_block: BlockNumber(0),
            end_block: BlockNumber(0),
            dir_link_count: 2, // `.` + `..`; grows as child dirs are added
            hardlink_target: None,
            xattr_block: None,
            file_extents: Vec::new(),
            extent_leaf_block: None,
            deleted: false,
        });
        fs.path_index.insert(String::new(), 0);
        // lost+found is an ext4 convention e2fsck nags without. Allocate
        // inode 11 for it at the fixed reserved slot.
        let _ = fs.add_dir_internal("/lost+found", FileMode::dir(0o700), 0, 0, LOST_FOUND_INODE);
        fs
    }

    pub fn with_min_size(mut self, bytes: u64) -> Self {
        self.min_size = bytes;
        self
    }

    pub fn with_uuid(mut self, uuid: [u8; 16]) -> Self {
        self.uuid = uuid;
        self
    }

    pub fn with_timestamp(mut self, secs_since_epoch: u32) -> Self {
        self.fs_timestamp = secs_since_epoch;
        self
    }

    pub fn add_dir(&mut self, path: &str, perm: u16) -> Result<()> {
        self.add_dir_internal(path, FileMode::dir(perm), 0, 0, 0)?;
        Ok(())
    }

    pub fn add_file(&mut self, path: &str, content: &[u8], perm: u16) -> Result<()> {
        if content.len() as u64 > MAX_FILE_SIZE {
            return Err(Ext4Error::FileTooLarge {
                bytes: content.len() as u64,
                max: MAX_FILE_SIZE,
            });
        }
        self.add_leaf_node(
            path,
            NodeContent::File(content.to_vec()),
            FileMode::regular(perm),
        )
    }

    /// Add an OCI whiteout marker for `path`.
    ///
    /// `add_whiteout("/upper/gone")` materializes `/upper/.wh.gone` as a
    /// 0:0 character-device inode, which is what overlayfs consumes.
    pub fn add_whiteout(&mut self, path: &str) -> Result<()> {
        let marker = whiteout_marker_path(path)?;
        self.add_leaf_node(&marker, NodeContent::Whiteout, FileMode::char_device(0))
    }

    /// Mark `path` as opaque for OCI overlay semantics by adding the
    /// `.wh..wh..opq` marker file inside it.
    pub fn add_opaque_dir(&mut self, path: &str) -> Result<()> {
        let marker = opaque_dir_marker_path(path);
        self.add_leaf_node(&marker, NodeContent::File(Vec::new()), FileMode::regular(0))
    }

    pub fn add_symlink(&mut self, path: &str, target: &str) -> Result<()> {
        if target.as_bytes().len() > MAX_SYMLINK_LEN {
            return Err(Ext4Error::SymlinkTargetTooLong {
                bytes: target.len(),
                max: MAX_SYMLINK_LEN,
            });
        }
        self.add_leaf_node(
            path,
            NodeContent::Symlink(target.to_string()),
            FileMode::symlink(0o777),
        )
    }

    /// Hardlink `link_path` to the inode at `target_path`. Target must
    /// exist and not be a directory.
    pub fn add_hardlink(&mut self, link_path: &str, target_path: &str) -> Result<()> {
        let target_key = normalize(target_path);
        let target_idx =
            *self
                .path_index
                .get(&target_key)
                .ok_or_else(|| Ext4Error::InvalidHardlinkTarget {
                    path: target_path.to_string(),
                })?;
        if self.nodes[target_idx].mode.is_dir() {
            return Err(Ext4Error::InvalidHardlinkTarget {
                path: target_path.to_string(),
            });
        }
        self.remove_existing_leaf(link_path, self.nodes[target_idx].mode)?;
        let parent = self.ensure_parent(link_path)?;
        let name = basename(link_path)?;
        let idx = self.nodes.len();
        let mode = self.nodes[target_idx].mode;
        self.nodes.push(Node {
            name: name.to_string(),
            parent: Some(parent),
            children: Vec::new(),
            content: NodeContent::Hardlink,
            mode,
            uid: 0,
            gid: 0,
            xattrs: Vec::new(),
            inode: None,
            start_block: BlockNumber(0),
            end_block: BlockNumber(0),
            dir_link_count: 0,
            hardlink_target: Some(target_idx),
            xattr_block: None,
            file_extents: Vec::new(),
            extent_leaf_block: None,
            deleted: false,
        });
        self.nodes[parent].children.push(idx);
        self.path_index.insert(normalize(link_path), idx);
        Ok(())
    }

    /// Attach an xattr to an existing path. Must be called after `add_*`.
    pub fn set_xattr(&mut self, path: &str, name: &str, value: &[u8]) -> Result<()> {
        let idx =
            *self
                .path_index
                .get(&normalize(path))
                .ok_or_else(|| Ext4Error::PathConflict {
                    path: path.to_string(),
                })?;
        self.nodes[idx].xattrs.push(Xattr {
            name: name.to_string(),
            value: value.to_vec(),
        });
        Ok(())
    }

    /// Remove `path` if it exists. Missing paths are ignored, matching OCI
    /// whiteout application.
    pub fn remove_path(&mut self, path: &str) -> Result<()> {
        let key = normalize(path);
        let Some(&idx) = self.path_index.get(&key) else {
            return Ok(());
        };
        self.delete_subtree(idx, true);
        Ok(())
    }

    /// Remove all children of `path` if it exists. Missing paths are ignored,
    /// matching OCI opaque-marker application.
    pub fn clear_dir(&mut self, path: &str) -> Result<()> {
        let key = normalize(path);
        let Some(&idx) = self.path_index.get(&key) else {
            return Ok(());
        };
        if !self.nodes[idx].mode.is_dir() {
            return Err(Ext4Error::PathConflict {
                path: path.to_string(),
            });
        }

        let children = std::mem::take(&mut self.nodes[idx].children);
        for child in children {
            self.delete_subtree(child, false);
        }
        self.nodes[idx].dir_link_count = 2;
        Ok(())
    }

    /// Serialize the filesystem to `out`. The writer seeks, so `out` must
    /// be a real file or a `Cursor<Vec<u8>>`-like seekable sink, not a
    /// pipe.
    pub fn write<W: Write + Seek>(&mut self, out: &mut W) -> Result<()> {
        Finalizer::new(self, out).run()
    }

    fn add_leaf_node(&mut self, path: &str, content: NodeContent, mode: FileMode) -> Result<()> {
        self.remove_existing_leaf(path, mode)?;
        let parent = self.ensure_parent(path)?;
        let name = basename(path)?;
        let idx = self.nodes.len();
        self.nodes.push(Node {
            name: name.to_string(),
            parent: Some(parent),
            children: Vec::new(),
            content,
            mode,
            uid: 0,
            gid: 0,
            xattrs: Vec::new(),
            inode: None,
            start_block: BlockNumber(0),
            end_block: BlockNumber(0),
            dir_link_count: 0,
            hardlink_target: None,
            xattr_block: None,
            file_extents: Vec::new(),
            extent_leaf_block: None,
            deleted: false,
        });
        self.nodes[parent].children.push(idx);
        self.path_index.insert(normalize(path), idx);
        Ok(())
    }

    fn remove_existing_leaf(&mut self, path: &str, replacement_mode: FileMode) -> Result<()> {
        let key = normalize(path);
        let Some(&idx) = self.path_index.get(&key) else {
            return Ok(());
        };

        if self.nodes[idx].mode.is_dir() != replacement_mode.is_dir() {
            return Err(Ext4Error::PathConflict {
                path: path.to_string(),
            });
        }

        self.delete_subtree(idx, true);
        Ok(())
    }

    fn node_path(&self, idx: usize) -> String {
        if idx == 0 {
            return "/".to_string();
        }
        let mut parts = Vec::new();
        let mut cursor = Some(idx);
        while let Some(i) = cursor {
            let node = &self.nodes[i];
            if !node.name.is_empty() {
                parts.push(node.name.clone());
            }
            cursor = node.parent;
        }
        parts.reverse();
        format!("/{}", parts.join("/"))
    }

    // ---- internals -----------------------------------------------------------

    fn add_dir_internal(
        &mut self,
        path: &str,
        mode: FileMode,
        uid: u32,
        gid: u32,
        fixed_inode: u32,
    ) -> Result<usize> {
        let key = normalize(path);
        if let Some(&idx) = self.path_index.get(&key) {
            if self.nodes[idx].mode.is_dir() {
                return Ok(idx);
            }
            return Err(Ext4Error::PathConflict {
                path: path.to_string(),
            });
        }
        let parent = self.ensure_parent(path)?;
        let name = basename(path)?;
        let idx = self.nodes.len();
        self.nodes.push(Node {
            name: name.to_string(),
            parent: Some(parent),
            children: Vec::new(),
            content: NodeContent::Directory,
            mode,
            uid,
            gid,
            xattrs: Vec::new(),
            inode: fixed_inode_opt(fixed_inode),
            start_block: BlockNumber(0),
            end_block: BlockNumber(0),
            dir_link_count: 2, // . + parent's ..
            hardlink_target: None,
            xattr_block: None,
            file_extents: Vec::new(),
            extent_leaf_block: None,
            deleted: false,
        });
        self.nodes[parent].children.push(idx);
        self.nodes[parent].dir_link_count += 1;
        self.path_index.insert(key, idx);
        Ok(idx)
    }

    fn delete_subtree(&mut self, idx: usize, unlink_from_parent: bool) {
        if self.nodes[idx].deleted {
            return;
        }

        let children = std::mem::take(&mut self.nodes[idx].children);
        for child in children {
            self.delete_subtree(child, false);
        }

        let key = self.node_path(idx);
        if key == "/" {
            self.path_index.remove("");
        } else {
            self.path_index.remove(&key);
        }
        self.nodes[idx].deleted = true;

        if unlink_from_parent {
            let Some(parent) = self.nodes[idx].parent else {
                return;
            };
            self.nodes[parent].children.retain(|&child| child != idx);
            if self.nodes[idx].mode.is_dir() && self.nodes[parent].dir_link_count > 2 {
                self.nodes[parent].dir_link_count -= 1;
            }
        }
    }

    /// Resolve and create ancestor directories as needed, returning the
    /// index of the immediate parent of `path`.
    fn ensure_parent(&mut self, path: &str) -> Result<usize> {
        let normalized = normalize(path);
        if normalized.is_empty() || normalized == "/" {
            return Err(Ext4Error::InvalidFilename {
                name: path.to_string(),
            });
        }
        let (parent_path, base) = split_parent(&normalized);
        if base.is_empty() || base == "." || base == ".." {
            return Err(Ext4Error::InvalidFilename {
                name: path.to_string(),
            });
        }
        if parent_path.is_empty() {
            return Ok(0); // root
        }
        if let Some(&idx) = self.path_index.get(parent_path) {
            if !self.nodes[idx].mode.is_dir() {
                return Err(Ext4Error::PathConflict {
                    path: parent_path.to_string(),
                });
            }
            return Ok(idx);
        }
        // Recurse: create ancestor. This is `mkdir -p` semantics.
        self.add_dir_internal(parent_path, FileMode::dir(0o755), 0, 0, 0)
    }
}

fn fixed_inode_opt(n: u32) -> Option<InodeNumber> {
    if n == 0 { None } else { Some(InodeNumber(n)) }
}

/// "/a/b/c" → "/a/b/c"; "a/b" → "/a/b"; "" → ""; "//" → "/".
fn normalize(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    if !path.starts_with('/') {
        out.push('/');
    }
    // Collapse repeated slashes; drop trailing slash unless the whole path is "/".
    let mut prev_slash = false;
    for ch in path.chars() {
        if ch == '/' {
            if prev_slash {
                continue;
            }
            out.push('/');
            prev_slash = true;
        } else {
            out.push(ch);
            prev_slash = false;
        }
    }
    if out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    if out == "/" {
        return String::new();
    }
    out
}

/// Given "/a/b/c", returns ("/a/b", "c"). Root handled as ("", "").
fn split_parent(normalized: &str) -> (&str, &str) {
    if let Some(idx) = normalized.rfind('/') {
        let parent = &normalized[..idx];
        let base = &normalized[idx + 1..];
        (parent, base)
    } else {
        ("", normalized)
    }
}

fn basename(path: &str) -> Result<String> {
    let normalized = normalize(path);
    let (_, base) = split_parent(&normalized);
    if base.is_empty() || base == "." || base == ".." {
        return Err(Ext4Error::InvalidFilename {
            name: path.to_string(),
        });
    }
    Ok(base.to_string())
}

fn whiteout_marker_path(path: &str) -> Result<String> {
    let normalized = normalize(path);
    let (parent, base) = split_parent(&normalized);
    if normalized.is_empty() || base.is_empty() || base == "." || base == ".." {
        return Err(Ext4Error::InvalidFilename {
            name: path.to_string(),
        });
    }
    Ok(if parent.is_empty() {
        format!("/.wh.{base}")
    } else {
        format!("{parent}/.wh.{base}")
    })
}

fn opaque_dir_marker_path(path: &str) -> String {
    let normalized = normalize(path);
    if normalized.is_empty() {
        "/.wh..wh..opq".to_string()
    } else {
        format!("{normalized}/.wh..wh..opq")
    }
}

// ---- Finalizer: the byte-layout engine ------------------------------------

/// One-shot state machine. Owns the open file and the in-flight bookkeeping
/// (current block, current inode, allocations).
struct Finalizer<'a, W: Write + Seek> {
    fs: &'a mut FileSystemBuilder,
    out: &'a mut W,
    pos: u64,
    /// Parallel to `fs.nodes` once allocation is done. Index = node index.
    inodes: Vec<Option<InodeNumber>>,
    /// Staged inode table entries, indexed by `inode_number - 1`.
    inode_table: Vec<Inode256>,
    /// Blocks consumed for directory+file data blocks (tracks the
    /// "high-water mark" of data-region blocks).
    next_free_block: BlockNumber,
    /// Superblock + GDT reservation in blocks. Set once in `layout_header()`.
    header_blocks: u32,
    /// Number of block groups.
    block_groups: u32,
    /// Inodes-per-group — comes from a rough optimization, sized so the
    /// inode table spans at least the blocks we need.
    inodes_per_group: u32,
    blocks_per_group: u32,
    /// Cached until the superblock writes them. Populated by
    /// `write_bitmaps_and_gdt`, consumed by `write_superblock`.
    total_disk_blocks_cache: u32,
    total_used_blocks_cache: u32,
    inode_used_count_cache: u32,
}

impl<'a, W: Write + Seek> Finalizer<'a, W> {
    fn new(fs: &'a mut FileSystemBuilder, out: &'a mut W) -> Self {
        Self {
            fs,
            out,
            pos: 0,
            inodes: Vec::new(),
            inode_table: Vec::new(),
            next_free_block: BlockNumber(0),
            header_blocks: 0,
            block_groups: 1,
            inodes_per_group: 0,
            blocks_per_group: 0,
            total_disk_blocks_cache: 0,
            total_used_blocks_cache: 0,
            inode_used_count_cache: 0,
        }
    }

    fn run(&mut self) -> Result<()> {
        self.allocate_inodes();
        self.layout_header()?;
        self.write_data_region()?;
        self.finalize_inode_table()?;
        self.write_bitmaps_and_gdt()?;
        self.write_superblock()?;
        self.flush()?;
        Ok(())
    }

    // ---- step 1: inode allocation -----------------------------------------

    /// Walk the node tree and assign inode numbers. Inode 2 is root; 11 is
    /// lost+found (already fixed); 3..=10 are reserved and get zero-filled
    /// stub entries; 12+ go to user nodes in BFS order.
    fn allocate_inodes(&mut self) {
        self.inodes.resize(self.fs.nodes.len(), None);

        // Root → inode 2.
        self.inodes[0] = Some(InodeNumber(ROOT_INODE));
        // Fixed-inode nodes (lost+found currently).
        for i in 1..self.fs.nodes.len() {
            if self.fs.nodes[i].deleted {
                continue;
            }
            if let Some(ino) = self.fs.nodes[i].inode {
                self.inodes[i] = Some(ino);
            }
        }

        // BFS the rest.
        let mut next = FIRST_FREE_INODE + 1; // lost+found is 11, first free is 12
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(0);
        while let Some(parent_idx) = queue.pop_front() {
            // iterate children in insertion order so the layout is
            // deterministic.
            let child_indices: Vec<usize> = self.fs.nodes[parent_idx].children.clone();
            for child in child_indices {
                if self.fs.nodes[child].deleted {
                    continue;
                }
                if self.inodes[child].is_none() {
                    if let Some(tgt) = self.fs.nodes[child].hardlink_target {
                        // Hardlinks don't get new inodes; they borrow the target's.
                        self.inodes[child] = self.inodes[tgt];
                    } else {
                        self.inodes[child] = Some(InodeNumber(next));
                        next += 1;
                    }
                }
                queue.push_back(child);
            }
        }

        // Size the inode table. We want at minimum `next` slots; ext4
        // expects inode table blocks to align nicely, and rounding up to
        // the inode-bitmap granularity (block_size * 8 = bits) matches
        // the layout mkfs chooses.
        let bits_per_bitmap = (self.fs.block_size.bytes() * 8) as u32;
        // Must be multiple of inodes_per_block (blocksize/INODE_SIZE = 16 at 4K)
        let inodes_per_block = self.fs.block_size.bytes() / INODE_SIZE;
        let min = next.saturating_sub(1).max(FIRST_FREE_INODE);
        // Keep the inode table block-aligned without forcing tiny sparse
        // images to allocate mkfs-sized inode tables. The table still grows in
        // block-aligned chunks when a large rootfs needs more inodes.
        let inc = (self.fs.block_size.bytes() * 64) / INODE_SIZE;
        let mut ipg = inc;
        while ipg < min {
            ipg = ipg.saturating_add(inc);
        }
        if ipg > bits_per_bitmap {
            ipg = bits_per_bitmap;
        }
        self.inodes_per_group = ipg;
        self.blocks_per_group = bits_per_bitmap;
        self.block_groups = self.estimate_block_groups(ipg);
        // Pre-fill inode table: one zeroed Inode256 per slot, sized to
        // `inodes_per_group` so the table length matches what the
        // superblock advertises.
        self.inode_table = vec![Zeroable::zeroed(); ipg as usize];
        let _ = inodes_per_block; // not directly needed past the above calc
    }

    fn estimate_block_groups(&self, inodes_per_group: u32) -> u32 {
        let block_size = self.fs.block_size.bytes();
        let blocks_per_group = self.blocks_per_group.max(1);
        let inode_table_blocks = (inodes_per_group * INODE_SIZE + block_size - 1) / block_size;
        let payload_blocks = self.estimate_payload_blocks();
        let min_blocks = div_ceil_u64(self.fs.min_size, self.fs.block_size.bytes_u64());

        let mut groups = 1u32;
        loop {
            let gdt_bytes = groups * GROUP_DESC_SIZE;
            let gdt_blocks = ((gdt_bytes + block_size - 1) / block_size).max(1);
            let header_blocks = first_gdt_block_for_size(self.fs.block_size) + gdt_blocks;
            let metadata_blocks = groups * (inode_table_blocks + 2);
            let needed_blocks =
                u64::from(header_blocks + payload_blocks + metadata_blocks).max(min_blocks);
            let next_groups = div_ceil_u64(needed_blocks, u64::from(blocks_per_group))
                .try_into()
                .unwrap_or(u32::MAX)
                .max(1);
            if next_groups == groups {
                return groups;
            }
            groups = next_groups;
        }
    }

    fn estimate_payload_blocks(&self) -> u32 {
        let block_size = self.fs.block_size.bytes() as usize;
        let mut blocks = 0u32;
        for (idx, node) in self.fs.nodes.iter().enumerate() {
            if node.deleted || self.inodes.get(idx).copied().flatten().is_none() {
                continue;
            }
            if node.hardlink_target.is_some() {
                continue;
            }
            match &node.content {
                NodeContent::Directory => {
                    blocks += self.directory_block_count(idx).unwrap_or(u32::MAX);
                }
                NodeContent::File(content) => {
                    if !content.is_empty() {
                        let file_blocks = div_ceil_usize(content.len(), block_size);
                        blocks += file_blocks;
                        let extents = div_ceil_u32(file_blocks, TARGET_BLOCKS_PER_FILE_EXTENT);
                        if extents > MAX_EXTENTS_PER_INODE {
                            blocks += 1;
                        }
                    }
                }
                NodeContent::Symlink(target) => {
                    if target.len() >= FAST_SYMLINK_MAX {
                        blocks += 1;
                    }
                }
                NodeContent::Whiteout | NodeContent::Hardlink => {}
            }
            if !node.xattrs.is_empty() && !Self::xattrs_fit_inline(&node.xattrs) {
                blocks += 1;
            }
        }
        blocks
    }

    // ---- step 2: reserve the header region --------------------------------

    fn layout_header(&mut self) -> Result<()> {
        // Block 0 contains the 1024-byte boot sector (zeros) + the 1024-byte
        // superblock. At 4 KiB block size the rest is padding.
        // Block 1..=N is the group descriptor table. With one group that's
        // one block's worth of GDT entries.
        let groups = self.block_groups;
        let gdt_bytes = groups * GROUP_DESC_SIZE;
        let gdt_blocks =
            ((gdt_bytes + self.fs.block_size.bytes() - 1) / self.fs.block_size.bytes()).max(1);
        self.header_blocks = self.first_gdt_block() + gdt_blocks;
        // Data region starts at header_blocks.
        self.next_free_block = BlockNumber(self.header_blocks);
        // Seek to the start of the data region.
        self.seek_to_block(self.next_free_block)?;
        Ok(())
    }

    // ---- step 3: data-region walk -----------------------------------------

    /// Write data blocks for every directory and file in a deterministic
    /// order (BFS by inode), tracking (start_block, end_block) on each
    /// node so the inode's extent tree can point at them.
    fn write_data_region(&mut self) -> Result<()> {
        // xattr blocks first: every node with xattrs whose total xattr
        // payload exceeds the inline capacity needs a dedicated block.
        // (For simplicity we always emit a block if there are any xattrs
        // that didn't fit inline.)
        let mut nodes_to_process: Vec<usize> = (0..self.fs.nodes.len()).collect();
        // BFS order ensures parent directories' children are all placed
        // before the parent's directory block is written (so `.` / `..` /
        // entries reference already-allocated inode numbers).
        // We don't actually need this order for correctness — the parent's
        // directory block writes the inode numbers already assigned above —
        // but it matches the Swift reference and keeps output deterministic.
        nodes_to_process.sort_by_key(|&i| self.inodes[i].map(|n| n.0).unwrap_or(u32::MAX));

        for idx in nodes_to_process {
            if self.fs.nodes[idx].deleted || self.inodes[idx].is_none() {
                continue;
            }
            // Hardlinks share their target's inode & blocks — skip.
            if self.fs.nodes[idx].hardlink_target.is_some() {
                continue;
            }
            match &self.fs.nodes[idx].content {
                NodeContent::Directory => self.write_directory_block(idx)?,
                NodeContent::File(_) => self.write_file_blocks(idx)?,
                NodeContent::Symlink(_) => self.write_symlink_blocks(idx)?,
                NodeContent::Whiteout => {}
                NodeContent::Hardlink => {}
            }
        }

        // After everything, allocate xattr blocks. They don't need to be
        // placed in any particular spot; allocating them *after* the data
        // region means we keep the "data region is contiguous" invariant
        // simple to reason about.
        let node_count = self.fs.nodes.len();
        for idx in 0..node_count {
            if self.fs.nodes[idx].deleted || self.inodes[idx].is_none() {
                continue;
            }
            if self.fs.nodes[idx].xattrs.is_empty() {
                continue;
            }
            // If everything fits inline, skip the block.
            if Self::xattrs_fit_inline(&self.fs.nodes[idx].xattrs) {
                continue;
            }
            let block = self.next_free_block;
            self.seek_to_block(block)?;
            let mut buf = vec![0u8; self.fs.block_size.bytes() as usize];
            write_xattr_block(&self.fs.nodes[idx].xattrs, &mut buf)?;
            self.write_bytes(&buf)?;
            self.next_free_block = block.next();
            self.fs.nodes[idx].xattr_block = Some(block);
        }

        Ok(())
    }

    fn write_directory_block(&mut self, idx: usize) -> Result<()> {
        // We pack `.`, `..`, and all children into linear directory entries.
        // Large directories are emitted as a contiguous run of blocks without
        // htree indexing; e2fsck accepts this and Linux mounts it normally.
        let bs = self.fs.block_size.bytes() as usize;
        let dot = self.inodes[idx].unwrap();
        let parent_idx = self.fs.nodes[idx].parent.unwrap_or(idx);
        let dotdot = self.inodes[parent_idx].unwrap();

        let mut entries = vec![
            DirEntrySpec {
                inode: dot,
                name: ".".to_owned(),
                file_type: FileMode::dir(0o755).dir_entry_type(),
            },
            DirEntrySpec {
                inode: dotdot,
                name: "..".to_owned(),
                file_type: FileMode::dir(0o755).dir_entry_type(),
            },
        ];
        for child_idx in self.sorted_live_children(idx) {
            let child_inode = self.inodes[child_idx].unwrap();
            entries.push(DirEntrySpec {
                inode: child_inode,
                name: self.fs.nodes[child_idx].name.clone(),
                file_type: self.fs.nodes[child_idx].mode.dir_entry_type(),
            });
        }

        let start = self.next_free_block;
        let mut entry_index = 0usize;
        while entry_index < entries.len() {
            let mut buf = vec![0u8; bs];
            let mut off = 0usize;

            while entry_index < entries.len() {
                let entry = &entries[entry_index];
                let entry_len = dir_entry_min_len(&entry.name);
                if off + entry_len > bs {
                    if off == 0 {
                        return Err(Ext4Error::DirEntryOverflow {
                            name: entry.name.clone(),
                            rec_len: entry_len.min(u16::MAX as usize) as u16,
                        });
                    }
                    write_dir_entry_tail(&mut buf, off, bs)?;
                    break;
                }

                let next_fits = entries
                    .get(entry_index + 1)
                    .map(|next| off + entry_len + dir_entry_min_len(&next.name) <= bs)
                    .unwrap_or(false);
                let last_in_block = !next_fits;
                off = write_dir_entry(
                    &mut buf,
                    off,
                    bs,
                    entry.inode,
                    &entry.name,
                    entry.file_type,
                    last_in_block,
                )?;
                entry_index += 1;
                if last_in_block {
                    break;
                }
            }

            let block = self.next_free_block;
            self.seek_to_block(block)?;
            self.write_bytes(&buf)?;
            self.next_free_block = block.next();
        }
        self.fs.nodes[idx].start_block = start;
        self.fs.nodes[idx].end_block = self.next_free_block;
        Ok(())
    }

    fn directory_block_count(&self, idx: usize) -> Result<u32> {
        let bs = self.fs.block_size.bytes() as usize;
        let mut blocks = 1u32;
        let mut off = 0usize;
        let mut names = vec![".", ".."];
        let child_names = self
            .sorted_live_children(idx)
            .into_iter()
            .map(|child| self.fs.nodes[child].name.as_str())
            .collect::<Vec<_>>();
        names.extend(child_names);

        for name in names {
            let entry_len = dir_entry_min_len(name);
            if entry_len > bs {
                return Err(Ext4Error::DirEntryOverflow {
                    name: name.to_owned(),
                    rec_len: entry_len.min(u16::MAX as usize) as u16,
                });
            }
            if off + entry_len > bs {
                blocks = blocks.saturating_add(1);
                off = 0;
            }
            off += entry_len;
        }
        Ok(blocks)
    }

    fn sorted_live_children(&self, idx: usize) -> Vec<usize> {
        let mut children_sorted: Vec<usize> = self.fs.nodes[idx].children.clone();
        children_sorted
            .retain(|&child| !self.fs.nodes[child].deleted && self.inodes[child].is_some());
        children_sorted.sort_by_key(|&c| self.inodes[c].map(|n| n.0).unwrap_or(0));
        children_sorted
    }

    fn write_file_blocks(&mut self, idx: usize) -> Result<()> {
        let bs = self.fs.block_size.bytes() as usize;
        let content_len = match &self.fs.nodes[idx].content {
            NodeContent::File(c) => c.len(),
            _ => unreachable!(),
        };
        if content_len == 0 {
            // Empty file: no data blocks.
            self.fs.nodes[idx].start_block = BlockNumber(0);
            self.fs.nodes[idx].end_block = BlockNumber(0);
            self.fs.nodes[idx].file_extents.clear();
            self.fs.nodes[idx].extent_leaf_block = None;
            return Ok(());
        }
        let start = self.next_free_block;
        self.seek_to_block(start)?;
        // Write content, padding to block boundary.
        // Extract content separately to avoid borrowing self.fs.nodes[idx]
        // while calling &mut self methods.
        let content = match &self.fs.nodes[idx].content {
            NodeContent::File(c) => c.clone(),
            _ => unreachable!(),
        };
        self.write_bytes(&content)?;
        let remainder = content.len() % bs;
        if remainder != 0 {
            let pad = vec![0u8; bs - remainder];
            self.write_bytes(&pad)?;
        }
        let blocks_written = ((content.len() + bs - 1) / bs) as u32;
        let end = BlockNumber(start.get() + blocks_written);
        self.next_free_block = end;
        self.fs.nodes[idx].start_block = start;
        self.fs.nodes[idx].end_block = end;
        let path = self.fs.node_path(idx);
        let extents =
            split_contiguous_extent(0, blocks_written, start, TARGET_BLOCKS_PER_FILE_EXTENT);
        let mut extent_leaf_block = None;
        if extents.len() > MAX_EXTENTS_PER_INODE as usize {
            let leaf_block = self.next_free_block;
            self.seek_to_block(leaf_block)?;
            let mut buf = vec![0u8; bs];
            write_extent_leaf_block(&extents, self.fs.block_size, &mut buf, &path)?;
            self.write_bytes(&buf)?;
            self.next_free_block = leaf_block.next();
            extent_leaf_block = Some(leaf_block);
        }
        self.fs.nodes[idx].file_extents = extents;
        self.fs.nodes[idx].extent_leaf_block = extent_leaf_block;
        Ok(())
    }

    fn write_symlink_blocks(&mut self, idx: usize) -> Result<()> {
        let target = match &self.fs.nodes[idx].content {
            NodeContent::Symlink(t) => t.clone(),
            _ => unreachable!(),
        };
        if target.len() < FAST_SYMLINK_MAX {
            // Stored inline in i_block — no data blocks.
            self.fs.nodes[idx].start_block = BlockNumber(0);
            self.fs.nodes[idx].end_block = BlockNumber(0);
            return Ok(());
        }
        // Slow symlink: one data block.
        let start = self.next_free_block;
        self.seek_to_block(start)?;
        let bs = self.fs.block_size.bytes() as usize;
        let mut buf = vec![0u8; bs];
        buf[..target.len()].copy_from_slice(target.as_bytes());
        self.write_bytes(&buf)?;
        let end = start.next();
        self.next_free_block = end;
        self.fs.nodes[idx].start_block = start;
        self.fs.nodes[idx].end_block = end;
        Ok(())
    }

    // ---- step 4: inode-table construction ---------------------------------

    fn finalize_inode_table(&mut self) -> Result<()> {
        // Reserved inodes 3..=10: leave as zeroed. Inode 1 (defective
        // block): also zeroed.
        // Root, lost+found, and every user node: fill in real records.
        for node_idx in 0..self.fs.nodes.len() {
            if self.fs.nodes[node_idx].deleted
                || self.inodes[node_idx].is_none()
                || self.fs.nodes[node_idx].hardlink_target.is_some()
            {
                continue; // processed via the target
            }
            let ino = self.inodes[node_idx].unwrap();
            let record = self.build_inode_record(node_idx)?;
            let slot = ino.table_index() as usize;
            self.inode_table[slot] = record;
        }

        // Hardlinks: bump the target's link count by the number of
        // hardlinks referring to it.
        for node_idx in 0..self.fs.nodes.len() {
            if self.fs.nodes[node_idx].deleted || self.inodes[node_idx].is_none() {
                continue;
            }
            if let Some(tgt) = self.fs.nodes[node_idx].hardlink_target {
                let ino = self.inodes[tgt].unwrap();
                let slot = ino.table_index() as usize;
                self.inode_table[slot].body.i_links_count += 1;
            }
        }
        Ok(())
    }

    fn build_inode_record(&self, idx: usize) -> Result<Inode256> {
        let node = &self.fs.nodes[idx];
        let mut body = InodeBody::zeroed();
        body.i_mode = node.mode.0;
        body.i_uid = (node.uid & 0xFFFF) as u16;
        body.i_uid_hi = ((node.uid >> 16) & 0xFFFF) as u16;
        body.i_gid = (node.gid & 0xFFFF) as u16;
        body.i_gid_hi = ((node.gid >> 16) & 0xFFFF) as u16;
        body.i_atime = self.fs.fs_timestamp;
        body.i_ctime = self.fs.fs_timestamp;
        body.i_mtime = self.fs.fs_timestamp;
        body.i_crtime = self.fs.fs_timestamp;
        body.i_extra_isize = INODE_EXTRA_ISIZE;
        // flags: we always use extents for files/dirs/slow symlinks, plus
        // huge_file so blocks count is in filesystem-blocks, not 512B units.
        // Fast symlinks keep flags=0 (they store data inline).
        let will_use_extents = match &node.content {
            // Every directory owns at least one data block for `.`/`..`.
            NodeContent::Directory => true,
            NodeContent::File(c) => !c.is_empty(),
            NodeContent::Symlink(t) => t.len() >= FAST_SYMLINK_MAX,
            NodeContent::Whiteout => false,
            NodeContent::Hardlink => false,
        };
        if will_use_extents {
            body.i_flags = inode_flag::EXTENTS | inode_flag::HUGE_FILE;
        } else if matches!(node.content, NodeContent::File(_)) {
            // Empty file: huge_file only (blocksLow in fs-blocks)
            body.i_flags = inode_flag::HUGE_FILE;
        }

        // links count & size
        match &node.content {
            NodeContent::Directory => {
                body.i_links_count = node.dir_link_count;
                let blocks = node.end_block.get().saturating_sub(node.start_block.get());
                body.i_size_lo = blocks.saturating_mul(self.fs.block_size.bytes());
            }
            NodeContent::File(c) => {
                body.i_links_count = 1;
                body.i_size_lo = (c.len() as u64 & 0xFFFF_FFFF) as u32;
                body.i_size_hi = ((c.len() as u64) >> 32) as u32;
            }
            NodeContent::Symlink(t) => {
                body.i_links_count = 1;
                body.i_size_lo = t.len() as u32;
            }
            NodeContent::Whiteout => {
                body.i_links_count = 1;
            }
            NodeContent::Hardlink => {}
        }

        // Extent tree / fast symlink embedding.
        match &node.content {
            NodeContent::Directory if will_use_extents => {
                let blocks = node.end_block.get().saturating_sub(node.start_block.get());
                encode_inline_extent(&mut body.i_block, 0, blocks, node.start_block)?;
                body.i_blocks_lo = blocks; // huge_file => fs-blocks not 512B
            }
            NodeContent::File(c) => {
                if !c.is_empty() {
                    if node.file_extents.is_empty() {
                        return Err(Ext4Error::Internal {
                            block: BlockNumber(0),
                            what: "non-empty file missing serialized extents",
                        });
                    }
                    encode_file_extent_root(
                        &mut body.i_block,
                        &node.file_extents,
                        node.extent_leaf_block,
                        &self.fs.node_path(idx),
                    )?;
                    body.i_blocks_lo = node
                        .file_extents
                        .iter()
                        .map(|extent| extent.ee_len as u32)
                        .sum::<u32>();
                    if node.extent_leaf_block.is_some() {
                        body.i_blocks_lo += 1;
                    }
                }
            }
            NodeContent::Symlink(target) => {
                if target.len() < FAST_SYMLINK_MAX {
                    // Fast symlink: copy target bytes into i_block.
                    body.i_block[..target.len()].copy_from_slice(target.as_bytes());
                    body.i_blocks_lo = 0;
                } else {
                    encode_inline_extent(&mut body.i_block, 0, 1, node.start_block)?;
                    body.i_blocks_lo = 1;
                }
            }
            NodeContent::Whiteout => {
                body.i_blocks_lo = 0;
            }
            _ => {}
        }

        // xattrs.
        let mut inode = Inode256 {
            body,
            inline_xattrs: [0; INODE_INLINE_XATTR_SIZE as usize],
        };
        if !node.xattrs.is_empty() {
            if Self::xattrs_fit_inline(&node.xattrs) {
                write_xattr_inline(&node.xattrs, &mut inode.inline_xattrs)?;
            } else if let Some(xblock) = node.xattr_block {
                // file_acl points to the xattr block.
                inode.body.i_file_acl_lo = xblock.get();
                inode.body.i_blocks_lo += 1;
                // And also populate inline space if there's room for a
                // subset — for the MVP we only use the block, inline stays
                // zeroed.
            } else {
                return Err(Ext4Error::Internal {
                    block: BlockNumber(0),
                    what: "xattr overflow but no xattr block allocated",
                });
            }
        }
        Ok(inode)
    }

    fn xattrs_fit_inline(xs: &[Xattr]) -> bool {
        // Inline xattr region = 96 bytes. Layout = 4-byte magic + entries
        // (name + padding) + values (packed from end). Empty `system.data`
        // header adds 16 bytes.
        let mut need = 4u32; // magic
        for x in xs {
            let name_len = split_xattr_name(&x.name).1.len() as u32;
            need += 16 + ((name_len + 3) & !3) + ((x.value.len() as u32 + 3) & !3);
        }
        need += 4; // end-of-entries sentinel
        need <= INODE_INLINE_XATTR_SIZE
    }

    // ---- step 5: bitmaps, inode table on disk, GDT -------------------------

    fn write_bitmaps_and_gdt(&mut self) -> Result<()> {
        // Inode-table position comes first (allocates blocks after the
        // current `next_free_block`). Then the block bitmap and inode bitmap.
        let inode_table_blocks = (self.inodes_per_group * INODE_SIZE + self.fs.block_size.bytes()
            - 1)
            / self.fs.block_size.bytes();
        let inode_table_start = self.next_free_block;
        self.seek_to_block(inode_table_start)?;
        // Flatten the whole inode table into one byte slice so we don't
        // conflict-borrow `&self` while `write_bytes` wants `&mut self`.
        let itable_bytes: Vec<u8> = {
            let mut v = Vec::with_capacity(self.inode_table.len() * INODE_SIZE as usize);
            for rec in &self.inode_table {
                v.extend_from_slice(bytemuck::bytes_of(rec));
            }
            v
        };
        self.write_bytes(&itable_bytes)?;
        // Pad to block boundary if the inode table wasn't block-aligned.
        let itable_bytes_written = (self.inode_table.len() as u64) * (INODE_SIZE as u64);
        let itable_blocks_expected = inode_table_blocks as u64 * self.fs.block_size.bytes_u64();
        if itable_bytes_written < itable_blocks_expected {
            let pad = vec![0u8; (itable_blocks_expected - itable_bytes_written) as usize];
            self.write_bytes(&pad)?;
        }
        if self.block_groups > 1 {
            let empty_table = vec![0u8; itable_blocks_expected as usize];
            for _ in 1..self.block_groups {
                self.write_bytes(&empty_table)?;
            }
        }
        self.next_free_block =
            BlockNumber(inode_table_start.get() + inode_table_blocks * self.block_groups);

        // Block bitmap: one bitmap per group. With flex_bg these metadata
        // blocks may live outside the group they describe.
        let block_bitmap_start = self.next_free_block;
        self.next_free_block = BlockNumber(block_bitmap_start.get() + self.block_groups);
        let inode_bitmap_start = self.next_free_block;
        self.next_free_block = BlockNumber(inode_bitmap_start.get() + self.block_groups);

        // The on-disk "allocated blocks" universe for this group covers
        // from 0..blocks_per_group. We mark as allocated:
        //   - header (sb + gdt)
        //   - all data blocks we wrote
        //   - inode table blocks
        //   - block bitmap block, inode bitmap block
        let total_used_blocks = self.next_free_block.get();
        // Compute the total disk blocks we need. Pad to at least one full
        // block group so the superblock's blocks_per_group is honored.
        let needed_blocks = total_used_blocks.max(self.blocks_per_group * self.block_groups);
        let total_disk_blocks = needed_blocks;
        let total_image_bytes = (total_disk_blocks as u64) * self.fs.block_size.bytes_u64();
        let total_image_bytes = total_image_bytes.max(self.fs.min_size);
        // Round image bytes up to a block group boundary.
        let bg_bytes = self.blocks_per_group as u64 * self.fs.block_size.bytes_u64();
        let aligned = (total_image_bytes + bg_bytes - 1) / bg_bytes * bg_bytes;
        let total_disk_blocks = (aligned / self.fs.block_size.bytes_u64()) as u32;

        self.block_groups = (total_disk_blocks / self.blocks_per_group).max(1);

        // Block bitmap buffers.
        let bs = self.fs.block_size.bytes() as usize;
        let first_data_block = self.first_data_block();
        let mut block_bitmaps = Vec::with_capacity(self.block_groups as usize);
        let mut free_blocks_by_group = Vec::with_capacity(self.block_groups as usize);
        for group in 0..self.block_groups {
            let group_first = first_data_block + group * self.blocks_per_group;
            let valid_blocks = total_disk_blocks
                .saturating_sub(group_first)
                .min(self.blocks_per_group);
            let used_blocks = total_used_blocks
                .saturating_sub(group_first)
                .min(valid_blocks);
            let mut bitmap = vec![0u8; bs];
            for local in 0..self.blocks_per_group {
                let global = group_first + local;
                if global < total_used_blocks || global >= total_disk_blocks {
                    set_bit(&mut bitmap, local);
                }
            }
            block_bitmaps.push(bitmap);
            free_blocks_by_group.push(valid_blocks - used_blocks);
        }

        let mut inode_bitmaps = vec![vec![0u8; bs]; self.block_groups as usize];
        let mut used_inodes_by_group = vec![0u32; self.block_groups as usize];
        for i in 0..10 {
            set_bit(&mut inode_bitmaps[0], i);
        }
        used_inodes_by_group[0] = 10;
        for (idx, ino_opt) in self.inodes.iter().enumerate() {
            if self.fs.nodes[idx].deleted {
                continue;
            }
            if let Some(ino) = *ino_opt {
                let inode_index = ino.get() - 1;
                let group = inode_index / self.inodes_per_group;
                let local = inode_index % self.inodes_per_group;
                if let Some(bitmap) = inode_bitmaps.get_mut(group as usize) {
                    set_bit(bitmap, local);
                }
            }
        }
        for bitmap in &mut inode_bitmaps {
            for i in self.inodes_per_group..(bs as u32 * 8) {
                set_bit(bitmap, i);
            }
        }

        // Compute stats: directories count, used inodes, used blocks.
        // Hardlinks share inodes with their targets — count each inode at
        // most once.
        let mut dirs = 0u16;
        let mut seen = std::collections::BTreeSet::<u32>::new();
        let mut inode_used_count = 10u32; // reserved: inodes 1..=10
        for (idx, ino_opt) in self.inodes.iter().enumerate() {
            if self.fs.nodes[idx].deleted {
                continue;
            }
            let Some(ino) = ino_opt.map(|n| n.get()) else {
                continue;
            };
            if !seen.insert(ino) {
                continue; // hardlink — target already counted
            }
            if ino > 10 {
                inode_used_count += 1;
                let group = (ino - 1) / self.inodes_per_group;
                if let Some(count) = used_inodes_by_group.get_mut(group as usize) {
                    *count += 1;
                }
            }
            // `dirs` must NOT double-count either — sort of moot because
            // hardlinks to directories are disallowed.
            if self.fs.nodes[idx].mode.is_dir() {
                dirs += 1;
            }
        }

        // Write the bitmaps.
        self.seek_to_block(block_bitmap_start)?;
        for bitmap in &block_bitmaps {
            self.write_bytes(bitmap)?;
        }
        self.seek_to_block(inode_bitmap_start)?;
        for bitmap in &inode_bitmaps {
            self.write_bytes(bitmap)?;
        }

        let descriptors = (0..self.block_groups)
            .map(|group| GroupDescriptor {
                bg_block_bitmap_lo: block_bitmap_start.get() + group,
                bg_inode_bitmap_lo: inode_bitmap_start.get() + group,
                bg_inode_table_lo: inode_table_start.get() + group * inode_table_blocks,
                bg_free_blocks_count_lo: (free_blocks_by_group[group as usize] & 0xFFFF) as u16,
                bg_free_inodes_count_lo: ((self.inodes_per_group
                    - used_inodes_by_group[group as usize])
                    & 0xFFFF) as u16,
                bg_used_dirs_count_lo: if group == 0 { dirs } else { 0 },
                bg_flags: 0,
                bg_exclude_bitmap_lo: 0,
                bg_block_bitmap_csum_lo: 0,
                bg_inode_bitmap_csum_lo: 0,
                bg_itable_unused_lo: 0,
                bg_checksum: 0,
            })
            .collect::<Vec<_>>();

        // Seek to block 1 and write GDT entries.
        self.seek_to_block(BlockNumber(self.first_gdt_block()))?;
        let gd_bytes: &[u8] = bytemuck::cast_slice(&descriptors);
        self.write_bytes(gd_bytes)?;
        // Zero-pad the rest of the GDT block.
        let consumed = gd_bytes.len() as u32;
        let gdt_block_bytes =
            self.header_blocks.saturating_sub(self.first_gdt_block()) * self.fs.block_size.bytes();
        if consumed < gdt_block_bytes {
            let pad = vec![0u8; (gdt_block_bytes - consumed) as usize];
            self.write_bytes(&pad)?;
        }

        // Expand the file to total_image_bytes if needed (sparse is fine on
        // filesystems that support it; write a single 0 byte at the end).
        let out_len = (total_disk_blocks as u64) * self.fs.block_size.bytes_u64();
        self.ensure_len(out_len)?;

        // Stash totals for the superblock.
        self.total_disk_blocks_cache = total_disk_blocks;
        self.total_used_blocks_cache = total_used_blocks;
        self.inode_used_count_cache = inode_used_count;
        Ok(())
    }

    // ---- step 6: superblock ----------------------------------------------

    fn write_superblock(&mut self) -> Result<()> {
        let mut sb: Superblock = Zeroable::zeroed();

        let total_inodes = self.inodes_per_group * self.block_groups;
        sb.s_inodes_count = total_inodes;
        sb.s_blocks_count_lo = self.total_disk_blocks_cache;
        sb.s_r_blocks_count_lo = 0;
        sb.s_free_blocks_count_lo = self
            .total_disk_blocks_cache
            .saturating_sub(self.total_used_blocks_cache);
        sb.s_free_inodes_count = total_inodes - self.inode_used_count_cache;
        sb.s_first_data_block = self.first_data_block();
        sb.s_log_block_size = self.fs.block_size.log();
        sb.s_log_cluster_size = self.fs.block_size.log();
        sb.s_blocks_per_group = self.blocks_per_group;
        sb.s_clusters_per_group = self.blocks_per_group;
        sb.s_inodes_per_group = self.inodes_per_group;
        sb.s_mtime = self.fs.fs_timestamp;
        sb.s_wtime = self.fs.fs_timestamp;
        sb.s_mnt_count = 0;
        sb.s_max_mnt_count = 0xFFFF;
        sb.s_magic = SUPERBLOCK_MAGIC;
        sb.s_state = 1; // cleanly unmounted
        sb.s_errors = 1; // continue on error
        sb.s_minor_rev_level = 0;
        sb.s_lastcheck = self.fs.fs_timestamp;
        sb.s_checkinterval = 0;
        sb.s_creator_os = 0; // 0=Linux (the kernel's EXT4_OS_LINUX)
        sb.s_rev_level = 1; // dynamic inode sizes
        sb.s_first_ino = FIRST_FREE_INODE; // 11; lost+found lives here
        sb.s_inode_size = INODE_SIZE as u16;
        sb.s_feature_compat = feature_compat::EXT_ATTR | feature_compat::SPARSE_SUPER2;
        sb.s_feature_incompat =
            feature_incompat::FILETYPE | feature_incompat::EXTENTS | feature_incompat::FLEX_BG;
        sb.s_feature_ro_compat = feature_ro_compat::LARGE_FILE
            | feature_ro_compat::HUGE_FILE
            | feature_ro_compat::EXTRA_ISIZE;
        sb.s_uuid = self.fs.uuid;
        sb.s_min_extra_isize = INODE_EXTRA_ISIZE;
        sb.s_want_extra_isize = INODE_EXTRA_ISIZE;
        sb.s_desc_size = GROUP_DESC_SIZE as u16;
        sb.s_log_groups_per_flex = 31; // large enough to make FLEX_BG vacuous
        sb.s_mkfs_time = self.fs.fs_timestamp;
        sb.s_lpf_ino = LOST_FOUND_INODE;
        // Everything else stays zero.

        // Seek to SUPERBLOCK_OFFSET (= 1024) and write the superblock.
        self.seek_absolute(SUPERBLOCK_OFFSET)?;
        let bytes: &[u8] = bytemuck::bytes_of(&sb);
        self.write_bytes(bytes)?;
        Ok(())
    }

    // ---- primitive I/O ----------------------------------------------------

    fn seek_to_block(&mut self, b: BlockNumber) -> Result<()> {
        self.seek_absolute(b.offset_bytes(self.fs.block_size))
    }

    fn first_data_block(&self) -> u32 {
        if self.fs.block_size.bytes() == 1024 {
            1
        } else {
            0
        }
    }

    fn first_gdt_block(&self) -> u32 {
        first_gdt_block_for_size(self.fs.block_size)
    }

    fn seek_absolute(&mut self, off: u64) -> Result<()> {
        self.out
            .seek(SeekFrom::Start(off))
            .map_err(|e| Ext4Error::Control {
                what: "seek",
                source: e,
            })?;
        self.pos = off;
        Ok(())
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.out.write_all(bytes).map_err(|e| Ext4Error::Write {
            offset: self.pos,
            bytes: bytes.len(),
            source: e,
        })?;
        self.pos += bytes.len() as u64;
        Ok(())
    }

    fn ensure_len(&mut self, len: u64) -> Result<()> {
        // Grow the file to at least `len`. We do this by seeking to len-1
        // and writing a zero byte, which works for both regular files and
        // for the VFS's sparse-file support. If the caller gives us a
        // `Cursor<Vec<u8>>`, this also extends the Vec.
        if len == 0 {
            return Ok(());
        }
        self.seek_absolute(len - 1)?;
        self.write_bytes(&[0])?;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.out.flush().map_err(|e| Ext4Error::Control {
            what: "flush",
            source: e,
        })
    }
}

// ---- free helpers ---------------------------------------------------------

struct DirEntrySpec {
    inode: InodeNumber,
    name: String,
    file_type: u8,
}

fn set_bit(buf: &mut [u8], bit: u32) {
    let byte = (bit / 8) as usize;
    let mask = 1u8 << (bit % 8);
    if byte < buf.len() {
        buf[byte] |= mask;
    }
}

fn div_ceil_u64(n: u64, d: u64) -> u64 {
    if n == 0 { 0 } else { 1 + ((n - 1) / d) }
}

fn div_ceil_u32(n: u32, d: u32) -> u32 {
    if n == 0 { 0 } else { 1 + ((n - 1) / d) }
}

fn div_ceil_usize(n: usize, d: usize) -> u32 {
    if n == 0 {
        0
    } else {
        u32::try_from(1 + ((n - 1) / d)).unwrap_or(u32::MAX)
    }
}

fn first_gdt_block_for_size(block_size: BlockSize) -> u32 {
    if block_size.bytes() == 1024 { 2 } else { 1 }
}

/// Write a single linear directory entry into `buf[off..]`. Returns the
/// new offset. The record length normally rounds the entry up to 4-byte
/// alignment, but if `last_in_block` is true, we stretch the rec_len to
/// reach the end of the block (ext4 convention).
fn write_dir_entry(
    buf: &mut [u8],
    off: usize,
    block_size: usize,
    inode: InodeNumber,
    name: &str,
    file_type: u8,
    last_in_block: bool,
) -> Result<usize> {
    let name_bytes = name.as_bytes();
    let header_size = std::mem::size_of::<DirEntryHeader>();
    let aligned = dir_entry_min_len(name);
    let rec_len = if last_in_block {
        block_size - off
    } else {
        aligned
    };
    if rec_len > u16::MAX as usize || off + rec_len > block_size {
        return Err(Ext4Error::DirEntryOverflow {
            name: name.to_string(),
            rec_len: rec_len.min(u16::MAX as usize) as u16,
        });
    }
    let hdr = DirEntryHeader {
        inode: inode.get(),
        rec_len: rec_len as u16,
        name_len: name_bytes.len() as u8,
        file_type,
    };
    buf[off..off + header_size].copy_from_slice(bytemuck::bytes_of(&hdr));
    buf[off + header_size..off + header_size + name_bytes.len()].copy_from_slice(name_bytes);
    // Trailing zero-padding (already zero).
    Ok(off + rec_len)
}

fn dir_entry_min_len(name: &str) -> usize {
    let header_size = std::mem::size_of::<DirEntryHeader>();
    let minimal = header_size + name.len();
    (minimal + 3) & !3
}

/// Write a trailing empty directory entry that consumes the rest of the
/// block. Used when the children iterator finishes before filling the
/// block and we didn't set `last_in_block=true` on the final child.
fn write_dir_entry_tail(buf: &mut [u8], off: usize, block_size: usize) -> Result<()> {
    if off >= block_size {
        return Ok(());
    }
    let header_size = std::mem::size_of::<DirEntryHeader>();
    let rec_len = block_size - off;
    if rec_len < header_size || rec_len > u16::MAX as usize {
        return Err(Ext4Error::DirEntryOverflow {
            name: String::new(),
            rec_len: rec_len as u16,
        });
    }
    let hdr = DirEntryHeader {
        inode: 0,
        rec_len: rec_len as u16,
        name_len: 0,
        file_type: 0,
    };
    buf[off..off + header_size].copy_from_slice(bytemuck::bytes_of(&hdr));
    Ok(())
}

fn split_contiguous_extent(
    logical_start: u32,
    blocks: u32,
    disk_start: BlockNumber,
    max_blocks_per_extent: u32,
) -> Vec<ExtentLeaf> {
    let mut extents = Vec::new();
    let mut remaining = blocks;
    let mut disk = disk_start.get();
    let mut logical = logical_start;
    let max_len = max_blocks_per_extent.min(MAX_BLOCKS_PER_EXTENT);
    while remaining > 0 {
        let len = remaining.min(max_len) as u16;
        extents.push(ExtentLeaf {
            ee_block: logical,
            ee_len: len,
            ee_start_hi: 0,
            ee_start_lo: disk,
        });
        disk += len as u32;
        logical += len as u32;
        remaining -= len as u32;
    }
    extents
}

fn extent_leaf_capacity(block_size: BlockSize) -> usize {
    ((block_size.bytes() as usize) - std::mem::size_of::<ExtentHeader>())
        / std::mem::size_of::<ExtentLeaf>()
}

fn write_extent_leaf_block(
    extents: &[ExtentLeaf],
    block_size: BlockSize,
    buf: &mut [u8],
    path: &str,
) -> Result<()> {
    let capacity = extent_leaf_capacity(block_size);
    if extents.len() > capacity {
        return Err(Ext4Error::ExtentDepthExceeded {
            path: path.to_string(),
            max_depth: 1,
        });
    }
    let hdr = ExtentHeader {
        eh_magic: EXTENT_HEADER_MAGIC,
        eh_entries: extents.len() as u16,
        eh_max: capacity as u16,
        eh_depth: 0,
        eh_generation: 0,
    };
    buf[..12].copy_from_slice(bytemuck::bytes_of(&hdr));
    let mut off = 12usize;
    for leaf in extents {
        buf[off..off + 12].copy_from_slice(bytemuck::bytes_of(leaf));
        off += 12;
    }
    Ok(())
}

fn encode_file_extent_root(
    i_block: &mut [u8; 60],
    extents: &[ExtentLeaf],
    extent_leaf_block: Option<BlockNumber>,
    path: &str,
) -> Result<()> {
    if extents.len() <= MAX_EXTENTS_PER_INODE as usize {
        let hdr = ExtentHeader {
            eh_magic: EXTENT_HEADER_MAGIC,
            eh_entries: extents.len() as u16,
            eh_max: MAX_EXTENTS_PER_INODE as u16,
            eh_depth: 0,
            eh_generation: 0,
        };
        let mut off = 0usize;
        i_block[off..off + 12].copy_from_slice(bytemuck::bytes_of(&hdr));
        off += 12;
        for leaf in extents {
            i_block[off..off + 12].copy_from_slice(bytemuck::bytes_of(leaf));
            off += 12;
        }
        return Ok(());
    }

    let Some(leaf_block) = extent_leaf_block else {
        return Err(Ext4Error::Internal {
            block: BlockNumber(0),
            what: "extent tree overflow but no leaf block was allocated",
        });
    };
    let hdr = ExtentHeader {
        eh_magic: EXTENT_HEADER_MAGIC,
        eh_entries: 1,
        eh_max: MAX_EXTENTS_PER_INODE as u16,
        eh_depth: 1,
        eh_generation: 0,
    };
    let first = extents.first().ok_or(Ext4Error::ExtentDepthExceeded {
        path: path.to_string(),
        max_depth: 1,
    })?;
    let index = ExtentIndex {
        ei_block: first.ee_block,
        ei_leaf_lo: leaf_block.get(),
        ei_leaf_hi: 0,
        ei_unused: 0,
    };
    i_block[..12].copy_from_slice(bytemuck::bytes_of(&hdr));
    i_block[12..24].copy_from_slice(bytemuck::bytes_of(&index));
    Ok(())
}

/// Encode a contiguous run into the inode's inline extent area. Used for
/// directories and slow symlinks, which never need an external leaf block.
fn encode_inline_extent(
    i_block: &mut [u8; 60],
    logical_start: u32,
    blocks: u32,
    disk_start: BlockNumber,
) -> Result<()> {
    let extents = split_contiguous_extent(logical_start, blocks, disk_start, MAX_BLOCKS_PER_EXTENT);
    if extents.len() > MAX_EXTENTS_PER_INODE as usize {
        return Err(Ext4Error::ExtentDepthExceeded {
            path: String::new(),
            max_depth: 0,
        });
    }
    encode_file_extent_root(i_block, &extents, None, "")
}

// ---- xattr encoding -------------------------------------------------------

/// Split a full xattr name into (name_index, suffix). ext4 encodes
/// well-known prefixes as small integers to save space:
///   "user."         → 1
///   "system.posix_acl_access" → 2
///   "system.posix_acl_default" → 3
///   "trusted."      → 4
///   "security."     → 6
///   anything else   → 0 (full name stored verbatim)
fn split_xattr_name(full: &str) -> (u8, &str) {
    if let Some(s) = full.strip_prefix("user.") {
        (1, s)
    } else if full == "system.posix_acl_access" {
        (2, "")
    } else if full == "system.posix_acl_default" {
        (3, "")
    } else if let Some(s) = full.strip_prefix("trusted.") {
        (4, s)
    } else if let Some(s) = full.strip_prefix("security.") {
        (6, s)
    } else {
        (0, full)
    }
}

/// Write xattrs into the 96-byte inline region of an inode. Caller has
/// already verified via `xattrs_fit_inline` that there's room.
///
/// Layout:
///   - 4 bytes magic (XATTR_IBODY_MAGIC)
///   - series of XattrEntry records (16 bytes + padded name), each record
///     4-byte aligned
///   - 4 bytes zero sentinel (end-of-entries)
///   - value bytes, packed from the end of the region, each 4-byte aligned
fn write_xattr_inline(
    xs: &[Xattr],
    buf: &mut [u8; INODE_INLINE_XATTR_SIZE as usize],
) -> Result<()> {
    // Inline xattr layout (from fs/ext4/xattr.h + lib/ext2fs/ext_attr.c):
    //
    //   [0..4)   : magic (XATTR_IBODY_MAGIC)
    //   [4..)    : entries, growing forward
    //   [..end)  : values, growing backward from end
    //
    // IMPORTANT: `e_value_offs` is relative to `start = buf + 4` (the
    // entries pointer), not to `buf` itself. e2fsprogs validates
    //   e_value_offs + e_value_size ≤ storage_size
    // where storage_size = buf.len() - 4 (the region past the magic).
    let magic = XattrIbodyHeader {
        h_magic: XATTR_IBODY_MAGIC,
    };
    buf[..4].copy_from_slice(bytemuck::bytes_of(&magic));

    let region_len = buf.len();
    let entries_base = 4usize;
    // Values live in [entries_base + entries_end, region_len), but their
    // offsets are relative to `buf + entries_base`.
    let mut value_offs_next = region_len - entries_base; // == storage_size
    let mut entries_off = entries_base;
    for x in xs {
        let (idx, suffix) = split_xattr_name(&x.name);
        let name_len = suffix.len() as u8;
        let name_pad = ((name_len as usize + 3) & !3).saturating_sub(name_len as usize);
        let entry_size = 16 + name_len as usize + name_pad;
        let value_pad = ((x.value.len() + 3) & !3).saturating_sub(x.value.len());
        let value_total = x.value.len() + value_pad;
        value_offs_next = value_offs_next
            .checked_sub(value_total)
            .ok_or(Ext4Error::Internal {
                block: BlockNumber(0),
                what: "inline xattr value region overflow",
            })?;
        let e = XattrEntry {
            e_name_len: name_len,
            e_name_index: idx,
            e_value_offs: value_offs_next as u16,
            e_value_inum: 0,
            e_value_size: x.value.len() as u32,
            e_hash: 0,
        };
        buf[entries_off..entries_off + 16].copy_from_slice(bytemuck::bytes_of(&e));
        buf[entries_off + 16..entries_off + 16 + name_len as usize]
            .copy_from_slice(suffix.as_bytes());
        entries_off += entry_size;
        // Write value at (entries_base + value_offs_next).
        let value_abs = entries_base + value_offs_next;
        buf[value_abs..value_abs + x.value.len()].copy_from_slice(&x.value);
    }
    // End-of-entries sentinel (4 zero bytes after the last entry) is
    // already zero from the caller's zero-init.
    Ok(())
}

/// Write xattrs into a full xattr block. Layout mirrors `ext4_xattr_header`:
/// 32-byte header + entries + zero padding + values packed from the end.
///
/// For block xattrs, `e_value_offs` is relative to the start of the *block*
/// (not to `entries`), because `value_start = block_buf` in
/// e2fsprogs's parser.
fn write_xattr_block(xs: &[Xattr], buf: &mut [u8]) -> Result<()> {
    use crate::layout::XATTR_MAGIC;
    // 32-byte block header:
    //   u32 h_magic; u32 h_refcount; u32 h_blocks; u32 h_hash; u32 h_checksum;
    //   u32 h_reserved[3];
    buf[0..4].copy_from_slice(&XATTR_MAGIC.to_le_bytes());
    buf[4..8].copy_from_slice(&1u32.to_le_bytes()); // refcount
    buf[8..12].copy_from_slice(&1u32.to_le_bytes()); // blocks
    // Values pack from end; offsets are absolute-within-block (= buf).
    let mut value_cursor_abs = buf.len();
    let mut entries_off = 32usize;
    for x in xs {
        let (idx, suffix) = split_xattr_name(&x.name);
        let name_len = suffix.len() as u8;
        let name_pad = ((name_len as usize + 3) & !3).saturating_sub(name_len as usize);
        let entry_size = 16 + name_len as usize + name_pad;
        let value_pad = ((x.value.len() + 3) & !3).saturating_sub(x.value.len());
        let value_total = x.value.len() + value_pad;
        value_cursor_abs =
            value_cursor_abs
                .checked_sub(value_total)
                .ok_or(Ext4Error::Internal {
                    block: BlockNumber(0),
                    what: "block xattr value region overflow",
                })?;
        let e = XattrEntry {
            e_name_len: name_len,
            e_name_index: idx,
            // For block xattrs, e_value_offs is absolute within the block.
            e_value_offs: value_cursor_abs as u16,
            e_value_inum: 0,
            e_value_size: x.value.len() as u32,
            e_hash: 0,
        };
        buf[entries_off..entries_off + 16].copy_from_slice(bytemuck::bytes_of(&e));
        buf[entries_off + 16..entries_off + 16 + name_len as usize]
            .copy_from_slice(suffix.as_bytes());
        entries_off += entry_size;
        buf[value_cursor_abs..value_cursor_abs + x.value.len()].copy_from_slice(&x.value);
    }
    Ok(())
}
