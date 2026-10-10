#!/usr/bin/env bash
# package-linux-release.sh -- build and package the native Linux release bundle.
#
# Produces a self-contained tarball containing:
#   * bin/cad-cli-tools  release headless CLI (includes the headless `render`),
#   * bin/yacr-linux     release GUI host (Slint + shared CAD wgpu renderer),
#                        linked with a relocatable `$ORIGIN/../lib` RPATH,
#   * lib/               the non-base-system shared libraries yacr-linux needs,
#   * docs/ and the CAD `fonts/` package (optionally the full catalogue),
#   * scripts/fetch-test-dwg.sh and scripts/render-smoke.sh.
# Software Vulkan (Mesa lavapipe) is supplied by the host at runtime for
# headless rendering; it is not bundled.
#
# Usage:
#   scripts/package-linux-release.sh
#
# Optional smoke tests (run against the staged package, not the build tree):
#   YACR_TEST_DWG=/path/to/input.dwg scripts/package-linux-release.sh
#       render the DWG with the packaged CLI and check the PNG is non-blank.
#   YACR_LINUX_SMOKE=1 scripts/package-linux-release.sh
#       run the packaged GUI headless (needs a reachable Vulkan ICD, e.g.
#       VK_ICD_FILENAMES=.../lvp_icd.json) and check the acceptance report shows
#       rendered CAD frames. A green smoke is software-GPU evidence only.
#
# Env:
#   WITH_FONTS   1 (default) to assemble the full CAD font package into `fonts/`
#                via scripts/fetch-fonts.sh (mlightcad catalogue + committed
#                QCAD osifont; needs network); 0 for an offline build that keeps
#                only the committed fonts/.
#   BUNDLE_LIBS  1 (default) to relocate the GUI's non-base shared libraries into
#                `lib/` and rely on the RPATH; 0 to ship the bare binary (the
#                host must then provide every dependency).
#   FONT_BASE_URL / FONT_FALLBACK_BASE_URL / FONT_RETRIES  passed through to
#                fetch-fonts.sh.
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
BUNDLE_LIBS="${BUNDLE_LIBS:-1}"

# RPATH recorded in bin/yacr-linux. `$ORIGIN` is expanded by the dynamic loader
# to the directory holding the executable, so this resolves to the package's
# lib/. Old-style DT_RPATH (--disable-new-dtags) is deliberate: unlike
# DT_RUNPATH it is inherited by the bundled libraries' own dependencies, which
# is what makes a fully relocated lib/ work.
GUI_RPATH='$ORIGIN/../lib'

echo "==> building cad-cli-tools (release, locked, offline)"
cargo build --release --offline --locked -p cad-cli-tools

echo "==> building yacr-linux GUI host (release, locked, offline, RPATH=${GUI_RPATH})"
cargo rustc -p app-linux --bin yacr-linux --release --offline --locked -- \
  -C "link-arg=-Wl,--disable-new-dtags,-rpath,${GUI_RPATH}"

BIN="target/release/cad-cli-tools"
GUI="target/release/yacr-linux"
[ -x "$BIN" ] || { echo "ERROR: missing $BIN" >&2; exit 1; }
[ -x "$GUI" ] || { echo "ERROR: missing $GUI" >&2; exit 1; }

echo "==> staging ${STAGE}"
rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/lib" "$STAGE/docs" "$STAGE/scripts" "$STAGE/fonts"
cp "$BIN" "$STAGE/bin/cad-cli-tools"
cp "$GUI" "$STAGE/bin/yacr-linux"
cp LICENSE THIRD_PARTY_NOTICES.md README.md "$STAGE/"
cp docs/cli.md docs/headless-render.md docs/render-backends.md docs/build.md \
   docs/validation.md docs/compatibility.md docs/linux-app.md docs/fonts.md \
   "$STAGE/docs/"
cp scripts/fetch-test-dwg.sh scripts/render-smoke.sh "$STAGE/scripts/"

# Desktop integration: the icon theme entry and a launcher so a window manager
# can associate the running window with the packaged icon. The SVG is the
# single source of truth (`assets/yacr-icon.svg`); a 256px PNG raster is kept
# for icon themes that do not read scalable SVGs. `Icon=yacr` matches the
# basename `yacr.svg`/`yacr.png` above.
mkdir -p "$STAGE/share/icons/hicolor/scalable/apps" \
         "$STAGE/share/icons/hicolor/256x256/apps" \
         "$STAGE/share/applications"
