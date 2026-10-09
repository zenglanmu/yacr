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
# Env:
#   WITH_FONTS  1 (default) to assemble the full CAD font package into
#               `fonts/` beside the binary via scripts/fetch-fonts.sh
#               (mlightcad catalogue + committed QCAD osifont; needs network);
#               0 for an offline build that keeps only the committed fonts/.
#   FONT_BASE_URL / FONT_FALLBACK_BASE_URL / FONT_RETRIES  passed through to
#               fetch-fonts.sh.
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
WITH_FONTS="${WITH_FONTS:-1}"

echo "==> building cad-cli-tools (release, locked, offline)"
cargo build --release --offline --locked -p cad-cli-tools

BIN="target/release/cad-cli-tools"
[ -x "$BIN" ] || { echo "ERROR: missing $BIN" >&2; exit 1; }

echo "==> staging ${STAGE}"
rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/docs" "$STAGE/scripts" "$STAGE/fonts"
cp "$BIN" "$STAGE/bin/cad-cli-tools"
cp LICENSE THIRD_PARTY_NOTICES.md README.md "$STAGE/"
cp docs/cli.md docs/headless-render.md docs/render-backends.md docs/build.md \
   docs/validation.md docs/compatibility.md "$STAGE/docs/"
cp scripts/fetch-test-dwg.sh scripts/render-smoke.sh "$STAGE/scripts/"

# CAD fonts: with WITH_FONTS=1 (default) the platform-agnostic packer downloads
# the mlightcad/cad-data catalogue and merges the committed fonts/ package
# (QCAD osifont), so a packaged CLI shapes every catalogue font locally without
# network. WITH_FONTS=0 keeps only the committed fonts/ for an offline build.
if [ "$WITH_FONTS" = "1" ]; then
  echo "==> assembling CAD fonts (fetch-fonts.sh) into $STAGE/fonts"
  "$ROOT/scripts/fetch-fonts.sh" "$STAGE/fonts"
else
  echo "==> WITH_FONTS=0: staging committed fonts/ only"
  cp -R "$ROOT"/fonts/. "$STAGE/fonts/"
fi

cat > "$STAGE/PACKAGE.txt" <<EOF
yacr Linux release package
==========================
version : ${VERSION}
arch    : ${ARCH}
built   : $(date -u +%Y-%m-%dT%H:%M:%SZ)
fonts   : ${WITH_FONTS}; 1 = full catalogue vendored (see below), 0 = osifont only

Contents
--------
bin/cad-cli-tools     headless CLI (scan/measure/build-representation/render/...)
fonts/                CAD font package in fonts.json format: the mlightcad/cad-data
                      catalogue plus the committed QCAD osifont.ttf (provenance in
                      fonts/SOURCE.md); auto-loaded from this sibling dir, missing
                      drawing fonts fall back to the default outline face
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

No unlicensed user DWG samples or golden images are bundled; the committed
QCAD flange fixture (fixtures/dxf/qcad-flange/, upstream terms in SOURCE.md) may
be included with the scripts. The mlightcad/cad-data fonts in fonts/ are
third-party and vendored by the deployer at packaging time (scripts/fetch-fonts.sh;
redistribution terms are the deployer's responsibility, see docs/fonts.md); only
the authorized QCAD osifont.ttf (GPL-3 with font exception) is committed to the
repository. This package is not a compatibility or performance claim.
See docs/validation.md and docs/headless-render.md.
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
