#!/usr/bin/env bash
# Real-machine desktop regression for app-linux: run every real-window /
# real-GPU acceptance test, one Cargo invocation each.
#
# The ignored tests in apps/app-linux/src/host/validation.rs each need a
# dedicated OS process. Window tests open a real winit window, and winit/Slint
# allows only ONE event loop per process: batching them makes the second window
# test die with `EventLoop can't be recreated`. So this runner spawns one
# `cargo test` per test, serially (GPU tests must not overlap).
#
# Real hardware GPU only: this script never sets VK_ICD_FILENAMES and clears any
# inherited value, so the Vulkan loader picks the real device (never lavapipe).
#
# Usage:
#   YACR_TEST_DWG=/abs/sample.dwg bash scripts/run-real-gpu-e2e.sh
#   bash scripts/run-real-gpu-e2e.sh --list
#
# Env:
#   YACR_TEST_DWG       absolute path to a real .dwg (REQUIRED)
#   YACR_REAL_E2E_ROOT  evidence root (default /tmp/opencode/yacr-real-gpu-e2e)
#
# Evidence: a fresh UTC-timestamped root is created and each test gets a NEW,
# not-yet-created subdirectory (the desktop tests refuse pre-existing dirs).
set -uo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Never inherit a software ICD from the caller: this run must use the real GPU.
unset VK_ICD_FILENAMES

# name|env-var|evidence-subdir  ("-" env-var = needs no evidence directory)
TESTS=(
  "desktop_dwg_event_loop_and_navigation_remain_responsive|YACR_DESKTOP_EVIDENCE|desktop-event-loop"
  "desktop_dwg_open_after_start_and_interaction_remain_responsive|YACR_DESKTOP_EVIDENCE|desktop-open-after-start"
  "real_dwg_cpu_load_stages|-|-"
  "real_gpu_grid_zoom_axes_acceptance|YACR_REAL_E2E_OUTPUT|real-gpu-grid-zoom-axes"
  "real_gpu_select_move_undo_redo_acceptance|YACR_REAL_FUNCTIONAL_OUTPUT|real-gpu-select-move-history"
  "real_gpu_maximized_grid_crosshair_navigation_acceptance|YACR_REAL_MAXIMIZED_OUTPUT|real-gpu-maximized-grid-crosshair"
)

if [[ "${1:-}" == "--list" ]]; then
  echo "Real-machine desktop acceptance tests (one process each):"
  for entry in "${TESTS[@]}"; do
    IFS='|' read -r name envvar subdir <<<"$entry"
    if [[ "$envvar" == "-" ]]; then
      printf '  %-62s env: YACR_TEST_DWG\n' "$name"
    else
      printf '  %-62s env: YACR_TEST_DWG, %s=<fresh dir>\n' "$name" "$envvar"
    fi
  done
  echo
  echo "Required: YACR_TEST_DWG=<absolute path to a real .dwg>"
  echo "Runs on the real hardware GPU; VK_ICD_FILENAMES is never set (software ICD cleared)."
  echo "Evidence root: \${YACR_REAL_E2E_ROOT:-/tmp/opencode/yacr-real-gpu-e2e}/<UTC-YYYYmmdd-HHMMSS>/"
  exit 0
fi

if [[ $# -gt 0 ]]; then
  echo "run-real-gpu-e2e: unknown argument: $1 (only --list is supported)" >&2
  exit 2
fi

if [[ -z "${YACR_TEST_DWG:-}" ]]; then
  echo "run-real-gpu-e2e: YACR_TEST_DWG is required (absolute path to a real .dwg)." >&2
  echo "  export YACR_TEST_DWG=\$HOME/sources/cad-test-files/<sample>.dwg" >&2
  echo "  (fetch one with: scripts/fetch-test-dwg.sh)" >&2
  exit 2
fi
if [[ ! -f "$YACR_TEST_DWG" ]]; then
  echo "run-real-gpu-e2e: YACR_TEST_DWG not found: $YACR_TEST_DWG" >&2
  exit 2
fi

EVIDENCE_ROOT="${YACR_REAL_E2E_ROOT:-/tmp/opencode/yacr-real-gpu-e2e}/$(date -u +%Y%m%d-%H%M%S)"
mkdir -p "$EVIDENCE_ROOT"

FAILED=0
RESULTS=()
for entry in "${TESTS[@]}"; do
  IFS='|' read -r name envvar subdir <<<"$entry"
  log="$EVIDENCE_ROOT/$name.log"
  dir="-"
  cmd=(cargo test -p app-linux --lib "$name" --locked -- --ignored --test-threads=1 --nocapture)
  echo
  echo "==> run-real-gpu-e2e: $name"
  start="$(date +%s)"
  if [[ "$envvar" != "-" ]]; then
    dir="$EVIDENCE_ROOT/$subdir"
    if [[ -e "$dir" ]]; then
      echo "run-real-gpu-e2e: refusing pre-existing evidence directory: $dir" >&2
      RESULTS+=("$name|FAIL|0|$dir")
      FAILED=1
      continue
    fi
    env "$envvar=$dir" "${cmd[@]}" >"$log" 2>&1
  else
    "${cmd[@]}" >"$log" 2>&1
  fi
  code=$?
  secs=$(( $(date +%s) - start ))
  if [[ $code -eq 0 ]]; then
    status="PASS"
  else
    status="FAIL"
    FAILED=1
  fi
  printf '    %s exit=%s %ss\n' "$status" "$code" "${secs}s"
  if [[ "$status" == "PASS" ]]; then
    tail -n 3 "$log" | sed 's/^/    | /'
  else
    echo "    --- last 25 lines of $log ---"
    tail -n 25 "$log" | sed 's/^/    | /'
  fi
  RESULTS+=("$name|$status|$secs|$dir")
done

echo
echo "==== real GPU E2E summary ===="
printf '%-64s %-4s %5s\n' "TEST" "RSLT" "SECS"
for row in "${RESULTS[@]}"; do
  IFS='|' read -r r_name r_status r_secs _ <<<"$row"
  printf '%-64s %-4s %5s\n' "$r_name" "$r_status" "$r_secs"
done

echo
echo "Evidence root: $EVIDENCE_ROOT"
for row in "${RESULTS[@]}"; do
  IFS='|' read -r r_name _ _ r_dir <<<"$row"
  printf '  %-62s %s\n' "$r_name" "$r_dir"
done

if [[ $FAILED -ne 0 ]]; then
  echo "run-real-gpu-e2e: FAILED (see logs under $EVIDENCE_ROOT)" >&2
  exit 1
fi
echo "run-real-gpu-e2e: all tests passed"
