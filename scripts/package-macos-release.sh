#!/usr/bin/env bash
# package-macos-release.sh -- build and package the native macOS release bundle.
#
# Must run on macOS: producing a Mach-O executable (and, on a universal build,
# combining two architectures with `lipo`) needs Apple's SDK and linker, which
# Rust cannot cross-compile to from Linux. Windows has an analogous native
# runner (see scripts/package-windows-release.sh); the macOS equivalent is the
# `macos-release` GitHub Actions job on `macos-latest`.
#
# Produces a tarball containing a self-contained `Yacr.app` bundle plus the
# headless CLI and the CAD font package:
#   * Yacr.app/Contents/MacOS/yacr-macos      release GUI host (Slint + shared
#                                             CAD wgpu renderer; Metal via wgpu),
#   * Yacr.app/Contents/Info.plist            bundle metadata (dwg/dxf viewer),
#   * Yacr.app/Contents/Resources/fonts/      the CAD font package (same assembly
#                                             as Linux/Windows),
#   * bin/cad-cli-tools                       release headless CLI (scan/.../render),
#   * bin/yacr-macos                          symlink to the GUI binary for CLI use,
#   * fonts                                   symlink to the bundled font package
#                                             so the CLI auto-detects it,
#   * docs/, scripts/, LICENSE, README.md, THIRD_PARTY_NOTICES.md, PACKAGE.txt.
#
# Usage:
#   scripts/package-macos-release.sh                 # universal (arm64 + x86_64)
#   MACOS_ARCH=arm64 scripts/package-macos-release.sh
#   MACOS_ARCH=x86_64 scripts/package-macos-release.sh
#
# Env:
#   MACOS_ARCH     'universal' (default; both arm64 and x86_64, combined with
#                  `lipo`), 'arm64' or 'x86_64' for a single architecture.
#   WITH_FONTS     1 (default) to assemble the full CAD font catalogue into the
#                  app bundle via scripts/fetch-fonts.sh (needs network); 0 keeps
#                  only the committed fonts/ (offline).
#   YACR_MACOS_SMOKE  1 to run the packaged CLI under the build host and assert
#                  the documented argument-parse behaviour (no window/GPU).
#   FONT_BASE_URL / FONT_FALLBACK_BASE_URL / FONT_RETRIES  passed to fetch-fonts.sh.
#
# Artifacts (under target/, which is gitignored):
#   target/macos/<arch>/dist/yacr-<version>-macos-<arch>.tar.gz
#   target/macos/<arch>/dist/yacr-<version>-macos-<arch>.tar.gz.sha256
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "ERROR: this script must run on macOS (Apple SDK/linker required)" >&2
  echo "       use the macos-check / macos-release GitHub Actions jobs" >&2
  exit 1
fi

for tool in cargo rustc lipo otool file tar; do
  command -v "$tool" >/dev/null || {
    echo "ERROR: required tool '$tool' is not on PATH" >&2
    exit 1
  }
done

case "${MACOS_ARCH:-universal}" in
  universal) TARGETS=(aarch64-apple-darwin x86_64-apple-darwin); ARCH_LABEL="universal" ;;
  arm64)     TARGETS=(aarch64-apple-darwin);                     ARCH_LABEL="arm64" ;;
  x86_64)    TARGETS=(x86_64-apple-darwin);                      ARCH_LABEL="x86_64" ;;
  *)
    echo "ERROR: MACOS_ARCH must be universal, arm64 or x86_64 (got '${MACOS_ARCH}')" >&2
    exit 1
    ;;
esac

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
NAME="yacr-${VERSION}-macos-${ARCH_LABEL}"
DIST="target/macos/${ARCH_LABEL}/dist"
STAGE="${DIST}/${NAME}"
WITH_FONTS="${WITH_FONTS:-1}"

for target in "${TARGETS[@]}"; do
  if ! rustup target list --installed | grep -qx "$target"; then
    echo "ERROR: rust target $target is not installed; run: rustup target add $target" >&2
    exit 1
  fi
done

echo "==> building cad-cli-tools and yacr-macos (release, locked): ${TARGETS[*]}"
for target in "${TARGETS[@]}"; do
  cargo build --release --locked --target "$target" -p cad-cli-tools -p app-macos
done

