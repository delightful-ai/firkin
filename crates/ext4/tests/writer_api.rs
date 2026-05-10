#![allow(missing_docs)]

use std::io::{Read, Seek, SeekFrom};

use firkin_ext4::layout::{
    DirEntryHeader, EXTENT_HEADER_MAGIC, ExtentHeader, ExtentLeaf, INODE_SIZE, ROOT_INODE,
    SUPERBLOCK_OFFSET, Superblock,
};
use firkin_ext4::{BlockSize, Features, LayerCompression, Writer, init_block};
use firkin_types::Size;

#[test]
fn writer_builds_an_in_memory_image_from_file_content() {
    let image = Writer::in_memory(Size::mib(8))
        .unwrap()
        .features(Features::spike_set())
        .block_size(BlockSize::Size4K)
        .write_file("/hello", b"hi\n", 0o644)
        .unwrap()
        .into_bytes()
        .unwrap();

    assert!(image.len() >= usize::try_from(Size::mib(8).as_bytes()).unwrap());
    let magic_offset = 1024 + 0x38;
    assert_eq!(&image[magic_offset..magic_offset + 2], &[0x53, 0xef]);
}

#[test]
fn writer_adds_a_host_directory_tree_under_guest_path() {
    let source = tempfile::tempdir().unwrap();
    std::fs::create_dir(source.path().join("bin")).unwrap();
    std::fs::write(source.path().join("bin/hello"), b"hello\n").unwrap();

    let image = Writer::in_memory(Size::mib(8))
        .unwrap()
        .write_directory("/app", source.path())
        .unwrap()
        .into_bytes()
        .unwrap();

    let magic_offset = 1024 + 0x38;
    assert_eq!(&image[magic_offset..magic_offset + 2], &[0x53, 0xef]);
}

#[test]
fn writer_extracts_an_uncompressed_oci_layer_tar() {
    let layer = tempfile::NamedTempFile::new().unwrap();
    {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(layer.path())
            .unwrap();
        let mut tar = tar::Builder::new(file);

        let mut dir = tar::Header::new_gnu();
        dir.set_entry_type(tar::EntryType::Directory);
        dir.set_path("bin").unwrap();
        dir.set_mode(0o755);
        dir.set_size(0);
        dir.set_cksum();
        tar.append(&dir, std::io::empty()).unwrap();

        let payload = b"hello from layer\n";
        let mut file_header = tar::Header::new_gnu();
        file_header.set_entry_type(tar::EntryType::Regular);
        file_header.set_path("bin/hello").unwrap();
        file_header.set_mode(0o755);
        file_header.set_size(u64::try_from(payload.len()).unwrap());
        file_header.set_cksum();
        tar.append(&file_header, &payload[..]).unwrap();

        let mut whiteout = tar::Header::new_gnu();
        whiteout.set_entry_type(tar::EntryType::Regular);
        whiteout.set_path("bin/.wh.old").unwrap();
        whiteout.set_mode(0o000);
        whiteout.set_size(0);
        whiteout.set_cksum();
        tar.append(&whiteout, std::io::empty()).unwrap();

        tar.finish().unwrap();
    }

    let image = Writer::in_memory(Size::mib(8))
        .unwrap()
        .write_layers_raw([(layer.path(), LayerCompression::None)])
        .unwrap()
        .into_bytes()
        .unwrap();

    let magic_offset = 1024 + 0x38;
    assert_eq!(&image[magic_offset..magic_offset + 2], &[0x53, 0xef]);
}

#[test]
fn writer_extracts_gzip_and_zstd_oci_layer_tars() {
    let tar_bytes = layer_tar_bytes();

    let gzip_layer = tempfile::NamedTempFile::new().unwrap();
    {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(gzip_layer.path())
            .unwrap();
        let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &tar_bytes).unwrap();
        encoder.finish().unwrap();
    }

    let zstd_layer = tempfile::NamedTempFile::new().unwrap();
    {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(zstd_layer.path())
            .unwrap();
        let mut encoder = zstd::stream::Encoder::new(file, 0).unwrap();
        std::io::Write::write_all(&mut encoder, &tar_bytes).unwrap();
        encoder.finish().unwrap();
    }

    let image = Writer::in_memory(Size::mib(8))
        .unwrap()
        .write_layers_raw([
            (gzip_layer.path(), LayerCompression::Gzip),
            (zstd_layer.path(), LayerCompression::Zstd),
        ])
        .unwrap()
        .into_bytes()
        .unwrap();

    let magic_offset = 1024 + 0x38;
    assert_eq!(&image[magic_offset..magic_offset + 2], &[0x53, 0xef]);
}

