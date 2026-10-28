import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';
import { canvasPublicModules } from './vite-canvas-modules.mjs';

// The bilingual catalogs live in the shared, platform-independent UI core
// (`crates/cad-ui-core/i18n`). The `@i18n` alias keeps components free of long
// relative paths and gives the two files a single home.
const i18nDir = fileURLToPath(new URL('../../crates/cad-ui-core/i18n/', import.meta.url));

// Which host this bundle targets: `tauri` (the default, and the only value the
// two desktop hosts ever use), `web` (the browser bundle served from
// `apps/app-web/dist-ui`) or `android` (the same bundle inside a WebView that
// still has the Tauri bridge). `src/host/runtime.ts` reads the injected constant;
// an unknown value is a build mistake and fails here instead of producing a
// bundle that routes its calls to the wrong transport.
const RUNTIMES = ['tauri', 'web', 'android'];
const runtime = process.env.YACR_RUNTIME ?? 'tauri';
if (!RUNTIMES.includes(runtime)) {
  throw new Error(`vite.config: YACR_RUNTIME must be one of ${RUNTIMES.join(', ')} (got ${JSON.stringify(runtime)})`);
}

export default defineConfig({
  base: './',
  // Build-time constant, not a runtime probe: the identifier is replaced by a
  // string literal, so a production bundle carries exactly one transport branch
  // per command class.
  define: {
    __YACR_RUNTIME__: JSON.stringify(runtime),
  },
  // Dev-only: serve the generated `dom-canvas` artifacts from `public/canvas/` as
  // modules (Vite's public-file guard refuses them otherwise). Production is
  // unaffected: the files are copied verbatim into `dist/` and loaded at runtime.
  plugins: [canvasPublicModules()],
  resolve: {
    alias: { '@i18n': i18nDir },
  },
  server: {
    host: '127.0.0.1',
    port: 1420,
    strictPort: true,
    fs: {
      allow: [
        fileURLToPath(new URL('.', import.meta.url)),
        i18nDir,
      ],
    },
  },
});
