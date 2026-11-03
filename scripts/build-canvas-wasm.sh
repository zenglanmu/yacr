#!/usr/bin/env bash
# build-canvas-wasm.sh — build the wasm CAD session and post-process it into the
# desktop UI's public canvas directory.
#
# What lands in `apps/desktop-ui/public/canvas/` (all **generated**, git-ignored):
#
#   app_web.js       wasm-bindgen `--target web` ES module (the whole session).
#   app_web_bg.wasm  the one wasm binary; both JS files initialise from it.
#   bootstrap.js     a tiny **hand-written-by-this-script** ES module, served from
#                    `public/` so no bundler transforms it. It initialises the
#                    module, publishes `{ready, ...module}` and wraps `open_bytes`:
#                    after a document opens it lazily fetches the faces that
#                    document needs from the project Pages CDN (`fonts/`) and
#                    installs them on the live session.
#
#                      import init, * as mod from './app_web.js';
#                      const wasm = init(…);
#                      globalThis.__YACR_CANVAS_MODULE__ = { ready: wasm, ...mod, open_bytes: wrapped };
#
# Why a `public/` bootstrap module and not an import from the app bundle, and not
# the `--target no-modules` classic script:
#
#   * a dynamic `import()` of the artifact **inside the bundled tree** does not
#     survive Vite 8's dev transform: it becomes
#     `import(__vite__injectQuery(url, 'import'))`, i.e. it requests
#     `/canvas/app_web.js?import`, which the dev server answers with **HTTP 500**
#     (error-overlay HTML) for a public-dir file. `@vite-ignore` does not stop the
#     query injection. A file under `public/` is served verbatim, so the import
#     graph starts outside the bundler and is never rewritten;
#   * the `--target no-modules` classic script cannot be used: its glue starts with
#     `if (… && document.currentScript !== null) { … document.currentScript.src … }`
#     and WebKitGTK leaves `document.currentScript` **`undefined`** while a
#     dynamically inserted classic script runs. `undefined !== null` is true, the
#     top-level `let wasm_bindgen` never initialises, and every later
#     `typeof wasm_bindgen` throws the TDZ error
#     `ReferenceError: Cannot access 'wasm_bindgen' before initialization.`
#     (Verified in WebKitGTK 2.52.6, independent of Tauri.)
#
# `eval` is not an option either: the host CSP has no `'unsafe-eval'`.
#
# Vite copies `apps/desktop-ui/public/**` verbatim into `dist/`, so these files
# have to exist before `npm run build` (or a dev server run) for a `dom-canvas`
# host to have anything to load. Only one 9.5 MB wasm binary is shipped.
#
# The wasm build itself, the wasm-bindgen pin check and the artifact/size
# reporting are **shared** with `scripts/build-web.sh` (the standalone browser
# bundle) through `scripts/lib/wasm-toolchain.sh`.
#
# Honesty rules for this entry point:
#   * it never skips silently — a missing target, a missing `wasm-bindgen`, a
#     version mismatch or a missing/empty artifact is a loud non-zero exit;
#   * it never substitutes a stub for a real build, so a successful run means the
#     real `app_web.wasm` was compiled from this tree (`--locked`, no network);
#   * a successful build is **not** runtime or pixel evidence: it says the
#     artifacts exist, nothing about a browser or a GPU.
#
# Usage:
#   bash scripts/build-canvas-wasm.sh
#
# Environment:
#   CARGO        cargo binary to use (default: `cargo` on PATH, else ~/.cargo/bin)
#   WASM_BINDGEN wasm-bindgen binary to use (default: PATH, else ~/.cargo/bin)
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
YACR_SCRIPT_NAME="$(basename -- "${BASH_SOURCE[0]}")"
# Toolchain resolution, the wasm-bindgen pin check, the `app-web` cargo build and
# the artifact guards are shared with `scripts/build-web.sh` (the browser bundle).
# shellcheck source=scripts/lib/wasm-toolchain.sh
. "$REPO_ROOT/scripts/lib/wasm-toolchain.sh"

