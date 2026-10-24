#!/usr/bin/env bash
#
# collect-evidence.sh - automated evidence-collection runner for the Linux GUI host.
#
# It builds the React + Tauri Linux host, launches the real binary under its
# env-gated probe driver, captures screenshots and logs into a fresh timestamped
# output directory, and records honest per-backend results in results.tsv.
#
# Both Linux pixel carriers are collected as REAL runs (ADR 0024):
#   * `wayland`  -> `surfaceMode=native-below`: app-owned wl_subsurface below the
#                   WebView, drawn by the host's own wgpu presenter;
#   * `x11`      -> `surfaceMode=dom-canvas` (X11/XWayland): the wasm CAD session
#                   inside the WebView draws into a DOM <canvas>. There is no
#                   native CAD child window and no native CAD session.
#
# Screenshots of BOTH carriers come from the compositor's own ScreenCast stream
# (`scripts/screencast-capture.py`, same-UID Mutter), because on this desktop the
# X11 window is a normal composited toplevel: `x11grab` and `gnome-screenshot`'s
# X11 read-back return all black, so they are not used.
#
# Refusals are classified from their stable marker line first; the exit code is
# supporting evidence only:
#   * `yacr: unsupported-runtime: webgl2` (exit code 6) - CURRENT: the
#     `dom-canvas` carrier cannot draw because the WebView has no usable WebGL2;
#   * `yacr: unsupported-backend: x11` (exit code 4) - HISTORICAL (2026-10-22 ..
#     2026-10-24): X11 was refused before GTK init. Not produced any more; the
#     parser is kept so an old binary is still classified honestly instead of
#     being misread as a normal run.
#
# The runner stays inside the current desktop session:
#   * it runs as the invoking same-UID user and does not elevate for its own
#     actions (the 2026-10-23 user authorization permits elevation, root,
#     session unlocking and installs in this environment; none is needed here),
#   * it never unlocks or otherwise modifies the session,
#   * it never changes portal/ScreenCast permission state,
#   * it never fakes evidence: no virtual/headless display is presented as a real
#     session, and software rendering is never reported as a real GPU,
#   * it never uses the app's fake marker mode; the real `--open` path is used.
#
# If no real display session is available it exits non-zero with an explicit
# message instead of faking success. Screenshots stay local and are never
# uploaded anywhere.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

prog="$(basename "${BASH_SOURCE[0]}")"

usage() {
    cat <<USAGE
Usage: $prog [options]

Build the React + Tauri Linux host, launch the real binary under its env-gated
probe driver, capture screenshots and logs into a fresh output directory, and
record honest per-backend results in results.tsv.

Options:
  --backend <auto|x11|wayland|both>  Backends to run (default: both).
  --dwg <path>                       Drawing to open
                                     (default: fixtures/dwg/synthetic-four-lines.dwg).
  --fonts-dir <path>                 Fonts directory (default: <root>/fonts).
  --out <dir>                        Output directory (default: timestamped dir).
  --lifecycle <mode>                 Probe lifecycle mode (default: exit).
  --capture-at <seconds>             Seconds after launch to take screenshots
                                     (default: 10).
  --timeout <seconds>                Hard cap; kill the process group after this
                                     (default: 90).
  --no-build                         Skip the frontend and cargo release build.
  --maximize                         Do not set YACR_PROBE_NO_MAXIMIZE
                                     (default: no-maximize).
  --stacking-regression              Also run the DOM-on-top stacking regression
                                     fixture on Wayland (YACR_PROBE_STACKING=1 +
                                     the init-script flag + a real --open) and
                                     record a STACKING_REGRESSION row.
  --dry-run                          Print the resolved plan and exit without
                                     building or launching.
  -h, --help                         Show this help.

The runner runs as the invoking same-UID desktop user on a real display session.
Both Linux carriers -- native Wayland (native-below) and X11/XWayland (dom-canvas)
-- are real runs; screenshots of both come from the same-UID Mutter ScreenCast
stream.

The 2026-10-23 user authorization permits privilege escalation, root, session
unlocking and installs in this environment; this runner needs none of them and
still never fakes evidence: a virtual/headless display is never presented as a
real session and software rendering is never reported as a real GPU. If no real
display session is available it exits non-zero and says so. Screenshots stay local
and are never uploaded.
USAGE
}

# --------------------------------------------------------------------------
# Arguments
# --------------------------------------------------------------------------
backend="both"
dwg=""
fonts_dir=""
out_dir=""
lifecycle="exit"
capture_at="10"
timeout_s="90"
do_build=1
maximize=0
stacking_regression=0
dry_run=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --backend) backend="${2:-}"; shift 2 ;;
        --dwg) dwg="${2:-}"; shift 2 ;;
        --fonts-dir) fonts_dir="${2:-}"; shift 2 ;;
        --out) out_dir="${2:-}"; shift 2 ;;
        --lifecycle) lifecycle="${2:-}"; shift 2 ;;
        --capture-at) capture_at="${2:-}"; shift 2 ;;
        --timeout) timeout_s="${2:-}"; shift 2 ;;
        --no-build) do_build=0; shift ;;
        --maximize) maximize=1; shift ;;
        --stacking-regression) stacking_regression=1; shift ;;
        --dry-run) dry_run=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "error: unknown argument: $1" >&2; usage >&2; exit 64 ;;
    esac
