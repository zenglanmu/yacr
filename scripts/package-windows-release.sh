#!/usr/bin/env bash
# package-windows-release.sh -- build and package the native Windows release zip
# for the current React/Tauri host.
#
# `apps/app-windows` is a real Tauri 2 / WebView2 host whose binary is
# `yacr-windows.exe` (see `[[bin]] name = "yacr-windows"`). The shared React UI
# lives in `apps/desktop-ui` and is a separate Vite bundle: at compile time
# `tauri::generate_context!` embeds `apps/desktop-ui/dist` into the executable,
# so the frontend MUST be built before `cargo build` or the zip ships a stale /
# missing UI. That ordering is enforced below; there is no silent fallback.
#
# Runs on a Windows GitHub runner (native MSVC, plain `cargo`) and on Linux:
#   * Windows: TARGET defaults to the rustc host MSVC triple and the CRT is
#     linked statically (`+crt-static`), so the package needs no VC++
#     redistributable. The WebView2 loader is linked statically as well
#     (webview2-com-sys → WebView2LoaderStatic), so no WebView2Loader.dll is
#     shipped; if one ever shows up as an import the non-system DLL check fails.
#   * Linux: the default is x86_64-pc-windows-gnu (needs MinGW-w64). For local
#     validation of the MSVC target, CARGO_XWIN=1 uses `cargo xwin` (CI still
#     uses plain `cargo` on windows-latest).
#
# Output (under target/, gitignored):
#   target/<TARGET>/release/dist/yacr-<version>-windows-x86_64/
#     bin/yacr-windows.exe, bin/cad-cli-tools.exe, yacr.ico, fonts/,
#     docs/, scripts/, LICENSE, README.md, THIRD_PARTY_NOTICES.md, PACKAGE.txt
#   target/<TARGET>/release/dist/yacr-<version>-windows-x86_64.zip
#   target/<TARGET>/release/dist/yacr-<version>-windows-x86_64.zip.sha256
#
# Usage:
#   scripts/package-windows-release.sh
#   TARGET=x86_64-pc-windows-gnu scripts/package-windows-release.sh   # Linux
#   CARGO_XWIN=1 TARGET=x86_64-pc-windows-msvc scripts/package-windows-release.sh
#
# Env:
#   TARGET        Windows rust target triple (x86_64 only; check-pe-imports.py
#                 validates 0x8664). Default: the rustc host on Windows, else
#                 x86_64-pc-windows-gnu.
#   WITH_FONTS    1 (default) to assemble the full CAD font catalogue into
#                 `fonts/` via scripts/fetch-fonts.sh (network); 0 stages the
#                 committed fonts/ only (offline). `fonts/fonts.json` is
#                 required in both modes.
#   SKIP_UI_BUILD 1 to reuse an existing apps/desktop-ui/dist (only accepted
#                 when dist/index.html exists and is non-empty).
#   CARGO_XWIN    1 to build an MSVC target on Linux through `cargo xwin`.
#   PYTHON        python interpreter (default: python3, then python).
#   YACR_WINDOWS_SMOKE  1 to run the packaged CLI under Wine (Linux only).
#                 Software translation, not a real-Windows acceptance.
#   RUSTFLAGS     honoured; on an MSVC target `-C target-feature=+crt-static` is
#                 appended unless already present.
#   FONT_BASE_URL / FONT_FALLBACK_BASE_URL / FONT_RETRIES  passed to
#                 fetch-fonts.sh.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

step() { printf '==> %s\n' "$*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

# --- target selection -------------------------------------------------------
HOST="$(rustc -vV | sed -n 's/^host: //p')"
[ -n "$HOST" ] || die "cannot determine the rustc host triple"
case "$HOST" in
  x86_64-pc-windows-*) DEFAULT_TARGET="$HOST" ;;
  *) DEFAULT_TARGET="x86_64-pc-windows-gnu" ;;
esac
TARGET="${TARGET:-$DEFAULT_TARGET}"
case "$TARGET" in
  x86_64-pc-windows-msvc | x86_64-pc-windows-gnu) ;;
  *-pc-windows-*)
    die "TARGET '$TARGET' is not supported: scripts/check-pe-imports.py validates PE32+ x86-64 (0x8664) only"
    ;;
  *) die "TARGET '$TARGET' is not a Windows target triple" ;;
esac

