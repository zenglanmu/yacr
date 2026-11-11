#!/usr/bin/env bash
# Headless Linux host smoke: run the real GTK/Wayland CAD host against a real
# fixture with a software Vulkan adapter, then gate the run's `[probe]` lines.
#
# Why this exists: the repository's compile/static CI cannot see the failure
# class where the native CAD surface is configured at one size and the render
# target at another (a persistent `TextureInvalid` refusal leaves the CAD region
# transparent). This runs the production `native-below` path under a headless
# Weston compositor, so the presenter really configures a Wayland surface and
# really presents frames.
#
# Software rendering boundary: the run uses lavapipe (or another CPU Vulkan
# adapter, allowed by `YACR_PROBE_ALLOW_CPU_ADAPTER=1`) and WebKitGTK's
# non-DMABUF path. This is a **presentation/geometry gate**, not GPU, pixel,
# performance or real-device evidence.
#
# Usage:
#   bash scripts/host-smoke-linux.sh [--binary PATH] [--dwg PATH] [--fonts DIR]
#                                    [--timeout SECONDS] [--evidence DIR] [--keep]
#
# Defaults: target/release/yacr, fixtures/dwg/synthetic-four-lines.dwg, fonts/,
# 30 seconds. The caller must build the release binary with the embedded UI:
#   cargo build --release -p app-linux --bin yacr \
#       --features tauri/custom-protocol --locked
#
# Exit: 0 = pass, 1 = gate failure, 2 = usage/precondition error.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
binary="$root/target/release/yacr"
dwg="$root/fixtures/dwg/synthetic-four-lines.dwg"
fonts="$root/fonts"
timeout_seconds=30
evidence=""
keep=0

usage() {
  sed -n '2,26p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary) binary="$2"; shift 2 ;;
    --dwg) dwg="$2"; shift 2 ;;
    --fonts) fonts="$2"; shift 2 ;;
    --timeout) timeout_seconds="$2"; shift 2 ;;
    --evidence) evidence="$2"; shift 2 ;;
    --keep) keep=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "host-smoke-linux: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

fail() { echo "host-smoke-linux: FAIL: $*" >&2; exit 1; }

[[ -x "$binary" ]] || fail "release binary not found/executable: $binary (build it first, see --help)"
[[ -f "$dwg" ]] || fail "drawing fixture not found: $dwg"
[[ -f "$fonts/fonts.json" ]] || fail "font package not found: $fonts/fonts.json"
command -v weston >/dev/null 2>&1 || fail "weston is required (headless compositor); install it first"
command -v python3 >/dev/null 2>&1 || fail "python3 is required for the log gate"
[[ "$timeout_seconds" =~ ^[0-9]+$ ]] && (( timeout_seconds > 0 )) || fail "--timeout must be a positive integer"
(( timeout_seconds >= 5 )) || fail "--timeout must be at least 5 seconds"

work=$(mktemp -d "${TMPDIR:-/tmp}/yacr-host-smoke.XXXXXX")
runtime="$work/runtime"
socket="wayland-yacr-smoke-$$"
weston_pid=""
cleanup() {
  if [[ -n "$weston_pid" ]]; then
    kill "$weston_pid" 2>/dev/null || true
    wait "$weston_pid" 2>/dev/null || true
  fi
  if (( keep )); then
    echo "host-smoke-linux: kept $work" >&2
  else
    rm -rf "$work"
  fi
}
trap cleanup EXIT

mkdir -p "$runtime" "$work/home/.config" "$work/home/.local/share" "$work/home/.cache"
chmod 700 "$runtime"
host_log="$work/host.log"
weston_log="$work/weston.log"

XDG_RUNTIME_DIR="$runtime" weston \
  --backend=headless-backend.so \
  --socket="$socket" \
  --width=1280 --height=800 \
  --idle-time=0 >"$weston_log" 2>&1 &
weston_pid=$!

# Wait for the compositor socket (bounded); a dead process aborts immediately.
for _ in $(seq 1 100); do
  [[ -S "$runtime/$socket" ]] && break
  kill -0 "$weston_pid" 2>/dev/null || { cat "$weston_log" >&2; fail "weston exited before its socket appeared"; }
  sleep 0.1
done
[[ -S "$runtime/$socket" ]] || { cat "$weston_log" >&2; fail "weston did not create $socket"; }

set +e
XDG_RUNTIME_DIR="$runtime" \
WAYLAND_DISPLAY="$socket" \
GDK_BACKEND=wayland \
HOME="$work/home" \
XDG_CONFIG_HOME="$work/home/.config" \
XDG_DATA_HOME="$work/home/.local/share" \
XDG_CACHE_HOME="$work/home/.cache" \
WEBKIT_DISABLE_DMABUF_RENDERER=1 \
YACR_PROBE_ALLOW_CPU_ADAPTER=1 \
YACR_PROBE_CAD_LOG=1 \
YACR_PROBE_NO_MAXIMIZE=1 \
  timeout --signal=INT "$timeout_seconds" "$binary" --open "$dwg" --fonts-dir "$fonts" \
  >"$host_log" 2>&1
host_status=$?
set -e

if [[ -n "$evidence" ]]; then
  mkdir -p "$evidence"
  cp "$host_log" "$evidence/host.log"
  cp "$weston_log" "$evidence/weston.log"
  printf '%s\n' "$binary" "$dwg" "$fonts" "$timeout_seconds" >"$evidence/parameters.txt"
fi

echo "host-smoke-linux: host exited with $host_status after ${timeout_seconds}s"
case "$host_status" in
  124|130) ;; # timeout / SIGINT: the bounded run ended as intended
  *) fail "host exited before the bounded run (status $host_status); see $host_log" ;;
esac

if ! python3 "$root/scripts/host-smoke-check.py" "$host_log"; then
  echo "host-smoke-linux: last host lines:" >&2
  tail -n 40 "$host_log" >&2
  exit 1
fi

echo "host-smoke-linux: PASS (headless Wayland + software Vulkan; presentation/geometry only)"
