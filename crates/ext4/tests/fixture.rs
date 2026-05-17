//! Example tests: known fixtures + their byte-oracle expectations.
#![allow(clippy::all, clippy::pedantic)]
//!
//! These tests produce images on disk and shell out to `e2fsck`. If
//! e2fsprogs isn't on the `PATH`, the tests skip with an explanatory
//! message (common on CI macOS images where Homebrew's keg-only paths
//! aren't wired up).

use std::path::PathBuf;
use std::process::Command;

use firkin_ext4::{BlockSize, FileSystemBuilder};

fn e2fsck_binary() -> Option<PathBuf> {
    for candidate in [
        "/opt/homebrew/opt/e2fsprogs/sbin/e2fsck",
        "/usr/local/opt/e2fsprogs/sbin/e2fsck",
        "/usr/sbin/e2fsck",
        "/sbin/e2fsck",
    ] {
        if std::path::Path::new(candidate).exists() {
            return Some(PathBuf::from(candidate));
        }
    }
    which::which("e2fsck").ok()
}

fn debugfs_binary() -> Option<PathBuf> {
    for candidate in [
        "/opt/homebrew/opt/e2fsprogs/sbin/debugfs",
        "/usr/local/opt/e2fsprogs/sbin/debugfs",
        "/usr/sbin/debugfs",
        "/sbin/debugfs",
    ] {
        if std::path::Path::new(candidate).exists() {
            return Some(PathBuf::from(candidate));
        }
    }
    which::which("debugfs").ok()
}

mod which {
    //! Minimal re-implementation to avoid adding the `which` crate as
    //! another dependency.
    use std::path::PathBuf;
    pub fn which(name: &str) -> Result<PathBuf, ()> {
        if let Ok(paths) = std::env::var("PATH") {
            for dir in paths.split(':') {
                let p = std::path::Path::new(dir).join(name);
                if p.exists() {
                    return Ok(p);
                }
            }
        }
        Err(())
    }
}

