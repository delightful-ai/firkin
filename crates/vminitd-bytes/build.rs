//! Resolve and verify the pinned vminitd ELF for `include_bytes!`.

use std::env;
use std::fs;
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=pin.toml");
    println!("cargo:rerun-if-changed=../../build-tools/build-vminitd/pin.toml");
    println!("cargo:rerun-if-env-changed=FIRKIN_VMINITD_PATH");
    println!("cargo:rerun-if-env-changed=FIRKIN_VMEXEC_PATH");

    let runtime_download = env::var_os("CARGO_FEATURE_RUNTIME_DOWNLOAD").is_some();
    let vendored = env::var_os("CARGO_FEATURE_VENDORED_VMINITD").is_some();
    assert!(
        !(runtime_download && vendored),
        "features runtime-download and vendored-vminitd are mutually exclusive"
    );

    let pin = Pin::read(&pin_path());
    println!("cargo:rustc-env=VMINITD_SHA256={}", pin.sha256);
    println!("cargo:rustc-env=VMEXEC_SHA256={}", pin.vmexec_sha256);
    println!("cargo:rustc-env=VMINITD_REVISION={}", pin.revision);

    if runtime_download {
        println!("cargo:rustc-env=VMINITD_AARCH64_PATH=");
        println!("cargo:rustc-env=VMEXEC_AARCH64_PATH=");
        return;
    }

    let vminitd_path = resolve_runtime_path(RuntimeArtifact::Vminitd, vendored, &pin);
    println!("cargo:rerun-if-changed={}", vminitd_path.display());
    let actual = sha256_file(&vminitd_path);
    assert_eq!(
        actual,
        pin.sha256,
        "vminitd SHA-256 mismatch for {}",
        vminitd_path.display()
    );
    println!(
        "cargo:rustc-env=VMINITD_AARCH64_PATH={}",
        vminitd_path.display()
    );

    let vmexec_path = resolve_runtime_path(RuntimeArtifact::Vmexec, vendored, &pin);
    println!("cargo:rerun-if-changed={}", vmexec_path.display());
    let actual = sha256_file(&vmexec_path);
    assert_eq!(
        actual,
        pin.vmexec_sha256,
        "vmexec SHA-256 mismatch for {}",
        vmexec_path.display()
    );
    println!(
        "cargo:rustc-env=VMEXEC_AARCH64_PATH={}",
        vmexec_path.display()
    );
}

fn repo_root() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn package_root() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
}

fn pin_path() -> PathBuf {
    let packaged_pin = package_root().join("pin.toml");
    if packaged_pin.exists() {
        return packaged_pin;
    }
    repo_root().join("build-tools/build-vminitd/pin.toml")
}

fn resolve_runtime_path(artifact: RuntimeArtifact, vendored: bool, pin: &Pin) -> PathBuf {
    if let Some(path) = env::var_os(artifact.override_env()) {
        return PathBuf::from(path);
    }

    let root = repo_root();
    let vendored_path = root
        .join("vendor/vminitd")
        .join(&pin.target)
        .join(artifact.file_name());
    if vendored {
        if vendored_path.exists() {
            return vendored_path;
        }
        panic!(
            "vendored-vminitd is enabled but {} does not exist",
            vendored_path.display()
        );
    }

    let local_build = root.join("vminitd/bin").join(artifact.file_name());
    if local_build.exists() {
        return local_build;
    }

    let cache_path = cargo_target_dir()
        .join("firkin-vminitd")
        .join(artifact.sha256(pin))
        .join(&pin.target)
        .join(artifact.file_name());
    if cache_path.exists() {
        return cache_path;
    }

    let url = artifact.url(pin);
    assert!(
        !url.is_empty(),
        "no {} artifact found; set {}, build vminitd/bin/{}, or add a real {} URL to build-tools/build-vminitd/pin.toml",
        artifact.file_name(),
        artifact.override_env(),
        artifact.file_name(),
        artifact.pin_url_key()
    );

    download_runtime_artifact(artifact, url, &cache_path, artifact.sha256(pin));
    cache_path
}

