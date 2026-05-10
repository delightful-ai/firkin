#!/usr/bin/env bash
set -euo pipefail

suite="${1:-agent-core}"
duration="${FIRKIN_BASELINE_DURATION:-60s}"
size_gib="${FIRKIN_RAMDISK_SIZE_GIB:-32}"
volume_name="${FIRKIN_RAMDISK_VOLUME_NAME:-FIRKINRAM}"
source_state="${FIRKIN_RAMDISK_SOURCE_STATE_DIR:-$HOME/.firkin/state}"
local_out_dir="${FIRKIN_RAMDISK_EVIDENCE_DIR:-target/firkin-live-evidence/ramdisk}"
name="${FIRKIN_BASELINE_NAME:-local-${suite}-${duration}-ramdisk-release}"
min_free_bytes="${FIRKIN_BASELINE_MIN_FREE_BYTES:-21474836480}"
working_set_bytes="${FIRKIN_RAMDISK_WORKING_SET_BYTES:-17179869184}"
preflight_only="${FIRKIN_RAMDISK_PREFLIGHT_ONLY:-0}"

if [[ ! -d "$source_state" ]]; then
  echo "ramdisk_source_state_missing=$source_state" >&2
  exit 1
fi

if [[ "$size_gib" =~ [^0-9] || "$size_gib" -le 0 ]]; then
  echo "invalid_ramdisk_size_gib=$size_gib" >&2
  exit 1
fi

if [[ "$min_free_bytes" =~ [^0-9] || "$min_free_bytes" -le 0 ]]; then
  echo "invalid_baseline_min_free_bytes=$min_free_bytes" >&2
  exit 1
fi

if [[ "$working_set_bytes" =~ [^0-9] || "$working_set_bytes" -lt 0 ]]; then
  echo "invalid_ramdisk_working_set_bytes=$working_set_bytes" >&2
  exit 1
fi

available_bytes_for() {
  df -Pk "$1" | awk 'NR == 2 { print $4 * 1024 }'
}

source_state_bytes="$(du -sk "$source_state" | awk '{ print $1 * 1024 }')"
ramdisk_size_bytes="$((size_gib * 1073741824))"
ramdisk_required_bytes="$((min_free_bytes + working_set_bytes))"
ramdisk_recommended_bytes="$((source_state_bytes + ramdisk_required_bytes))"
ramdisk_recommended_gib="$(((ramdisk_recommended_bytes + 1073741824 - 1) / 1073741824))"

emit_preflight() {
  cat <<EOF
ramdisk_preflight=signed-live-baseline
suite=$suite
duration=$duration
baseline_name=$name
ramdisk_size_gib=$size_gib
ramdisk_size_bytes=$ramdisk_size_bytes
source_state=$source_state
source_state_bytes=$source_state_bytes
baseline_min_free_bytes=$min_free_bytes
ramdisk_working_set_bytes=$working_set_bytes
ramdisk_required_available_after_state_copy_bytes=$ramdisk_required_bytes
ramdisk_recommended_size_gib=$ramdisk_recommended_gib
ramdisk_obviously_too_small=$([[ "$ramdisk_size_bytes" -lt "$ramdisk_recommended_bytes" ]] && echo true || echo false)
will_build_release=true
will_run_signed_live=true
EOF
}

if [[ "$preflight_only" == "1" ]]; then
  emit_preflight
  exit 0
fi

device=""
mounted=""

cleanup() {
  if [[ "${FIRKIN_RAMDISK_KEEP:-0}" == "1" ]]; then
    if [[ -n "$mounted" ]]; then
      echo "ramdisk_kept=$mounted"
    fi
    return
  fi
  if [[ -n "$device" ]]; then
    hdiutil detach "$device" >/dev/null || true
  fi
}
trap cleanup EXIT

sectors=$((size_gib * 2097152))
device="$(hdiutil attach -nomount "ram://$sectors" | awk 'NF { print $1; exit }')"
if [[ -z "$device" ]]; then
  echo "ramdisk_device_missing" >&2
  exit 1
fi
diskutil erasevolume APFS "$volume_name" "$device" >/dev/null
mounted="$(diskutil info -plist "/Volumes/$volume_name" | python3 -c 'import plistlib, sys; print(plistlib.loads(sys.stdin.buffer.read()).get("MountPoint") or "")')"
if [[ -z "$mounted" || ! -d "$mounted" ]]; then
  echo "ramdisk_mount_missing=$device" >&2
  exit 1
fi

state_root="$mounted/firkin/state"
benchmark_root="$mounted/firkin/benchmarks"
evidence_root="$mounted/firkin/evidence"
mkdir -p "$(dirname "$state_root")" "$benchmark_root" "$evidence_root"
rsync -a --delete "$source_state/" "$state_root/"

ramdisk_available_bytes="$(available_bytes_for "$mounted")"
if (( ramdisk_available_bytes < ramdisk_required_bytes )); then
  cat >&2 <<EOF
ramdisk_capacity=insufficient
ramdisk_mount=$mounted
ramdisk_size_gib=$size_gib
ramdisk_available_bytes=$ramdisk_available_bytes
ramdisk_required_bytes=$ramdisk_required_bytes
baseline_min_free_bytes=$min_free_bytes
ramdisk_working_set_bytes=$working_set_bytes
next_action=increase FIRKIN_RAMDISK_SIZE_GIB or lower FIRKIN_RAMDISK_WORKING_SET_BYTES only for a non-representative wiring smoke
EOF
  exit 1
fi

mkdir -p "$local_out_dir"
ramdisk_out="$evidence_root/${name}.json"

FIRKIN_STATE_DIR="$state_root" \
  FIRKIN_BENCHMARK_DIR="$benchmark_root" \
  FIRKIN_BASELINE_NAME="$name" \
  FIRKIN_BASELINE_OUT="$ramdisk_out" \
  scripts/run-firkin-decision-baseline.sh "$suite"

rsync -a --exclude 'restore-staging/' --exclude '*.tmp' "$evidence_root/" "$local_out_dir/"
if [[ -d "$benchmark_root/baselines" ]]; then
  mkdir -p "$local_out_dir/baselines"
  rsync -a --exclude 'restore-staging/' --exclude '*.tmp' "$benchmark_root/baselines/" "$local_out_dir/baselines/"
fi

ramdisk_index="$local_out_dir/${name}.ramdisk-artifacts.txt"
{
  echo "ramdisk_artifact_index=$ramdisk_index"
  echo "ramdisk_baseline_artifact=$local_out_dir/${name}.json"
  echo "ramdisk_baseline_storage_context=$local_out_dir/${name}.storage.txt"
  for artifact in "$local_out_dir/${name}"* "$local_out_dir/baselines/${name}"*; do
    [[ -e "$artifact" ]] || continue
    echo "ramdisk_related_artifact=$artifact"
  done
} >"$ramdisk_index"

echo "ramdisk_baseline_artifact=$local_out_dir/${name}.json"
echo "ramdisk_baseline_storage_context=$local_out_dir/${name}.storage.txt"
echo "ramdisk_artifact_index=$ramdisk_index"
