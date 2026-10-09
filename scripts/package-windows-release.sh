#!/usr/bin/env bash
# package-windows-release.sh -- build and package the native Windows release zip.
#
# Designed to run both on a Windows GitHub runner (native MSVC) and on a Linux
# host that cross-compiles the GNU target:
#   * on Windows, TARGET defaults to the MSVC host triple and the CRT is linked
#     statically (`+crt-static`), so the package needs no VC++ redistributable;
#   * on Linux, pass TARGET=x86_64-pc-windows-gnu and install MinGW-w64.
#
# Produces a zip containing:
#   * bin/yacr.exe            release GUI host (Slint + shared CAD wgpu renderer),
#   * bin/cad-cli-tools.exe   release headless CLI (scan/.../render),
#   * fonts/                  the CAD font package (same assembly as Linux),
#   * docs/, scripts/, LICENSE, README, THIRD_PARTY_NOTICES and PACKAGE.txt,
#   * any toolchain runtime DLL the exes import that is not a Windows system
#     library. MSVC with `+crt-static` and Rust's windows-gnu std normally need
#     none, so bin/ is usually just the two executables; an unresolved
#     non-system import aborts the package.
#
# Usage:
#   scripts/package-windows-release.sh
#   TARGET=x86_64-pc-windows-gnu scripts/package-windows-release.sh   # from Linux
#
# Env:
#   TARGET        rust target triple. Default: the rustc host on Windows, else
#                 x86_64-pc-windows-gnu.
#   WITH_FONTS    1 (default) to assemble the full CAD font catalogue into
#                 `fonts/` via scripts/fetch-fonts.sh (needs network); 0 keeps
#                 only the committed fonts/ (offline).
#   BUNDLE_DLLS   1 (default) to relocate non-system runtime DLLs into bin/;
#                 0 to ship bare executables (requires crt-static / static std).
#   CARGO_XWIN    1 to build the MSVC target locally on Linux through
#                 `cargo xwin` (Windows CI uses plain `cargo`).
#   PYTHON        python interpreter (default: python3, then python).
#   YACR_WINDOWS_SMOKE  1 to run the packaged CLI/GUI under Wine (Linux only)
#                 and assert the documented argument-parse behaviour.
#   RUSTFLAGS     honoured; on an MSVC target `-C target-feature=+crt-static` is
#                 appended unless it is already present.
#   FONT_BASE_URL / FONT_FALLBACK_BASE_URL / FONT_RETRIES  passed to fetch-fonts.sh.
#
# Artifacts (under target/, which is gitignored):
#   target/<TARGET>/release/dist/yacr-<version>-windows-x86_64.zip
#   target/<TARGET>/release/dist/yacr-<version>-windows-x86_64.zip.sha256
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

HOST="$(rustc -vV | sed -n 's/^host: //p')"
case "$HOST" in
  *-pc-windows-*) DEFAULT_TARGET="$HOST" ;;
  *) DEFAULT_TARGET="x86_64-pc-windows-gnu" ;;
esac
TARGET="${TARGET:-$DEFAULT_TARGET}"
if [[ "$TARGET" != *-pc-windows-* ]]; then
  echo "ERROR: TARGET '$TARGET' is not a Windows target triple" >&2
  exit 1
fi

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
NAME="yacr-${VERSION}-windows-x86_64"
RELEASE="target/${TARGET}/release"
DIST="${RELEASE}/dist"
STAGE="${DIST}/${NAME}"
WITH_FONTS="${WITH_FONTS:-1}"
BUNDLE_DLLS="${BUNDLE_DLLS:-1}"

# Resolve a Python interpreter portably (Git Bash on Windows may only have
# `python`). Used for PE inspection, zipping and hashing.
if [ -z "${PYTHON:-}" ]; then
  for candidate in python3 python; do
    if command -v "$candidate" >/dev/null 2>&1; then
      PYTHON="$candidate"
      break
    fi
  done
fi
export PYTHON
[ -n "$PYTHON" ] || { echo "ERROR: python3 (or python) is required" >&2; exit 1; }

# MSVC: statically link the CRT so a stock Windows 10/11 install (no VC++
# redistributable) can run the package. Rust's windows-gnu std is already static.
if [[ "$TARGET" == *-msvc ]] && [[ "${RUSTFLAGS:-}" != *crt-static* ]]; then
  export RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+crt-static"
fi

if [[ "$TARGET" == *-gnu ]] && ! command -v x86_64-w64-mingw32-gcc >/dev/null; then
  echo "ERROR: GNU target $TARGET needs MinGW-w64 (gcc-mingw-w64-x86-64)" >&2
  exit 1
fi
if ! rustup target list --installed | grep -qx "$TARGET"; then
  echo "ERROR: rust target $TARGET is not installed; run: rustup target add $TARGET" >&2
  exit 1
fi

echo "==> building cad-cli-tools.exe and yacr.exe (release, locked, $TARGET)"
if [ "${CARGO_XWIN:-0}" = "1" ]; then
  # Local Linux validation of the MSVC target (Windows CI uses plain `cargo`).
  XWIN_ACCEPT_LICENSE=1 cargo xwin build --release --locked --target "$TARGET" -p cad-cli-tools -p app-windows
