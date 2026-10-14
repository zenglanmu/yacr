#!/usr/bin/env bash
# Primary acceptance: the actual Linux host + shared Slint/CAD renderer.
#
# Real hardware GPU by default: this script never sets VK_ICD_FILENAMES, so the
# Vulkan loader picks the real device. An inherited VK_ICD_FILENAMES is honored
# as explicit user intent. Software rendering (Mesa lavapipe) is opt-in only:
#
#   YACR_SOFTWARE_GPU=1 bash scripts/check-linux-app.sh   # software; labeled
#
# With no software opt-in and no hardware render node / inherited ICD, the script
# fails with how to opt in instead of silently degrading to a software adapter.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
if [[ "${YACR_SOFTWARE_GPU:-0}" == "1" ]]; then
  export VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-$(find /usr/share/vulkan/icd.d -name '*lvp*.json' -print -quit)}"
  if [[ -z "${VK_ICD_FILENAMES:-}" || ! -s "$VK_ICD_FILENAMES" ]]; then
    echo "check-linux-app: YACR_SOFTWARE_GPU=1 but no Mesa lavapipe ICD found under /usr/share/vulkan/icd.d (install mesa-vulkan-drivers)" >&2
    exit 2
  fi
  echo "check-linux-app: SOFTWARE GPU mode (YACR_SOFTWARE_GPU=1): VK_ICD_FILENAMES=$VK_ICD_FILENAMES; evidence below is software-rendered, not a real GPU" >&2
elif [[ -z "${VK_ICD_FILENAMES:-}" ]] && ! compgen -G "/dev/dri/renderD*" >/dev/null; then
  echo "check-linux-app: no hardware GPU render node (/dev/dri/renderD*) and no VK_ICD_FILENAMES." >&2
  echo "  Run this on a machine with a GPU, or opt into software Vulkan explicitly:" >&2
  echo "    YACR_SOFTWARE_GPU=1 bash scripts/check-linux-app.sh   # Mesa lavapipe (software; labeled)" >&2
  exit 2
fi
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/opencode/yacr-linux-runtime-$(id -u)}"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
OUTPUT="${YACR_LINUX_OUTPUT:-/tmp/opencode/yacr-linux-$(date +%Y%m%d-%H%M%S)-$$}"
test ! -e "$OUTPUT"
mkdir -p "$(dirname "$OUTPUT")"
cargo build -p app-linux --bin yacr-linux --release --locked
args=(--headless --output "$OUTPUT")
if [[ -n "${YACR_TEST_DWG:-}" ]]; then args+=(--open "$YACR_TEST_DWG"); fi
"${CARGO_TARGET_DIR:-target}/release/yacr-linux" "${args[@]}"
python3 - "$OUTPUT" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
r = json.loads((p / "report.json").read_text())
assert r["host"] == "app-linux"
assert r["cadFrames"] > 0 and r["renderError"] is None
assert r["navigationCameraChanged"] and r["navigationPixelsChanged"]
for name in ("initial.png", "navigation.png"):
    assert (p / name).read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
print(f'Linux host smoke passed: {p}; source={r["source"]}; visual acceptance requires review')
PY
