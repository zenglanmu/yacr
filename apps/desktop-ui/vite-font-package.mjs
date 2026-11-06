import { createReadStream } from 'node:fs';
import { cp, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// The one CAD font package (`fonts/fonts.json` + faces) normally lives at the
// repository root. Only the desktop (tauri) bundle serves it next to the UI:
//
//   tauri   -> apps/desktop-ui/dist/fonts        (X11/XWayland dom-canvas, offline)
//   web     -> no copy: the page fetches the site CDN's `fonts/` lazily
//   android -> no copy: native-below ships the package as APK assets
//
// `YACR_FONT_PACKAGE_DIR` overrides the source directory. Release packaging sets
// it to a directory assembled by `scripts/fetch-fonts.sh` (the full mlightcad
// catalogue merged with the committed fonts), so the embedded X11/XWayland
// package carries every downloaded face. Without the variable `npm run build`
// stays self-contained and offline with the committed package. `build:web` must
// never carry the faces into the browser bundle: the plugin only runs for tauri.
const REPO_FONTS = fileURLToPath(new URL('../../fonts/', import.meta.url));
const PACKAGE_ENV = 'YACR_FONT_PACKAGE_DIR';
const REQUIRED_FILES = ['fonts.json', 'osifont.ttf', 'NotoSansCJKsc-Regular.otf'];

/** The package to stage/serve: an explicit override, else the committed one. */
function fontDir() {
  const override = process.env[PACKAGE_ENV];
  return override && override.trim() !== '' ? path.resolve(override) : REPO_FONTS;
}

/** Throw when the package is incomplete. */
async function assertPackage(dir) {
  for (const name of REQUIRED_FILES) {
    const file = path.join(dir, name);
    const info = await stat(file).catch(() => null);
    if (!info || !info.isFile() || info.size === 0) {
      throw new Error(`vite-font-package: font package is missing ${file}`);
    }
  }
}

/** Resolve a request path to a bare file inside the package, or null. */
function packageFile(dir, url) {
  const raw = String(url ?? '').split('?')[0].split('#')[0];
  if (!raw.startsWith('/fonts/')) return null;
  let name;
  try {
    name = decodeURIComponent(raw.slice('/fonts/'.length));
  } catch {
    return null;
  }
  // The catalogue only uses bare file names; reject traversal outright.
  if (!name || name.includes('/') || name.includes('\\') || name === '.' || name === '..') return null;
  return path.join(dir, name);
}

/**
 * Vite plugin: serve the package in development and copy it into every
 * non-Android production bundle.
 */
export function fontPackage(runtime) {
  let outDir = null;
  return {
    name: 'yacr-font-package',
    apply: 'build',
    configResolved(config) {
      outDir = path.resolve(config.root, config.build.outDir);
    },
    async closeBundle() {
      // Only the desktop (tauri) bundle keeps the package next to the UI: its
      // X11/XWayland `dom-canvas` carrier may need it offline. The Web carrier
      // fetches the same package lazily from the site CDN, and Android ships it
      // as APK assets, so neither bundle should carry a second copy.
      if (runtime !== 'tauri') return;
      if (outDir === null) throw new Error('vite-font-package: outDir was not resolved');
      const dir = fontDir();
      await assertPackage(dir);
      await cp(dir, path.join(outDir, 'fonts'), { recursive: true });
    },
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const file = packageFile(fontDir(), request.url);
        if (file === null) return next();
        const stream = createReadStream(file);
        stream.on('error', () => next());
        stream.pipe(response);
      });
    },
  };
}