else
  cargo build --release --locked --target "$TARGET" -p cad-cli-tools -p app-windows
fi

CLI="${RELEASE}/cad-cli-tools.exe"
GUI="${RELEASE}/yacr.exe"
[ -f "$CLI" ] || { echo "ERROR: missing $CLI" >&2; exit 1; }
[ -f "$GUI" ] || { echo "ERROR: missing $GUI" >&2; exit 1; }

echo "==> staging ${STAGE}"
rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/docs" "$STAGE/scripts" "$STAGE/fonts"
cp "$CLI" "$STAGE/bin/cad-cli-tools.exe"
cp "$GUI" "$STAGE/bin/yacr.exe"
cp LICENSE THIRD_PARTY_NOTICES.md README.md "$STAGE/"
cp docs/cli.md docs/headless-render.md docs/render-backends.md docs/build.md \
   docs/validation.md docs/compatibility.md docs/windows-app.md docs/fonts.md \
   "$STAGE/docs/"
cp scripts/fetch-test-dwg.sh scripts/render-smoke.sh "$STAGE/scripts/"

# Windows system libraries that must come from the OS. Everything else an exe
# imports has to be statically linked or shipped in bin/. `ucrtbase.dll` and the
# API sets are OS components; a `vcruntime*`/`msvcp*` import means the MSVC
# build was not crt-static and is rejected rather than silently shipped.
SYSTEM_DLL_RE='^(kernel32|user32|gdi32|advapi32|shell32|shlwapi|ole32|oleaut32|combase|comctl32|comdlg32|rpcrt4|setupapi|cfgmgr32|dwmapi|dwrite|d2d1|dxgi|d3d11|d3d12|dcomp|vulkan-1|opengl32|msvcrt|ucrtbase|ntdll|ws2_32|mswsock|winmm|imm32|uxtheme|bcrypt|bcryptprimitives|crypt32|secur32|propsys|userenv|uiautomationcore|windowscodecs|version|dbghelp|iphlpapi|dnsapi|normaliz|hid|winhttp|wintrust|sechost|ncrypt|powrprof|psapi|sspicli|oleacc|schannel|msimg32|usp10|d3dcompiler_47|api-ms-win-[a-z0-9-]+|ext-ms-win-[a-z0-9-]+)\.dll$'