#[test]
fn writer_applies_oci_whiteouts_by_deleting_lower_entries() {
    let base_layer = tempfile::NamedTempFile::new().unwrap();
    write_layer(
        base_layer.path(),
        [
            LayerEntry::dir("app", 0o755),
            LayerEntry::file("app/keep", b"keep\n", 0o644),
            LayerEntry::file("app/gone", b"gone\n", 0o644),
        ],
    );

    let whiteout_layer = tempfile::NamedTempFile::new().unwrap();
    write_layer(
        whiteout_layer.path(),
        [LayerEntry::file("app/.wh.gone", b"", 0o000)],
    );

    let image = Writer::in_memory(Size::mib(8))
        .unwrap()
        .write_layers_raw([
            (base_layer.path(), LayerCompression::None),
            (whiteout_layer.path(), LayerCompression::None),
        ])
        .unwrap()
        .into_bytes()
        .unwrap();

    let fs = Ext4View::new(&image);
    assert!(fs.path_exists("/app/keep"));
    assert!(!fs.path_exists("/app/gone"));
    assert!(!fs.path_exists("/app/.wh.gone"));
}

#[test]
fn writer_applies_oci_opaque_directory_markers_by_clearing_children() {
    let base_layer = tempfile::NamedTempFile::new().unwrap();
    write_layer(
        base_layer.path(),
        [
            LayerEntry::dir("cache", 0o755),
            LayerEntry::file("cache/lower", b"lower\n", 0o644),
        ],
    );

    let opaque_layer = tempfile::NamedTempFile::new().unwrap();
    write_layer(
        opaque_layer.path(),
        [
            LayerEntry::dir("cache", 0o755),
            LayerEntry::file("cache/.wh..wh..opq", b"", 0o000),
            LayerEntry::file("cache/upper", b"upper\n", 0o644),
        ],
    );

    let image = Writer::in_memory(Size::mib(8))
        .unwrap()
        .write_layers_raw([
            (base_layer.path(), LayerCompression::None),
            (opaque_layer.path(), LayerCompression::None),
        ])
        .unwrap()
        .into_bytes()
        .unwrap();

    let fs = Ext4View::new(&image);
    assert!(!fs.path_exists("/cache/lower"));
    assert!(fs.path_exists("/cache/upper"));
    assert!(!fs.path_exists("/cache/.wh..wh..opq"));
}

#[test]
fn init_block_synthesize_to_writes_vminitd_bootstrap_tree() {
    let dest = tempfile::NamedTempFile::new().unwrap();
    init_block::synthesize_to(b"tiny static vminitd", b"tiny static vmexec", dest.path()).unwrap();

    let image = std::fs::read(dest.path()).unwrap();
    let fs = Ext4View::new(&image);

    assert_eq!(fs.read_file("/sbin/vminitd"), b"tiny static vminitd");
    assert_eq!(fs.read_file("/sbin/vmexec"), b"tiny static vmexec");
    assert_eq!(
        fs.read_file("/etc/passwd"),
        b"root:x:0:0:root:/root:/bin/sh\n"
    );
    assert_eq!(fs.read_file("/etc/hosts"), b"127.0.0.1 localhost\n");
    assert_eq!(fs.read_file("/etc/resolv.conf"), b"");
    assert_eq!(fs.read_symlink("/proc/self/exe"), "sbin/vminitd");
    assert_eq!(fs.read_symlink("/sbin/init"), "/sbin/vminitd");
    assert!(fs.path_exists("/bin"));
    assert!(fs.path_exists("/proc"));
    assert!(fs.path_exists("/proc/self"));
    assert!(fs.path_exists("/sys"));
    assert!(fs.path_exists("/run"));
    assert!(fs.path_exists("/dev"));
    assert!(fs.path_exists("/mnt"));
    assert!(fs.path_exists("/var"));
    assert!(fs.path_exists("/tmp"));
}

