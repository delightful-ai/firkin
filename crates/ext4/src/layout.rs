//! On-disk byte layouts. Everything here is `#[repr(C)]` + `bytemuck::Pod`
//! so we can cast a struct to `&[u8]` for serialization without a custom
//! encoder. ext4 is little-endian on the wire; on LE hosts (x86_64 / arm64,
//! which are the only platforms we target) the cast is zero-cost.
//!
//! **Field order and sizes are load-bearing.** Do not reorder. Do not
//! change `u16` to `u32`. If you need a new field, append it at the end.
//!
//! Reference: Linux `fs/ext4/ext4.h` and the ext4 wiki:
//!   https://www.kernel.org/doc/html/latest/filesystems/ext4/
//!
//! What we implement is a subset:
//!   - classic 32-bit (no 64bit feature) — block count fits in 32 bits
//!   - extents on, dir_index off, metadata_csum off, no journal
//!   - filetype + flex_bg + sparse_super2 + ext_attr + large_file +
//!     huge_file + extra_isize
//!
//! This exactly mirrors the Swift reference in EXT4+Formatter.swift; the
//! mkfs.ext4 equivalent options are documented in tests/fixtures/README.

use bytemuck::{Pod, Zeroable};

pub const SUPERBLOCK_OFFSET: u64 = 1024;
pub const SUPERBLOCK_MAGIC: u16 = 0xEF53;
pub const EXTENT_HEADER_MAGIC: u16 = 0xF30A;

/// 256 bytes on disk. The kernel allocates this many bytes per inode when
/// `inode_size=256` (our default); 160 bytes are used by declared fields
/// and the trailing 96 bytes are available for inline xattrs.
pub const INODE_SIZE: u32 = 256;
/// Declared-field region inside an inode. The `extra_isize` value in the
/// superblock is `INODE_BODY_SIZE - 128`.
pub const INODE_BODY_SIZE: u32 = 160;
pub const INODE_EXTRA_ISIZE: u16 = (INODE_BODY_SIZE as u16) - 128;
pub const INODE_INLINE_XATTR_SIZE: u32 = INODE_SIZE - INODE_BODY_SIZE;

pub const GROUP_DESC_SIZE: u32 = 32;
pub const ROOT_INODE: u32 = 2;
pub const LOST_FOUND_INODE: u32 = 11;
pub const FIRST_FREE_INODE: u32 = 11;
pub const MAX_EXTENTS_PER_INODE: u32 = 4;
pub const MAX_BLOCKS_PER_EXTENT: u32 = 0x8000;

// ---- feature-flag bit sets we use -----------------------------------------

pub mod feature_compat {
    pub const EXT_ATTR: u32 = 0x0008;
    pub const SPARSE_SUPER2: u32 = 0x0200;
}

pub mod feature_incompat {
    pub const FILETYPE: u32 = 0x0002;
    pub const EXTENTS: u32 = 0x0040;
    pub const FLEX_BG: u32 = 0x0200;
}

pub mod feature_ro_compat {
    pub const LARGE_FILE: u32 = 0x0002;
    pub const HUGE_FILE: u32 = 0x0008;
    pub const EXTRA_ISIZE: u32 = 0x0040;
}

pub mod inode_flag {
    pub const HUGE_FILE: u32 = 0x0004_0000;
    pub const EXTENTS: u32 = 0x0008_0000;
}

// ---- Superblock (1024 bytes) ----------------------------------------------

