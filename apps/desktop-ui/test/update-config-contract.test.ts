// Cross-language contract for the update constants.
//
// The chrome opens `RELEASE_PAGE_URL`; the native shell's `cad_open_release_page`
// opens the same page from Rust. Both must spell the same literal, and the
// command must stay in `SHELL_ONLY_COMMANDS` so a bridged `dom-canvas` carrier
// asks the shell instead of the wasm session.
//
// `.ts` by filename only: Node's type stripping runs it, `tsc` never sees it.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

import { RELEASE_MANIFEST_URL, RELEASE_PAGE_URL } from '../src/update/config.ts';
import { SHELL_ONLY_COMMANDS } from '../src/host/invoke.ts';

const RELEASE_RS = new URL('../../../crates/cad-desktop-shell/src/release.rs', import.meta.url);

test('the config constants are the published landing URLs', () => {
  assert.equal(RELEASE_MANIFEST_URL, 'https://yacr.pages.dev/downloads.json');
  assert.equal(RELEASE_PAGE_URL, 'https://yacr.pages.dev/#download');
});

test('the Rust shell and the chrome agree on the release page URL', async (t) => {
  let source;
  try {
    source = await readFile(RELEASE_RS, 'utf8');
  } catch (error) {
    if (error && error.code === 'ENOENT') {
      // The Rust lane owns `release.rs`; this lane must not create it. Until it
      // exists the cross-language equality cannot be asserted (NOT RUN).
      t.skip('crates/cad-desktop-shell/src/release.rs is absent; cross-language URL check not run');
      return;
    }
    throw error;
  }

  const match = /RELEASE_PAGE_URL[^\n]*?"([^"]+)"/.exec(source);
  assert.ok(match, 'release.rs must define RELEASE_PAGE_URL');
  assert.equal(match[1], RELEASE_PAGE_URL, 'the shell and the chrome must open the same page');
});

test('cad_open_release_page is a shell capability', () => {
  assert.ok(
    SHELL_ONLY_COMMANDS.includes('cad_open_release_page'),
    'a bridged dom-canvas carrier must ask the shell, not the wasm session',
  );
});
