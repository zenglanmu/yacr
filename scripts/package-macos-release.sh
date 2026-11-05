#!/usr/bin/env bash
# package-macos-release.sh — build and package the macOS release of the current
# React/Tauri host (`apps/app-macos`, executable `yacr-macos`).
#
# Must run on macOS: producing Mach-O executables (and combining two
# architectures with `lipo`) needs Apple's SDK and linker, which Rust cannot
# cross-compile to from Linux.
#
# The host is a transparent WKWebView carrying the shared React UI
# (`apps/desktop-ui`) above an in-process native CAD child NSView presented by
# wgpu/Metal. `tauri::generate_context!` embeds `apps/desktop-ui/dist` at compile
# time (via `apps/app-macos/build.rs` → `tauri-build` reading `tauri.conf.json`),
# so the frontend MUST be rebuilt before `cargo build`; a stale or missing dist
# would be baked into the binary.
#
# Usage:
#   scripts/package-macos-release.sh                 # universal (arm64 + x86_64)
#   MACOS_ARCH=arm64 scripts/package-macos-release.sh
#   MACOS_ARCH=x86_64 scripts/package-macos-release.sh
#
# Env:
#   MACOS_ARCH     'universal' (default; both targets combined with `lipo`),
#                  'arm64' or 'x86_64' for a thin build.
#   SKIP_UI_BUILD  1 to reuse an existing `apps/desktop-ui/dist`; accepted only
#                  when `dist/index.html` exists and is non-empty.
#   WITH_FONTS     1 (default) assembles the full CAD font catalogue with
#                  `scripts/fetch-fonts.sh` (network); 0 stages only the
#                  committed `fonts/` package (offline).
#   FONT_BASE_URL / FONT_FALLBACK_BASE_URL / FONT_RETRIES  passed to fetch-fonts.sh.
#
# Artifacts (under target/, which is gitignored):
#   target/macos/<arch>/dist/yacr-<version>-macos-<arch>.tar.gz
#   target/macos/<arch>/dist/yacr-<version>-macos-<arch>.tar.gz.sha256
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

die() {
  echo "ERROR: $*" >&2
  exit 1
}

if [ "$(uname -s)" != "Darwin" ]; then
  echo "ERROR: this script must run on macOS (Apple SDK/linker required)" >&2
  echo "       use a macOS runner; Mach-O cannot be produced from Linux" >&2
  exit 1
fi

for tool in cargo rustc rustup lipo otool file tar; do
  command -v "$tool" >/dev/null 2>&1 || die "required tool '$tool' is not on PATH"
done

case "${MACOS_ARCH:-universal}" in
  universal) TARGETS=(aarch64-apple-darwin x86_64-apple-darwin); ARCH_LABEL="universal" ;;
  arm64)     TARGETS=(aarch64-apple-darwin);                     ARCH_LABEL="arm64" ;;
  x86_64)    TARGETS=(x86_64-apple-darwin);                      ARCH_LABEL="x86_64" ;;
  *)
    die "MACOS_ARCH must be universal, arm64 or x86_64 (got '${MACOS_ARCH}')"
    ;;
esac

case "${WITH_FONTS:-1}" in
  0|1) WITH_FONTS="${WITH_FONTS:-1}" ;;
  *) die "WITH_FONTS must be 0 or 1 (got '${WITH_FONTS}')" ;;
esac

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[ -n "$VERSION" ] || die "could not read the workspace version from Cargo.toml"
NAME="yacr-${VERSION}-macos-${ARCH_LABEL}"
DIST="target/macos/${ARCH_LABEL}/dist"
STAGE="${DIST}/${NAME}"

for target in "${TARGETS[@]}"; do
  if ! rustup target list --installed | grep -qx "$target"; then
    die "rust target $target is not installed; run: rustup target add $target"
  fi
done

# The React UI is embedded at compile time, so build it before cargo. Reusing a
# previous build is allowed only when the dist entry point really exists; a
# missing or empty index.html would silently embed a broken frontend.
UI_DIST="apps/desktop-ui/dist"
if [ "${SKIP_UI_BUILD:-0}" = "1" ]; then
  [ -s "${UI_DIST}/index.html" ] \
    || die "SKIP_UI_BUILD=1 but ${UI_DIST}/index.html is missing or empty; run (cd apps/desktop-ui && npm ci && npm run build)"
  echo "==> SKIP_UI_BUILD=1: reusing ${UI_DIST}"
else
  command -v npm >/dev/null 2>&1 || die "required tool 'npm' is not on PATH"
  echo "==> building the React frontend embedded by tauri::generate_context!"
  ( cd apps/desktop-ui && npm ci && npm run build )
fi

[ -s "${UI_DIST}/index.html" ] || die "frontend build produced no ${UI_DIST}/index.html"
[ -s "${UI_DIST}/plugins/tch.js" ] \
  || die "frontend build produced no ${UI_DIST}/plugins/tch.js; the Tianzheng vendor plugin would not be embedded (is the plugins/tch submodule initialized?)"