/// Matches `struct ext4_super_block` bit-for-bit up to byte 1024.
///
/// Source: fs/ext4/ext4.h in Linux 6.x.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Superblock {
    pub s_inodes_count: u32,
    pub s_blocks_count_lo: u32,
    pub s_r_blocks_count_lo: u32,
    pub s_free_blocks_count_lo: u32,
    pub s_free_inodes_count: u32,
    pub s_first_data_block: u32,
    pub s_log_block_size: u32,
    pub s_log_cluster_size: u32,
    pub s_blocks_per_group: u32,
    pub s_clusters_per_group: u32,
    pub s_inodes_per_group: u32,
    pub s_mtime: u32,
    pub s_wtime: u32,
    pub s_mnt_count: u16,
    pub s_max_mnt_count: u16,
    pub s_magic: u16,
    pub s_state: u16,
    pub s_errors: u16,
    pub s_minor_rev_level: u16,
    pub s_lastcheck: u32,
    pub s_checkinterval: u32,
    pub s_creator_os: u32,
    pub s_rev_level: u32,
    pub s_def_resuid: u16,
    pub s_def_resgid: u16,
    pub s_first_ino: u32,
    pub s_inode_size: u16,
    pub s_block_group_nr: u16,
    pub s_feature_compat: u32,
    pub s_feature_incompat: u32,
    pub s_feature_ro_compat: u32,
    pub s_uuid: [u8; 16],
    pub s_volume_name: [u8; 16],
    pub s_last_mounted: [u8; 64],
    pub s_algorithm_usage_bitmap: u32,
    pub s_prealloc_blocks: u8,
    pub s_prealloc_dir_blocks: u8,
    pub s_reserved_gdt_blocks: u16,
    pub s_journal_uuid: [u8; 16],
    pub s_journal_inum: u32,
    pub s_journal_dev: u32,
    pub s_last_orphan: u32,
    pub s_hash_seed: [u32; 4],
    pub s_def_hash_version: u8,
    pub s_jnl_backup_type: u8,
    pub s_desc_size: u16,
    pub s_default_mount_opts: u32,
    pub s_first_meta_bg: u32,
    pub s_mkfs_time: u32,
    pub s_jnl_blocks: [u32; 17],
    pub s_blocks_count_hi: u32,
    pub s_r_blocks_count_hi: u32,
    pub s_free_blocks_count_hi: u32,
    pub s_min_extra_isize: u16,
    pub s_want_extra_isize: u16,
    pub s_flags: u32,
    pub s_raid_stride: u16,
    pub s_mmp_interval: u16,
    pub s_mmp_block: u64,
    pub s_raid_stripe_width: u32,
    pub s_log_groups_per_flex: u8,
    pub s_checksum_type: u8,
    pub s_reserved_pad: u16,
    pub s_kbytes_written: u64,
    pub s_snapshot_inum: u32,
    pub s_snapshot_id: u32,
    pub s_snapshot_r_blocks_count: u64,
    pub s_snapshot_list: u32,
    pub s_error_count: u32,
    pub s_first_error_time: u32,
    pub s_first_error_ino: u32,
    pub s_first_error_block: u64,
    pub s_first_error_func: [u8; 32],
    pub s_first_error_line: u32,
    pub s_last_error_time: u32,
    pub s_last_error_ino: u32,
    pub s_last_error_line: u32,
    pub s_last_error_block: u64,
    pub s_last_error_func: [u8; 32],
    pub s_mount_opts: [u8; 64],
    pub s_usr_quota_inum: u32,
    pub s_grp_quota_inum: u32,
    pub s_overhead_clusters: u32,
    pub s_backup_bgs: [u32; 2],
    pub s_encrypt_algos: [u8; 4],
    pub s_encrypt_pw_salt: [u8; 16],
    pub s_lpf_ino: u32,
    pub s_prj_quota_inum: u32,
    pub s_checksum_seed: u32,
    pub s_wtime_hi: u8,
    pub s_mtime_hi: u8,
    pub s_mkfs_time_hi: u8,
    pub s_lastcheck_hi: u8,
    pub s_first_error_time_hi: u8,
    pub s_last_error_time_hi: u8,
    pub s_pad: [u8; 2],
    pub s_reserved: [u32; 96],
    pub s_checksum: u32,
}

const _: () = {
    assert!(std::mem::size_of::<Superblock>() == 1024);
};

// ---- Group descriptor (32 bytes; we do not use 64bit GDTs) ----------------

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable, Default)]
pub struct GroupDescriptor {
    pub bg_block_bitmap_lo: u32,
    pub bg_inode_bitmap_lo: u32,
    pub bg_inode_table_lo: u32,
    pub bg_free_blocks_count_lo: u16,
    pub bg_free_inodes_count_lo: u16,
    pub bg_used_dirs_count_lo: u16,
    pub bg_flags: u16,
    pub bg_exclude_bitmap_lo: u32,
    pub bg_block_bitmap_csum_lo: u16,
    pub bg_inode_bitmap_csum_lo: u16,
    pub bg_itable_unused_lo: u16,
    pub bg_checksum: u16,
}

const _: () = {
    assert!(std::mem::size_of::<GroupDescriptor>() == 32);
};

