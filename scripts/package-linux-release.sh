#!/usr/bin/env bash
# package-linux-release.sh -- build and package the native Linux release as a Flatpak.
#
# Produces a single-file Flatpak bundle for dev.yacr.app:
#   * bin/yacr-linux      release GUI host (Slint + shared CAD wgpu renderer),
#   * bin/cad-cli-tools   release headless CLI (includes the headless `render`),
#   * fonts/              the CAD font package, auto-loaded from the binary's
#                         sibling `fonts/` dir by both the CLI and the GUI.
#
# The manifest (packaging/flatpak/dev.yacr.app.yml) packages *prebuilt* binaries:
# this script builds them on the host and stages them under
# target/flatpak/payload, then runs flatpak-builder. Nothing is compiled inside
# the sandbox. The org.freedesktop.Platform runtime supplies glibc, fontconfig,
# freetype, the GL/Vulkan loader and the windowing libraries, so no host
# libraries are relocated into the bundle (they would couple it to the build
# distribution). runtime-version 26.08 ships glibc 2.44, newer than the glibc of
# supported build hosts, so a host build loads inside the sandbox.
#
# Usage:
#   scripts/package-linux-release.sh
#
# Requirements:
#   * flatpak + flatpak-builder on PATH (CI installs both), and the runtime/sdk:
#       flatpak install -y flathub org.freedesktop.Platform//26.08 org.freedesktop.Sdk//26.08
#
# Optional smoke tests:
#   YACR_TEST_DWG=/path/to/input.dwg scripts/package-linux-release.sh
#       render the DWG with the staged CLI and check the PNG is non-blank
#       (host-side, before packaging).
#   YACR_FLATPAK_SMOKE=1 scripts/package-linux-release.sh
#       run `cad-cli-tools --help` inside the built sandbox via
#       `flatpak-builder --run`, proving the Flatpak actually launches.
#
# Env:
#   WITH_FONTS   1 (default) to assemble the full CAD font package into
#                fonts/ via scripts/fetch-fonts.sh (mlightcad catalogue +
#                committed QCAD osifont; needs network); 0 for an offline build
#                that keeps only the committed fonts/.
#   FONTS_DIR    optional prepared font directory (must contain fonts.json).
#                When set, the download is skipped and this directory is copied
#                into the payload instead -- lets a host (or CI cache) fetch the
#                fonts once and hand them to flatpak-builder, so the packaging
#                step itself needs no network. Overrides WITH_FONTS.
#   FONT_BASE_URL / FONT_FALLBACK_BASE_URL / FONT_RETRIES  passed through to
#                fetch-fonts.sh.
#
# Artifacts (under target/, which is gitignored):
#   target/release/dist/yacr-<version>-linux-<arch>.flatpak
#   target/release/dist/yacr-<version>-linux-<arch>.flatpak.sha256
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
ARCH="$(uname -m)"
NAME="yacr-${VERSION}-linux-${ARCH}"
DIST="target/release/dist"
APP_ID="dev.yacr.app"
MANIFEST="packaging/flatpak/dev.yacr.app.yml"
PAYLOAD="target/flatpak/payload"
BUILD="target/flatpak/build"
REPO="target/flatpak/repo"
STATE="target/flatpak/state"
WITH_FONTS="${WITH_FONTS:-1}"

echo "==> building cad-cli-tools (release, locked, offline)"
cargo build --release --offline --locked -p cad-cli-tools

echo "==> building yacr-linux GUI host (release, locked, offline)"
cargo build --release --offline --locked -p app-linux --bin yacr-linux

BIN="target/release/cad-cli-tools"
GUI="target/release/yacr-linux"
[ -x "$BIN" ] || { echo "ERROR: missing $BIN" >&2; exit 1; }
[ -x "$GUI" ] || { echo "ERROR: missing $GUI" >&2; exit 1; }

echo "==> staging prebuilt payload in ${PAYLOAD}"
rm -rf "$PAYLOAD"
mkdir -p "$PAYLOAD/bin" "$PAYLOAD/fonts"
cp "$BIN" "$PAYLOAD/bin/cad-cli-tools"
cp "$GUI" "$PAYLOAD/bin/yacr-linux"

