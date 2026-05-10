//! Synthesis for the vminitd boot block image.

use std::path::PathBuf;

use firkin_types::Size;
use sha2::{Digest, Sha256};

use crate::{Result, Writer};

const INIT_BLOCK_MIN_BYTES: u64 = 8 * 1024 * 1024;
const INIT_BLOCK_OVERHEAD_BYTES: u64 = 16 * 1024 * 1024;

/// Synthesize `init.block` from vminitd/vmexec ELFs and return the cached path.
///
/// # Errors
///
/// Returns [`crate::Error`] when the cache directory or ext4 image cannot be written.
pub fn synthesize(vminitd: &[u8], vmexec: &[u8]) -> Result<PathBuf> {
    let path = cache_path(vminitd, vmexec);
    if path.exists() {
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| crate::Error::Control {
            what: "create init.block cache directory",
            source,
        })?;
    }
    synthesize_to(vminitd, vmexec, &path)?;
    Ok(path)
}

/// Synthesize `init.block` from vminitd/vmexec ELFs into a caller-selected path.
///
/// # Errors
///
/// Returns [`crate::Error`] when the ext4 image cannot be written.
pub fn synthesize_to(vminitd: &[u8], vmexec: &[u8], dest: impl Into<PathBuf>) -> Result<()> {
    let digest = runtime_digest(vminitd, vmexec);
    let size = init_block_size(vminitd, vmexec);
    Writer::new(dest, size)?
        .uuid(uuid_from_digest(digest))
        .write_dir("/bin", 0o755)?
        .write_dir("/sbin", 0o755)?
        .write_dir("/dev", 0o755)?
        .write_dir("/sys", 0o755)?
        .write_dir("/proc", 0o755)?
        .write_dir("/proc/self", 0o755)?
        .write_dir("/run", 0o755)?
        .write_dir("/mnt", 0o755)?
        .write_dir("/var", 0o755)?
        .write_dir("/etc", 0o755)?
        .write_dir("/tmp", 0o1777)?
        .write_dir("/root", 0o700)?
        .write_file("/sbin/vminitd", vminitd, 0o755)?
        .write_file("/sbin/vmexec", vmexec, 0o755)?
        .write_symlink("/proc/self/exe", "sbin/vminitd")?
        .write_symlink("/sbin/init", "/sbin/vminitd")?
        .write_file("/etc/passwd", b"root:x:0:0:root:/root:/bin/sh\n", 0o644)?
        .write_file("/etc/hosts", b"127.0.0.1 localhost\n", 0o644)?
        .write_file("/etc/resolv.conf", b"", 0o644)?
        .finalize()?;
    Ok(())
}

/// Return the cache path that [`synthesize`] would use for this ELF.
#[must_use]
pub fn cache_path(vminitd: &[u8], vmexec: &[u8]) -> PathBuf {
    cache_dir().join("init-blocks").join(format!(
        "{}.ext4",
        hex_digest(runtime_digest(vminitd, vmexec))
    ))
}

/// Return the cache root for init-block artifacts.
#[must_use]
pub fn cache_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("FIRKIN_CACHE") {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(path).join("firkin");
    }
    if let Some(path) = std::env::var_os("HOME") {
        return PathBuf::from(path).join(".cache").join("firkin");
    }
    std::env::temp_dir().join("firkin")
}

fn init_block_size(vminitd: &[u8], vmexec: &[u8]) -> Size {
    let bytes = u64::try_from(vminitd.len())
        .unwrap_or(u64::MAX)
        .saturating_add(u64::try_from(vmexec.len()).unwrap_or(u64::MAX))
        .saturating_add(INIT_BLOCK_OVERHEAD_BYTES)
        .max(INIT_BLOCK_MIN_BYTES);
    Size::bytes(bytes)
}

fn runtime_digest(vminitd: &[u8], vmexec: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"vminitd\0");
    hasher.update(vminitd);
    hasher.update(b"vmexec\0");
    hasher.update(vmexec);
    hasher.finalize().into()
}

fn uuid_from_digest(digest: [u8; 32]) -> [u8; 16] {
    digest[..16].try_into().expect("sha256 digest has 32 bytes")
}

fn hex_digest(digest: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}