# --- layout -----------------------------------------------------------------
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[ -n "$VERSION" ] || die "cannot read the workspace version from Cargo.toml"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]] || die "unexpected workspace version '$VERSION'"
NAME="yacr-${VERSION}-windows-x86_64"
RELEASE="target/${TARGET}/release"
DIST="${RELEASE}/dist"
STAGE="${DIST}/${NAME}"
GUI="yacr-windows.exe"
CLI="cad-cli-tools.exe"
WITH_FONTS="${WITH_FONTS:-1}"

# --- toolchain resolution ---------------------------------------------------
if [ -z "${PYTHON:-}" ]; then
  for candidate in python3 python; do
    if command -v "$candidate" >/dev/null 2>&1; then
      PYTHON="$candidate"
      break
    fi
  done
fi
[ -n "${PYTHON:-}" ] || die "python3 (or python) is required for the PE checks and the zip step"
export PYTHON

# MSVC: statically link the CRT so a stock Windows 10/11 install runs the
# package without the VC++ redistributable. Rust's windows-gnu std is static.
if [[ "$TARGET" == *-msvc ]] && [[ "${RUSTFLAGS:-}" != *crt-static* ]]; then
  export RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+crt-static"
fi

if [[ "$TARGET" == *-gnu ]] && ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  die "GNU target $TARGET needs MinGW-w64 (gcc-mingw-w64-x86-64)"
fi
if command -v rustup >/dev/null 2>&1 && [ "${CARGO_XWIN:-0}" != "1" ]; then
  if ! rustup target list --installed 2>/dev/null | grep -qx "$TARGET"; then
    die "rust target $TARGET is not installed; run: rustup target add $TARGET"
  fi
fi
if [ "${CARGO_XWIN:-0}" = "1" ]; then
  [[ "$TARGET" == *-msvc ]] || die "CARGO_XWIN=1 only applies to an MSVC target (got $TARGET)"
  command -v cargo-xwin >/dev/null 2>&1 \
    || die "CARGO_XWIN=1 but cargo-xwin is not installed (cargo install cargo-xwin)"
fi

# --- frontend (must precede cargo: tauri::generate_context! embeds dist) -----
UI_DIR="$ROOT/apps/desktop-ui"
UI_INDEX="$UI_DIR/dist/index.html"
if [ "${SKIP_UI_BUILD:-0}" = "1" ]; then
  [ -s "$UI_INDEX" ] \
    || die "SKIP_UI_BUILD=1 but $UI_INDEX is missing or empty; the Tauri host would embed no UI"
  step "reusing existing React UI bundle at apps/desktop-ui/dist"
else
  command -v npm >/dev/null 2>&1 || die "npm is required to build the React UI (apps/desktop-ui)"
  [ -f "$UI_DIR/package-lock.json" ] || die "apps/desktop-ui/package-lock.json is missing; npm ci cannot run"
  step "building the React UI bundle (apps/desktop-ui: npm ci && npm run build)"
  ( cd "$UI_DIR" && npm ci && npm run build )
  [ -s "$UI_INDEX" ] || die "React UI build produced no apps/desktop-ui/dist/index.html"
fi
UI_PLUGIN="$UI_DIR/dist/plugins/tch.js"
[ -s "$UI_PLUGIN" ] \
  || die "$UI_PLUGIN is missing or empty; the Tianzheng vendor plugin would not be embedded (is the plugins/tch submodule initialized?)"

# --- build ------------------------------------------------------------------
step "building $GUI and $CLI (release, locked, $TARGET)"
if [ "${CARGO_XWIN:-0}" = "1" ]; then
  # Local Linux validation of the MSVC target (Windows CI uses plain cargo).
  XWIN_ACCEPT_LICENSE=1 cargo xwin build --release --locked --target "$TARGET" \
    -p cad-cli-tools -p app-windows --features tauri/custom-protocol
else
  # `tauri/custom-protocol` is what embeds `apps/desktop-ui/dist` into the
  # executable; without it the release binary still expects the dev server.
  cargo build --release --locked --target "$TARGET" -p cad-cli-tools -p app-windows \
    --features tauri/custom-protocol
fi

GUI_PATH="${RELEASE}/${GUI}"
CLI_PATH="${RELEASE}/${CLI}"
[ -f "$GUI_PATH" ] || die "missing $GUI_PATH (cargo build did not produce the Tauri host binary)"
[ -f "$CLI_PATH" ] || die "missing $CLI_PATH (cargo build did not produce the CLI binary)"