readonly OUT_DIR="$REPO_ROOT/apps/desktop-ui/public/canvas"
readonly ESM_JS="app_web.js"
readonly WASM_ARTIFACT="app_web_bg.wasm"
readonly BOOTSTRAP_JS="bootstrap.js"
# The bootstrap module's content. Fixed on purpose: it is the one file the loader
# in `apps/desktop-ui/src/cad/canvas-host.ts` waits on, and it must name the *existing*
# `app_web_bg.wasm` explicitly (the wasm-bindgen default would be derived from
# `app_web.js`'s own URL, which is the same file — spelling it out keeps the
# contract visible in one place).
#
# `ready` resolves from `init(...)` **only**: fonts never gate the session. The
# package is fetched lazily, after a document opens, from the project Pages CDN
# (same-origin `fonts/`), and only when the drawing needs text or names a face.
BOOTSTRAP_SOURCE=$(cat <<'BOOTSTRAP'
import init, * as mod from './app_web.js';

const wasm = init(new URL('./app_web_bg.wasm', import.meta.url).href);

// Catalogue names the host always considers for a text-bearing drawing. Noto is
// the platform default face (it covers Latin and CJK); `osifont`/`arial`/`simplex`
// are the small fallbacks the loader can name when a requested face is missing.
// A drawing's own requested faces are added from `cad_fonts.requested` below.
const DEFAULT_FONT_FALLBACK_NAMES = ['noto sans cjk sc', 'osifont', 'arial', 'simplex'];

/**
 * Where the catalogue and faces live.
 *
 * Default: `fonts/` resolved against the document's base URI, i.e. the site the
 * page itself came from (the project's Pages CDN at `https://yacr-examples.pages.dev/fonts/`).
 * A host that serves the package elsewhere sets `__YACR_FONT_BASE_URL__`.
 */
function fontBaseUrl() {
  const configured = globalThis.__YACR_FONT_BASE_URL__;
  if (typeof configured === 'string' && configured.trim() !== '') {
    return configured.endsWith('/') ? configured : `${configured}/`;
  }
  return new URL('fonts/', document.baseURI).href;
}

/**
 * The `cad_fonts` projection of the live session, normalised.
 *
 * `invoke` answers the same `{ok, value}` envelope the desktop IPC does; a
 * refusal is the session's own reason, never an empty state read as "nothing to
 * do".
 */
async function readFontState() {
  const raw = await mod.invoke('cad_fonts', '{}');
  let envelope;
  try {
    envelope = JSON.parse(raw);
  } catch (error) {
    throw new Error(`cad_fonts: unreadable answer (${String(error)})`);
  }
  if (envelope === null || typeof envelope !== 'object') {
    throw new Error('cad_fonts: answer is not an object');
  }
  if (envelope.ok !== true) {
    throw new Error(`cad_fonts: ${envelope.error ?? 'refused without a reason'}`);
  }
  const value = envelope.value;
  if (value === null || typeof value !== 'object') {
    throw new Error('cad_fonts: value is not an object');
  }
  return {
    faces: Array.isArray(value.faces) ? value.faces : [],
    catalogEntries: Number(value.catalogEntries ?? 0),
    requested: Array.isArray(value.requested)
      ? value.requested.filter((key) => typeof key === 'string' && key !== '')
      : [],
    textNeeded: value.textNeeded === true,
    locale: typeof value.locale === 'string' ? value.locale.toLowerCase() : '',
  };
}

/** Every catalogue key one entry can be matched by (`file`, stem, `name[]`). */
function fontEntryKeys(entry) {
  const keys = [];
  if (entry === null || typeof entry !== 'object') return keys;
  if (typeof entry.file === 'string' && entry.file !== '') {
    keys.push(entry.file);
    const dot = entry.file.lastIndexOf('.');
    if (dot > 0) keys.push(entry.file.slice(0, dot));
  }
  if (Array.isArray(entry.name)) {
    for (const name of entry.name) {
      if (typeof name === 'string' && name !== '') keys.push(name);
    }
  }
  return keys.map((key) => key.toLowerCase());
}

/**
 * The catalogue entries this drawing can use: its own requested names plus the
 * default and locale fallbacks. Fetching only these keeps the multi-MB CJK face
 * off a drawing that names its own Latin font.
 */
function selectFontEntries(entries, requested) {
  const wanted = new Set(DEFAULT_FONT_FALLBACK_NAMES);
  for (const key of requested) wanted.add(key.toLowerCase());
  return entries.filter((entry) => {
    if (entry === null || typeof entry !== 'object') return false;
    if (typeof entry.file !== 'string' || entry.file === '') return false;
    return fontEntryKeys(entry).some((key) => wanted.has(key));
  });
}

/** Fetch the needed faces, install them, and re-run the install on the session. */
async function installFontPackage() {
  const state = await readFontState();
  // A package is already in force (its real load reached the session report).
  if (state.catalogEntries > 0 || state.faces.some((face) => face && face.status === 'loaded')) {
    return globalThis.__YACR_FONT_PACKAGE__ ?? null;
  }
  // No text and no named face: nothing to shape, nothing to fetch.
  if (!state.textNeeded && state.requested.length === 0) return null;

  const base = fontBaseUrl();
  const catalogUrl = new URL('fonts.json', base);
  const catalogResponse = await fetch(catalogUrl);
  if (!catalogResponse.ok) {
    throw new Error(`${catalogUrl.href}: HTTP ${catalogResponse.status}`);
  }
  const catalogText = await catalogResponse.text();
  const entries = JSON.parse(catalogText);
  if (!Array.isArray(entries)) {
    throw new Error('fonts.json must be a JSON array');
  }
  const selected = selectFontEntries(entries, state.requested);
  if (selected.length === 0) {
    throw new Error("fonts.json carries no entry for this drawing's font needs");
  }
  const files = {};
  for (const entry of selected) {
    const name = entry.file;
    const url = new URL(encodeURIComponent(name), base);
    const response = await fetch(url);
    if (!response.ok) {
      // A catalogue face the package promises but cannot be fetched is a loud
      // failure, never dropped so the drawing silently shapes with less.
      throw new Error(`${url.href}: HTTP ${response.status}`);
    }
    files[name] = new Uint8Array(await response.arrayBuffer());
  }
  const summary = JSON.parse(mod.set_font_package(catalogText, files));
  // The package is installed but the *live* session installed no engine when it
  // opened; re-run the host resource install on the document it already holds.
  const sessionStatus = JSON.parse(mod.apply_font_package());
  mod.frame();
  const result = { ...summary, sessionStatus };
  globalThis.__YACR_FONT_PACKAGE__ = result;
  delete globalThis.__YACR_FONT_PACKAGE_ERROR__;
  return result;
}

/**
 * The lazy entry point: one in-flight/successful load per wasm instance.
 *
 * A failure is **not** memoised: it is published on
 * `__YACR_FONT_PACKAGE_ERROR__` and rethrown, so the next open retries instead
 * of inheriting a promise that can never succeed.
 */
let fontPackageLoad = null;

function ensureFontPackage() {
  if (fontPackageLoad === null) {
    fontPackageLoad = installFontPackage().catch((error) => {
      fontPackageLoad = null;
      globalThis.__YACR_FONT_PACKAGE_ERROR__ = String(error);
      console.error('yacr: font package failed to load:', error);
      throw error;
    });
  }
  return fontPackageLoad;
}

// The module namespace is not mutated: the published object is a new one with
// the one export this host must wrap.
const session = { ...mod };
session.open_bytes = async (...args) => {
  const result = await mod.open_bytes(...args);
  // Font installation is a resource step, not part of opening: a CDN failure
  // keeps the document open and stays visible on `__YACR_FONT_PACKAGE_ERROR__`
  // (and the session's `fonts.failed` status), exactly like the native hosts.
  // It must not turn a successfully opened drawing into a rejected open.
  try {
    await ensureFontPackage();
  } catch {
    // Published by `ensureFontPackage`; the open result is returned unchanged.
  }
  return result;
};
globalThis.__YACR_CANVAS_MODULE__ = { ready: wasm, ...session, ensureFontPackage };
BOOTSTRAP
)
readonly BOOTSTRAP_SOURCE