cp "$ROOT/assets/yacr-icon.svg" "$STAGE/share/icons/hicolor/scalable/apps/yacr.svg"
cp "$ROOT/assets/icons/yacr-256.png" "$STAGE/share/icons/hicolor/256x256/apps/yacr.png"
cat > "$STAGE/share/applications/yacr.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=yacr
Exec=yacr-linux %F
Icon=yacr
Terminal=false
Categories=Graphics;Engineering;
MimeType=application/dwg;application/dxf;
EOF

# Libraries that must come from the host: glibc and the GCC/C++ runtime. They
# are coupled to the kernel and loader, so shipping them in a tarball risks a
# hard break on a different glibc; everything else in the GUI's dynamic closure
# is relocated so the bundle does not require the host to provide
# fontconfig/freetype (Slint's font path) or their dependencies.
BASE_LIB_RE='^(ld-linux[^/]*\.so.*|libc\.so.*|libm\.so.*|libdl\.so.*|libpthread\.so.*|librt\.so.*|libgcc_s\.so.*|libstdc\+\+\.so.*|libresolv\.so.*|libnsl\.so.*|libutil\.so.*)$'

copy_gui_libs() {
  local binary="$1" dest="$2" ldd_out path base count=0
  if ! ldd_out="$(ldd "$binary" 2>&1)"; then
    echo "ERROR: unresolved shared-library dependency for $binary" >&2
    printf '%s\n' "$ldd_out" >&2
    return 1
  fi
  while IFS= read -r path; do
    [ -n "$path" ] || continue
    base="$(basename "$path")"
    if [[ "$base" =~ $BASE_LIB_RE ]]; then
      continue
    fi
    cp -L "$path" "$dest/$base"
    count=$((count + 1))
  done < <(printf '%s\n' "$ldd_out" | awk '/=>/ && $3 != "not" {print $3} /^\t\// {print $1}')
  echo "  relocated ${count} non-base shared libraries into lib/"
}

if [ "$BUNDLE_LIBS" = "1" ]; then
  echo "==> bundling yacr-linux shared libraries"
  copy_gui_libs "$GUI" "$STAGE/lib"
else
  echo "==> BUNDLE_LIBS=0: shipping the bare GUI binary (host must provide deps)"
fi

# CAD fonts: with WITH_FONTS=1 (default) the platform-agnostic packer downloads
# the mlightcad/cad-data catalogue and merges the committed fonts/ package
# (QCAD osifont), so a packaged CLI and GUI shape every catalogue font locally
# without network. WITH_FONTS=0 keeps only the committed fonts/ for an offline
# build.
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
libs    : ${BUNDLE_LIBS}; 1 = non-base GUI libraries relocated into lib/ (RPATH), 0 = bare binary