done

case "$backend" in
    auto|x11|wayland|both) ;;
    *) echo "error: invalid --backend '$backend' (want auto|x11|wayland|both)" >&2; exit 64 ;;
esac
if ! [[ "$capture_at" =~ ^[0-9]+$ ]]; then
    echo "error: --capture-at must be a non-negative integer" >&2; exit 64
fi
if ! [[ "$timeout_s" =~ ^[0-9]+$ ]]; then
    echo "error: --timeout must be a non-negative integer" >&2; exit 64
fi

# --------------------------------------------------------------------------
# Paths and defaults
# --------------------------------------------------------------------------
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(git -C "$script_dir" rev-parse --show-toplevel)"

[[ -n "$dwg" ]] || dwg="$root/fixtures/dwg/synthetic-four-lines.dwg"
[[ -n "$fonts_dir" ]] || fonts_dir="$root/fonts"
binary="$root/target/release/yacr"
screencast="$root/scripts/screencast-capture.py"

out_root="${YACR_EVIDENCE_ROOT:-/tmp/opencode}"
[[ -n "$out_dir" ]] || out_dir="$out_root/yacr-evidence-$(date -u +%Y%m%dT%H%M%SZ)"

# --------------------------------------------------------------------------
# Session detection (never elevates, never unlocks, never forces a backend)
# --------------------------------------------------------------------------
uid="$(id -u)"
runtime_dir="${XDG_RUNTIME_DIR:-/run/user/$uid}"

x11_display=""
if [[ -n "${DISPLAY:-}" ]]; then
    x11_display="$DISPLAY"
elif [[ -e /tmp/.X11-unix/X0 ]]; then
    x11_display=":0"
fi

shopt -s nullglob
xauth_candidates=( "$runtime_dir"/.mutter-Xwaylandauth.* )
shopt -u nullglob
x11_xauthority=""
if (( ${#xauth_candidates[@]} > 0 )); then
    x11_xauthority="${xauth_candidates[0]}"
fi

wayland_display="${WAYLAND_DISPLAY:-wayland-0}"

x11_available=0
if [[ -n "$x11_display" ]]; then
    x11_available=1
fi
wayland_available=0
if [[ -e "$runtime_dir/$wayland_display" ]]; then
    wayland_available=1
fi

backend_available() {
    case "$1" in
        x11) [[ "$x11_available" -eq 1 ]] ;;
        wayland) [[ "$wayland_available" -eq 1 ]] ;;
        *) return 1 ;;
    esac
}

case "$backend" in
    x11) candidates=(x11) ;;
    wayland) candidates=(wayland) ;;
    auto|both) candidates=(x11 wayland) ;;
esac

# Build the launch environment for a backend into the global `env_prefix` array.
build_env_prefix() {
    local name="$1"
    env_prefix=()
    if [[ "$name" == "x11" ]]; then
        env_prefix=(env "GDK_BACKEND=x11" "DISPLAY=$x11_display")
        if [[ -n "$x11_xauthority" ]]; then
            env_prefix+=("XAUTHORITY=$x11_xauthority")
        fi
    else
        env_prefix=(env -u DISPLAY -u XAUTHORITY "GDK_BACKEND=wayland" \
            "WAYLAND_DISPLAY=$wayland_display" "XDG_RUNTIME_DIR=$runtime_dir")
    fi
    env_prefix+=("YACR_PROBE_CAD_LOG=1" "YACR_PROBE_LIFECYCLE=$lifecycle")
    if [[ "$maximize" -eq 0 ]]; then
        env_prefix+=("YACR_PROBE_NO_MAXIMIZE=1")
    fi
}

# --------------------------------------------------------------------------
# Refusal classification (evidence emitted on stderr, merged into run.log)
# --------------------------------------------------------------------------
# Stable marker lines. The marker line is the PRIMARY classification signal; the
# exit code is recorded as supporting evidence only (a driver may override it).
#
# `runtime` : the `dom-canvas` carrier cannot draw (no usable WebGL2), exit 6.
#             Current refusal; emitted after the WebView exists, from the
#             frontend's `canvas_runtime_failed` report.
# `backend` : the HISTORICAL pre-GTK X11 refusal, exit 4. Not produced since
#             ADR 0024 stage 4 (X11 now starts on `dom-canvas`); kept so an old
#             binary/build is still classified instead of being misread.
unsupported_runtime_marker="yacr: unsupported-runtime: webgl2"
unsupported_backend_marker="yacr: unsupported-backend: x11"

# detect_refusal <run.log> -> sets `refusal_kind` to runtime|backend|<empty>.
detect_refusal() {
    refusal_kind=""
    if grep -qxF "$unsupported_runtime_marker" "$1" 2>/dev/null; then
        refusal_kind="runtime"
    elif grep -qxF "$unsupported_backend_marker" "$1" 2>/dev/null; then
        refusal_kind="backend"
    fi
}