#[test]
fn init_block_cache_path_is_sha256_keyed_under_the_firkin_cache() {
    let path = init_block::cache_path(b"same vminitd input", b"same vmexec input");

    assert_eq!(path.parent().unwrap().file_name().unwrap(), "init-blocks");
    assert_eq!(path.extension().unwrap(), "ext4");
}

#[test]
fn writer_serializes_images_that_need_multiple_block_groups() {
    let payload = vec![0u8; usize::try_from(Size::mib(9).as_bytes()).unwrap()];
    let image = Writer::in_memory(Size::mib(12))
        .unwrap()
        .block_size(BlockSize::Size1K)
        .write_file("/big", &payload, 0o644)
        .unwrap()
        .into_bytes()
        .unwrap();

    let fs = Ext4View::new(&image);
    assert!(fs.path_exists("/big"));
    assert!(image.len() > usize::try_from(Size::mib(8).as_bytes()).unwrap());
}

fn layer_tar_bytes() -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());

    let mut dir = tar::Header::new_gnu();
    dir.set_entry_type(tar::EntryType::Directory);
    dir.set_path("opt").unwrap();
    dir.set_mode(0o755);
    dir.set_size(0);
    dir.set_cksum();
    tar.append(&dir, std::io::empty()).unwrap();

    let payload = b"compressed layer\n";
    let mut file_header = tar::Header::new_gnu();
    file_header.set_entry_type(tar::EntryType::Regular);
    file_header.set_path("opt/payload").unwrap();
    file_header.set_mode(0o644);
    file_header.set_size(u64::try_from(payload.len()).unwrap());
    file_header.set_cksum();
    tar.append(&file_header, &payload[..]).unwrap();

    tar.finish().unwrap();
    tar.into_inner().unwrap()
}

#[derive(Clone, Copy)]
enum LayerEntry<'a> {
    Dir {
        path: &'a str,
        mode: u32,
    },
    File {
        path: &'a str,
        content: &'a [u8],
        mode: u32,
    },
}

impl<'a> LayerEntry<'a> {
    fn dir(path: &'a str, mode: u32) -> Self {
        Self::Dir { path, mode }
    }

    fn file(path: &'a str, content: &'a [u8], mode: u32) -> Self {
        Self::File {
            path,
            content,
            mode,
        }
    }
}

fn write_layer<'a>(path: &std::path::Path, entries: impl IntoIterator<Item = LayerEntry<'a>>) {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .unwrap();
    let mut tar = tar::Builder::new(file);

    for entry in entries {
        match entry {
            LayerEntry::Dir { path, mode } => {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Directory);
                header.set_path(path).unwrap();
                header.set_mode(mode);
                header.set_size(0);
                header.set_cksum();
                tar.append(&header, std::io::empty()).unwrap();
            }
            LayerEntry::File {
                path,
                content,
                mode,
            } => {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Regular);
                header.set_path(path).unwrap();
                header.set_mode(mode);
                header.set_size(u64::try_from(content.len()).unwrap());
                header.set_cksum();
                tar.append(&header, content).unwrap();
            }
        }
    }

    tar.finish().unwrap();
}

struct Ext4View<'a> {
    image: &'a [u8],
    block_size: usize,
    inode_table_offset: usize,
}

impl<'a> Ext4View<'a> {
    fn new(image: &'a [u8]) -> Self {
        let mut cursor = std::io::Cursor::new(image);
        cursor.seek(SeekFrom::Start(SUPERBLOCK_OFFSET)).unwrap();
        let mut sb_bytes = [0u8; 1024];
        cursor.read_exact(&mut sb_bytes).unwrap();
        let sb: Superblock = bytemuck::pod_read_unaligned(&sb_bytes);
        let block_size = usize::try_from(1024u32 << sb.s_log_block_size).unwrap();

        let gdt_offset = if block_size == 1024 {
            block_size * 2
        } else {
            block_size
        };
        let inode_table_block =
            u32::from_le_bytes(image[gdt_offset + 8..gdt_offset + 12].try_into().unwrap());
        let inode_table_offset = usize::try_from(inode_table_block).unwrap() * block_size;
        Self {
            image,
            block_size,
            inode_table_offset,
        }
    }

