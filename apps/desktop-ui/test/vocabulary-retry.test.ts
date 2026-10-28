// A cold `dom-canvas` load can fail the vocabulary fetch once (a bootstrap
// fetch or wasm instantiation error) and the old one-shot loader then left every
// command reporting "not supported" until a manual reload. These tests pin the
// retry: it repeats the same request, backs off, and rethrows the last error
// unchanged when the budget is spent — it never invents a vocabulary.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

import {
  VOCABULARY_ATTEMPTS,
  VOCABULARY_FIRST_DELAY_MS,
  VOCABULARY_MAX_DELAY_MS,
  loadCommandVocabulary,
  vocabularyDelayMs,
} from '../src/commands/vocabulary.ts';

test('the retry delay doubles and is capped', () => {
  assert.equal(vocabularyDelayMs(0), VOCABULARY_FIRST_DELAY_MS);
  assert.equal(vocabularyDelayMs(1), VOCABULARY_FIRST_DELAY_MS * 2);
  assert.equal(vocabularyDelayMs(2), VOCABULARY_FIRST_DELAY_MS * 4);
  assert.equal(vocabularyDelayMs(20), VOCABULARY_MAX_DELAY_MS);
  // Defensive: a nonsense attempt number must not produce NaN or an endless wait.
  assert.equal(vocabularyDelayMs(-3), VOCABULARY_FIRST_DELAY_MS);
  assert.equal(vocabularyDelayMs(Number.NaN), VOCABULARY_FIRST_DELAY_MS);
});

test('a transient failure is retried and the eventual answer is returned', async () => {
  const waits: number[] = [];
  let calls = 0;
  const answer = await loadCommandVocabulary(async () => {
    calls += 1;
    if (calls < 3) throw new Error(`transient ${calls}`);
    return ['FIT', 'ZOOMIN'];
  }, { wait: async ms => { waits.push(ms); } });
  assert.deepEqual(answer, ['FIT', 'ZOOMIN']);
  assert.equal(calls, 3);
  assert.deepEqual(waits, [VOCABULARY_FIRST_DELAY_MS, VOCABULARY_FIRST_DELAY_MS * 2]);
});

test('the exhausted budget rethrows the last error unchanged', async () => {
  const seen: number[] = [];
  let calls = 0;
  await assert.rejects(
    loadCommandVocabulary(async () => {
      calls += 1;
      throw new Error(`boom ${calls}`);
    }, { attempts: 3, wait: async () => {}, onRetry: attempt => { seen.push(attempt); } }),
    (error: Error) => {
      assert.equal(error.message, 'boom 3');
      return true;
    },
  );
  assert.equal(calls, 3);
  assert.deepEqual(seen, [1, 2, 3]);
});

test('one attempt means no retry and no wait', async () => {
  let waits = 0;
  let calls = 0;
  await assert.rejects(
    loadCommandVocabulary(async () => { calls += 1; throw new Error('once'); }, {
      attempts: 1,
      wait: async () => { waits += 1; },
    }),
  );
  assert.equal(calls, 1);
  assert.equal(waits, 0);
  assert.ok(VOCABULARY_ATTEMPTS > 1);
});

test('the chrome loads the vocabulary through the retrying loader', async () => {
  const hook = await readFile(new URL('../src/commands/use-host-vocabulary.ts', import.meta.url), 'utf8');
  assert.match(hook, /loadCommandVocabulary\(\(\) => invoke\('cad_commands'\)\)/);
  assert.equal(/await invoke\('cad_commands'\)/.test(hook), false, 'the hook still awaits the raw invoke');
  // And the app consumes that hook rather than fetching on its own.
  const app = await readFile(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /useHostVocabulary\(setError\)/);
  assert.equal(/invoke\('cad_commands'\)/.test(app), false, 'App bypasses the retrying loader');
});

test('a failed bootstrap is not cached for the life of the page', async () => {
  const source = await readFile(new URL('../src/cad/canvas-host.ts', import.meta.url), 'utf8');
  const loader = source.slice(source.indexOf('export function loadCanvasModule'), source.indexOf('async function bootstrap'));
  assert.match(loader, /\.catch\(/);
  assert.match(loader, /bootstrapByEnv\.delete\(env\)/);
  // The delete must happen before the rejection is rethrown, otherwise the stale
  // entry stays and the next caller joins the failed promise again.
  assert.ok(
    loader.indexOf('bootstrapByEnv.delete(env)') < loader.indexOf('throw error'),
    'the failed entry is rethrown before it is dropped',
  );
});