die() {
  yacr_die "$*"
}

# --- toolchain -------------------------------------------------------------

yacr_init_wasm_toolchain

# --- output directory ------------------------------------------------------

# Only ever a generated, git-ignored directory under the UI package: refuse
# anything else before deleting, so a mangled variable cannot wipe source.
yacr_rotate_generated_dir \
  "$OUT_DIR" "$REPO_ROOT/apps/desktop-ui/public/canvas" "apps/desktop-ui/public/canvas"

# --- build -----------------------------------------------------------------

yacr_build_app_web_wasm
yacr_bindgen_web "$OUT_DIR"

printf 'writing %s (public/, so no bundler rewrites its imports)\n' "$BOOTSTRAP_JS"
printf '%s\n' "$BOOTSTRAP_SOURCE" > "$OUT_DIR/$BOOTSTRAP_JS"
[ -s "$OUT_DIR/$BOOTSTRAP_JS" ] || die "generated $BOOTSTRAP_JS is missing or empty: $OUT_DIR/$BOOTSTRAP_JS"

for artifact in "$OUT_DIR/$ESM_JS" "$OUT_DIR/$WASM_ARTIFACT" "$OUT_DIR/$BOOTSTRAP_JS"; do
  [ -s "$artifact" ] || die "expected artifact missing or empty: $artifact"