# record_refusal <name> <mode> <kind> <dir> <pgid> <screenshot> <extra note>
#
# A refusing session ends the process itself; the wrapper's exit code file is
# read (supporting only) and the process group is reaped. No screenshot is ever
# invented: the caller passes the real screenshot state, and a refused carrier
# yields `missing` plus the reason.
record_refusal() {
    local name="$1" mode="$2" kind="$3" dir="$4" pgid="$5"
    local screenshot="$6" extra="${7:-}"
    local wait0=$SECONDS
    while [[ ! -s "$dir/app.rc" && $((SECONDS - wait0)) -lt 5 ]]; do
        sleep 0.1
    done
    local code="-"
    if [[ -s "$dir/app.rc" ]]; then
        code="$(tr -dc '0-9' < "$dir/app.rc" | head -c 10)"
    fi
    [[ -n "$code" ]] || code="-"
    if kill -0 -"$pgid" 2>/dev/null; then
        kill -TERM -"$pgid" 2>/dev/null || true
    fi
    if [[ "$kind" == "runtime" ]]; then
        record_row "$name" "$mode" "UNSUPPORTED_RUNTIME" "$code" "$screenshot" "-" \
            "marker='$unsupported_runtime_marker' (expected exit 6); dom-canvas WebGL2 carrier cannot draw; ${extra:-no screenshot}"
        echo "[collect] $name/$mode: verdict=UNSUPPORTED_RUNTIME exit=$code (marker)"
    else
        record_row "$name" "$mode" "UNSUPPORTED_BACKEND" "$code" "$screenshot" "-" \
            "marker='$unsupported_backend_marker' (expected exit 4, HISTORICAL: not produced since ADR 0024 stage 4); ${extra:-no screenshot}"
        echo "[collect] $name/$mode: verdict=UNSUPPORTED_BACKEND exit=$code (historical marker)"
    fi
    any_ran=1
    ran_backends+=("$name/$mode")
}

# --------------------------------------------------------------------------
# Dry run: print the resolved plan and stop.
# --------------------------------------------------------------------------
if [[ "$dry_run" -eq 1 ]]; then
    echo "dry-run: resolved plan (nothing is built or launched)"
    echo "  repo root      : $root"
    echo "  output root    : $out_dir"
    echo "  binary         : $binary"
    echo "  dwg            : $dwg"
    echo "  fonts dir      : $fonts_dir"
    echo "  backend mode   : $backend"
    echo "  lifecycle mode : $lifecycle"
    echo "  capture at     : ${capture_at}s"
    echo "  hard timeout   : ${timeout_s}s"
    echo "  maximize       : $([[ "$maximize" -eq 1 ]] && echo yes || echo no)"
    echo "  stacking reg   : $([[ "$stacking_regression" -eq 1 ]] && echo yes || echo no)"
    echo "  build          : $([[ "$do_build" -eq 1 ]] && echo yes || echo no)"
    echo "  build outputs  : frontend build (skip npm ci if node_modules present)"
    echo "                   cargo build --release --locked -p app-linux --bin yacr --features tauri/custom-protocol"
    echo "  uid            : $uid"
    echo "  runtime dir    : $runtime_dir"
    echo "  x11 display    : ${x11_display:-<none>} (available=$x11_available)"
    echo "  x11 authority  : ${x11_xauthority:-<none>}"
    echo "  wayland disp   : $wayland_display (available=$wayland_available)"
    for name in "${candidates[@]}"; do
        build_env_prefix "$name"
        echo "  --- backend: $name ---"
        if backend_available "$name"; then
            echo "    available    : yes"
        else
            echo "    available    : no (would be recorded as SKIPPED)"
        fi
        echo "    run.log      : $out_dir/$name/run.log"
        echo "    env          : ${env_prefix[*]}"
        echo "    argv         : $binary --open $dwg --fonts-dir $fonts_dir"
        if [[ "$name" == "x11" ]]; then
            echo "    surfaceMode  : dom-canvas (wasm CAD session in a DOM canvas; no native CAD child)"
            echo "    screenshots  : $out_dir/x11/x11-frame.png (Mutter ScreenCast via $screencast)"
        else
            echo "    surfaceMode  : native-below (app-owned wl_subsurface under the WebView)"
            echo "    screenshots  : $out_dir/wayland/wayland-frame.png (Mutter ScreenCast via $screencast)"
            if [[ "$stacking_regression" -eq 1 ]]; then
                echo "    fixture      : separate launch -> $out_dir/wayland-stacking-regression/run.log"
                echo "    fixture env  : $(build_env_prefix "$name"; printf '%s ' "${env_prefix[@]}"; echo 'YACR_PROBE_STACKING=1 YACR_PROBE_EVIDENCE=1')"
                echo "    fixture frames: initial.png, popup-open.png, after-synthetic-input.png, after-synthetic-input-later.png"
            fi
        fi
    done
    echo "  results file   : $out_dir/results.tsv"
    exit 0
