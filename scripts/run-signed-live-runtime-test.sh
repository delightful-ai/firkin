#!/usr/bin/env bash
set -euo pipefail

package="firkin-runtime"
features="snapshot"
test_binary="live_snapshot_restore"
profile="debug"
build=true

usage() {
    echo "usage: $0 [--package <crate>] [--features <features>] [--test <integration-test-binary>] [--profile debug|release] [--no-build] <ignored-test-name> [test-args...]" >&2
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --package)
            if [[ $# -lt 2 ]]; then
                usage
                exit 64
            fi
            package="$2"
            shift 2
            ;;
        --features)
            if [[ $# -lt 2 ]]; then
                usage
                exit 64
            fi
            features="$2"
            shift 2
            ;;
        --test)
            if [[ $# -lt 2 ]]; then
                usage
                exit 64
            fi
            test_binary="$2"
            shift 2
            ;;
        --profile)
            if [[ $# -lt 2 ]]; then
                usage
                exit 64
            fi
            case "$2" in
                debug|release)
                    profile="$2"
                    ;;
                *)
                    usage
                    exit 64
                    ;;
            esac
            shift 2
            ;;
        --release)
            profile="release"
            shift
            ;;
        --no-build)
            build=false
            shift
            ;;
        --)
            shift
            break
            ;;
        -*)
            usage
            exit 64
            ;;
        *)
            break
            ;;
    esac
done

if [[ $# -lt 1 ]]; then
    usage
    exit 64
fi

test_name="$1"
shift

if [[ "${build}" == true ]]; then
    cargo_args=(test -q -p "${package}")
    if [[ -n "${features}" ]]; then
        cargo_args+=(--features "${features}")
    fi
    cargo_args+=(--test "${test_binary}")
    if [[ "${profile}" == "release" ]]; then
        cargo_args+=(--release)
    fi
    cargo_args+=(--no-run --message-format=json)
    cargo_json="$(mktemp)"
    cargo "${cargo_args[@]}" | tee "${cargo_json}" >/dev/null
    test_bin="$(
        python3 - "${test_binary}" "${cargo_json}" <<'PY'
import json
import sys

test_binary = sys.argv[1]
path = sys.argv[2]
selected = None
with open(path, "r", encoding="utf-8") as handle:
    for line in handle:
        line = line.strip()
        if not line:
            continue
        message = json.loads(line)
        target = message.get("target") or {}
        if (
            message.get("reason") == "compiler-artifact"
            and target.get("name") == test_binary
            and "test" in target.get("kind", [])
            and message.get("executable")
        ):
            selected = message["executable"]
if selected:
    print(selected)
PY
    )"
    rm -f "${cargo_json}"
else
    target_dir="${CARGO_TARGET_DIR:-target}"
    profile_dir="${target_dir}/${profile}"
    test_bin=""
    while IFS= read -r candidate; do
        test_list="$("${candidate}" --list --include-ignored 2>/dev/null || true)"
        if grep -Fq "${test_name}: " <<<"${test_list}"; then
            test_bin="${candidate}"
            break
        fi
    done < <(
        find "${profile_dir}/deps" -maxdepth 1 -type f -perm -111 -name "${test_binary}-*" \
            -exec stat -f '%m %N' {} \; \
            | sort -nr \
            | awk '{print $2}'
    )
fi

if [[ -z "${test_bin}" ]]; then
    echo "failed to locate ${profile} ${test_binary} test binary" >&2
    exit 1
fi

/usr/bin/codesign --force --sign - --timestamp=none \
    --entitlements signing/vz.entitlements \
    "${test_bin}"

/usr/bin/codesign -d --entitlements :- "${test_bin}" 2>&1 \
    | grep -E 'Executable=|com.apple.security.virtualization'

"${test_bin}" "${test_name}" --include-ignored --exact --nocapture "$@"