fn cargo_target_dir() -> PathBuf {
    env::var_os("CARGO_TARGET_DIR").map_or_else(|| repo_root().join("target"), PathBuf::from)
}

fn sha256_file(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap_or_else(|error| {
        panic!("open {}: {error}", path.display());
    });
    let mut hasher = Sha256::new();
    let mut buf = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf).unwrap_or_else(|error| {
            panic!("read {}: {error}", path.display());
        });
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    format!("{:x}", hasher.finalize())
}

fn download_runtime_artifact(
    artifact: RuntimeArtifact,
    url: &str,
    cache_path: &Path,
    expected_sha256: &str,
) {
    let parent = cache_path
        .parent()
        .unwrap_or_else(|| panic!("{} has no parent", cache_path.display()));
    fs::create_dir_all(parent).unwrap_or_else(|error| {
        panic!("create {}: {error}", parent.display());
    });
    let partial = cache_path.with_extension("part");
    let response = ureq::get(url)
        .call()
        .unwrap_or_else(|error| panic!("download {} from {url}: {error}", artifact.file_name()));
    let mut input = response.into_reader();
    let mut output = fs::File::create(&partial).unwrap_or_else(|error| {
        panic!("create {}: {error}", partial.display());
    });
    std::io::copy(&mut input, &mut output).unwrap_or_else(|error| {
        panic!("write {} from {url}: {error}", partial.display());
    });
    output.flush().unwrap_or_else(|error| {
        panic!("flush {}: {error}", partial.display());
    });
    drop(output);

    let actual = sha256_file(&partial);
    if actual != expected_sha256 {
        let _ = fs::remove_file(&partial);
        panic!(
            "{} SHA-256 mismatch for {url}: expected {expected_sha256}, got {actual}",
            artifact.file_name()
        );
    }
    fs::rename(&partial, cache_path).unwrap_or_else(|error| {
        panic!(
            "move {} to {}: {error}",
            partial.display(),
            cache_path.display()
        );
    });
}

#[derive(Debug)]
struct Pin {
    revision: String,
    target: String,
    sha256: String,
    vmexec_sha256: String,
    vminitd_url: String,
    vmexec_url: String,
}

impl Pin {
    fn read(path: &Path) -> Self {
        let text = fs::read_to_string(path).unwrap_or_else(|error| {
            panic!("read {}: {error}", path.display());
        });
        Self {
            revision: required_pin_value(&text, "revision"),
            target: required_pin_value(&text, "target"),
            sha256: required_pin_value(&text, "sha256"),
            vmexec_sha256: required_pin_value(&text, "vmexec_sha256"),
            vminitd_url: required_pin_value(&text, "vminitd_url"),
            vmexec_url: required_pin_value(&text, "vmexec_url"),
        }
    }
}

#[derive(Clone, Copy)]
enum RuntimeArtifact {
    Vminitd,
    Vmexec,
}

impl RuntimeArtifact {
    fn file_name(self) -> &'static str {
        match self {
            Self::Vminitd => "vminitd",
            Self::Vmexec => "vmexec",
        }
    }

    fn override_env(self) -> &'static str {
        match self {
            Self::Vminitd => "FIRKIN_VMINITD_PATH",
            Self::Vmexec => "FIRKIN_VMEXEC_PATH",
        }
    }

    fn sha256(self, pin: &Pin) -> &str {
        match self {
            Self::Vminitd => &pin.sha256,
            Self::Vmexec => &pin.vmexec_sha256,
        }
    }

    fn url(self, pin: &Pin) -> &str {
        match self {
            Self::Vminitd => &pin.vminitd_url,
            Self::Vmexec => &pin.vmexec_url,
        }
    }

    fn pin_url_key(self) -> &'static str {
        match self {
            Self::Vminitd => "vminitd_url",
            Self::Vmexec => "vmexec_url",
        }
    }
}

fn required_pin_value(text: &str, key: &str) -> String {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .find_map(|(candidate, value)| {
            (candidate.trim() == key).then(|| value.trim().trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| panic!("pin.toml missing {key}"))
}
