//! Law tests: structural invariants that must hold for *any* image we
#![allow(clippy::all, clippy::pedantic)]
//! produce, regardless of content.
//!
//! Each test kills a family of wrong implementations: "any writer that
//! gets X wrong would fail here." We keep the oracle tight — no
//! "the image is 'valid'"-style assertions, because that's an invariant
//! e2fsck tests later.

use std::io::{Cursor, Read, Seek, SeekFrom};

use firkin_ext4::layout::{
    EXTENT_HEADER_MAGIC, ExtentHeader, ExtentIndex, INODE_SIZE, SUPERBLOCK_MAGIC,
    SUPERBLOCK_OFFSET, Superblock,
};
use firkin_ext4::{BlockSize, FileSystemBuilder};

fn build_minimal() -> Vec<u8> {
    let mut fs = FileSystemBuilder::new(BlockSize::DEFAULT);
    fs.add_file("/hello", b"hi\n", 0o644).unwrap();
    let mut buf = Cursor::new(Vec::new());
    fs.write(&mut buf).unwrap();
    buf.into_inner()
}

#[test]
fn superblock_magic_is_at_canonical_offset() {
    // Law: any ext4 image in the Linux universe must have magic 0xEF53 at
    // byte offset 1024 + 0x38 (the `s_magic` field within the superblock).
    //
    // Kills: any writer that put the superblock in block 0's first 1024
    // bytes (boot-sector region) or misaligned the magic field.
    let img = build_minimal();
    let magic_offset = (SUPERBLOCK_OFFSET + 0x38) as usize;
    let magic_bytes = &img[magic_offset..magic_offset + 2];
    let got = u16::from_le_bytes([magic_bytes[0], magic_bytes[1]]);
    assert_eq!(
        got, SUPERBLOCK_MAGIC,
        "s_magic mismatch at offset {}",
        magic_offset
    );
}

#[test]
fn superblock_block_size_round_trips() {
    // Law: s_log_block_size = log2(block_size) - 10. Our default is
    // 4096, which should round-trip to 2.
    let img = build_minimal();
    let mut cur = Cursor::new(&img);
    cur.seek(SeekFrom::Start(SUPERBLOCK_OFFSET)).unwrap();
    let mut sb_bytes = [0u8; 1024];
    cur.read_exact(&mut sb_bytes).unwrap();
    let sb: Superblock = bytemuck::pod_read_unaligned(&sb_bytes);
    assert_eq!(sb.s_magic, SUPERBLOCK_MAGIC);
    assert_eq!(sb.s_log_block_size, 2, "log(4096)-10 = 2");
    assert_eq!(1024u32 << sb.s_log_block_size, 4096);
    assert_eq!(sb.s_inode_size, INODE_SIZE as u16);
}

#[test]
fn image_size_is_at_least_min_size() {
    // Law: if caller asks for min_size=N, the output must be ≥ N bytes.
    // We always also pad to a full block-group boundary, so the actual
    // size may exceed N.
    let min_size = 8 * 1024 * 1024;
    let mut fs = FileSystemBuilder::new(BlockSize::DEFAULT).with_min_size(min_size);
    fs.add_file("/hello", b"hi\n", 0o644).unwrap();
    let mut buf = Cursor::new(Vec::new());
    fs.write(&mut buf).unwrap();
    assert!(
        buf.get_ref().len() as u64 >= min_size,
        "image {} < min_size {}",
        buf.get_ref().len(),
        min_size
    );
}

#[test]
fn hello_file_extent_header_is_present() {
    // Law: any non-empty regular file's inode must carry an extent tree
    // with magic 0xF30A as its first 2 bytes of i_block. Kills: a writer
    // that forgets to set EXTENTS flag or skips the extent header.
    //
    // We find inode 12 (first free; lost+found is 11, hello is 12) and
    // check its i_block bytes.
    let img = build_minimal();
    let mut cur = Cursor::new(&img);
    cur.seek(SeekFrom::Start(SUPERBLOCK_OFFSET)).unwrap();
    let mut sb_bytes = [0u8; 1024];
    cur.read_exact(&mut sb_bytes).unwrap();
    let sb: Superblock = bytemuck::pod_read_unaligned(&sb_bytes);

    // GDT is at block 1; read the inode table block from the first group
    // descriptor.
    let block_size = 1024u32 << sb.s_log_block_size;
    let gdt_off = block_size as u64;
    cur.seek(SeekFrom::Start(gdt_off)).unwrap();
    let mut gdt = [0u8; 32];
    cur.read_exact(&mut gdt).unwrap();
    let inode_table_blk = u32::from_le_bytes(gdt[8..12].try_into().unwrap());
    let inode_table_off = (inode_table_blk as u64) * (block_size as u64);

    // Inode 12 = table_index 11, each inode is INODE_SIZE bytes.
    let inode12_off = inode_table_off + 11 * (INODE_SIZE as u64);
    cur.seek(SeekFrom::Start(inode12_off)).unwrap();
    let mut inode = [0u8; INODE_SIZE as usize];
    cur.read_exact(&mut inode).unwrap();

    // i_block starts at byte 40 in the inode body (mode=2 + uid=2 + size_lo=4
    // + atime=4 + ctime=4 + mtime=4 + dtime=4 + gid=2 + links=2 + blocks_lo=4
    // + flags=4 + version=4 = 40). The ExtentHeader's magic is the first
    // 2 bytes of i_block.
    let i_block_off = 40usize;
    let eh_magic = u16::from_le_bytes([inode[i_block_off], inode[i_block_off + 1]]);
    assert_eq!(eh_magic, EXTENT_HEADER_MAGIC, "extent header magic");

    // Parse the full header and confirm entries=1, depth=0.
    let eh = bytemuck::pod_read_unaligned::<ExtentHeader>(&inode[i_block_off..i_block_off + 12]);
    assert_eq!(eh.eh_entries, 1);
    assert_eq!(eh.eh_depth, 0);
}

