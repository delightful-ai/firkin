//! Domain errors for the ext4 writer.
//!
//! Variants are *behaviors*, not *causes*. A caller who sees
//! `ImageTooSmall` knows they must give a larger size; a caller who sees
//! `Write` knows their output stream failed and they need to handle I/O.
//! We never expose `io::Error` directly; it always rides inside a
//! domain-named variant.

use crate::types::{BlockNumber, InodeNumber};
use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Ext4Error {
    /// The caller-requested image size cannot fit even the structural
    /// metadata (superblock + GDT + bitmaps + inode table) for a single
    /// block group. Bump the size or lower inodes-per-group.
    #[error("image too small: need ≥ {needed} bytes, got {actual} bytes")]
    ImageTooSmall { needed: u64, actual: u64 },

    /// A reserved inode number was handed to a public API that only
    /// accepts freely-allocatable inodes, or the caller asked to resolve an
    /// inode we never assigned.
    #[error("inode {0} is reserved or out of range")]
    InvalidInode(InodeNumber),

    /// Caller asked for a block size we don't implement (ext4 allows 1, 2,
    /// or 4 KiB; this crate supports all three).
    #[error("invalid block size {0}: must be 1024, 2048, or 4096")]
    InvalidBlockSize(u32),

    /// Caller requested an ext4 feature this library version does not emit.
    #[error("unsupported ext4 feature requested: {feature}")]
    UnsupportedFeature { feature: String },

    /// A file exceeds this crate's supported max (128 GiB, inherited from
    /// the Swift reference).
    #[error("file too large: {bytes} bytes exceeds supported max {max} bytes")]
    FileTooLarge { bytes: u64, max: u64 },

    /// A single file needs more extents than we can pack into a 2-level
    /// tree. Raise `MAX_EXTENT_TREE_ENTRIES` and recompile or split.
    #[error("extent tree would require depth > {max_depth} for file at {path}")]
    ExtentDepthExceeded { path: String, max_depth: u16 },

    /// A single block ran out of space for directory entries. The writer
    /// normally starts a new block instead of raising this; this fires
    /// only when even an empty block can't hold one entry (name too long).
    #[error("directory entry would overflow block (name={name:?}, rec_len={rec_len})")]
    DirEntryOverflow { name: String, rec_len: u16 },

    /// Attempt to create a path under a non-directory or a path that
    /// already exists with an incompatible type.
    #[error("path {path:?} conflicts with an existing entry")]
    PathConflict { path: String },

    /// Empty or invalid filename (contains `/`, is `.` or `..`).
    #[error("invalid filename {name:?}")]
    InvalidFilename { name: String },

    /// Symlink target longer than 4095 bytes (we don't currently support
    /// fast symlinks longer than fit inline or one data block).
    #[error("symlink target too long: {bytes} bytes (max {max})")]
    SymlinkTargetTooLong { bytes: usize, max: usize },

    /// Hardlink target must exist and not be a directory.
    #[error("hardlink target {path:?} is missing or a directory")]
    InvalidHardlinkTarget { path: String },

    /// Couldn't write N bytes starting at image offset — the underlying
    /// `io::Write` refused or short-wrote.
    #[error("write failed at offset {offset} ({bytes} bytes)")]
    Write {
        offset: u64,
        bytes: usize,
        #[source]
        source: io::Error,
    },

    /// `seek`/`set_len`/`flush` on the output failed.
    #[error("image io control failed ({what})")]
    Control {
        what: &'static str,
        #[source]
        source: io::Error,
    },

    /// A bookkeeping bug in this crate — an invariant we promised
    /// ourselves got violated. If you see this, file a bug.
    #[error("internal invariant violated at block {block}: {what}")]
    Internal {
        block: BlockNumber,
        what: &'static str,
    },
}

pub type Result<T> = std::result::Result<T, Ext4Error>;
