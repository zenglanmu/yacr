// The command vocabulary is fetched once at mount, but on a `dom-canvas` carrier
// it can only be answered after the wasm session module exists. On a cold load
// that is seconds after the page is interactive, so a command typed immediately
// used to be refused as "not supported" and had to be retyped. These are source
// contracts for the ordering that fixes it: the loader keeps its promise (in
// `commands/use-host-vocabulary.ts`, extracted from `App.tsx` for the file-size
// ceiling), and admission awaits it before reading the catalog.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

const APP = new URL('../src/App.tsx', import.meta.url);
const HOOK = new URL('../src/commands/use-host-vocabulary.ts', import.meta.url);

test('the vocabulary loader keeps its in-flight promise', async () => {
  const hook = await readFile(HOOK, 'utf8');
  assert.match(hook, /const vocabulary = useRef<Promise<unknown> \| null>\(null\);/);
  assert.match(hook, /const pending = loadCommandVocabulary\(\(\) => invoke\('cad_commands'\)\);/);
  assert.match(hook, /vocabulary\.current = pending;/);
  // The hook hands that same ref to the chrome, which is what admission awaits.
  assert.match(hook, /return \{ catalogNames, aliasPairs, vocabulary, carrierEpoch \};/);
});

test('admission awaits the vocabulary before reading the catalog', async () => {
  const source = await readFile(APP, 'utf8');
  const awaitAt = source.indexOf('await vocabularyRef.current');
  const admissionAt = source.indexOf('queueCadCommand(invoke, text, status, false)');
  assert.ok(awaitAt > 0, 'submitCommand does not await the vocabulary load');
  assert.ok(admissionAt > awaitAt, 'admission happens before the vocabulary is awaited');
  // A rejected load must not turn into an unhandled rejection: the loader reports
  // it, and the command stays unsupported.
  const slice = source.slice(awaitAt - 260, awaitAt + 120);
  assert.match(slice, /try \{ await vocabularyRef\.current; \} catch/);
});

test('the alias table is loaded separately and never gates admission', async () => {
  const hook = await readFile(HOOK, 'utf8');
  assert.match(hook, /invoke\('cad_command_aliases'\)/);
  assert.ok(
    hook.indexOf("invoke('cad_command_aliases')") > hook.indexOf("loadCommandVocabulary(() => invoke('cad_commands'))"),
    'the alias table must stay a separate, non-gating request',
  );
  assert.ok(!/vocabulary\.current = .*cad_command_aliases/.test(hook), 'aliases must never become the admission source');
});
