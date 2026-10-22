#!/usr/bin/env bash
# package-linux-release.sh — build and package the React/Tauri Linux host as a Flatpak.
#
# Pipeline (nothing is compiled inside the sandbox; the manifest only installs a
# prebuilt payload):
#   1. build the shared desktop UI (apps/desktop-ui: `npm ci` + `npm run build`);
#   2. build the release binary with `--features tauri/custom-protocol` so the
#      built `ui/dist` is embedded — without that feature the binary expects the
#      Vite dev server and cannot run standalone;
#   3. stage target/flatpak/payload/{bin/yacr,fonts/} (committed fonts/ only);
#   4. run flatpak-builder against packaging/flatpak/dev.yacr.app.yml;
#   5. export a single-file bundle to target/release/dist/.
#
# Env (defaults keep the repo environment working without hardcoding one mirror
# name into the script):
#   FLATPAK_REMOTE_DEPS   remote for `flatpak-builder --install-deps-from`
#                         (default: flathub-mirror — a local mirror of Flathub;
#                         use `flathub` or any configured remote elsewhere).
#   FLATPAK_REMOTE_BUILD  remote that must provide org.gnome.Platform//51 and
#                         org.gnome.Sdk//51 and is named in the install hint
#                         (default: flathub-mirror, same as above).
#
# Every missing prerequisite is a hard, actionable failure: this script never
# falls back to a partial or unverified artifact.
#
# Artifacts (under target/, which is gitignored):
#   target/flatpak/payload/                                     staged input
#   target/release/dist/yacr-<version>-linux-<arch>.flatpak
#   target/release/dist/yacr-<version>-linux-<arch>.flatpak.sha256
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

APP_ID="dev.yacr.app"
RUNTIME_VERSION="51"
RUNTIME_REF="org.gnome.Platform//${RUNTIME_VERSION}"
SDK_REF="org.gnome.Sdk//${RUNTIME_VERSION}"
MANIFEST="packaging/flatpak/dev.yacr.app.yml"
UI_DIR="apps/desktop-ui"
TAURI_CONF="apps/app-linux/tauri.conf.json"
PAYLOAD="target/flatpak/payload"
BUILD="target/flatpak/build"
REPO="target/flatpak/repo"
DIST="target/release/dist"
REMOTE_DEPS="${FLATPAK_REMOTE_DEPS:-flathub-mirror}"
REMOTE_BUILD="${FLATPAK_REMOTE_BUILD:-flathub-mirror}"

ARCH="$(uname -m)"

die() {
    echo "error: $*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "$2"
}

# --- preflight -------------------------------------------------------------
need cargo "cargo not found; install the Rust toolchain (rust-toolchain.toml) or add \$HOME/.cargo/bin to PATH."
need npm "npm not found; install Node.js + npm (the React frontend is built with Vite)."
need node "node not found; install Node.js + npm (the React frontend is built with Vite)."
need flatpak "flatpak not found; install flatpak."
need flatpak-builder "flatpak-builder not found; install the distro package, or the org.flatpak.Builder app and put a flatpak-builder wrapper on PATH."
need python3 "python3 not found; it is used to read the version from ${TAURI_CONF}."
[ -f "$MANIFEST" ] || die "manifest missing: ${MANIFEST}"
[ -f "${UI_DIR}/package-lock.json" ] || die "${UI_DIR}/package-lock.json missing; 'npm ci' needs a committed lockfile."
[ -d fonts ] || die "committed fonts/ directory missing; the manifest installs it to /app/fonts."

VERSION="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$TAURI_CONF")"
[ -n "$VERSION" ] || die "could not read the version from ${TAURI_CONF}"
NAME="yacr-${VERSION}-linux-${ARCH}"
BUNDLE="${DIST}/${NAME}.flatpak"

echo "==> yacr Linux release packaging"
echo "    root      : ${ROOT}"
echo "    app-id    : ${APP_ID} ${VERSION}"
echo "    arch      : ${ARCH}"
echo "    runtime   : ${RUNTIME_REF} (sdk ${SDK_REF})"
echo "    remotes   : deps=${REMOTE_DEPS} runtime=${REMOTE_BUILD}"
echo "    cargo     : $(cargo --version)"
echo "    node/npm  : $(node --version) / $(npm --version)"
echo "    flatpak   : $(flatpak --version) / builder $(flatpak-builder --version)"

# --- prerequisites: runtime + SDK -----------------------------------------
# Checked in both installations: flatpak-builder builds with `--user`, but a
# system-wide runtime is usable too. The build must not depend on a silent
# network install, so a missing runtime/SDK is a hard failure with a hint.
have_ref() {
    local ref="$1"
    flatpak info --user "$ref" >/dev/null 2>&1 && return 0
    flatpak info --system "$ref" >/dev/null 2>&1 && return 0
    return 1
}

