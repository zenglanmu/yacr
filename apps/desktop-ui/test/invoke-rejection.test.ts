// One rejection identity on both transports (P4 contract).
//
// Tauri's IPC rejects with the host's refusal as a bare string; the wasm adapter
// rejects with `Error(reason)`. Callers compare/render the refusal text, so the
// lift to `Error` happens once, here, instead of at every call site.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createInvoke } from '../src/host/invoke.ts';

function tauriInvoke(rejection: unknown) {
  return createInvoke({
    runtime: 'tauri',
    carrier: () => 'native-below',
    tauriPresent: () => true,
    tauri: () => Promise.reject(rejection),
    web: { invoke: () => Promise.reject(new Error('wrong transport')) },
  });
}

test('a bare Tauri rejection becomes an Error with the verbatim refusal', async () => {
  await assert.rejects(
    () => tauriInvoke('unsupported-runtime: cad_status')('cad_status'),
    (error: unknown) => error instanceof Error && error.message === 'unsupported-runtime: cad_status',
  );
});

test('an Error rejection keeps its identity on the native path', async () => {
  const raised = new Error('host refused');
  await assert.rejects(
    () => tauriInvoke(raised)('cad_command'),
    (error: unknown) => error === raised,
  );
});

test('the wasm path already throws Error and is not double-wrapped', async () => {
  const invoke = createInvoke({
    runtime: 'web',
    carrier: () => 'dom-canvas',
    tauriPresent: () => false,
    tauri: () => Promise.reject('wrong transport'),
    web: { invoke: () => Promise.reject(new Error('web refused')) },
  });
  await assert.rejects(
    () => invoke('cad_status'),
    (error: unknown) => error instanceof Error && error.message === 'web refused',
  );
});

test('a resolved value passes through unchanged on both routes', async () => {
  const native = createInvoke({
    runtime: 'tauri', carrier: () => 'native-below', tauriPresent: () => true,
    tauri: () => Promise.resolve({ ok: 'native' }), web: { invoke: () => Promise.resolve(null) },
  });
  assert.deepEqual(await native('cad_status'), { ok: 'native' });
  const wasm = createInvoke({
    runtime: 'web', carrier: () => 'dom-canvas', tauriPresent: () => false,
    tauri: () => Promise.resolve(null), web: { invoke: () => Promise.resolve({ ok: 'wasm' }) },
  });
  assert.deepEqual(await wasm('cad_status'), { ok: 'wasm' });
});