fi

# --------------------------------------------------------------------------
# Output directory and results file
# --------------------------------------------------------------------------
mkdir -p "$out_dir"
results="$out_dir/results.tsv"
printf 'backend\tcase\tverdict\texit_code\tscreenshot\tcaptured_utc\tnotes\n' > "$results"

build_failed=0
any_ran=0
any_launch_failed=0
ran_backends=()
skipped_backends=()
declare -a launched_pgids=()

record_row() {
    # backend case verdict exit_code screenshot captured_utc notes
    local notes="${7:-}"
    notes="${notes//$'\t'/ }"
    notes="${notes//$'\n'/ }"
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$1" "$2" "$3" "$4" "$5" "$6" "$notes" >> "$results"
}

# --------------------------------------------------------------------------
# Build phase
# --------------------------------------------------------------------------
if [[ "$do_build" -eq 1 ]]; then
    mkdir -p "$out_dir/build"
    echo "[collect] building frontend"
    ui_dir="$root/apps/desktop-ui"
    frontend_log="$out_dir/build/frontend-build.log"
    if [[ ! -d "$ui_dir/node_modules" ]]; then
        echo "[collect] node_modules missing; running npm ci"
        if ! (cd "$ui_dir" && npm ci) >"$frontend_log" 2>&1; then
            echo "error: npm ci failed (see $frontend_log)" >&2
            build_failed=1
        fi
    fi
    if [[ "$build_failed" -eq 0 ]]; then
        if ! (cd "$ui_dir" && npm run build) >>"$frontend_log" 2>&1; then
            echo "error: npm run build failed (see $frontend_log)" >&2
            build_failed=1
        fi
    fi
    if [[ "$build_failed" -eq 0 ]]; then
        echo "[collect] cargo release build"
        if ! cargo build --release --locked -p app-linux --bin yacr \
            --features tauri/custom-protocol \
            >"$out_dir/build/cargo-release-build.log" 2>&1; then
            echo "error: cargo build failed (see $out_dir/build/cargo-release-build.log)" >&2
            build_failed=1
        fi
    fi
else
    echo "[collect] --no-build: skipping frontend and cargo build"
    if [[ ! -x "$binary" ]]; then
        echo "error: --no-build but $binary is missing" >&2
        build_failed=1
    fi
fi

if [[ "$build_failed" -eq 1 ]]; then
    mkdir -p "$out_dir/build"
    for name in "${candidates[@]}"; do
        mkdir -p "$out_dir/$name"
        record_row "$name" "run" "BUILD_FAILED" "-" "missing" "-" "build did not complete"
    done
    echo "[collect] FAILED: build failed; see $out_dir/build/" >&2
    exit 3
fi

# --------------------------------------------------------------------------
# Capture helpers
# --------------------------------------------------------------------------
write_gpu_identity() {
    {
        if command -v nvidia-smi >/dev/null 2>&1; then
            nvidia-smi 2>&1 || true
        else
            echo "nvidia-smi: not available on this host"
        fi
    } > "$1"
}

# Capture one compositor frame into <dir>/<label>.png.
#
# Used for BOTH carriers: the native Wayland surface and the X11/XWayland
# `dom-canvas` toplevel are both composited by the session's compositor, so the
# same same-UID Mutter ScreenCast stream is the only capture that yields pixels
# here (`x11grab` / `gnome-screenshot`'s X11 read-back comes back all black).
# Sets `capture_reason` on failure so the results row can state the real reason
# instead of inventing a screenshot.
# The frame is the FULL primary monitor and may include unrelated windows; this
# helper never treats a capture as a visual verdict.
capture_reason=""
capture_screencast_frame() {
    local dir="$1" label="$2" log="$3"
    capture_reason=""
    if ! command -v gst-launch-1.0 >/dev/null 2>&1 || [[ ! -f "$screencast" ]]; then
        capture_reason="gst-launch-1.0 or screencast-capture.py unavailable"
        echo "[collect] warning: $capture_reason; $label.png missing" >&2
        return 1
    fi
    if ! python3 "$screencast" --out "$dir/$label.png" >>"$log" 2>&1; then
        capture_reason="Mutter ScreenCast capture failed (see $log)"
        echo "[collect] warning: $capture_reason for $label" >&2
        return 1
    fi
    if [[ ! -s "$dir/$label.png" ]]; then
        capture_reason="ScreenCast reported success but wrote no frame"
        echo "[collect] warning: $capture_reason for $label" >&2
        return 1
    fi
    return 0
}