#[test]
fn determinism_same_input_same_bytes() {
    // Law: with timestamps & UUID pinned to zero, two runs produce a
    // bit-identical image. Kills: any nondeterminism we accidentally
    // introduce (HashMap iteration, system time, random UUIDs).
    let run = || {
        let mut fs = FileSystemBuilder::new(BlockSize::DEFAULT)
            .with_uuid([0; 16])
            .with_timestamp(0);
        fs.add_file("/hello", b"hi\n", 0o644).unwrap();
        fs.add_dir("/dir", 0o755).unwrap();
        fs.add_file("/dir/world", b"world\n", 0o644).unwrap();
        fs.add_dir("/overlay", 0o755).unwrap();
        fs.add_whiteout("/overlay/gone").unwrap();
        fs.add_opaque_dir("/overlay").unwrap();
        let mut buf = Cursor::new(Vec::new());
        fs.write(&mut buf).unwrap();
        buf.into_inner()
    };
    let a = run();
    let b = run();
    assert_eq!(a.len(), b.len());
    // Walk in chunks so a failing-byte message is useful.
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        if x != y {
            panic!("byte {} differs: {:02x} vs {:02x}", i, x, y);
        }
    }
}

#[test]
fn oversized_file_uses_a_depth_one_extent_tree() {
    // Example: once a file's extent set no longer fits in the inode's four
    // inline slots, the root must become an index node (depth=1) that points
    // at a leaf block. Kills: writers that silently truncate extents, keep
    // claiming depth=0, or forget to allocate the external leaf block.
    let mut fs = FileSystemBuilder::new(BlockSize::DEFAULT);
    let buf = vec![0x5Au8; 80 * 1024 * 1024 + 1];
    fs.add_file("/big", &buf, 0o644).unwrap();
    let mut cur = Cursor::new(Vec::new());
    fs.write(&mut cur).unwrap();
    let img = cur.into_inner();

    let mut cur = Cursor::new(&img);
    cur.seek(SeekFrom::Start(SUPERBLOCK_OFFSET)).unwrap();
    let mut sb_bytes = [0u8; 1024];
    cur.read_exact(&mut sb_bytes).unwrap();
    let sb: Superblock = bytemuck::pod_read_unaligned(&sb_bytes);

    let block_size = 1024u32 << sb.s_log_block_size;
    let gdt_off = block_size as u64;
    cur.seek(SeekFrom::Start(gdt_off)).unwrap();
    let mut gdt = [0u8; 32];
    cur.read_exact(&mut gdt).unwrap();
    let inode_table_blk = u32::from_le_bytes(gdt[8..12].try_into().unwrap());
    let inode_table_off = (inode_table_blk as u64) * (block_size as u64);

    let inode12_off = inode_table_off + 11 * (INODE_SIZE as u64);
    cur.seek(SeekFrom::Start(inode12_off)).unwrap();
    let mut inode = [0u8; INODE_SIZE as usize];
    cur.read_exact(&mut inode).unwrap();

    let i_block_off = 40usize;
    let eh = bytemuck::pod_read_unaligned::<ExtentHeader>(&inode[i_block_off..i_block_off + 12]);
    assert_eq!(eh.eh_magic, EXTENT_HEADER_MAGIC, "extent header magic");
    assert_eq!(eh.eh_depth, 1, "root must become an index node");
    assert!(
        eh.eh_entries >= 1,
        "depth-1 root must point at least one leaf block"
    );

    let first_index =
        bytemuck::pod_read_unaligned::<ExtentIndex>(&inode[i_block_off + 12..i_block_off + 24]);
    assert_ne!(
        first_index.ei_leaf_lo, 0,
        "depth-1 extent tree must reference an external leaf block"
    );
}

#[test]
fn invalid_filename_is_rejected() {
    // Example: writer rejects "." and ".." as filenames.
    let mut fs = FileSystemBuilder::new(BlockSize::DEFAULT);
    assert!(fs.add_file("/.", b"", 0o644).is_err());
    assert!(fs.add_file("/..", b"", 0o644).is_err());
    assert!(fs.add_file("/", b"", 0o644).is_err());
}

#[test]
fn hardlink_to_missing_target_is_rejected() {
    let mut fs = FileSystemBuilder::new(BlockSize::DEFAULT);
    assert!(fs.add_hardlink("/b", "/nope").is_err());
}

#[test]
fn hardlink_to_directory_is_rejected() {
    let mut fs = FileSystemBuilder::new(BlockSize::DEFAULT);
    fs.add_dir("/d", 0o755).unwrap();
    assert!(fs.add_hardlink("/l", "/d").is_err());
}
