#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
pin_file="$script_dir/pin.toml"
target_root="${CARGO_TARGET_DIR:-$repo_root/target}/firkin-vminitd"

pin_value() {
    local key="$1"
    awk -F '=' -v key="$key" '
        $1 ~ "^[[:space:]]*" key "[[:space:]]*$" {
            value = $2
            sub(/^[[:space:]]*/, "", value)
            sub(/[[:space:]]*$/, "", value)
            gsub(/^"|"$/, "", value)
            print value
            found = 1
        }
        END { if (!found) exit 1 }
    ' "$pin_file"
}

sha256_file() {
    local path="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$path" | awk '{print $1}'
    else
        shasum -a 256 "$path" | awk '{print $1}'
    fi
}

install_local_artifact() {
    local name="$1"
    local source_path="$2"
    local cache_path="$3"
    local expected_sha="$4"

    test -f "$source_path" || {
        echo "$name source does not exist: $source_path" >&2
        exit 1
    }

    local actual
    actual="$(sha256_file "$source_path")"
    if [[ "$actual" != "$expected_sha" ]]; then
        echo "$name SHA-256 mismatch for $source_path: expected $expected_sha, got $actual" >&2
        exit 1
    fi

    mkdir -p "$(dirname "$cache_path")"
    cp "$source_path" "$cache_path.part"
    chmod 0755 "$cache_path.part"
    mv "$cache_path.part" "$cache_path"
    echo "$cache_path"
}

fetch_artifact() {
    local name="$1"
    local expected_sha="$2"
    local url="$3"
    local target="$4"
    local cache_path="$target_root/$expected_sha/$target/$name"
    local override_var

    case "$name" in
        vminitd) override_var="FIRKIN_VMINITD_PATH" ;;
        vmexec) override_var="FIRKIN_VMEXEC_PATH" ;;
        *) echo "unknown runtime artifact: $name" >&2; exit 1 ;;
    esac

    local override_path="${!override_var:-}"
    if [[ -n "$override_path" ]]; then
        install_local_artifact "$name" "$override_path" "$cache_path" "$expected_sha"
        return
    fi

    if [[ -f "$cache_path" ]]; then
        local actual
        actual="$(sha256_file "$cache_path")"
        [[ "$actual" == "$expected_sha" ]] || {
            echo "$name cache SHA-256 mismatch: expected $expected_sha, got $actual" >&2
            exit 1
        }
        echo "$cache_path"
        return
    fi

    if [[ -z "$url" ]]; then
        echo "no URL pinned for $name in $pin_file" >&2
        echo "set $override_var or pin a real release asset URL" >&2
        exit 1
    fi

    mkdir -p "$(dirname "$cache_path")"
    curl -fL --retry 3 --retry-delay 1 -o "$cache_path.part" "$url"
    local actual
    actual="$(sha256_file "$cache_path.part")"
    if [[ "$actual" != "$expected_sha" ]]; then
        rm -f "$cache_path.part"
        echo "$name SHA-256 mismatch for $url: expected $expected_sha, got $actual" >&2
        exit 1
    fi
    chmod 0755 "$cache_path.part"
    mv "$cache_path.part" "$cache_path"
    echo "$cache_path"
}

target="$(pin_value target)"
fetch_artifact vminitd "$(pin_value sha256)" "$(pin_value vminitd_url)" "$target"
fetch_artifact vmexec "$(pin_value vmexec_sha256)" "$(pin_value vmexec_url)" "$target"