# Start one real session for the current `env_prefix`.
#
# Sets the global `pidfile`, `rcfile` and `launcher`; the caller owns the pid/rc
# files under <dir>. The launch line is the single place the real binary is
# spawned, so every case (production, fixture) is provably the same --open path.
start_session() {
    local dir="$1" run_log="$2"
    pidfile="$dir/app.pid"
    rcfile="$dir/app.rc"
    : > "$run_log"
    rm -f "$pidfile" "$rcfile"
    # `env` execs `setsid` in place, so the `bash -c` process below stays the
    # session/process-group leader whose PID is `$$`.
    "${env_prefix[@]}" setsid bash -c '
printf "%s\n" "$$" > "$1"
rc=0
"$2" --open "$3" --fonts-dir "$4" || rc=$?
printf "%s\n" "$rc" > "$5"
exit "$rc"
' _ "$pidfile" "$binary" "$dwg" "$fonts_dir" "$rcfile" >"$run_log" 2>&1 &
    launcher=$!
}

# --------------------------------------------------------------------------
# Per-backend run
# --------------------------------------------------------------------------
run_backend() {
    local name="$1"
    local mode="${2:-production}"
    local dir
    if [[ "$mode" == "stacking-regression" ]]; then
        dir="$out_dir/$name-stacking-regression"
    else
        dir="$out_dir/$name"
    fi
    mkdir -p "$dir"
    local run_log="$dir/run.log"

    build_env_prefix "$name"
    if [[ "$mode" == "stacking-regression" ]]; then
        # Pinned fixture trigger. The env var is only one of three parts: the host
        # injects the init-script flag and this is a real `--open` session. The
        # synthetic pointer/wheel input comes from the existing evidence driver.
        env_prefix+=("YACR_PROBE_STACKING=1" "YACR_PROBE_EVIDENCE=1")
        echo "[collect] launching $name stacking-regression fixture (separate launch)"
    else
        echo "[collect] launching $name backend"
    fi

    # Apply the resolved backend environment to the REAL launch. `env_prefix`
    # starts with the literal `env` (plus `-u DISPLAY -u XAUTHORITY` on Wayland)
    # and carries the probe variables, so the app sees the correct display
    # backend and lifecycle mode.
    start_session "$dir" "$run_log"

    local t0=$SECONDS

    # Wait for the inner shell to publish its pid (10s cap).
    while [[ ! -s "$pidfile" ]]; do
        if [[ $((SECONDS - t0)) -ge 10 ]]; then break; fi
        if ! kill -0 "$launcher" 2>/dev/null && [[ ! -s "$pidfile" ]]; then break; fi
        sleep 0.2
    done

    if [[ ! -s "$pidfile" ]]; then
        record_row "$name" "$mode" "LAUNCH_FAILED" "-" "missing" "-" \
            "no pid captured within 10s; see $run_log"
        any_launch_failed=1
        return
    fi

    local pid pgid
    pid="$(tr -dc '0-9' < "$pidfile" | head -c 10)"
    pgid="$pid"
    launched_pgids+=("$pgid")
    echo "[collect] $name pid/pgid=$pgid (log: $run_log)"

    # Readiness marker (up to 20s).
    local ready=0
    while [[ $((SECONDS - t0)) -lt 20 ]]; do
        if grep -q 'display backend policy=' "$run_log" 2>/dev/null; then ready=1; break; fi
        if [[ -s "$rcfile" ]]; then break; fi
        sleep 0.25
    done
    if [[ "$ready" -eq 1 ]]; then
        echo "[collect] $name backend policy line observed"
    else
        echo "[collect] warning: no backend policy line within 20s for $name"
    fi

    # Refusal polling. A refusal can arrive AFTER the window exists: the
    # `dom-canvas` carrier reports a missing WebGL2 from the frontend once the
    # WebView mounts (`canvas_runtime_failed`). So the markers are polled for the
    # whole capture window instead of being sampled once. The marker line is the
    # primary signal; a refusal has no usable carrier frame, so it must not be
    # reported as a normal run.
    local refusal_kind=""
    while [[ $((SECONDS - t0)) -lt "$capture_at" ]]; do
        detect_refusal "$run_log"
        if [[ -n "$refusal_kind" ]]; then break; fi
        if [[ -s "$rcfile" ]]; then break; fi
        sleep 0.2
    done
    detect_refusal "$run_log"
    if [[ -n "$refusal_kind" ]]; then
        record_refusal "$name" "$mode" "$refusal_kind" "$dir" "$pgid" "missing" \
            "refused before any capture; no carrier frame exists"
        return
    fi

    # Sampling and screenshots. Each branch is one launch: the production frame
    # and the multi-frame fixture never share a frame, timestamp or row.
    local captured_utc="-"
    local capture_reason=""
    if [[ "$mode" == "stacking-regression" ]]; then
        # Multi-frame fixture protocol: anchor on the first real presenter
        # readiness (not on launch) so `initial` exists even on a slow cold
        # start. The fixture auto-opens its popup 9s after mount (matching
        # AUTO_POPUP_DELAY_MS=9000); the existing
        # YACR_PROBE_EVIDENCE driver injects synthetic pointer/wheel input ~8s
        # after its own start, before `after-synthetic-input`.
        local anchor0=$SECONDS
        while [[ $((SECONDS - anchor0)) -lt 25 ]]; do
            if grep -q 'bridge wayland presenter ready' "$run_log" 2>/dev/null; then break; fi
            if [[ -s "$rcfile" ]]; then break; fi
            sleep 0.25
        done
        local sample0=$SECONDS
        echo "[collect] fixture sampling anchored ${sample0}s/$((sample0 - t0))s after launch"
        local -a frame_labels=(initial popup-open after-synthetic-input after-synthetic-input-later)
        local -a frame_offsets=(2 12 15 18)
        local i
        for i in "${!frame_labels[@]}"; do
            local frame_target=$((sample0 + frame_offsets[i]))
            while [[ $SECONDS -lt $frame_target && ! -s "$rcfile" ]]; do
                sleep 0.2
            done
            if [[ -s "$rcfile" ]]; then
                echo "[collect] warning: fixture exited before ${frame_labels[i]} frame" >&2
            else
                capture_screencast_frame "$dir" "${frame_labels[i]}" "$run_log" || true
            fi
        done
        if [[ -s "$dir/initial.png" && -s "$dir/popup-open.png" \
            && -s "$dir/after-synthetic-input.png" ]]; then
            captured_utc="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        fi
    else
        local frame_label="wayland-frame"
        if [[ "$name" == "x11" ]]; then frame_label="x11-frame"; fi
        while [[ $((SECONDS - t0)) -lt "$capture_at" ]]; do
            if [[ -s "$rcfile" ]]; then break; fi
            sleep 0.2
        done
        # Both carriers are captured through the compositor's own ScreenCast
        # stream. The X11/XWayland window is a normal composited toplevel, and its
        # X11 read-back (x11grab / a window drawable grab) is all black here.
        if [[ -s "$rcfile" ]]; then
            # The session is already gone: a frame now would be whatever window
            # happens to be on screen, not this run. Record the reason instead.
            capture_reason="app exited before the capture point; no frame taken"
            echo "[collect] warning: $name $capture_reason" >&2
        else
            capture_screencast_frame "$dir" "$frame_label" "$run_log" || true
        fi
        if [[ -s "$dir/$frame_label.png" ]]; then
            captured_utc="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        fi
    fi

    # Wait for a natural exit, capped by the hard timeout.
    local timed_out=0
    while [[ ! -s "$rcfile" ]]; do
        if [[ $((SECONDS - t0)) -ge "$timeout_s" ]]; then timed_out=1; break; fi
        if ! kill -0 "$pid" 2>/dev/null && [[ ! -s "$rcfile" ]]; then
            sleep 0.3
            if [[ ! -s "$rcfile" ]]; then break; fi
        fi
        sleep 0.25
    done

    local verdict notes
    if [[ "$timed_out" -eq 1 ]]; then
        verdict="PLANNED_TIMEOUT"
        notes="process group SIGTERM after ${timeout_s}s"
        kill -TERM -"$pgid" 2>/dev/null || true
    else
        verdict="NATURAL_EXIT"
        notes="app exited on its own"
    fi

    # Reap the launcher to observe the real wrapper exit status.
    local launcher_rc=0
    set +e
    wait "$launcher" 2>/dev/null
    launcher_rc=$?
    set -e

    local exit_code=""
    if [[ -s "$rcfile" ]]; then
        exit_code="$(tr -dc '0-9' < "$rcfile" | head -c 10)"
    else
        exit_code="$launcher_rc"
    fi
    [[ -n "$exit_code" ]] || exit_code="-"

    # Make sure the process group is fully gone.
    if kill -0 -"$pgid" 2>/dev/null; then
        kill -TERM -"$pgid" 2>/dev/null || true
        sleep 1
        if kill -0 -"$pgid" 2>/dev/null; then
            kill -KILL -"$pgid" 2>/dev/null || true
        fi
    fi

    # Late refusal re-check: the `dom-canvas` carrier can report a missing
    # WebGL2 after the capture window. Classify by the marker, not by the exit
    # code, so the row is never misread as a normal exit.
    detect_refusal "$run_log"
    if [[ -n "$refusal_kind" ]]; then
        local late_shot="missing"
        if [[ "$name" == "x11" && -s "$dir/x11-frame.png" ]]; then
            late_shot="captured"
        elif [[ "$name" == "wayland" && -s "$dir/wayland-frame.png" ]]; then
            late_shot="captured"
        elif [[ "$mode" == "stacking-regression" && -s "$dir/initial.png" ]]; then
            late_shot="partial"
        fi
        local late_note="refusal observed after the capture window"
        if [[ "$late_shot" != "missing" ]]; then
            late_note="$late_note; the frame predates/coincides with the refusal and is not carrier evidence"
        fi
        record_refusal "$name" "$mode" "$refusal_kind" "$dir" "$pgid" "$late_shot" "$late_note"
        return
    fi

    # Screenshot verdict.
    local screenshot="missing"
    if [[ "$name" == "x11" ]]; then
        if [[ -s "$dir/x11-frame.png" ]]; then
            screenshot="captured"
        fi
        # Report whether the run's own log confirms the carrier it started on.
        if grep -q 'surfaceMode=dom-canvas' "$run_log" 2>/dev/null; then
            notes="$notes; surfaceMode=dom-canvas (carrier line observed); compositor frame (Mutter ScreenCast) of the full monitor; the X11 window has no native CAD child"
        else
            notes="$notes; surfaceMode=dom-canvas (carrier line NOT observed in run.log); compositor frame (Mutter ScreenCast) of the full monitor; the X11 window has no native CAD child"
        fi
        if [[ "$screenshot" != "captured" ]]; then
            notes="$notes; screenshot missing: ${capture_reason:-no frame written}"
        fi
    elif [[ "$mode" == "stacking-regression" ]]; then
        local frames_present=0
        local label
        for label in initial popup-open after-synthetic-input after-synthetic-input-later; do
            if [[ -s "$dir/$label.png" ]]; then frames_present=$((frames_present + 1)); fi
        done
        if [[ -s "$dir/initial.png" && -s "$dir/popup-open.png" \
            && -s "$dir/after-synthetic-input.png" ]]; then
            screenshot="captured"
        elif [[ "$frames_present" -gt 0 ]]; then
            screenshot="partial"
        fi
        notes="$notes; classification=STACKING_REGRESSION; single launch; frames=${frames_present}/4 (initial,popup-open,after-synthetic-input,after-synthetic-input-later); fixture-only synthetic pointer moves on .native-input + YACR_PROBE_EVIDENCE input; fixture auto-opened popup; full-monitor compositor frames, no visual verdict"
        if [[ "$screenshot" != "captured" ]]; then
            notes="$notes; frames missing: ${capture_reason:-no frame written}"
        fi
    else
        if [[ -s "$dir/wayland-frame.png" ]]; then
            screenshot="captured"
        fi
        notes="$notes; surfaceMode=native-below; production chrome over CAD; compositor frame (Mutter ScreenCast) of the full monitor"
        if [[ "$screenshot" != "captured" ]]; then
            notes="$notes; screenshot missing: ${capture_reason:-no frame written}"
        fi
    fi

    write_gpu_identity "$dir/gpu-identity.txt"

    record_row "$name" "$mode" "$verdict" "$exit_code" "$screenshot" "$captured_utc" "$notes"

    any_ran=1
    ran_backends+=("$name/$mode")
    echo "[collect] $name/$mode: verdict=$verdict exit=$exit_code screenshot=$screenshot"
}