missing_refs=()
have_ref "$RUNTIME_REF" || missing_refs+=("$RUNTIME_REF")
have_ref "$SDK_REF" || missing_refs+=("$SDK_REF")
if [ "${#missing_refs[@]}" -gt 0 ]; then
    echo "error: required Flatpak runtime/sdk not installed: ${missing_refs[*]}" >&2
    echo "       install them from the '${REMOTE_BUILD}' remote, e.g.:" >&2
    echo "         flatpak install --user ${REMOTE_BUILD} ${RUNTIME_REF} ${SDK_REF}" >&2
    echo "       (or set FLATPAK_REMOTE_BUILD to the remote that carries them)" >&2
    exit 1
fi

# --- 1. frontend -----------------------------------------------------------
echo "==> building the React frontend (${UI_DIR})"
(
    cd "$UI_DIR"
    npm ci
    npm run build
)
[ -f "${UI_DIR}/dist/index.html" ] || die "${UI_DIR}/dist/index.html missing after 'npm run build'"

# --- 2. release binary -----------------------------------------------------
# `tauri/custom-protocol` embeds ui/dist; a plain release build embeds the dev
# URL and would not run outside `npm run dev`.
echo "==> building the release binary (custom-protocol)"
cargo build --release --locked -p app-linux --bin yacr --features tauri/custom-protocol

BINARY="target/release/yacr"
[ -f "$BINARY" ] || die "release binary missing: ${BINARY}"

# Verify the built frontend is really embedded. Grepping for the dev URL is NOT a
# valid test: the embedded `tauri.conf.json` carries `build.devUrl` in both modes,
# so that string is always present. The discriminating signal is the hashed asset
# filenames that only exist inside a `custom-protocol` build.
mapfile -t DIST_ASSETS < <(find "${UI_DIR}/dist/assets" -maxdepth 1 -type f -printf '%f\n' | sort)
[ "${#DIST_ASSETS[@]}" -gt 0 ] || die "no built assets under ${UI_DIR}/dist/assets"
for asset in "${DIST_ASSETS[@]}"; do
    grep -aqF "$asset" "$BINARY" \
        || die "${BINARY} does not embed ${asset}; rebuild with --features tauri/custom-protocol."
done
grep -aqF 'tauri://localhost' "$BINARY" \
    || die "${BINARY} does not reference tauri://localhost; unexpected Tauri build configuration."
echo "    embedded assets verified: ${#DIST_ASSETS[@]} file(s)"

# --- 3. payload ------------------------------------------------------------
echo "==> staging the prebuilt payload in ${PAYLOAD}"
rm -rf "$PAYLOAD"
mkdir -p "${PAYLOAD}/bin" "${PAYLOAD}/fonts"
install -m755 "$BINARY" "${PAYLOAD}/bin/yacr"
cp -a fonts/. "${PAYLOAD}/fonts/"

[ -x "${PAYLOAD}/bin/yacr" ] || die "payload binary missing or not executable: ${PAYLOAD}/bin/yacr"
[ -f "${PAYLOAD}/fonts/fonts.json" ] || die "payload font package incomplete: ${PAYLOAD}/fonts/fonts.json missing"

# --- 4. flatpak-builder ----------------------------------------------------
echo "==> building the Flatpak (${MANIFEST})"
rm -rf "$BUILD" "$REPO"
flatpak-builder \
    --force-clean \
    --user \
    --install-deps-from="$REMOTE_DEPS" \
    --repo="$REPO" \
    "$BUILD" \
    "$MANIFEST"

# --- 5. bundle -------------------------------------------------------------
echo "==> exporting ${BUNDLE}"
mkdir -p "$DIST"
rm -f "$BUNDLE" "${BUNDLE}.sha256"
flatpak build-bundle "$REPO" "$BUNDLE" "$APP_ID"
[ -s "$BUNDLE" ] || die "flatpak build-bundle produced no bundle: ${BUNDLE}"
( cd "$DIST" && sha256sum "$(basename "$BUNDLE")" > "$(basename "$BUNDLE").sha256" )

echo
echo "artifact : ${BUNDLE}"
echo "size     : $(wc -c < "$BUNDLE") bytes"
echo "sha256   : $(cut -d' ' -f1 "${BUNDLE}.sha256")"
echo "install  : flatpak install --user ${BUNDLE}"
echo "run      : flatpak run ${APP_ID}"
echo
echo "note: a produced bundle is not acceptance evidence on its own; run the app"
echo "      and record the result separately."