# --- staging ----------------------------------------------------------------
step "staging ${STAGE}"
rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/docs" "$STAGE/scripts" "$STAGE/fonts"
cp "$GUI_PATH" "$STAGE/bin/$GUI"
cp "$CLI_PATH" "$STAGE/bin/$CLI"
cp README.md LICENSE THIRD_PARTY_NOTICES.md "$STAGE/"
# App icon (from assets/yacr-icon.svg). yacr-windows.exe also embeds this icon
# through apps/app-windows/build.rs -> tauri-build; the standalone copy is for
# shortcuts / installers that want the raw asset.
cp "$ROOT/assets/yacr-icon.ico" "$STAGE/yacr.ico"
[ -s "$STAGE/yacr.ico" ] || die "assets/yacr-icon.ico is missing or empty"

# Only ship documents that exist in this tree (the docs set evolves).
DOCS=(
  docs/cli.md
  docs/headless-render.md
  docs/render-backends.md
  docs/build.md
  docs/validation.md
  docs/compatibility.md
  docs/windows-app.md
  docs/fonts.md
)
COPIED_DOCS=0
for doc in "${DOCS[@]}"; do
  if [ -f "$ROOT/$doc" ]; then
    cp "$ROOT/$doc" "$STAGE/docs/"
    COPIED_DOCS=$((COPIED_DOCS + 1))
  else
    step "skipping absent doc $doc"
  fi
done
[ "$COPIED_DOCS" -gt 0 ] || die "none of the expected docs/*.md files exist"

for helper in scripts/fetch-test-dwg.sh scripts/render-smoke.sh; do
  [ -f "$ROOT/$helper" ] || die "missing required helper $helper"
  cp "$ROOT/$helper" "$STAGE/scripts/"
done

# --- PE + icon verification -------------------------------------------------
step "verifying executables are PE32+ x86-64 and imports are resolvable"
declare -A SEEN_DLLS=()
BUNDLED_DLLS=()