# CAD fonts are assembled on the HOST, before flatpak-builder runs; the manifest
# only copies the already-present payload directory into /app/fonts, so no
# network is needed inside the sandbox.
#   * FONTS_DIR set   -> use that prepared directory (host/CI cache), no download;
#   * WITH_FONTS=1    -> scripts/fetch-fonts.sh downloads the mlightcad/cad-data
#                        catalogue and merges the committed fonts/ (QCAD osifont)
#                        so the packaged CLI and GUI shape every catalogue font
#                        locally without network;
#   * WITH_FONTS=0    -> offline build that keeps only the committed fonts/.
if [ -n "${FONTS_DIR:-}" ]; then
  [ -f "$FONTS_DIR/fonts.json" ] || {
    echo "ERROR: FONTS_DIR must contain fonts.json: $FONTS_DIR" >&2
    exit 1
  }
  echo "==> using prepared font package from $FONTS_DIR (no download)"
  cp -R "$FONTS_DIR"/. "$PAYLOAD/fonts/"
elif [ "$WITH_FONTS" = "1" ]; then
  echo "==> assembling CAD fonts (fetch-fonts.sh) into $PAYLOAD/fonts"
  "$ROOT/scripts/fetch-fonts.sh" "$PAYLOAD/fonts"
else
  echo "==> WITH_FONTS=0: staging committed fonts/ only"
  cp -R "$ROOT"/fonts/. "$PAYLOAD/fonts/"
fi

echo "==> verifying staged CLI runs"
"$PAYLOAD/bin/cad-cli-tools" --help >/dev/null

echo "==> verifying staged GUI loads"
# yacr-linux has no --help; `--headless` without --output is a documented parse
# error that is only printed after the dynamic loader has resolved every
# dependency, then exits 1.
set +e
gui_out="$("$PAYLOAD/bin/yacr-linux" --headless 2>&1)"
gui_status=$?
set -e
if [ "$gui_status" -eq 0 ] || [[ "$gui_out" != *"--headless requires --output NEW_DIRECTORY"* ]]; then
  echo "ERROR: staged yacr-linux failed to load/parse (status=${gui_status})" >&2
  printf '%s\n' "$gui_out" >&2
  exit 1
fi
if ldd "$PAYLOAD/bin/yacr-linux" | grep -q 'not found'; then
  echo "ERROR: staged yacr-linux has unresolved shared libraries" >&2
  ldd "$PAYLOAD/bin/yacr-linux" >&2
  exit 1
fi

if [ -n "${YACR_TEST_DWG:-}" ]; then
  echo "==> CLI smoke test with ${YACR_TEST_DWG}"
  YACR_CLI="$BIN" "$ROOT/scripts/render-smoke.sh" "$YACR_TEST_DWG" \
    "${DIST}/${NAME}.smoke.png" 800 600
fi

# flatpak-builder may come from the distro package or the org.flatpak.Builder
# Flatpak; accept either.
if command -v flatpak-builder >/dev/null 2>&1; then
  FLATPAK_BUILDER=(flatpak-builder)
elif flatpak info org.flatpak.Builder >/dev/null 2>&1; then
  FLATPAK_BUILDER=(flatpak run --command=flatpak-builder org.flatpak.Builder)
else
  echo "ERROR: flatpak-builder not found." >&2
  echo "  Install it, or: flatpak install -y flathub org.flatpak.Builder" >&2
  echo "  and ensure the runtime/sdk: flatpak install -y flathub \\" >&2
  echo "    org.freedesktop.Platform//26.08 org.freedesktop.Sdk//26.08" >&2
  exit 1
fi

echo "==> building the Flatpak (flatpak-builder)"
rm -rf "$BUILD" "$REPO"
"${FLATPAK_BUILDER[@]}" \
  --force-clean \
  --state-dir="$STATE" \
  --repo="$REPO" \
  "$BUILD" \
  "$MANIFEST"

echo "==> bundling ${APP_ID}"
BUNDLE="${DIST}/${NAME}.flatpak"
mkdir -p "$DIST"
rm -f "$BUNDLE" "${BUNDLE}.sha256"
flatpak build-bundle "$REPO" "$BUNDLE" "$APP_ID"
( cd "$DIST" && sha256sum "$(basename "$BUNDLE")" > "$(basename "$BUNDLE").sha256" )

if [ "${YACR_FLATPAK_SMOKE:-0}" = "1" ]; then
  echo "==> Flatpak sandbox smoke: cad-cli-tools --help"
  "${FLATPAK_BUILDER[@]}" --run "$BUILD" "$MANIFEST" cad-cli-tools --help >/dev/null
  echo "    sandbox launch OK"
fi

echo
echo "artifact : ${BUNDLE}"
echo "size     : $(wc -c < "$BUNDLE") bytes"
echo "sha256   : $(cut -d' ' -f1 "${BUNDLE}.sha256")"
echo "install  : flatpak install --user ${BUNDLE}"
echo "run      : flatpak run ${APP_ID}"
