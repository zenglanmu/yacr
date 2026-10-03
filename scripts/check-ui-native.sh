#!/usr/bin/env bash
# Actual shared Slint component, no display server or Android emulator.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
export VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
export YACR_UI_OUTPUT="${YACR_UI_OUTPUT:-/tmp/opencode/yacr-ui-$(date +%Y%m%d-%H%M%S)}"
cargo test -p cad-ui-slint --test concept_offscreen --locked -- --test-threads=1 --nocapture
echo "Slint/lavapipe screenshots: $YACR_UI_OUTPUT (synthetic CAD content)"
