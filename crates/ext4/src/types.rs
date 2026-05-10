//! Newtypes for the units that appear in ext4. Keeping these separate from
//! `u32`/`u64` eliminates a whole class of "which number?" bugs — the
//! compiler catches e.g. mixing an inode number with a block number.
//!
//! Design note: these are thin wrappers, not smart constructors. ext4 has
//! very few "validated at this edge" invariants on these numbers themselves
//! — what matters is *how* they compose with the rest of the filesystem,
//! which the builder enforces.

use std::fmt;

/// Absolute block index inside an ext4 image. Zero-based; the superblock
/// lives at block 0 (offset 1024) under the default 4 KiB block size.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct BlockNumber(pub u32);

impl BlockNumber {
    pub const fn get(self) -> u32 {
        self.0
    }

    pub const fn offset_bytes(self, bs: BlockSize) -> u64 {
        (self.0 as u64) * (bs.bytes() as u64)
    }

    /// Returns the block that follows this one.
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Display for BlockNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "blk#{}", self.0)
    }
}

/// 1-based inode number. Inode 1 is the defective-block inode; inode 2 is
/// the root directory; inodes 3..=10 are reserved (journal, quotas, etc.);
/// inode 11 is the first freely allocatable.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct InodeNumber(pub u32);

impl InodeNumber {
    pub const ROOT: Self = Self(2);
    pub const LOST_FOUND: Self = Self(11);
    pub const FIRST_FREE: Self = Self(11);

    pub const fn get(self) -> u32 {
        self.0
    }

    /// Zero-based index into the inode table.
    pub const fn table_index(self) -> u32 {
        self.0 - 1
    }
}

impl fmt::Display for InodeNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ino#{}", self.0)
    }
}

/// Block size in bytes. ext4 permits 1024, 2048, or 4096; this crate is
/// `const`-friendly so callers don't have to pattern-match at runtime.
///
/// We go through a type not a `u32` so that "log block size" (which is
/// `log2(bytes/1024)` in the superblock) falls out without a second source
/// of truth.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BlockSize(u32);

impl BlockSize {
    #[allow(non_upper_case_globals)]
    pub const Size1K: Self = Self(1024);
    #[allow(non_upper_case_globals)]
    pub const Size2K: Self = Self(2048);
    #[allow(non_upper_case_globals)]
    pub const Size4K: Self = Self(4096);

    pub const K1: Self = Self(1024);
    pub const K2: Self = Self(2048);
    pub const K4: Self = Self(4096);

    /// Canonical default used by every image this crate produces unless the
    /// caller overrides it. 4 KiB matches mkfs.ext4's default on >512 MiB.
    pub const DEFAULT: Self = Self::K4;

    pub const fn bytes(self) -> u32 {
        self.0
    }

    pub const fn bytes_u64(self) -> u64 {
        self.0 as u64
    }

    /// log2(bytes) - 10. The superblock's `s_log_block_size` field.
    pub const fn log(self) -> u32 {
        match self.0 {
            1024 => 0,
            2048 => 1,
            4096 => 2,
            _ => panic!("BlockSize constructed with an invalid value"),
        }
    }

    pub const fn try_new(bytes: u32) -> Option<Self> {
        match bytes {
            1024 => Some(Self::K1),
            2048 => Some(Self::K2),
            4096 => Some(Self::K4),
            _ => None,
        }
    }
}

/// File mode bits. Thin wrapper over `u16` but keeps the ext4-canonical
/// octal literals compile-time visible at call sites.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct FileMode(pub u16);

impl FileMode {
    pub const IFDIR: u16 = 0x4000;
    pub const IFCHR: u16 = 0x2000;
    pub const IFREG: u16 = 0x8000;
    pub const IFLNK: u16 = 0xA000;
    pub const TYPE_MASK: u16 = 0xF000;

    pub const fn dir(perm: u16) -> Self {
        Self(Self::IFDIR | (perm & 0o7777))
    }
    pub const fn char_device(perm: u16) -> Self {
        Self(Self::IFCHR | (perm & 0o7777))
    }
    pub const fn regular(perm: u16) -> Self {
        Self(Self::IFREG | (perm & 0o7777))
    }
    pub const fn symlink(perm: u16) -> Self {
        Self(Self::IFLNK | (perm & 0o7777))
    }

    pub const fn is_dir(self) -> bool {
        (self.0 & Self::TYPE_MASK) == Self::IFDIR
    }
    pub const fn is_char_device(self) -> bool {
        (self.0 & Self::TYPE_MASK) == Self::IFCHR
    }
    pub const fn is_regular(self) -> bool {
        (self.0 & Self::TYPE_MASK) == Self::IFREG
    }
    pub const fn is_symlink(self) -> bool {
        (self.0 & Self::TYPE_MASK) == Self::IFLNK
    }

    /// Directory-entry `file_type` byte.
    pub const fn dir_entry_type(self) -> u8 {
        match self.0 & Self::TYPE_MASK {
            Self::IFREG => 1,
            Self::IFDIR => 2,
            Self::IFCHR => 3,
            Self::IFLNK => 7,
            _ => 0, // unknown
        }
    }
}