mingw_lib_dirs() {
  local dir
  for dir in /usr/lib/gcc/x86_64-w64-mingw32/*-win32 \
             /usr/lib/gcc/x86_64-w64-mingw32/*-posix \
             /usr/x86_64-w64-mingw32/lib; do
    [ -d "$dir" ] && printf '%s\n' "$dir"
  done
}

# Library names files may carry mixed case on Windows; compare case-insensitively.
find_bundled_or_mingw() {
  local name="$1"
  if [ -f "$STAGE/bin/$name" ]; then
    printf '%s\n' "$STAGE/bin/$name"
    return 0
  fi
  local dir
  while IFS= read -r dir; do
    if [ -f "$dir/$name" ]; then
      printf '%s\n' "$dir/$name"
      return 0
    fi
  done < <(mingw_lib_dirs)
  return 1
}

copy_runtime_dlls() {
  local exe="$1" dest="$2" name found count=0
  while IFS= read -r name; do
    [ -n "$name" ] || continue
    if [[ "$name" =~ $SYSTEM_DLL_RE ]]; then
      continue
    fi
    if ! found="$(find_bundled_or_mingw "$name")"; then
      echo "ERROR: $exe imports non-system '$name'; it is neither crt-static nor available to bundle" >&2
      return 1
    fi
    if [ "$found" != "$dest/$name" ]; then
      cp -L "$found" "$dest/$name"
    fi
    count=$((count + 1))
  done < <("$PYTHON" "$ROOT/scripts/check-pe-imports.py" "$exe")
  echo "  bundled ${count} non-system runtime DLL(s) for $(basename "$exe")"
}

echo "==> verifying executables are PE32+ x86-64 and imports are resolvable"
for exe in "$STAGE/bin/cad-cli-tools.exe" "$STAGE/bin/yacr.exe"; do
  "$PYTHON" "$ROOT/scripts/check-pe-imports.py" --machine "$exe" >/dev/null
done
if [ "$BUNDLE_DLLS" = "1" ]; then
  copy_runtime_dlls "$STAGE/bin/cad-cli-tools.exe" "$STAGE/bin"
  copy_runtime_dlls "$STAGE/bin/yacr.exe" "$STAGE/bin"
else
  echo "==> BUNDLE_DLLS=0: shipping bare executables"
fi

if [ "$WITH_FONTS" = "1" ]; then
  echo "==> assembling CAD fonts (fetch-fonts.sh) into $STAGE/fonts"
  "$ROOT/scripts/fetch-fonts.sh" "$STAGE/fonts"
else
  echo "==> WITH_FONTS=0: staging committed fonts/ only"
  cp -R "$ROOT"/fonts/. "$STAGE/fonts/"
fi

cat > "$STAGE/PACKAGE.txt" <<EOF
yacr Windows release package
============================
version : ${VERSION}
target  : ${TARGET} (PE32+ x86-64)
built   : $(date -u +%Y-%m-%dT%H:%M:%SZ)
fonts   : ${WITH_FONTS}; 1 = full catalogue vendored (see below), 0 = osifont only
dlls    : ${BUNDLE_DLLS}; 1 = non-system runtime DLLs copied into bin/ (usually none)

Contents
--------
bin/yacr.exe            GUI host: Slint + the shared CAD wgpu renderer. Runs the
                        same UI as the Linux yacr-linux build (DX12/Vulkan via
                        wgpu). Without --open it shows the built-in demo; the
                        Open button uses the native file dialog.
bin/cad-cli-tools.exe   headless CLI (scan/measure/build-representation/render/...).
fonts/                  CAD font package in fonts.json format: the mlightcad/cad-data
                        catalogue plus the committed QCAD osifont.ttf (provenance in
                        fonts/SOURCE.md); auto-loaded from this sibling dir by both
                        executables. Missing drawing fonts fall back to the default
                        outline face / the Windows system font.
docs/                   CLI, headless render, backends, build, validation, compat,
                        Windows host and font notes
scripts/                fetch-test-dwg.sh (real-DWG corpus) and render-smoke.sh
                        (POSIX shell helpers; on Windows run them from Git Bash/WSL)
README.md, LICENSE, THIRD_PARTY_NOTICES.md

Quick start (Windows)
---------------------
  bin\\yacr.exe                          REM windowed GUI
  bin\\yacr.exe --open C:\\path\\drawing.dwg
  bin\\cad-cli-tools.exe --help
  bin\\cad-cli-tools.exe render drawing.dwg --png frame.png --width 1280 --height 720

Runtime notes (not a compatibility claim)
-----------------------------------------
  * The GUI needs a Windows GPU driver exposing DX12 or Vulkan through wgpu.
    MSVC builds link the CRT statically; no VC++ redistributable is required.
  * The window, device selection and the native Open dialog depend on the host
    GPU/driver and are not a compatibility claim; see docs/windows-app.md.
  * \`--headless\` offscreen rendering requires software Vulkan (Mesa lavapipe or
    equivalent) installed on the Windows host; it is not bundled.

No unlicensed user DWG samples or golden images are bundled. The mlightcad/cad-data
fonts in fonts/ are third-party and vendored by the deployer at packaging time
(scripts/fetch-fonts.sh); redistribution terms are the deployer's responsibility
(see docs/fonts.md). Only the authorized QCAD osifont.ttf (GPL-3 with font
exception) is committed to the repository. This package is not a compatibility or
performance claim.
See docs/windows-app.md and docs/validation.md.
EOF

if [ "${YACR_WINDOWS_SMOKE:-0}" = "1" ]; then
  command -v wine >/dev/null || {
    echo "ERROR: YACR_WINDOWS_SMOKE=1 but wine is not installed" >&2
    exit 1
  }
  echo "==> Wine smoke: packaged CLI --help"
  WINEDEBUG="${WINEDEBUG:--all}" wine "$STAGE/bin/cad-cli-tools.exe" --help >/dev/null
  echo "==> Wine smoke: packaged GUI rejects a bad option (cannot open a window headless)"
  set +e
  gui_out="$(WINEDEBUG="${WINEDEBUG:--all}" wine "$STAGE/bin/yacr.exe" --bogus value 2>&1)"
  gui_status=$?
  set -e
  if [ "$gui_status" -eq 0 ] || [[ "$gui_out" != *"unknown option or invalid value: --bogus"* ]]; then
    echo "ERROR: packaged yacr.exe did not report the documented parse error (status=${gui_status})" >&2
    printf '%s\n' "$gui_out" >&2
    exit 1
  fi
  echo "    (Wine is software translation, not a real-Windows acceptance; window and GPU NOT RUN)"
fi

ZIP="${DIST}/${NAME}.zip"
echo "==> creating ${ZIP}"
rm -f "$ZIP" "${ZIP}.sha256"
"$PYTHON" - "$DIST" "$NAME" <<'PY'
import hashlib, pathlib, sys, zipfile

dist, name = pathlib.Path(sys.argv[1]), sys.argv[2]
root = dist / name
out = dist / f"{name}.zip"
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
    for path in sorted(root.rglob("*")):
        archive.write(path, path.relative_to(dist).as_posix())
digest = hashlib.sha256(out.read_bytes()).hexdigest()
(out.parent / f"{name}.zip.sha256").write_text(f"{digest}  {name}.zip\n")
print(f"zip: {out.name} ({out.stat().st_size} bytes)")
PY

echo
echo "artifact : ${ZIP}"
echo "size     : $(wc -c < "$ZIP") bytes"
echo "sha256   : $(cut -d' ' -f1 "${ZIP}.sha256")"
