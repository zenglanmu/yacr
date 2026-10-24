import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';
import { canvasPublicModules } from './vite-canvas-modules.mjs';

// The bilingual catalogs live in the shared, platform-independent UI core
// (`crates/cad-ui-core/i18n`). The `@i18n` alias keeps components free of long
// relative paths and gives the two files a single home.
const i18nDir = fileURLToPath(new URL('../../crates/cad-ui-core/i18n/', import.meta.url));

export default defineConfig({
  base: './',
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