# Combine the per-target executables: `lipo -create` for a universal build, a
# plain copy for a single-architecture (thin) build.
lipo_thin() {
  local relative="$1" output="$2" inputs=() target
  for target in "${TARGETS[@]}"; do
    local path="target/${target}/release/${relative}"
    [ -f "$path" ] || { echo "ERROR: missing $path" >&2; exit 1; }
    inputs+=("$path")
  done
  mkdir -p "$(dirname "$output")"
  if [ "${#inputs[@]}" -eq 1 ]; then
    # A thin build needs no lipo; copy the single architecture through.
    cp "${inputs[0]}" "$output"
  else
    lipo -create "${inputs[@]}" -output "$output"
  fi
}

GUI_BUILT="${STAGE}/Yacr.app/Contents/MacOS/yacr-macos"
CLI_BUILT="${STAGE}/bin/cad-cli-tools"

echo "==> staging ${STAGE}"
rm -rf "$STAGE"
mkdir -p "${STAGE}/Yacr.app/Contents/MacOS" \
         "${STAGE}/Yacr.app/Contents/Resources/fonts" \
         "${STAGE}/bin" "${STAGE}/docs" "${STAGE}/scripts"

lipo_thin "yacr-macos" "$GUI_BUILT"
lipo_thin "cad-cli-tools" "$CLI_BUILT"
chmod +x "$GUI_BUILT" "$CLI_BUILT"

cp LICENSE THIRD_PARTY_NOTICES.md README.md "$STAGE/"
cp docs/cli.md docs/headless-render.md docs/render-backends.md docs/build.md \
   docs/validation.md docs/compatibility.md docs/macos-app.md docs/fonts.md \
   "$STAGE/docs/"
cp scripts/fetch-test-dwg.sh scripts/render-smoke.sh "$STAGE/scripts/"

# The CLI and the GUI both auto-detect the `fonts/` package next to the
# executable (or, for a `.app`, `Contents/Resources/fonts`). Keep a single real
# copy inside the bundle and expose it to the sibling CLI through a symlink so
# the archive does not double the ~55 MiB font catalogue.
ln -s "Yacr.app/Contents/Resources/fonts" "${STAGE}/fonts"
ln -s "../Yacr.app/Contents/MacOS/yacr-macos" "${STAGE}/bin/yacr-macos"

cat > "${STAGE}/Yacr.app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Yacr</string>
  <key>CFBundleDisplayName</key><string>Yacr</string>
  <key>CFBundleIdentifier</key><string>invalid.example.yacr</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleExecutable</key><string>yacr-macos</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleSignature</key><string>????</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>CAD drawing</string>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>CFBundleTypeExtensions</key>
      <array><string>dwg</string><string>dxf</string></array>
    </dict>
  </array>
</dict>
</plist>
EOF

if [ "$WITH_FONTS" = "1" ]; then
  echo "==> assembling CAD fonts (fetch-fonts.sh) into the app bundle"
  "$ROOT/scripts/fetch-fonts.sh" "${STAGE}/Yacr.app/Contents/Resources/fonts"
else
  echo "==> WITH_FONTS=0: staging committed fonts/ only"
  cp -R "$ROOT"/fonts/. "${STAGE}/Yacr.app/Contents/Resources/fonts/"
fi