done

# Exactly one wasm binary: the bootstrap initialises from this one, so a second
# `*_bg.wasm` would be dead weight (and a sign a second target crept back in).
wasm_count=0
while IFS= read -r -d '' found; do
  wasm_count=$((wasm_count + 1))
done < <(find "$OUT_DIR" -maxdepth 1 -type f -name '*_bg.wasm' -print0)
[ "$wasm_count" -eq 1 ] || die "expected exactly one *_bg.wasm in $OUT_DIR, found $wasm_count"

# The bootstrap must import the module this build emitted and name the same binary.
grep -qF "./$ESM_JS" "$OUT_DIR/$BOOTSTRAP_JS" \
  || die "$BOOTSTRAP_JS does not import ./$ESM_JS"
grep -qF "$WASM_ARTIFACT" "$OUT_DIR/$BOOTSTRAP_JS" \
  || die "$BOOTSTRAP_JS does not reference $WASM_ARTIFACT"
# The lazy loader must stay: `ready` resolves from init alone, and after an open
# the bootstrap fetches the CDN package, installs it and re-runs the install on
# the live session. A bootstrap that dropped any step would silently open text
# drawings with `unsupported-resources: fonts`.
grep -qF "ensureFontPackage" "$OUT_DIR/$BOOTSTRAP_JS" \
  || die "$BOOTSTRAP_JS has no lazy ensureFontPackage()"
grep -qF "set_font_package" "$OUT_DIR/$BOOTSTRAP_JS" \
  || die "$BOOTSTRAP_JS does not call set_font_package"
grep -qF "apply_font_package" "$OUT_DIR/$BOOTSTRAP_JS" \
  || die "$BOOTSTRAP_JS does not call apply_font_package"
grep -qF "__YACR_FONT_BASE_URL__" "$OUT_DIR/$BOOTSTRAP_JS" \
  || die "$BOOTSTRAP_JS does not read __YACR_FONT_BASE_URL__"
if grep -qF "wasm.then(" "$OUT_DIR/$BOOTSTRAP_JS"; then
  die "$BOOTSTRAP_JS gates ready on something other than init()"
fi

# --- report ----------------------------------------------------------------

printf 'artifacts in %s\n' "$OUT_DIR"
for artifact in "$OUT_DIR/$ESM_JS" "$OUT_DIR/$WASM_ARTIFACT" "$OUT_DIR/$BOOTSTRAP_JS"; do
  yacr_report_artifact "$artifact"
done
printf 'note: a built artifact is not runtime, GPU or pixel evidence.\n'