mingw_lib_dirs() {
  local dir
  for dir in /usr/lib/gcc/x86_64-w64-mingw32/*-win32 \
             /usr/lib/gcc/x86_64-w64-mingw32/*-posix \
             /usr/x86_64-w64-mingw32/lib; do
    [ -d "$dir" ] && printf '%s\n' "$dir"
  done
}

# Library names may carry mixed case on Windows; compare case-insensitively.
find_runtime_dll() {
  local name="$1" dir
  if [ -f "$STAGE/bin/$name" ]; then
    printf '%s\n' "$STAGE/bin/$name"
    return 0
  fi
  while IFS= read -r dir; do
    for candidate in "$dir/$name" "$dir/${name,,}" "$dir/${name^^}"; do
      if [ -f "$candidate" ]; then
        printf '%s\n' "$candidate"
        return 0
      fi
    done
  done < <(mingw_lib_dirs)
  return 1
}

# Windows system libraries that must come from the OS. Everything else an exe
# imports has to be statically linked or shipped in bin/. `ucrtbase.dll` and the
# API sets are OS components; a `vcruntime*`/`msvcp*` import means the MSVC build
# was not crt-static, and `webview2loader.dll` means the WebView2 loader was not
# linked statically -- both are rejected instead of silently shipped.
SYSTEM_DLL_RE='^(kernel32|user32|gdi32|advapi32|shell32|shlwapi|ole32|oleaut32|combase|comctl32|comdlg32|rpcrt4|setupapi|cfgmgr32|dwmapi|dwrite|d2d1|dxgi|d3d11|d3d12|dcomp|vulkan-1|opengl32|msvcrt|ucrtbase|ntdll|ws2_32|mswsock|winmm|imm32|uxtheme|bcrypt|bcryptprimitives|crypt32|secur32|propsys|userenv|uiautomationcore|windowscodecs|version|dbghelp|iphlpapi|dnsapi|normaliz|hid|winhttp|wintrust|sechost|ncrypt|powrprof|psapi|sspicli|oleacc|schannel|msimg32|usp10|d3dcompiler_47|api-ms-win-[a-z0-9-]+|ext-ms-win-[a-z0-9-]+)\.dll$'

copy_runtime_dlls() {
  local exe="$1" imports name found
  imports="$("$PYTHON" "$ROOT/scripts/check-pe-imports.py" "$exe")" \
    || die "cannot list imports of $(basename "$exe")"
  while IFS= read -r name; do
    # On Windows a text-mode Python stream may emit CRLF; strip the CR so the
    # `^kernel32\.dll$`-style system match cannot be defeated.
    name="${name%$'\r'}"
    [ -n "$name" ] || continue
    if [[ "$name" =~ $SYSTEM_DLL_RE ]]; then
      continue
    fi
    found="$(find_runtime_dll "$name")" \
      || die "$(basename "$exe") imports non-system '$name'; it is neither crt-static nor available to bundle"
    if [ "$found" != "$STAGE/bin/$name" ]; then
      cp -L "$found" "$STAGE/bin/$name"
    fi
    if [ -z "${SEEN_DLLS[${name,,}]:-}" ]; then
      SEEN_DLLS[${name,,}]=1
      BUNDLED_DLLS+=("$name")
    fi
  done <<< "$imports"
}

for exe in "$STAGE/bin/$CLI" "$STAGE/bin/$GUI"; do
  machine="$("$PYTHON" "$ROOT/scripts/check-pe-imports.py" --machine "$exe")" \
    || die "not a readable PE32+ x86-64 image: $exe"
  [[ "$machine" == *"machine=0x8664"* ]] \
    || die "unexpected PE machine for $exe: $machine"
done
# The GUI must carry the embedded app icon (apps/app-windows/build.rs); a build
# that silently dropped it is rejected here.
"$PYTHON" "$ROOT/scripts/check-pe-imports.py" --has-icon "$STAGE/bin/$GUI" \
  || die "$GUI has no embedded app icon resource"

copy_runtime_dlls "$STAGE/bin/$CLI"
copy_runtime_dlls "$STAGE/bin/$GUI"

DLL_SUMMARY="none (static CRT; WebView2Loader is linked statically)"
if [ "${#BUNDLED_DLLS[@]}" -gt 0 ]; then
  DLL_SUMMARY="$(printf '%s\n' "${BUNDLED_DLLS[@]}" | sort -u | tr '\n' ' ')"
  DLL_SUMMARY="${DLL_SUMMARY% }"
fi

# --- fonts ------------------------------------------------------------------
if [ "$WITH_FONTS" = "1" ]; then
  step "assembling CAD fonts (scripts/fetch-fonts.sh) into fonts/"
  "$ROOT/scripts/fetch-fonts.sh" "$STAGE/fonts"
else
  step "WITH_FONTS=0: staging the committed fonts/ only"
  cp -R "$ROOT"/fonts/. "$STAGE/fonts/"
fi
[ -s "$STAGE/fonts/fonts.json" ] \
  || die "fonts/fonts.json is missing or empty; both executables resolve faces through it"

# --- package manifest -------------------------------------------------------
cat > "$STAGE/PACKAGE.txt" <<EOF
yacr Windows release package
============================
version : ${VERSION}
target  : ${TARGET} (PE32+ x86-64)
built   : $(date -u +%Y-%m-%dT%H:%M:%SZ)
fonts   : ${WITH_FONTS} (1 = full catalogue vendored into fonts/, 0 = committed fonts/ only)
dlls    : ${DLL_SUMMARY}

Contents
--------
bin/yacr-windows.exe    GUI host (Tauri 2 / WebView2 + shared CAD wgpu renderer).
                        Runs the same React UI bundle as the other desktop hosts.
                        Without --open it shows the demo scene; the Open button
                        uses the native file dialog.
bin/cad-cli-tools.exe   headless CLI (scan/measure/build-representation/render/...).
fonts/                  CAD font package in fonts.json format: the
                        mlightcad/cad-data catalogue plus the committed QCAD
                        osifont.ttf (provenance in fonts/SOURCE.md); auto-loaded
                        from this sibling dir by both executables. Missing
                        drawing fonts fall back to the default outline face / the
                        Windows system font.
docs/                   CLI, headless render, backends, build, validation,
                        compatibility, Windows host and font notes.
scripts/                fetch-test-dwg.sh (real-DWG corpus) and render-smoke.sh
                        (POSIX shell helpers; on Windows run them from Git Bash
                        or WSL).
yacr.ico                the app icon (from assets/yacr-icon.svg) for shortcuts
                        and installers; bin/yacr-windows.exe also embeds it in
                        its PE resources.
README.md, LICENSE, THIRD_PARTY_NOTICES.md

Quick start (Windows)
---------------------
  bin\\yacr-windows.exe
  bin\\yacr-windows.exe --open C:\\path\\drawing.dwg
  bin\\cad-cli-tools.exe --help
  bin\\cad-cli-tools.exe render drawing.dwg --png frame.png --width 1280 --height 720

Runtime notes (not a compatibility claim)
-----------------------------------------
  * The GUI is a Tauri / WebView2 host: the Microsoft Edge WebView2 Runtime must
    be installed on the target machine (it is not bundled). When it is missing
    the host reports the WebView2 diagnostic and exits with code 5.
  * The CAD surface needs a GPU driver exposing DX12 or Vulkan through wgpu.
    MSVC builds link the CRT statically; no VC++ redistributable is required,
    and the WebView2 loader is linked statically (no WebView2Loader.dll).
  * Window behaviour, layer order, DPI, native dialogs and the wgpu adapter are
    host/GPU dependent; this package is not a compatibility claim. See
    docs/windows-app.md.
  * Offscreen rendering (--headless) requires a Vulkan-capable adapter (a real
    GPU driver, or a software ICD such as Mesa lavapipe installed on the host);
    no software ICD is bundled.

Code signing
------------
This archive is UNSIGNED -- it is NOT code-signed. Windows SmartScreen may show
an "unknown publisher" warning on first launch.

No unlicensed user DWG samples or golden images are bundled. The mlightcad/cad-data
fonts in fonts/ are third-party and vendored by the deployer at packaging time
(scripts/fetch-fonts.sh); redistribution terms are the deployer's responsibility
(see docs/fonts.md). Only the authorized QCAD osifont.ttf (GPL-3 with font
exception) is committed to the repository.
EOF

# --- optional Wine smoke (software translation, not acceptance) -------------
if [ "${YACR_WINDOWS_SMOKE:-0}" = "1" ]; then
  command -v wine >/dev/null 2>&1 \
    || die "YACR_WINDOWS_SMOKE=1 but wine is not installed"
  step "Wine smoke: packaged CLI --help"
  WINEDEBUG="${WINEDEBUG:--all}" wine "$STAGE/bin/$CLI" --help >/dev/null
  echo "    (Wine is software translation, not a real-Windows acceptance; the GUI/window/GPU are NOT RUN)"
fi

# --- zip + checksum ---------------------------------------------------------
ZIP="${DIST}/${NAME}.zip"
step "creating ${ZIP}"
rm -f "$ZIP" "${ZIP}.sha256"
"$PYTHON" - "$DIST" "$NAME" <<'PY'
import hashlib
import pathlib
import sys
import zipfile

dist, name = pathlib.Path(sys.argv[1]), sys.argv[2]
root = dist / name
out = dist / f"{name}.zip"
if not (root / "bin" / "yacr-windows.exe").is_file():
    raise SystemExit(f"refusing to zip: {root}/bin/yacr-windows.exe is missing")
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
    for path in sorted(root.rglob("*")):
        archive.write(path, path.relative_to(dist).as_posix())
digest = hashlib.sha256(out.read_bytes()).hexdigest()
(out.parent / f"{name}.zip.sha256").write_text(f"{digest}  {name}.zip\n")
print(f"zip: {out.name} ({out.stat().st_size} bytes)")
PY

[ -s "$ZIP" ] || die "no zip was produced at $ZIP"
[ -s "${ZIP}.sha256" ] || die "no checksum was produced at ${ZIP}.sha256"
"$PYTHON" - "$ZIP" "${ZIP}.sha256" <<'PY'
import hashlib
import pathlib
import re
import sys

zip_path, sha_path = (pathlib.Path(arg) for arg in sys.argv[1:3])
expected = sha_path.read_text(encoding="utf-8").split()[0]
if not re.fullmatch(r"[0-9a-f]{64}", expected):
    raise SystemExit(f"{sha_path}: not a sha256 digest: {expected!r}")
actual = hashlib.sha256(zip_path.read_bytes()).hexdigest()
if actual != expected:
    raise SystemExit(f"{sha_path}: digest does not match {zip_path.name}")
PY

echo
echo "artifact : ${ZIP}"
echo "size     : $(wc -c < "$ZIP") bytes"
echo "sha256   : $(cut -d' ' -f1 "${ZIP}.sha256")"
