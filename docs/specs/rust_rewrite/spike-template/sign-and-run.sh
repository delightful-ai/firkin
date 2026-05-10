#!/usr/bin/env bash
# Build + ad-hoc-sign + run the spike. Re-signs on every run because cargo
# overwrites the binary (stripping its signature).
#
# Usage:
#   ./sign-and-run.sh                         # debug build, unbounded run
#   PROFILE=release ./sign-and-run.sh         # release build
#   SPIKE_TIMEOUT_SECS=10 ./sign-and-run.sh   # bounded run (watchdog)
#
# When SPIKE_TIMEOUT_SECS is set and the binary hasn't exited by then, we
# SIGTERM → SIGKILL it and treat 143 (SIGTERM) / 137 (SIGKILL) as SUCCESS.
# Use this for long-running guests (e.g. vminitd) where "reached a stable
# state and is still running" is the acceptance criterion.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

PROFILE="${PROFILE:-debug}"
if [[ "$PROFILE" == "release" ]]; then
    cargo build --release
    TARGET_DIR="target/release"
else
    cargo build
    TARGET_DIR="target/debug"
fi

# Locate the binary. Crate name != dir name after scaffold, so search.
BIN=""
if [[ -x "$TARGET_DIR/$(basename "$HERE")" ]]; then
    BIN="$TARGET_DIR/$(basename "$HERE")"
else
    BIN=$(find "$TARGET_DIR" -maxdepth 1 -type f -perm -u+x ! -name '*.d' | head -1)
fi
[[ -z "$BIN" ]] && { echo "No built binary found under $TARGET_DIR/"; exit 1; }

codesign --force --sign - --entitlements "$HERE/entitlements.plist" "$BIN"
echo "[sign] ad-hoc signed $BIN with $HERE/entitlements.plist"

# Unbounded: just exec and let the binary decide when to exit.
if [[ -z "${SPIKE_TIMEOUT_SECS:-}" ]]; then
    exec "$BIN" "$@"
fi

# Bounded: spawn, watchdog, translate SIGTERM/SIGKILL-at-timeout into success.
"$BIN" "$@" &
PID=$!
TIMEOUT="$SPIKE_TIMEOUT_SECS"
echo "[run] pid=$PID, bounding to ${TIMEOUT}s"

# Sleep as a background job so we can race it against the binary exiting.
( sleep "$TIMEOUT"; kill -TERM "$PID" 2>/dev/null ) &
SLEEPER=$!

# Wait for the binary. `set -e` would abort on any non-zero exit (including
# 143 SIGTERM, which is the normal watchdog path), so capture explicitly.
# NB: `if ! wait; then RC=$?; fi` does NOT work — `!` inverts the exit so
# `$?` in the `then` branch is always 0. Use `|| RC=$?` instead.
RC=0
wait "$PID" || RC=$?

# Cancel the sleeper if the binary finished first.
kill "$SLEEPER" 2>/dev/null || true
wait "$SLEEPER" 2>/dev/null || true

# Give it a moment to settle, then SIGKILL if still stuck.
if kill -0 "$PID" 2>/dev/null; then
    sleep 1
    kill -KILL "$PID" 2>/dev/null || true
    wait "$PID" 2>/dev/null || true
    RC=137
fi

# 143 = SIGTERM, 137 = SIGKILL. Both mean "watchdog fired; process was alive".
if [[ "$RC" == "143" || "$RC" == "137" ]]; then
    echo "[run] spike killed at timeout — process was running (success)"
    exit 0
fi
exit "$RC"
