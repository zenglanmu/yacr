// The command vocabulary is fetched once at mount, but on a `dom-canvas` carrier
// it can only be answered after the wasm session module exists. On a cold load
// that is seconds after the page is interactive, so a command typed immediately
// used to be refused as "not supported" and had to be retyped. These are source
// contracts for the ordering that fixes it: the loader keeps its promise, and
// admission awaits it before reading the catalog.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

const APP = new URL('../src/App.tsx', import.meta.url);

test('the vocabulary loader keeps its in-flight promise', async () => {
  const source = await readFile(APP, 'utf8');
  assert.match(source, /const vocabularyRef = useRef<Promise<unknown> \| null>\(null\);/);
  assert.match(source, /const vocabulary = invoke\('cad_commands'\);/);
  assert.match(source, /vocabularyRef\.current = vocabulary;/);
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
  const source = await readFile(APP, 'utf8');
  assert.match(source, /invoke\('cad_command_aliases'\)/);
  assert.ok(
    source.indexOf("invoke('cad_command_aliases')") > source.indexOf("const vocabulary = invoke('cad_commands')"),
    'the alias table must stay a separate, non-gating request',
  );
});
