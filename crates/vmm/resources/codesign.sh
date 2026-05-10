#!/usr/bin/env sh
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: $0 <binary>" >&2
    exit 64
fi

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
/usr/bin/codesign \
    --force \
    --sign - \
    --timestamp=none \
    --entitlements "$SCRIPT_DIR/entitlements.plist" \
    "$1"