    fn path_exists(&self, path: &str) -> bool {
        self.lookup_inode(path).is_some()
    }

    fn read_file(&self, path: &str) -> Vec<u8> {
        let inode = self.lookup_inode(path).unwrap();
        let inode_bytes = self.inode_bytes(inode);
        let size = inode_size(inode_bytes);
        if size == 0 {
            return Vec::new();
        }
        let data_block = first_extent_start(inode_bytes);
        let block_offset = usize::try_from(data_block).unwrap() * self.block_size;
        self.image[block_offset..block_offset + usize::try_from(size).unwrap()].to_vec()
    }

    fn read_symlink(&self, path: &str) -> String {
        let inode = self.lookup_inode(path).unwrap();
        let inode_bytes = self.inode_bytes(inode);
        let size = usize::try_from(inode_size(inode_bytes)).unwrap();
        let target = &inode_bytes[40..40 + size];
        std::str::from_utf8(target).unwrap().to_owned()
    }

    fn lookup_inode(&self, path: &str) -> Option<u32> {
        let mut inode = ROOT_INODE;
        for component in path.split('/').filter(|component| !component.is_empty()) {
            inode = self.lookup_child(inode, component)?;
        }
        Some(inode)
    }

    fn lookup_child(&self, parent_inode: u32, name: &str) -> Option<u32> {
        self.directory_entries(parent_inode)
            .into_iter()
            .find_map(|entry| (entry.name == name).then_some(entry.inode))
    }

    fn directory_entries(&self, inode: u32) -> Vec<DirEntry> {
        let inode_bytes = self.inode_bytes(inode);
        let data_block = first_extent_start(inode_bytes);
        let block_offset = usize::try_from(data_block).unwrap() * self.block_size;
        let mut offset = block_offset;
        let end = block_offset + self.block_size;
        let mut entries = Vec::new();

        while offset + std::mem::size_of::<DirEntryHeader>() <= end {
            let header: DirEntryHeader = bytemuck::pod_read_unaligned(
                &self.image[offset..offset + std::mem::size_of::<DirEntryHeader>()],
            );
            if header.rec_len == 0 {
                break;
            }
            let rec_len = usize::from(header.rec_len);
            if header.inode != 0 && header.name_len > 0 {
                let name_start = offset + std::mem::size_of::<DirEntryHeader>();
                let name_end = name_start + usize::from(header.name_len);
                let name = std::str::from_utf8(&self.image[name_start..name_end])
                    .unwrap()
                    .to_owned();
                if name != "." && name != ".." {
                    entries.push(DirEntry {
                        name,
                        inode: header.inode,
                    });
                }
            }
            offset += rec_len;
        }

        entries
    }

    fn inode_bytes(&self, inode: u32) -> &[u8] {
        let offset =
            self.inode_table_offset + usize::try_from(inode - 1).unwrap() * INODE_SIZE as usize;
        &self.image[offset..offset + INODE_SIZE as usize]
    }
}

struct DirEntry {
    name: String,
    inode: u32,
}

fn first_extent_start(inode: &[u8]) -> u32 {
    const I_BLOCK_OFFSET: usize = 40;
    let i_block = &inode[I_BLOCK_OFFSET..I_BLOCK_OFFSET + 60];
    let header: ExtentHeader = bytemuck::pod_read_unaligned(&i_block[..12]);
    assert_eq!(header.eh_magic, EXTENT_HEADER_MAGIC);
    assert_eq!(header.eh_depth, 0);
    assert!(header.eh_entries >= 1);
    let leaf: ExtentLeaf = bytemuck::pod_read_unaligned(&i_block[12..24]);
    leaf.ee_start_lo
}

fn inode_size(inode: &[u8]) -> u64 {
    let lo = u32::from_le_bytes(inode[4..8].try_into().unwrap());
    let hi = u32::from_le_bytes(inode[108..112].try_into().unwrap());
    u64::from(lo) | (u64::from(hi) << 32)
}
