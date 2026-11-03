// Contract for the Vite plugin that stages the committed CAD font package next
// to every non-Android UI bundle (and serves it in development).
//
// The copy itself is exercised by `scripts/build-web-ui.sh` and the Python
// package contract; here we pin the two facts a config regression would break:
// the plugin is build-only and the Android bundle stays untouched (its fonts are
// APK assets, and the wasm session is not instantiated there).
import test from 'node:test';
import assert from 'node:assert/strict';
import { fontPackage } from '../vite-font-package.mjs';

test('font package plugin is build-only and skips the Web and Android bundles', async () => {
  for (const runtime of ['web', 'android']) {
    const plugin = fontPackage(runtime);
    assert.equal(plugin.name, 'yacr-font-package');
    assert.equal(plugin.apply, 'build');
    plugin.configResolved({ root: '/nonexistent/yacr-ui', build: { outDir: 'dist' } });
    // Web fetches the site CDN lazily; Android ships APK assets. Neither may
    // copy the 16 MB face into its UI bundle.
    await plugin.closeBundle();
  }
});
