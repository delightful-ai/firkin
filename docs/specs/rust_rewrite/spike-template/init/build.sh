#!/usr/bin/env bash
# Build a static arm64 musl init and pack it as a cpio initrd. No local
# cross-compiler needed — we shell out to docker/alpine.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ASSETS="$HERE/../assets"
mkdir -p "$ASSETS"

docker run --rm --platform linux/arm64 -v "$HERE:/src" -v "$ASSETS:/out" \
    alpine:3.20 sh -c '
set -eux
apk add --no-cache musl-dev gcc make cpio linux-headers >/dev/null
cd /src
gcc -static -Os -s -o /tmp/init init.c
mkdir -p /tmp/initroot
cp /tmp/init /tmp/initroot/init
cd /tmp/initroot
find . | cpio -o -H newc > /out/initrd.cpio
ls -lh /out/initrd.cpio
'
