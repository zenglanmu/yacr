#!/usr/bin/env bash
# Primary acceptance: the actual Linux host + shared Slint/CAD renderer on software Vulkan.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-$(find /usr/share/vulkan/icd.d -name '*lvp*.json' -print -quit)}"
test -s "$VK_ICD_FILENAMES"
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
