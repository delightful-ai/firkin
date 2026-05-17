---
name: firkin-external-consumer
description: Use when verifying Firkin can be installed and used from a fresh external Rust project without local path dependencies.
---

# Firkin External Consumer

Use this skill after publishing Firkin crates or when checking whether a normal
user can install and compile Firkin from crates.io.

## Goal

Prove a fresh project outside the Firkin workspace can resolve, build, and use
Firkin from crates.io. Do not use `path =`, `patch.crates-io`, or local git
dependencies for the proof.

## Smoke Project

```bash
rm -rf /tmp/firkin-consumer-smoke
cargo new /tmp/firkin-consumer-smoke --bin
cd /tmp/firkin-consumer-smoke
cargo add firkin@0.0.3
cargo add tokio@1 --features macros,rt-multi-thread
```

Use a compile-only smoke that does not boot a VM:

```rust
use firkin::{Rootfs, RuntimeCapabilities, RuntimeCapability};
use firkin::types::{ContainerId, Size};

static SMOKE_CAPABILITIES: &[RuntimeCapability] = &[
    RuntimeCapability::supported("compile", Some("consumer smoke")),
];

fn main() {
    let id = ContainerId::new("agent-1").unwrap();
    let rootfs = Rootfs::raw_block("/tmp/rootfs.img");
    let caps = RuntimeCapabilities::new("external-smoke", SMOKE_CAPABILITIES, &[]);

    assert_eq!(id.as_str(), "agent-1");
    assert!(matches!(rootfs, Rootfs::RawBlock { .. }));
    assert!(caps.supports("compile"));
    assert_eq!(Size::mib(1).as_bytes(), 1024 * 1024);
}
```

Then run:

```bash
cargo check
cargo run
```

## CLI Install Smoke

The `fk` CLI crate is not published in `0.0.3`; it is built from the repo
release workflow. Do not use `cargo install firkin-cli` as a success criterion
for this alpha unless `publish = false` is intentionally removed later.