for name in "${candidates[@]}"; do
    if backend_available "$name"; then
        run_backend "$name" "production"
        if [[ "$name" == "wayland" && "$stacking_regression" -eq 1 ]]; then
            run_backend "$name" "stacking-regression"
        fi
    else
        mkdir -p "$out_dir/$name"
        record_row "$name" "production" "SKIPPED" "-" "missing" "-" \
            "backend unavailable in this session (no display/$name socket)"
        skipped_backends+=("$name")
        if [[ "$name" == "wayland" && "$stacking_regression" -eq 1 ]]; then
            mkdir -p "$out_dir/$name-stacking-regression"
            record_row "$name" "stacking-regression" "SKIPPED" "-" "missing" "-" \
                "Wayland unavailable; fixture not launched"
            skipped_backends+=("$name/stacking-regression")
        fi
        echo "[collect] $name: SKIPPED (unavailable)"
    fi
done

# --------------------------------------------------------------------------
# Final cleanup of any surviving process group
# --------------------------------------------------------------------------
for pgid in "${launched_pgids[@]:-}"; do
    if [[ -n "$pgid" ]] && kill -0 -"$pgid" 2>/dev/null; then
        kill -TERM -"$pgid" 2>/dev/null || true
    fi
done

# Record a top-level NO_DISPLAY verdict when nothing could be attempted, so the
# results file never silently reports an empty run as success.
if [[ "$any_ran" -eq 0 ]]; then
    record_row "none" "production" "NO_DISPLAY" "-" "missing" "-" \
        "no display session available; nothing was run"
