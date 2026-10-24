// Dev-only Vite plugin: serve the generated `dom-canvas` artifacts as ES modules.
//
// Why this exists
// ---------------
// `scripts/build-canvas-wasm.sh` writes three generated files into
// `public/canvas/` (`app_web.js`, `app_web_bg.wasm`, `bootstrap.js`), and the
// desktop transport (`src/canvas-host.mjs`) loads them **at runtime** with
// `<script type="module" src="/canvas/bootstrap.js">`. That is what keeps the wasm
// session out of this bundle on every carrier: `native-below` hosts never load it.
//
// Outside the dev server this is fine: Vite copies `public/**` verbatim into
// `dist/` and the runtime request hits the static files. The **dev** server is
// different: a request for a `public/` file that Vite classifies as a module
// source is refused by Vite's own public-file guard ("This file is in /public and
// will be copied as-is during build without going through the plugin transforms,
// and therefore should not be imported from source code. It can only be
// referenced via HTML tags."), which surfaces as the error overlay instead of the
// bootstrap module. The same rule produced the earlier `?import` HTTP 500.
//
// This plugin answers those requests **before** that guard (`enforce: 'pre'`,
// `apply: 'serve'` only), reading the bytes from `public/canvas/` and returning
// them as a module. Production is untouched: there is no plugin at build time and
// the dist files are still just copied.
//
// Scope and refusals
// ------------------
// Only flat `/canvas/<name>.js` requests that exist on disk are handled. Anything
// else - another directory, a `.wasm` (Vite's static handler already serves it as
// `application/wasm`), a nested path, a traversal, a query string, a missing file
// - returns nothing, so normal resolution continues and Vite's own diagnostics
// stay in charge. No request can ever read a file outside `public/canvas/`.
import { existsSync, readFileSync, realpathSync, statSync } from 'node:fs';
import { basename, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The package directory (`apps/desktop-ui`), the default dev-server root. */
const PACKAGE_DIR = fileURLToPath(new URL('.', import.meta.url));

/** URL prefix the transport requests (matches `CANVAS_BASE` in `src/canvas-host.mjs`). */
export const CANVAS_URL_PREFIX = '/canvas/';

/**
 * Whether an id is a flat `/canvas/<name>.js` request this plugin may answer.
 *
 * Deliberately strict: exactly one path segment, `.js` suffix, no query/hash, no
 * traversal, no dot-file. A `.wasm` is not a JS module and is left to Vite's
 * static handler; a nested path or a query is not part of the generated layout.
 */
export function isCanvasModuleRequest(id) {
  if (typeof id !== 'string' || !id.startsWith(CANVAS_URL_PREFIX) || !id.endsWith('.js')) return false;
  const name = id.slice(CANVAS_URL_PREFIX.length);
  if (name.length <= '.js'.length) return false;
  if (name.includes('/') || name.includes('\\') || name.includes('?') || name.includes('#')) return false;
  if (name.startsWith('.')) return false;
  return true;
}

/**
 * The absolute file for a canvas module request, or `null` when it must not be
 * served.
 *
 * `canvasDir` is the resolved `public/canvas` directory. The returned path is
 * always a direct child of it: the id shape already forbids separators, and the
 * `resolve`/`join` comparison is the belt-and-braces check for a caller that
 * bypasses [`isCanvasModuleRequest`].
 */
export function canvasModulePath(canvasDir, id) {
  if (typeof canvasDir !== 'string' || canvasDir.length === 0 || !isCanvasModuleRequest(id)) return null;
  const name = id.slice(CANVAS_URL_PREFIX.length);
  const root = resolve(canvasDir);
  const file = resolve(root, name);
  if (file !== join(root, name) || !file.startsWith(root + sep)) return null;
  return file;
}

/**
 * The real path of a servable canvas module, or `null`.
 *
 * Three gates: the id shape ([`canvasModulePath`]), the file must exist, and its
 * **real** path (symlinks resolved) must stay inside the real `public/canvas`
 * directory. A missing artifact is not an error here - this plugin simply declines
 * and Vite's own resolution/diagnostics stay in charge.
 */
function canvasModuleFile(canvasDir, id) {
  const file = canvasModulePath(canvasDir, id);
  if (file === null) return null;
  try {
    const rootReal = realpathSync(resolve(canvasDir));
    const real = realpathSync(file);
    if (!real.startsWith(rootReal + sep)) return null;
    return statSync(real).isFile() ? real : null;
  } catch {
    return null;
  }
}

/**
 * The plugin itself.
 *
 * `root` defaults to this package; Vite's resolved config overrides it in
 * `configResolved`, so a custom `root` or `publicDir` is honoured. Tests may pass
 * an explicit `root` to exercise the "not handled" paths without a dev server.
 */
export function canvasPublicModules({ root = PACKAGE_DIR } = {}) {
  let canvasDir = join(root, 'public', 'canvas');
  return {
    name: 'yacr-canvas-public-modules',
    // Dev only: at build time the public files are copied verbatim and the
    // runtime request is answered by the static file server, as before.
    apply: 'serve',
    // Must run before Vite's public-dir guard (and before alias/asset plugins).
    enforce: 'pre',
    configResolved(config) {
      const publicDir = typeof config.publicDir === 'string' && config.publicDir.length > 0
        ? config.publicDir
        : join(config.root, 'public');
      canvasDir = join(publicDir, 'canvas');
    },
    configureServer(server) {
      // The artifacts are regenerated by `scripts/build-canvas-wasm.sh`, often while
      // the dev server is running. `public/` is not in Vite's module graph by file
      // path, so a rebuilt module would otherwise keep serving the cached transform
      // until a dev-server restart: watch the directory and drop the entry for that
      // URL when a file changes or appears.
      if (!existsSync(canvasDir)) return;
      const watcher = server.watcher;
      if (typeof watcher?.add !== 'function' || typeof watcher.on !== 'function') return;
      watcher.add(canvasDir);
      const invalidate = file => {
        if (typeof file !== 'string' || !file.startsWith(canvasDir + sep) || !file.endsWith('.js')) return;
        const id = CANVAS_URL_PREFIX + basename(file);
        const module = server.moduleGraph?.getModuleById?.(id);
        if (module) server.moduleGraph.invalidateModule(module);
      };
      watcher.on('change', invalidate);
      watcher.on('add', invalidate);
    },
    resolveId(id) {
      if (canvasModuleFile(canvasDir, id) === null) return null;
      // The public URL *is* the module id: the browser asked for exactly this, and
      // the bootstrap's `import './app_web.js'` resolves back into this plugin.
      return id;
    },
    load(id) {
      const file = canvasModuleFile(canvasDir, id);
      if (file === null) return null;
      // Verbatim bytes: these are generated ES modules, not source to transform.
      return { code: readFileSync(file, 'utf8'), map: null };
    },
  };
}
