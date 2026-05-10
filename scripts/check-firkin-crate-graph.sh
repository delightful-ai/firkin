#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

failed=0

check_absent_path() {
  local path="$1"

  if [[ -e "$path" ]]; then
    printf 'deleted Firkin topology path is present: %s\n' "$path" >&2
    failed=1
  fi
}

check_crate_exists() {
  local crate="$1"
  local manifest="crates/${crate}/Cargo.toml"

  if [[ ! -f "$manifest" ]]; then
    printf 'expected Firkin crate manifest is missing: %s\n' "$manifest" >&2
    failed=1
    return 1
  fi
}

check_no_normal_dep() {
  local crate="$1"
  local dep="$2"
  local manifest="crates/${crate}/Cargo.toml"

  check_crate_exists "$crate" || return

  if awk '
    /^\[dependencies\]$/ { in_deps = 1; next }
    /^\[/ { in_deps = 0 }
    in_deps { print }
  ' "$manifest" | rg -q "^[[:space:]]*${dep}[[:space:]]*="; then
    printf 'forbidden normal Firkin crate dependency: %s -> %s\n' "$crate" "$dep" >&2
    failed=1
  fi
}

# Hard-cut split sentinels. These crates/modules must not come back as
# compatibility shims.
check_absent_path crates/substrate
check_absent_path crates/e2b
check_absent_path crates/runtime/src/single_node

# firkin-sandbox is the public law surface. It may name neutral primitive
# crates, but concrete runtime, compatibility, evidence, benchmark, and VM
# mechanics must never leak into it.
for dep in \
  firkin-admission \
  firkin-benchmark \
  firkin-core \
  firkin-e2b-contract \
  firkin-e2b-server \
  firkin-e2b-wire \
  firkin-evidence \
  firkin-ext4 \
  firkin-hygiene \
  firkin-oci \
  firkin-runtime \
  firkin-single-node \
  firkin-template \
  firkin-vminitd-bytes \
  firkin-vminitd-client \
  firkin-vmm \
  firkin-vsock
do
  check_no_normal_dep sandbox "$dep"
done

# firkin-envd is the neutral envd protocol/data-plane law surface. E2B may
# translate over it from server/runtime edges, but envd must not learn product,
# runtime, evidence, benchmark, or VM laws.
for dep in \
  firkin-admission \
  firkin-artifacts \
  firkin-benchmark \
  firkin-core \
  firkin-e2b-contract \
  firkin-e2b-server \
  firkin-e2b-wire \
  firkin-evidence \
  firkin-ext4 \
  firkin-hygiene \
  firkin-oci \
  firkin-runtime \
  firkin-sandbox \
  firkin-single-node \
  firkin-template \
  firkin-trace \
  firkin-types \
  firkin-vminitd-bytes \
  firkin-vminitd-client \
  firkin-vmm \
  firkin-vsock
do
  check_no_normal_dep envd "$dep"
done

# firkin-types and firkin-trace are leaf-like primitive crates.
for dep in \
  firkin-admission \
  firkin-artifacts \
  firkin-benchmark \
  firkin-core \
  firkin-e2b-contract \
  firkin-e2b-server \
  firkin-e2b-wire \
  firkin-evidence \
  firkin-ext4 \
  firkin-hygiene \
  firkin-oci \
  firkin-runtime \
  firkin-single-node \
  firkin-template \
  firkin-trace \
  firkin-vminitd-bytes \
  firkin-vminitd-client \
  firkin-vmm \
  firkin-vsock
do
  check_no_normal_dep types "$dep"
done

for dep in \
  firkin-admission \
  firkin-artifacts \
  firkin-benchmark \
  firkin-core \
  firkin-e2b-contract \
  firkin-e2b-server \
  firkin-e2b-wire \
  firkin-evidence \
  firkin-ext4 \
  firkin-hygiene \
  firkin-oci \
  firkin-runtime \
  firkin-single-node \
  firkin-vminitd-client \
  firkin-vmm \
  firkin-vsock
do
  check_no_normal_dep trace "$dep"
done

# firkin-evidence validates claims and artifacts. It consumes trace samples but
# must not run benchmarks, operate VMs, or own runtime policy.
for dep in \
  firkin-admission \
  firkin-artifacts \
  firkin-benchmark \
  firkin-core \
  firkin-e2b-contract \
  firkin-e2b-server \
  firkin-e2b-wire \
  firkin-hygiene \
  firkin-runtime \
  firkin-single-node \
  firkin-template
do
  check_no_normal_dep evidence "$dep"
done

# firkin-template owns host checkout/setup/cache-warm execution. VZ snapshot
# mechanics belong in runtime/single-node composition crates.
for dep in \
  firkin-benchmark \
  firkin-core \
  firkin-e2b-contract \
  firkin-e2b-server \
  firkin-e2b-wire \
  firkin-evidence \
  firkin-runtime \
  firkin-single-node \
  firkin-vminitd-client \
  firkin-vmm \
  firkin-vsock
do
  check_no_normal_dep template "$dep"
done

# firkin-core owns VM/container mechanics. It stays below production scheduling,
# template service policy, evidence, benchmark, and E2B/Cube API semantics.
for dep in \
  firkin-benchmark \
  firkin-e2b-contract \
  firkin-e2b-server \
  firkin-e2b-wire \
  firkin-evidence \
  firkin-runtime \
  firkin-single-node \
  firkin-template
do
  check_no_normal_dep core "$dep"
done

# firkin-runtime composes runtime operations, but one-host Apple/VZ backend
# composition and evidence/benchmark policy stay above it.
for dep in \
  firkin-benchmark \
  firkin-evidence \
  firkin-single-node
do
  check_no_normal_dep runtime "$dep"
done

# Split E2B crates keep wire DTOs, runtime-facing contracts, and local servers
# separate.
for dep in \
  firkin-admission \
  firkin-artifacts \
  firkin-benchmark \
  firkin-core \
  firkin-e2b-contract \
  firkin-e2b-server \
  firkin-envd \
  firkin-evidence \
  firkin-ext4 \
  firkin-hygiene \
  firkin-oci \
  firkin-runtime \
  firkin-single-node \
  firkin-template \
  firkin-vminitd-client \
  firkin-vmm \
  firkin-vsock
do
  check_no_normal_dep e2b-wire "$dep"
done

for dep in \
  firkin-admission \
  firkin-artifacts \
  firkin-benchmark \
  firkin-core \
  firkin-e2b-server \
  firkin-envd \
  firkin-evidence \
  firkin-ext4 \
  firkin-hygiene \
  firkin-oci \
  firkin-runtime \
  firkin-single-node \
  firkin-template \
  firkin-vminitd-client \
  firkin-vmm \
  firkin-vsock
do
  check_no_normal_dep e2b-contract "$dep"
done

# firkin-benchmark is high in the graph. Runtime/library crates below it must
# never depend on benchmark policy.
for crate in \
  admission \
  artifacts \
  core \
  e2b-contract \
  e2b-server \
  e2b-wire \
  envd \
  evidence \
  ext4 \
  hygiene \
  oci \
  runtime \
  sandbox \
  single-node \
  template \
  trace \
  types \
  vminitd-bytes \
  vminitd-client \
  vmm \
  vsock
do
  check_no_normal_dep "$crate" firkin-benchmark
done

exit "$failed"