# Every Mach-O dependency must come from the OS (libSystem under /usr/lib, or a
# framework under /System/Library). An `@rpath`/Homebrew dylib would not exist on
# a stock Mac, so refuse to ship it instead of producing a package that only
# runs on the build machine.
is_system_dependency() {
  case "$1" in
    /usr/lib/*|/System/Library/*|/System/iOSSupport/*) return 0 ;;
    *) return 1 ;;
  esac
}
verify_macho() {
  local binary="$1" description
  file "$binary" | grep -q 'Mach-O' || {
    echo "ERROR: $binary is not a Mach-O executable" >&2
    file "$binary" >&2
    exit 1
  }
  description="$(lipo -archs "$binary")"
  echo "  $(basename "$binary"): Mach-O ${description}"
  while IFS= read -r dependency; do
    [ -n "$dependency" ] || continue
    if ! is_system_dependency "$dependency"; then
      echo "ERROR: $binary links non-system library '$dependency'" >&2
      otool -L "$binary" >&2
      exit 1
    fi
  # `otool -L` prints a `path:` (or `path (architecture ARCH):`) header per
  # architecture followed by indented dependency lines. Only the indented lines
  # are dependencies; for a universal binary there is more than one header, so
  # `tail -n +2` is not enough.
  done < <(otool -L "$binary" | awk '/^[[:space:]]/ {print $1}')
}

echo "==> verifying the executables are Mach-O with only system dependencies"
verify_macho "$CLI_BUILT"
verify_macho "$GUI_BUILT"

if [ "${YACR_MACOS_SMOKE:-0}" = "1" ]; then
  echo "==> smoke: packaged CLI --help"
  "$CLI_BUILT" --help >/dev/null
  echo "==> smoke: packaged GUI rejects a bad option before opening a window"
  set +e
  gui_out="$("$GUI_BUILT" --bogus value 2>&1)"
  gui_status=$?
  set -e
  if [ "$gui_status" -eq 0 ] || [[ "$gui_out" != *"unknown option or invalid value: --bogus"* ]]; then
    echo "ERROR: packaged yacr-macos did not report the documented parse error (status=${gui_status})" >&2
    printf '%s\n' "$gui_out" >&2
    exit 1
  fi
  echo "    (argument parsing only; no window, GPU, or native dialog is exercised)"
fi

cat > "$STAGE/PACKAGE.txt" <<EOF
yacr macOS release package
==========================
version : ${VERSION}
arch    : ${ARCH_LABEL} (Mach-O; targets: ${TARGETS[*]})
built   : $(date -u +%Y-%m-%dT%H:%M:%SZ)
fonts   : ${WITH_FONTS}; 1 = full catalogue vendored in the app bundle, 0 = osifont only

Contents
--------
Yacr.app                        self-contained GUI bundle: double-click to launch.
                                Slint + the shared CAD wgpu renderer (Metal via wgpu).
Yacr.app/Contents/MacOS/yacr-macos
                                the GUI executable inside the bundle.
bin/yacr-macos                  symlink to the GUI executable (run from a shell).
bin/cad-cli-tools               headless CLI (scan/measure/build-representation/render/...).
Yacr.app/Contents/Resources/fonts/
                                the CAD font package in fonts.json format: the
                                mlightcad/cad-data catalogue plus the committed QCAD
                                osifont.ttf (provenance in fonts/SOURCE.md). Loaded
                                automatically by the GUI; missing drawing fonts fall
                                back to the macOS system font.
fonts                           symlink to Yacr.app/Contents/Resources/fonts, so
                                bin/cad-cli-tools finds the package next to itself.
docs/                           CLI, headless render, backends, build, validation,
                                compat, macOS host and font notes.
scripts/                        fetch-test-dwg.sh and render-smoke.sh (POSIX shell).
README.md, LICENSE, THIRD_PARTY_NOTICES.md

Quick start
-----------
  open Yacr.app                                         # GUI (Finder)
  ./bin/yacr-macos --open /path/drawing.dwg             # GUI from a shell
  ./bin/cad-cli-tools --help
  ./bin/cad-cli-tools render drawing.dwg --png frame.png --width 1280 --height 720

Runtime notes (not a compatibility claim)
-----------------------------------------
  * The GUI needs a Mac with a Metal-capable GPU (wgpu's Metal backend).
  * The package is unsigned and not notarized; Gatekeeper may warn until the
    deployer signs it. A signed/notarized distribution is the deployer's job.
  * \`--headless\` offscreen validation requires software Vulkan and is a
    Linux/Windows acceptance path only; it is reported as unsupported on macOS.

No unlicensed user DWG samples or golden images are bundled. The mlightcad/cad-data
fonts are third-party and vendored at packaging time (scripts/fetch-fonts.sh); only
the authorized QCAD osifont.ttf (GPL-3 with font exception) is committed to the
repository. Redistribution terms are the deployer's responsibility (docs/fonts.md).
This package is not a compatibility or performance claim.
See docs/macos-app.md and docs/validation.md.
EOF

TARBALL="${DIST}/${NAME}.tar.gz"
echo "==> creating ${TARBALL}"
rm -f "$TARBALL" "${TARBALL}.sha256"
# COPYFILE_DISABLE stops bsdtar from adding AppleDouble `._*` resource-fork
# members; the archive stays portable to Linux/Windows extraction tools.
( cd "$DIST" && COPYFILE_DISABLE=1 tar -czf "${NAME}.tar.gz" "$NAME" )
( cd "$DIST" && if command -v shasum >/dev/null; then shasum -a 256 "${NAME}.tar.gz" > "${NAME}.tar.gz.sha256"; else sha256sum "${NAME}.tar.gz" > "${NAME}.tar.gz.sha256"; fi )

echo
echo "artifact : ${TARBALL}"
echo "size     : $(wc -c < "$TARBALL") bytes"
echo "sha256   : $(cut -d' ' -f1 "${TARBALL}.sha256")"
