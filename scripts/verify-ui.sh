#!/usr/bin/env bash
# verify-ui: run the layered headless UI debug loop on the real Linux host.
#
# Layer 2 of the test plan: a headless graphics environment (Slint's official
# offscreen platform + Mesa lavapipe software Vulkan) actually runs the
# production LinuxApp, drives fixed UI scenarios, screenshots the composited
# GPU frame and collects failures.
#
# It does NOT create a coding harness and does NOT install X11/Wayland: Slint's
# Platform/WindowAdapter is the virtual window runtime and lavapipe is the GPU.
# Xvfb may be present, but the automation path does not depend on it. A real
# GPU / real drawing / real device is never implied by this script.
#
# Usage:
#   bash scripts/verify-ui.sh
#   YACR_TEST_DWG=/abs/sample.dxf bash scripts/verify-ui.sh
#
# Env:
#   YACR_VERIFY_OUTPUT   evidence directory (must not exist; default below)
#   YACR_VERIFY_TIMEOUT  per-layer wall-clock budget in seconds (default 900)
#   YACR_VERIFY_PROFILE  debug|release cargo profile (default debug)
#   VK_ICD_FILENAMES     override the software-Vulkan ICD
set -uo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PROFILE="${YACR_VERIFY_PROFILE:-debug}"
if [[ "$PROFILE" == "release" ]]; then PROFILE_FLAG="--release"; else PROFILE_FLAG=""; fi
TIMEOUT="${YACR_VERIFY_TIMEOUT:-900}"
export VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-$(find /usr/share/vulkan/icd.d -name '*lvp*.json' -print -quit)}"
if [[ -z "${VK_ICD_FILENAMES:-}" || ! -s "$VK_ICD_FILENAMES" ]]; then
  echo "verify-ui: no software Vulkan ICD (set VK_ICD_FILENAMES); refusing to fake a GPU" >&2
  exit 2
fi
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/opencode/yacr-verify-runtime-$(id -u)}"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

OUTPUT="${YACR_VERIFY_OUTPUT:-/tmp/opencode/yacr-verify-ui-$(date +%Y%m%d-%H%M%S)-$$}"
if [[ -e "$OUTPUT" ]]; then
  echo "verify-ui: output already exists, refusing to overwrite old evidence: $OUTPUT" >&2
  exit 2
fi
# Note: the app/host layers create their own evidence directories and refuse an
# existing one, so the script must not pre-create them.
mkdir -p "$OUTPUT/logs" "$OUTPUT/screenshots" "$OUTPUT/slint" "$OUTPUT/scenario"
STATUS_FILE="$OUTPUT/.layers.tsv"
: >"$STATUS_FILE"

record() { # name status exit elapsed log
  printf '%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" "$5" >>"$STATUS_FILE"
}

# run_layer <name> <required:0|1> <env assignments...> -- <command...>
run_layer() {
  local name="$1" required="$2"; shift 2
  local envs=()
  while [[ "$1" != "--" ]]; do envs+=("$1"); shift; done
  shift
  local log="$OUTPUT/logs/$name.log"
  echo "==> verify-ui: $name"
  local start end code
  start="$(date +%s)"
  timeout "$TIMEOUT" env "${envs[@]}" "$@" >"$log" 2>&1
  code=$?
  end="$(date +%s)"
  local elapsed=$((end - start))
  local status
  if [[ $code -eq 0 ]]; then status="passed"
  elif [[ $code -eq 124 || $code -eq 137 ]]; then status="timeout"
  else status="failed"; fi
  record "$name" "$status" "$code" "$elapsed" "$log"
  printf '    %-16s %-8s exit=%s %ss\n' "$name" "$status" "$code" "${elapsed}s"
  if [[ $required -eq 1 && "$status" != "passed" ]]; then
    echo "    (required layer failed; see $log)"
    FAILED=1
  fi
  return 0
}
FAILED=0

# 1) Pure UI contracts: no GPU, fast, catches catalog/state-model regressions.
run_layer ui-unit 1 -- cargo test -p cad-ui-slint --lib --locked
# 2) Real Slint component on lavapipe: layout matrix, config presets, mobile
#    panels, click hit-testing, screenshots.
run_layer ui-offscreen 1 YACR_UI_OUTPUT="$OUTPUT/slint" -- \
  cargo test -p cad-ui-slint --test concept_offscreen --locked -- --test-threads=1
# 3) Fixed scenario replay on the production LinuxApp (measure/cancel/annotate/
#    draw/undo/layer/config/locale/resize) with screenshots.
run_layer scenario 1 YACR_VERIFY_OUTPUT="$OUTPUT/scenario" -- \
  cargo test -p app-linux --test verify_ui --locked -- --test-threads=1
# 4) Real host command/transaction/file-picker contracts.
run_layer host-contracts 1 -- \
  cargo test -p app-linux --test host_contracts --locked -- --test-threads=1
# 5) The actual release/debug application binary in --headless acceptance mode.
run_layer app-build 1 -- cargo build -p app-linux --bin yacr-linux $PROFILE_FLAG --locked
BIN="${CARGO_TARGET_DIR:-$ROOT/target}/$PROFILE/yacr-linux"
if [[ -x "$BIN" ]]; then
  run_layer app-smoke 1 -- "$BIN" --headless --output "$OUTPUT/app"
  # 6) Optional: a real, external drawing the caller explicitly points at.
  #    When requested it is required: a broken open must fail the run.
  if [[ -n "${YACR_TEST_DWG:-}" ]]; then
    run_layer app-dwg 1 -- "$BIN" --headless --output "$OUTPUT/app-dwg" --open "$YACR_TEST_DWG"
  else
    record app-dwg skipped 0 0 ""
  fi
else
  record app-smoke failed 127 0 "$OUTPUT/logs/app-build.log"
  FAILED=1
fi

# Consolidate screenshots into one reviewable directory with layer prefixes.
shopt -s nullglob
for png in "$OUTPUT/slint"/*.png; do cp "$png" "$OUTPUT/screenshots/slint-$(basename "$png")"; done
for png in "$OUTPUT/scenario"/*.png; do cp "$png" "$OUTPUT/screenshots/scenario-$(basename "$png")"; done

python3 "$ROOT/scripts/verify-ui-summary.py" "$OUTPUT" "$FAILED"
exit $?
