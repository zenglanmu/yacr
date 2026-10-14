#!/usr/bin/env bash
# Actual shared Slint component, no display server or Android emulator.
#
# Real hardware GPU by default: no VK_ICD_FILENAMES is forced, so the Vulkan
# loader picks the real device (an inherited VK_ICD_FILENAMES is honored as
# explicit user intent). Software rendering (Mesa lavapipe) is opt-in only:
#
#   YACR_SOFTWARE_GPU=1 bash scripts/check-ui-native.sh   # software; labeled
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
if [[ "${YACR_SOFTWARE_GPU:-0}" == "1" ]]; then
  export VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
  if [[ ! -s "$VK_ICD_FILENAMES" ]]; then
    echo "check-ui-native: YACR_SOFTWARE_GPU=1 but no Mesa lavapipe ICD at $VK_ICD_FILENAMES" >&2
    exit 2
  fi
  echo "check-ui-native: SOFTWARE GPU mode (YACR_SOFTWARE_GPU=1): VK_ICD_FILENAMES=$VK_ICD_FILENAMES; evidence below is software-rendered, not a real GPU" >&2
elif [[ -z "${VK_ICD_FILENAMES:-}" ]] && ! compgen -G "/dev/dri/renderD*" >/dev/null; then
  echo "check-ui-native: no hardware GPU render node (/dev/dri/renderD*) and no VK_ICD_FILENAMES." >&2
  echo "  Run this on a machine with a GPU, or opt into software Vulkan explicitly:" >&2
  echo "    YACR_SOFTWARE_GPU=1 bash scripts/check-ui-native.sh   # Mesa lavapipe (software; labeled)" >&2
  exit 2
fi
export YACR_UI_OUTPUT="${YACR_UI_OUTPUT:-/tmp/opencode/yacr-ui-$(date +%Y%m%d-%H%M%S)}"
cargo test -p cad-ui-slint --test concept_offscreen --locked -- --test-threads=1 --nocapture
echo "Slint offscreen screenshots: $YACR_UI_OUTPUT (synthetic CAD content; adapter logged above)"