fi

# --------------------------------------------------------------------------
# README with results and the authorization boundary
# --------------------------------------------------------------------------
timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
{
    echo "# YACR Linux host evidence run"
    echo
    echo "Generated (UTC): $timestamp"
    echo "Repository root: $root"
    echo "Output root: $out_dir"
    echo "Binary: $binary"
    echo
    echo "## What ran"
    echo "- Backends requested: $backend"
    echo "- Backends executed: ${ran_backends[*]:-<none>}"
    echo "- Backends skipped: ${skipped_backends[*]:-<none>}"
    echo "- Pixel carriers: wayland=native-below (native wgpu subsurface under the WebView);"
    echo "  x11=dom-canvas (wasm CAD session drawing into a DOM canvas; no native CAD child)"
    echo "- Lifecycle probe mode: $lifecycle"
    echo "- Build: $([[ "$do_build" -eq 1 ]] && echo 'frontend + cargo release' || echo 'skipped (--no-build)')"
    echo "- Stacking regression fixture (Wayland): $([[ "$stacking_regression" -eq 1 ]] && echo yes || echo no)"
    echo "- Capture delay: ${capture_at}s; hard timeout: ${timeout_s}s"
    if [[ "$stacking_regression" -eq 1 ]]; then
        echo "- Fixture frames: one separate launch, sampled at presenter+2s/presenter+12s/presenter+15s/presenter+18s"
        echo "  (initial / popup-open / after-synthetic-input / after-synthetic-input-later)"
    fi
    echo "- DWG: $dwg"
    echo "- Fonts dir: $fonts_dir"
    echo "- No-maximize env set: $([[ "$maximize" -eq 0 ]] && echo yes || echo no)"
    echo
    echo "## Per-backend results (results.tsv)"
    echo '```'
    cat "$results"
    echo '```'
    echo
    echo "## Artifacts (absolute paths)"
    echo "- $out_dir/results.tsv"
    if [[ "$do_build" -eq 1 ]]; then
        echo "- $out_dir/build/cargo-release-build.log"
        echo "- $out_dir/build/frontend-build.log"
    fi
    echo "- $out_dir/x11/run.log; (only when the backend ran, not on a refusal path) gpu-identity.txt, x11-frame.png"
    echo "- $out_dir/wayland/run.log, wayland-frame.png, gpu-identity.txt (production launch)"
    if [[ "$stacking_regression" -eq 1 ]]; then
        echo "- $out_dir/wayland-stacking-regression/run.log, initial.png, popup-open.png, after-synthetic-input.png, after-synthetic-input-later.png, gpu-identity.txt (separate fixture launch)"
    fi
    echo "- A refused backend produces no usable carrier frame, so its row says \`missing\`;"
    echo "  the marker line in run.log and the results.tsv row are the evidence, and the"
    echo "  note records why a capture was not available."
    echo
    echo "## Evidence boundary"
    echo "- Both carriers are captured the same way: one full-monitor frame from the"
    echo "  same-UID Mutter ScreenCast API ($screencast). On this desktop the X11/XWayland"
    echo "  window is a normal composited toplevel, and its X11 read-back"
    echo "  (x11grab / a window-drawable grab) comes back all black, so it is not used."
    echo "- The frame is the FULL primary monitor, so other windows on the desktop may"
    echo "  appear in it. Every artifact is written to this local output directory and is"
    echo "  never uploaded anywhere."
    echo "- A captured screenshot is not a visual-correctness or pixel-accuracy judgement,"
    echo "  and it is not evidence that OS-level input reached the app."
    echo "- Production and fixture rows are separate launches with separate logs and frames;"
    echo "  no row copies another launch's exit code, timestamp or screenshot. A reader can"
    echo "  tell the mode from the case column and the frame path."
    echo "- The fixture row records a single launch sampled four times. Marker motion is"
    echo "  judgeable by comparing initial vs after-synthetic-input vs after-later; the"
    echo "  popup frame shows the fixture popup across the chrome/CAD edge. The fixture"
    echo "  auto-opens its popup 9s after mount and dispatches click-free pointer moves"
    echo "  on .native-input at 8/11/14/17s (fixture-only hooks, no physical input); the"
    echo "  existing YACR_PROBE_EVIDENCE driver injects more synthetic input."
    echo "- Those frames are still whole-monitor compositor output and are not a"
    echo "  stacking/visual PASS; an independent image review is required."
    echo "- Classification is marker-first, exit code second:"
    echo "  * \`$unsupported_runtime_marker\` + exit 6 = UNSUPPORTED_RUNTIME, the current"
    echo "    dom-canvas refusal (the WebView has no usable WebGL2)."
    echo "  * \`$unsupported_backend_marker\` + exit 4 = UNSUPPORTED_BACKEND, the"
    echo "    HISTORICAL pre-GTK X11 refusal. It is no longer produced (X11 now starts on"
    echo "    dom-canvas); the parser is kept so an old binary is classified, not misread."
    echo
    echo "## Authorization boundary"
    echo "- 2026-10-23 user decision: this environment permits privilege escalation,"
    echo "  running as root, session unlocking/switching and package installation. This"
    echo "  runner needs none of that: it runs as the invoking same-UID desktop user"
    echo "  (uid $uid), never runs as root for its own actions, never unlocks or otherwise"
    echo "  modifies the session, and never changes portal or ScreenCast permission state."
    echo "- Still forbidden, and never faked: a virtual/headless display presented as"
    echo "  real-session evidence, and software rendering (llvmpipe / SwiftShader /"
    echo "  lavapipe) reported as a real GPU. With no real display session the runner exits"
    echo "  non-zero instead of faking success."
    echo "- Screenshots are captured from the local session and written to this local"
    echo "  output directory only; they are never uploaded anywhere."
} > "$out_dir/README.md"

echo "[collect] results: $results"
echo "[collect] summary: $out_dir/README.md"

if [[ "$any_ran" -eq 0 ]]; then
    echo "error: no display session available; nothing was run" >&2
    exit 2
fi
if [[ "$any_launch_failed" -eq 1 ]]; then
    echo "error: at least one backend failed to launch" >&2
    exit 1
fi
exit 0