Contents
--------
bin/cad-cli-tools     headless CLI (scan/measure/build-representation/render/...)
bin/yacr-linux        GUI host: Slint + the shared CAD wgpu renderer. Windowed by
                      default (needs a display server and a GPU/Vulkan stack);
                      \`--headless --output NEW_DIR\` renders offscreen frames and a
                      report.json.
lib/                  shared libraries for bin/yacr-linux that are not part of the
                      base system (fontconfig/freetype and their dependencies).
                      bin/yacr-linux carries an \$ORIGIN/../lib RPATH, so these take
                      precedence over the host's copies. glibc, the loader and
                      libgcc/libstdc++ are NOT bundled: the host must supply a
                      compatible glibc and the fontconfig \`fc-match\` tool.
fonts/                CAD font package in fonts.json format: the mlightcad/cad-data
                      catalogue plus the committed QCAD osifont.ttf (provenance in
                      fonts/SOURCE.md); auto-loaded from this sibling dir by both the
                      CLI and the GUI. Missing drawing fonts fall back to the default
                      outline face / the system fontconfig default.
docs/                 CLI, headless render, backends, build, validation, compat,
                      Linux host and font notes
scripts/fetch-test-dwg.sh   download a curated real-DWG corpus to /tmp
scripts/render-smoke.sh     render a DWG and verify the PNG is non-blank
share/icons/hicolor/scalable/apps/yacr.svg
share/icons/hicolor/256x256/apps/yacr.png
                      the app icon (from assets/yacr-icon.svg) in a standard icon
                      theme location.
share/applications/yacr.desktop
                      launcher entry (Icon=yacr, Exec=yacr-linux); install it and
                      the icon theme under \$HOME/.local/share or /usr/share for a
                      window manager to associate the running window with the icon.
README.md, LICENSE, THIRD_PARTY_NOTICES.md

Quick start
-----------
  # force Mesa lavapipe (software Vulkan); path varies by distro
  export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
  ./bin/cad-cli-tools --help
  ./bin/cad-cli-tools render drawing.dwg --png frame.png --width 1280 --height 720
  ./scripts/render-smoke.sh drawing.dwg frame.png
  ./bin/yacr-linux                                    # desktop window
  ./bin/yacr-linux --headless --output /tmp/yacr-out  # offscreen frames + report

Runtime notes (not a compatibility claim)
-----------------------------------------
  * The GUI still needs a kernel, a matching-or-newer glibc and, in windowed mode,
    a display server plus a working Vulkan driver. Software Vulkan (Mesa
    lavapipe) is not bundled; install it or point VK_ICD_FILENAMES at a driver.
  * The desktop Open dialog uses xdg-desktop-portal / D-Bus; without them use
    --open PATH (portals are not installed by this package).
  * Bundled libraries are host-distribution-specific; on a different distro or
    libc generation prefer BUNDLE_LIBS=0 or rebuild from source.

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
  echo "==> CLI smoke test with ${YACR_TEST_DWG}"
  YACR_CLI="$BIN" "$STAGE/scripts/render-smoke.sh" "$YACR_TEST_DWG" \
    "${DIST}/${NAME}.smoke.png" 800 600
fi

echo "==> verifying packaged CLI runs"
"$STAGE/bin/cad-cli-tools" --help >/dev/null

echo "==> verifying packaged GUI loads"
# yacr-linux has no --help; `--headless` without --output is a documented parse
# error that is only printed after the dynamic loader has resolved every
# dependency, then exits 1.
set +e
gui_out="$("$STAGE/bin/yacr-linux" --headless 2>&1)"
gui_status=$?
set -e
if [ "$gui_status" -eq 0 ] || [[ "$gui_out" != *"--headless requires --output NEW_DIRECTORY"* ]]; then
  echo "ERROR: packaged yacr-linux failed to load/parse (status=${gui_status})" >&2
  printf '%s\n' "$gui_out" >&2
  exit 1
fi
if ldd "$STAGE/bin/yacr-linux" | grep -q 'not found'; then
  echo "ERROR: packaged yacr-linux has unresolved shared libraries" >&2
  ldd "$STAGE/bin/yacr-linux" >&2
  exit 1
fi
if [ "$BUNDLE_LIBS" = "1" ]; then
  if ! readelf -d "$STAGE/bin/yacr-linux" | grep -q 'Library rpath: \[\$ORIGIN/\.\./lib\]'; then
    echo "ERROR: yacr-linux is missing the \$ORIGIN/../lib RPATH" >&2
    readelf -d "$STAGE/bin/yacr-linux" | grep -i path >&2 || true
    exit 1
  fi
  # At least one relocated library must resolve from the staged package, proving
  # the RPATH is honoured instead of silently falling back to the host.
  if ! ldd "$STAGE/bin/yacr-linux" | grep -Fq "$STAGE"; then
    echo "ERROR: no packaged library resolved from lib/; the RPATH is not honoured" >&2
    ldd "$STAGE/bin/yacr-linux" >&2
    exit 1
  fi
fi

if [ "${YACR_LINUX_SMOKE:-0}" = "1" ]; then
  echo "==> GUI headless smoke with the packaged yacr-linux"
  SMOKE_DIR="${DIST}/${NAME}.gui-smoke"
  if [ -e "$SMOKE_DIR" ]; then
    echo "ERROR: smoke output dir already exists: $SMOKE_DIR" >&2
    exit 1
  fi
  "$STAGE/bin/yacr-linux" --headless --output "$SMOKE_DIR" --size 1280x800
  python3 - "$SMOKE_DIR" <<'PY'
import json, pathlib, sys

p = pathlib.Path(sys.argv[1])
r = json.loads((p / "report.json").read_text())
assert r["host"] == "app-linux", r.get("host")
assert r["cadFrames"] > 0, r.get("cadFrames")
assert r["renderError"] is None, r.get("renderError")
for name in ("initial.png", "navigation.png"):
    assert (p / name).read_bytes().startswith(b"\x89PNG\r\n\x1a\n"), name
print(f"packaged GUI headless smoke passed: {p}; manual visual review still required")
PY
fi

TARBALL="${DIST}/${NAME}.tar.gz"
echo "==> creating ${TARBALL}"
rm -f "$TARBALL" "${TARBALL}.sha256"
tar -C "$DIST" -czf "$TARBALL" "$NAME"
( cd "$DIST" && sha256sum "${NAME}.tar.gz" > "${NAME}.tar.gz.sha256" )

echo
echo "artifact : ${TARBALL}"
echo "size     : $(wc -c < "$TARBALL") bytes"
echo "sha256   : $(cut -d' ' -f1 "${TARBALL}.sha256")"
