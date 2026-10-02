#!/usr/bin/env bash
# package-linux-release.sh -- build and package the native Linux release CLI.
#
# Produces a self-contained tarball with the release `cad-cli-tools` binary
# (which includes the headless `render` operation), the repository docs, and the
# sample-fetch / render-smoke helper scripts. Software Vulkan (Mesa lavapipe) is
# used at runtime for headless verification; it is not bundled.
#
# Usage:
#   scripts/package-linux-release.sh
#
# Optional smoke test (renders a real DWG and checks the PNG is non-blank):
#   YACR_TEST_DWG=/path/to/input.dwg scripts/package-linux-release.sh
#
# Artifacts (under target/, which is gitignored):
#   target/release/dist/yacr-<version>-linux-<arch>.tar.gz
#   target/release/dist/yacr-<version>-linux-<arch>.tar.gz.sha256
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
ARCH="$(uname -m)"
NAME="yacr-${VERSION}-linux-${ARCH}"
DIST="target/release/dist"
STAGE="${DIST}/${NAME}"

echo "==> building cad-cli-tools (release, locked, offline)"
cargo build --release --offline --locked -p cad-cli-tools

BIN="target/release/cad-cli-tools"
[ -x "$BIN" ] || { echo "ERROR: missing $BIN" >&2; exit 1; }

echo "==> staging ${STAGE}"
rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/docs" "$STAGE/scripts"
cp "$BIN" "$STAGE/bin/cad-cli-tools"
cp LICENSE THIRD_PARTY_NOTICES.md README.md "$STAGE/"
cp docs/cli.md docs/headless-render.md docs/render-backends.md docs/build.md \
   docs/validation.md docs/compatibility.md "$STAGE/docs/"
cp scripts/fetch-test-dwg.sh scripts/render-smoke.sh "$STAGE/scripts/"

cat > "$STAGE/PACKAGE.txt" <<EOF
yacr Linux release package
==========================
version : ${VERSION}
arch    : ${ARCH}
built   : $(date -u +%Y-%m-%dT%H:%M:%SZ)

Contents
--------
bin/cad-cli-tools     headless CLI (scan/measure/build-representation/render/...)
docs/                 CLI, headless render, backends, build, validation, compat
scripts/fetch-test-dwg.sh   download a curated real-DWG corpus to /tmp
scripts/render-smoke.sh     render a DWG and verify the PNG is non-blank
README.md, LICENSE, THIRD_PARTY_NOTICES.md

Quick start
-----------
  # force Mesa lavapipe (software Vulkan); path varies by distro
  export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
  ./bin/cad-cli-tools --help
  ./bin/cad-cli-tools render drawing.dwg --png frame.png --width 1280 --height 720
  ./scripts/render-smoke.sh drawing.dwg frame.png

No real DWG samples, fonts, or golden images are bundled: fixtures/manifest is
empty and no sample authorization exists. This package is not a compatibility or
performance claim. See docs/validation.md and docs/headless-render.md.
EOF

if [ -n "${YACR_TEST_DWG:-}" ]; then
  echo "==> smoke test with ${YACR_TEST_DWG}"
  YACR_CLI="$BIN" "$STAGE/scripts/render-smoke.sh" "$YACR_TEST_DWG" \
    "${DIST}/${NAME}.smoke.png" 800 600
fi

echo "==> verifying packaged CLI runs"
"$STAGE/bin/cad-cli-tools" --help >/dev/null

TARBALL="${DIST}/${NAME}.tar.gz"
echo "==> creating ${TARBALL}"
rm -f "$TARBALL" "${TARBALL}.sha256"
tar -C "$DIST" -czf "$TARBALL" "$NAME"
( cd "$DIST" && sha256sum "${NAME}.tar.gz" > "${NAME}.tar.gz.sha256" )

echo
echo "artifact : ${TARBALL}"
echo "size     : $(wc -c < "$TARBALL") bytes"
echo "sha256   : $(cut -d' ' -f1 "${TARBALL}.sha256")"