echo "==> building cad-cli-tools and yacr-macos (release, locked): ${TARGETS[*]}"
for target in "${TARGETS[@]}"; do
  # `tauri/custom-protocol` is what embeds `apps/desktop-ui/dist` into the
  # executable; without it the release binary still expects the dev server.
  cargo build --release --locked --target "$target" -p cad-cli-tools -p app-macos \
    --features tauri/custom-protocol
done

# Map each rust target to the Mach-O architecture name reported by `lipo`.
arch_for_target() {
  case "$1" in
    aarch64-apple-darwin) echo arm64 ;;
    x86_64-apple-darwin) echo x86_64 ;;
    *) die "unsupported target: $1" ;;
  esac
}
EXPECTED_ARCHES=()
for target in "${TARGETS[@]}"; do
  EXPECTED_ARCHES+=("$(arch_for_target "$target")")
done

# Combine the per-target executables: `lipo -create` for a universal build, a
# plain copy for a single-architecture (thin) build.
lipo_thin() {
  local relative="$1" output="$2" inputs=() target path
  for target in "${TARGETS[@]}"; do
    path="target/${target}/release/${relative}"
    [ -f "$path" ] || die "missing built executable: $path"
    inputs+=("$path")
  done
  mkdir -p "$(dirname "$output")"
  if [ "${#inputs[@]}" -eq 1 ]; then
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

# Finder/Dock icon (committed .icns derived from assets/yacr-icon.svg).
[ -s assets/yacr-icon.icns ] || die "missing app icon assets/yacr-icon.icns"
cp assets/yacr-icon.icns "${STAGE}/Yacr.app/Contents/Resources/Yacr.icns"

for file in LICENSE README.md THIRD_PARTY_NOTICES.md; do
  [ -f "$file" ] || die "missing required release file: $file"
  cp "$file" "$STAGE/"
done

# Ship the documentation that exists in this tree; missing optional pages are
# skipped, but a package with no host documentation is refused.
[ -f docs/macos-app.md ] || die "missing docs/macos-app.md"
for doc in docs/cli.md docs/headless-render.md docs/render-backends.md \
           docs/build.md docs/validation.md docs/compatibility.md \
           docs/architecture.md docs/testing-dwg.md docs/fonts.md \
           docs/react-linux-app.md docs/macos-app.md; do
  if [ -f "$doc" ]; then
    cp "$doc" "${STAGE}/docs/"
  fi
done

for script in scripts/fetch-test-dwg.sh scripts/render-smoke.sh; do
  [ -f "$script" ] || die "missing packaging helper: $script"
  cp "$script" "${STAGE}/scripts/"
done
chmod +x "${STAGE}"/scripts/*.sh

# The GUI and CLI both auto-detect the `fonts/` package next to the executable
# (or, for a `.app`, `Contents/Resources/fonts`). Keep one real copy inside the
# bundle and expose it to the sibling CLI through a symlink so the archive does
# not double the font catalogue.
ln -s "Yacr.app/Contents/Resources/fonts" "${STAGE}/fonts"
ln -s "../Yacr.app/Contents/MacOS/yacr-macos" "${STAGE}/bin/yacr-macos"

cat > "${STAGE}/Yacr.app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Yacr</string>
  <key>CFBundleDisplayName</key><string>Yacr</string>
  <key>CFBundleIdentifier</key><string>io.github.zenglanmu.yacr</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleExecutable</key><string>yacr-macos</string>
  <key>CFBundleIconFile</key><string>Yacr</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleSignature</key><string>????</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSPrincipalClass</key><string>NSApplication</string>
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

FONTS_STAGE="${STAGE}/Yacr.app/Contents/Resources/fonts"
if [ "$WITH_FONTS" = "1" ]; then
  echo "==> assembling the full CAD font catalogue (scripts/fetch-fonts.sh)"
  [ -x scripts/fetch-fonts.sh ] || die "missing executable scripts/fetch-fonts.sh"
  "${ROOT}/scripts/fetch-fonts.sh" "$FONTS_STAGE"
else
  echo "==> WITH_FONTS=0: staging the committed fonts/ package only"
  cp -R "${ROOT}/fonts/." "$FONTS_STAGE/"
fi
[ -s "${FONTS_STAGE}/fonts.json" ] || die "font package has no fonts.json: $FONTS_STAGE"

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
  local binary="$1" actual expected found count
  [ -f "$binary" ] || die "missing Mach-O executable: $binary"
  file "$binary" | grep -q 'Mach-O' || die "$binary is not a Mach-O executable"
  actual="$(lipo -archs "$binary")"
  [ -n "$actual" ] || die "lipo -archs reported no architectures for $binary"
  count="$(printf '%s\n' "$actual" | wc -w | tr -d ' ')"
  [ "$count" -eq "${#EXPECTED_ARCHES[@]}" ] \
    || die "$binary has architectures '$actual', expected ${EXPECTED_ARCHES[*]}"
  found=0
  for expected in "${EXPECTED_ARCHES[@]}"; do
    case " $actual " in
      *" $expected "*) found=$((found + 1)) ;;
    esac
  done
  [ "$found" -eq "${#EXPECTED_ARCHES[@]}" ] \
    || die "$binary architectures '$actual' do not match ${EXPECTED_ARCHES[*]}"
  echo "  $(basename "$binary"): Mach-O ${actual}"
  while IFS= read -r dependency; do
    [ -n "$dependency" ] || continue
    if ! is_system_dependency "$dependency"; then
      echo "ERROR: $binary links non-system library '$dependency'" >&2
      otool -L "$binary" >&2
      exit 1
    fi
  # `otool -L` prints a `path:` (or `path (architecture ARCH):`) header per
  # architecture followed by indented dependency lines. Only the indented lines
  # are dependencies; for a universal binary there is more than one header.
  done < <(otool -L "$binary" | awk '/^[[:space:]]/ {print $1}')
}

echo "==> verifying the executables are Mach-O with only system dependencies"
verify_macho "$CLI_BUILT"
verify_macho "$GUI_BUILT"

[ -L "${STAGE}/bin/yacr-macos" ] || die "missing bin/yacr-macos symlink"
[ -x "${STAGE}/bin/yacr-macos" ] || die "bin/yacr-macos does not resolve to the GUI executable"
[ -L "${STAGE}/fonts" ] || die "missing fonts symlink"
[ -s "${STAGE}/fonts/fonts.json" ] || die "fonts symlink does not resolve to the font package"

cat > "$STAGE/PACKAGE.txt" <<EOF
yacr macOS release package
==========================
version : ${VERSION}
arch    : ${ARCH_LABEL} (Mach-O; targets: ${TARGETS[*]})
built   : $(date -u +%Y-%m-%dT%H:%M:%SZ)
fonts   : ${WITH_FONTS}; 1 = full catalogue vendored in the app bundle, 0 = committed fonts/ only

Contents
--------
Yacr.app                        self-contained GUI bundle: double-click to launch.
                                React UI (apps/desktop-ui) in a transparent WKWebView
                                above an in-process native CAD NSView presented by
                                wgpu/Metal.
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
Yacr.app/Contents/Resources/Yacr.icns
                                the Finder/Dock app icon (from assets/yacr-icon.svg).
fonts                           symlink to Yacr.app/Contents/Resources/fonts, so
                                bin/cad-cli-tools finds the package next to itself.
docs/                           CLI, headless render, backends, build, validation,
                                compatibility, architecture, macOS host and font notes.
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
  * macOS may warn on first launch while the bundle is unsigned (below).
  * The shared React bundle is embedded in the executable at build time; the
    .app does not load a frontend from disk.

Signature status
----------------
This package is unsigned and not notarized. Gatekeeper may warn until the
deployer signs and notarizes it; producing a signed distribution is the
deployer's job.

No unlicensed user DWG samples or golden images are bundled. The mlightcad/cad-data
fonts are third-party and vendored at packaging time (scripts/fetch-fonts.sh); only
the authorized QCAD osifont.ttf (GPL-3 with font exception) is committed to the
repository. Redistribution terms are the deployer's responsibility (docs/fonts.md).
This package is not a compatibility or performance claim.
EOF

TARBALL="${DIST}/${NAME}.tar.gz"
SHA256="${TARBALL}.sha256"
echo "==> creating ${TARBALL}"
rm -f "$TARBALL" "$SHA256"
# COPYFILE_DISABLE stops bsdtar from adding AppleDouble `._*` resource-fork
# members; the archive stays portable to Linux/Windows extraction tools.
( cd "$DIST" && COPYFILE_DISABLE=1 tar -czf "${NAME}.tar.gz" "$NAME" )
if command -v shasum >/dev/null 2>&1; then
  ( cd "$DIST" && shasum -a 256 "${NAME}.tar.gz" > "${NAME}.tar.gz.sha256" )
else
  ( cd "$DIST" && sha256sum "${NAME}.tar.gz" > "${NAME}.tar.gz.sha256" )
fi

[ -s "$TARBALL" ] || die "release artifact is missing or empty: $TARBALL"
[ -s "$SHA256" ] || die "sha256 file is missing or empty: $SHA256"
[ "$(wc -c < "$TARBALL" | tr -d ' ')" -gt 0 ] || die "release artifact is empty: $TARBALL"

echo
echo "artifact : ${TARBALL}"
echo "size     : $(wc -c < "$TARBALL") bytes"
echo "sha256   : $(cut -d' ' -f1 "$SHA256")"