// ---- Inode (256 bytes with our extra_isize=32 config) ---------------------
//
// We split the 256-byte inode into two structs:
//   - `InodeBody` = the 160 declared bytes (mode → projid).
//   - `Inode256`  = 160-byte body + 96 bytes of inline xattr space.
//
// Writing as one 256-byte struct keeps the write-side simple and lets us
// zero the inline-xattr region explicitly.

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct InodeBody {
    pub i_mode: u16,
    pub i_uid: u16,
    pub i_size_lo: u32,
    pub i_atime: u32,
    pub i_ctime: u32,
    pub i_mtime: u32,
    pub i_dtime: u32,
    pub i_gid: u16,
    pub i_links_count: u16,
    pub i_blocks_lo: u32,
    pub i_flags: u32,
    pub i_version: u32,    // also called osd1 / linux1.l_i_version
    pub i_block: [u8; 60], // extent header + 4 extents inline, OR fast symlink
    pub i_generation: u32,
    pub i_file_acl_lo: u32, // xattr block (low 32 bits)
    pub i_size_hi: u32,
    pub i_obso_faddr: u32,
    pub i_blocks_hi: u16,
    pub i_file_acl_hi: u16,
    pub i_uid_hi: u16,
    pub i_gid_hi: u16,
    pub i_checksum_lo: u16,
    pub i_reserved: u16,
    pub i_extra_isize: u16,
    pub i_checksum_hi: u16,
    pub i_ctime_extra: u32,
    pub i_mtime_extra: u32,
    pub i_atime_extra: u32,
    pub i_crtime: u32,
    pub i_crtime_extra: u32,
    pub i_version_hi: u32,
    pub i_projid: u32,
}

const _: () = {
    assert!(std::mem::size_of::<InodeBody>() == 160);
};

/// 256-byte on-disk inode with a zero-filled inline-xattr trailer.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Inode256 {
    pub body: InodeBody,
    pub inline_xattrs: [u8; INODE_INLINE_XATTR_SIZE as usize],
}

const _: () = {
    assert!(std::mem::size_of::<Inode256>() == 256);
};

// ---- Extent tree ----------------------------------------------------------

/// 12 bytes. Appears as the first record of any extent tree node.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ExtentHeader {
    pub eh_magic: u16,
    pub eh_entries: u16,
    pub eh_max: u16,
    pub eh_depth: u16,
    pub eh_generation: u32,
}

/// 12 bytes. A non-leaf node entry, pointing to a deeper node.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ExtentIndex {
    pub ei_block: u32,
    pub ei_leaf_lo: u32,
    pub ei_leaf_hi: u16,
    pub ei_unused: u16,
}

/// 12 bytes. A leaf — one contiguous run of `ee_len` blocks starting at
/// `ee_start_*` on disk, covering logical blocks `[ee_block, ee_block+ee_len)`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ExtentLeaf {
    pub ee_block: u32,
    pub ee_len: u16,
    pub ee_start_hi: u16,
    pub ee_start_lo: u32,
}

const _: () = {
    assert!(std::mem::size_of::<ExtentHeader>() == 12);
    assert!(std::mem::size_of::<ExtentIndex>() == 12);
    assert!(std::mem::size_of::<ExtentLeaf>() == 12);
};

// ---- Directory entry ------------------------------------------------------

/// First 8 bytes of an ext4 "linear" directory entry (`ext4_dir_entry_2`).
/// The trailing name (up to 255 bytes) + zero-padding to `rec_len` follows.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct DirEntryHeader {
    pub inode: u32,
    pub rec_len: u16,
    pub name_len: u8,
    pub file_type: u8,
}

const _: () = {
    assert!(std::mem::size_of::<DirEntryHeader>() == 8);
};

// ---- xattrs ---------------------------------------------------------------

pub const XATTR_MAGIC: u32 = 0xEA02_0000;
pub const XATTR_IBODY_MAGIC: u32 = 0xEA02_0000;

/// Header for an inline xattr region (inside the 96-byte inode tail).
/// Kernel name: `struct ext4_xattr_ibody_header`. Layout = one 32-bit magic
/// at the start, entries follow, zero-padding at the end. For the block
/// variant there's a larger `ext4_xattr_header` — we're not implementing
/// that in the MVP.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct XattrIbodyHeader {
    pub h_magic: u32,
}

/// 16-byte xattr entry fixed header; name + padding follow.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct XattrEntry {
    pub e_name_len: u8,
    pub e_name_index: u8,
    pub e_value_offs: u16,
    pub e_value_inum: u32,
    pub e_value_size: u32,
    pub e_hash: u32,
}

const _: () = {
    assert!(std::mem::size_of::<XattrIbodyHeader>() == 4);
    assert!(std::mem::size_of::<XattrEntry>() == 16);
};