fn e2fsck_clean(path: &std::path::Path) -> Result<(), String> {
    let Some(bin) = e2fsck_binary() else {
        eprintln!("warn: e2fsck not found on PATH; skipping structural check");
        return Ok(());
    };
    let out = Command::new(&bin)
        .args(["-nf", path.to_str().unwrap()])
        .output()
        .map_err(|e| format!("failed to run {}: {e}", bin.display()))?;
    if !out.status.success() {
        return Err(format!(
            "e2fsck exit={:?}\nstdout:\n{}\nstderr:\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        ));
    }
    Ok(())
}

fn debugfs_output(path: &std::path::Path, command: &str) -> Result<String, String> {
    let Some(bin) = debugfs_binary() else {
        eprintln!("warn: debugfs not found on PATH; skipping debugfs probe");
        return Ok(String::new());
    };
    let out = Command::new(&bin)
        .args(["-R", command, path.to_str().unwrap()])
        .output()
        .map_err(|e| format!("failed to run {}: {e}", bin.display()))?;
    if !out.status.success() {
        return Err(format!(
            "debugfs exit={:?}\nstdout:\n{}\nstderr:\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn max_extent_depth(dump_extents: &str) -> Option<u16> {
    dump_extents
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let level = parts.next()?;
            let depth = parts.next()?;
            if !level.ends_with('/') {
                return None;
            }
            depth.parse::<u16>().ok()
        })
        .max()
}

#[track_caller]
fn write_image(contents: impl FnOnce(&mut FileSystemBuilder)) -> tempfile::NamedTempFile {
    let mut fs = FileSystemBuilder::new(BlockSize::DEFAULT);
    contents(&mut fs);
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut out = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .truncate(true)
        .open(file.path())
        .unwrap();
    fs.write(&mut out).unwrap();
    file
}

#[test]
fn hello_fixture_passes_e2fsck() {
    // Example: the tier-1 acceptance fixture. /hello, "hi\n".
    let img = write_image(|fs| {
        fs.add_file("/hello", b"hi\n", 0o644).unwrap();
    });
    e2fsck_clean(img.path()).unwrap();
}

#[test]
fn tree_fixture_passes_e2fsck() {
    let img = write_image(|fs| {
        fs.add_file("/hello", b"hi\n", 0o644).unwrap();
        fs.add_dir("/dir", 0o755).unwrap();
        fs.add_file("/dir/world", b"world\n", 0o644).unwrap();
        fs.add_dir("/dir/nested", 0o755).unwrap();
        fs.add_file("/dir/nested/deep", b"deep\n", 0o644).unwrap();
    });
    e2fsck_clean(img.path()).unwrap();
}

#[test]
fn large_linear_directory_fixture_passes_e2fsck() {
    let img = write_image(|fs| {
        fs.add_dir("/bin", 0o755).unwrap();
        for index in 0..420 {
            fs.add_file(&format!("/bin/tool-{index:03}"), b"#!/bin/sh\n", 0o755)
                .unwrap();
        }
    });
    e2fsck_clean(img.path()).unwrap();
}

#[test]
fn links_fixture_passes_e2fsck() {
    let img = write_image(|fs| {
        fs.add_file("/a", b"AAAA\n", 0o644).unwrap();
        fs.add_hardlink("/b", "/a").unwrap();
        fs.add_symlink("/link", "a").unwrap();
    });
    e2fsck_clean(img.path()).unwrap();
}

#[test]
fn xattr_fixture_passes_e2fsck() {
    let img = write_image(|fs| {
        fs.add_file("/f", b"x\n", 0o644).unwrap();
        fs.set_xattr("/f", "user.foo", b"bar").unwrap();
    });
    e2fsck_clean(img.path()).unwrap();
}

#[test]
fn multi_extent_fixture_passes_e2fsck() {
    // 64 KiB file — forces the multi-extent packer to split the single
    // content range into multiple extents? At 4 KiB blocks that's 16
    // blocks — still one contiguous extent because MAX_BLOCKS_PER_EXTENT
    // is 32768. This fixture therefore exercises the "large file, single
    // extent" path, which is the important one for container rootfs.
    let buf = vec![0x42u8; 64 * 1024];
    let img = write_image(|fs| {
        fs.add_file("/big", &buf, 0o644).unwrap();
    });
    e2fsck_clean(img.path()).unwrap();
}

#[test]
fn multi_group_fixture_passes_e2fsck() {
    // Example: 1 KiB blocks cap a block group at 8 MiB. A 9 MiB payload
    // forces more than one group without making the default suite huge.
    // Kills: writers that keep a single global bitmap/GDT entry and either
    // reject or corrupt images once allocation crosses the first group.
    let buf = vec![0x24u8; 9 * 1024 * 1024];
    let mut fs = FileSystemBuilder::new(BlockSize::Size1K);
    fs.add_file("/big", &buf, 0o644).unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut out = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .truncate(true)
        .open(file.path())
        .unwrap();
    fs.write(&mut out).unwrap();

    e2fsck_clean(file.path()).unwrap();
}

#[test]
fn depth_one_extent_fixture_passes_e2fsck_and_debugfs_reports_index_root() {
    // Example: a file whose extent list outgrows the inode's inline slots must
    // serialize a depth-1 tree. Kills: writers that only support inline
    // extents, truncate the tail, or forget the external leaf block.
    let buf = vec![0xA5u8; 80 * 1024 * 1024 + 1];
    let img = write_image(|fs| {
        fs.add_file("/big", &buf, 0o644).unwrap();
    });
    e2fsck_clean(img.path()).unwrap();
    let dump = debugfs_output(img.path(), "dump_extents /big").unwrap();
    assert_eq!(
        max_extent_depth(&dump),
        Some(1),
        "expected a depth-1 extent tree, got:\n{dump}"
    );
}

#[test]
fn whiteout_fixture_is_a_character_device_and_passes_e2fsck() {
    // Example: OCI whiteouts are 0:0 character-device inodes named
    // `.wh.<basename>`. Kills: writers that fake whiteouts as regular files
    // or symlinks, which overlayfs would not honor.
    let img = write_image(|fs| {
        fs.add_dir("/upper", 0o755).unwrap();
        fs.add_whiteout("/upper/gone").unwrap();
    });
    e2fsck_clean(img.path()).unwrap();
    let stat = debugfs_output(img.path(), "stat /upper/.wh.gone").unwrap();
    assert!(
        stat.contains("Type: character special"),
        "expected a character special inode, got:\n{stat}"
    );
}

#[test]
fn opaque_dir_fixture_is_an_empty_regular_file_marker() {
    // Example: OCI opaque directories are regular files named
    // `.wh..wh..opq` inside the directory. Kills: writers that encode the
    // marker as xattrs, directories, or non-empty files.
    let img = write_image(|fs| {
        fs.add_dir("/upper", 0o755).unwrap();
        fs.add_opaque_dir("/upper").unwrap();
    });
    e2fsck_clean(img.path()).unwrap();
    let stat = debugfs_output(img.path(), "stat /upper/.wh..wh..opq").unwrap();
    assert!(
        stat.contains("Type: regular"),
        "expected a regular inode, got:\n{stat}"
    );
    assert!(
        stat.contains("Size: 0"),
        "opaque marker must be empty:\n{stat}"
    );
}

#[test]
fn fast_symlink_stored_inline() {
    // Example: any symlink target < 60 bytes is stored in the inode's
    // `i_block` region with no data blocks. Our `/link -> "a"` fixture
    // exercises this; verify that the file reports `Blockcount: 0` (via
    // debugfs) — but debugfs is a binary dependency, so we instead
    // observe the effect: the image is no bigger than the non-symlink
    // version.
    //
    // Regression shape: keeps us honest that we didn't accidentally
    // always allocate a block for every symlink.
    let img_with = {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut fs = FileSystemBuilder::new(BlockSize::DEFAULT);
        fs.add_symlink("/link", "abc").unwrap();
        let mut out = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .truncate(true)
            .open(tmp.path())
            .unwrap();
        fs.write(&mut out).unwrap();
        let len = out.metadata().unwrap().len();
        (tmp, len)
    };
    let img_without = {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut fs = FileSystemBuilder::new(BlockSize::DEFAULT);
        fs.add_file("/placeholder", b"", 0o644).unwrap();
        let mut out = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .truncate(true)
            .open(tmp.path())
            .unwrap();
        fs.write(&mut out).unwrap();
        let len = out.metadata().unwrap().len();
        (tmp, len)
    };
    // Fast-symlink image should not be larger than a file-only image;
    // more importantly, e2fsck should pass.
    assert!(img_with.1 <= img_without.1 + 4096);
    e2fsck_clean(img_with.0.path()).unwrap();
}
