// Contract tests for the plugin manager's **logic** (no UI): the registry wire
// mapping, the honest unsupported answer, base64, source validation, the
// install flow, the active-decoder loader, and the remembered `vendor-settled`.
//
// Same conventions as the rest of `test/`: `.ts` by filename only, plain
// JavaScript bodies, Node's type stripping runs it, `tsc` never sees it.
import test from 'node:test';
import assert from 'node:assert/strict';

import {
  base64FromBytes,
  bytesFromBase64,
  installPluginFile,
  lastVendorSettled,
  readPluginStatus,
  recordVendorSettled,
  resetPluginManagerForTests,
  setPluginEnabled,
  subscribeVendorSettled,
  uninstallPlugin,
  validatePluginSource,
} from '../src/plugins/plugin-manager.ts';
import {
  loadVendorPlugin,
  resetVendorPluginLoaderForTests,
  runVendorPass,
} from '../src/vendor/vendor-pass.ts';

const MAX_BYTES = 4 * 1024 * 1024;

/** One valid registry row. */
function row(overrides = {}) {
  return {
    id: 'tch',
    name: 'Tianzheng',
    version: '0.1.0',
    abi: 1,
    source: 'builtin',
    enabled: true,
    sessionActive: true,
    status: 'ok',
    ...overrides,
  };
}

/** One valid status frame. */
function statusFrame(overrides = {}) {
  return {
    supported: true,
    carrier: 'native-below',
    plugins: [row()],
    activeDecoder: 'tch',
    sessionActiveDecoder: 'tch',
    restartRequired: false,
    diagnostics: [],
    ...overrides,
  };
}

/** One valid mutation answer over a status frame. */
function mutationFrame(overrides = {}) {
  return { accepted: true, restartRequired: false, status: statusFrame(), ...overrides };
}

/** An `invoke` double that records every call. */
function recorder(answers) {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push([command, args]);
    const answer = answers[command];
    if (typeof answer === 'function') return answer(args, command);
    if (answer !== undefined) return answer;
    throw new Error(`unexpected command: ${command}`);
  };
  return { calls, invoke };
}

function restoreGlobal(previous) {
  if (previous === undefined) delete globalThis.__YACR_VENDOR_PLUGIN__;
  else globalThis.__YACR_VENDOR_PLUGIN__ = previous;
}

// ---------------------------------------------------------------------------
// The registry wire
// ---------------------------------------------------------------------------

test('readPluginStatus maps every field of the shell frame', async () => {
  const { calls, invoke } = recorder({ cad_plugin_status: statusFrame() });
  const frame = await readPluginStatus(invoke);
  assert.deepEqual(frame, statusFrame());
  assert.deepEqual(calls, [['cad_plugin_status', undefined]]);
});

test('readPluginStatus keeps the optional installedAt and manifest status', async () => {
  const raw = statusFrame({
    plugins: [row({ source: 'user', status: 'manifest_missing', installedAt: '2026-11-05T10:00:00Z' })],
    activeDecoder: null,
    sessionActiveDecoder: null,
    restartRequired: true,
    diagnostics: ['plugin.manifest_missing: user-ab'],
  });
  const { invoke } = recorder({ cad_plugin_status: raw });
  assert.deepEqual(await readPluginStatus(invoke), raw);
});

test('a carrier without the plugin commands answers supported:false, never an empty list', async () => {
  const invoke = async () => { throw new Error('unsupported-runtime: cad_plugin_status'); };
  const frame = await readPluginStatus(invoke);
  assert.equal(frame.supported, false);
  assert.deepEqual(frame.plugins, []);
  assert.equal(frame.activeDecoder, null);
  assert.equal(frame.sessionActiveDecoder, null);
  assert.equal(frame.restartRequired, false);
  assert.deepEqual(frame.diagnostics, ['unsupported-runtime: cad_plugin_status']);
});

test('a real host failure is rethrown, not turned into "unsupported"', async () => {
  const invoke = async () => { throw new Error('shell: config directory unreadable'); };
  await assert.rejects(readPluginStatus(invoke), /config directory unreadable/);
});

test('setPluginEnabled and uninstallPlugin send the contract arguments', async () => {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push([command, args]);
    return mutationFrame({ status: statusFrame({ plugins: [row({ enabled: false })] }) });
  };
  assert.equal((await setPluginEnabled(invoke, 'tch', false)).status.plugins[0].enabled, false);
  assert.equal((await uninstallPlugin(invoke, 'user-ab')).status.plugins[0].id, 'tch');
  assert.deepEqual(calls, [
    ['cad_plugin_enable', { id: 'tch', enabled: false }],
    ['cad_plugin_uninstall', { id: 'user-ab' }],
  ]);
});

// ---------------------------------------------------------------------------
// base64
// ---------------------------------------------------------------------------

test('base64 round-trips arbitrary bytes, including the empty payload', () => {
  const bytes = Uint8Array.from({ length: 256 }, (_, index) => index);
  const encoded = base64FromBytes(bytes);
  assert.equal(encoded, Buffer.from(bytes).toString('base64'));
  assert.deepEqual([...bytesFromBase64(encoded)], [...bytes]);
  assert.deepEqual([...bytesFromBase64('')], []);
  // Whitespace from a wrapped wire payload is tolerated.
  assert.deepEqual([...bytesFromBase64(`${encoded.slice(0, 8)}\n${encoded.slice(8)}`)], [...bytes]);
});

// ---------------------------------------------------------------------------
// Source validation (importer injected: Node cannot import a Blob URL)
// ---------------------------------------------------------------------------

test('validatePluginSource accepts ABI v1 and reads the optional manifest fields', async () => {
  const previous = globalThis.__YACR_VENDOR_PLUGIN__;
  const sentinel = { abiVersion: 9, decode() {} };
  globalThis.__YACR_VENDOR_PLUGIN__ = sentinel;
  try {
    let imported = '';
    const answer = await validatePluginSource('export const x = 1;', async url => {
      imported = url;
      globalThis.__YACR_VENDOR_PLUGIN__ = {
        abiVersion: 1,
        decode() { return new ArrayBuffer(0); },
        id: 'demo',
        name: 'Demo',
        version: '1.2.3',
      };
    });
    assert.ok(imported.startsWith('blob:'));
    assert.deepEqual(answer, { ok: true, meta: { id: 'demo', name: 'Demo', version: '1.2.3' } });
    assert.equal(globalThis.__YACR_VENDOR_PLUGIN__, sentinel, 'the previous registration is restored');
  } finally {
    restoreGlobal(previous);
  }
});

test('validatePluginSource reports stable reasons and restores the global', async () => {
  const previous = globalThis.__YACR_VENDOR_PLUGIN__;
  const sentinel = { abiVersion: 1, decode() {} };
  globalThis.__YACR_VENDOR_PLUGIN__ = sentinel;
  try {
    assert.deepEqual(
      await validatePluginSource('source', async () => {
        globalThis.__YACR_VENDOR_PLUGIN__ = { abiVersion: 2, decode() {} };
      }),
      { ok: false, reason: 'plugin.abi_mismatch: 2' },
    );
    assert.deepEqual(
      await validatePluginSource('source', async () => {
        globalThis.__YACR_VENDOR_PLUGIN__ = { abiVersion: 1 };
      }),
      { ok: false, reason: 'plugin.contract_invalid' },
    );
    assert.deepEqual(
      await validatePluginSource('source', async () => {
        globalThis.__YACR_VENDOR_PLUGIN__ = { decode() {} };
      }),
      { ok: false, reason: 'plugin.contract_invalid' },
    );
    assert.deepEqual(
      await validatePluginSource('source', async () => { throw new Error('SyntaxError'); }),
      { ok: false, reason: 'plugin.contract_invalid' },
    );
    assert.equal(globalThis.__YACR_VENDOR_PLUGIN__, sentinel);
  } finally {
    restoreGlobal(previous);
  }
});

// ---------------------------------------------------------------------------
// Upload flow
// ---------------------------------------------------------------------------

function fakeFile(name, bytes) {
  return {
    name,
    size: bytes.byteLength,
    arrayBuffer: async () => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength),
  };
}

test('installPluginFile refuses a wrong extension before reading the file', async () => {
  const { calls, invoke } = recorder({ cad_plugin_status: statusFrame() });
  let read = false;
  const file = { name: 'plugin.txt', size: 4, arrayBuffer: async () => { read = true; return new ArrayBuffer(0); } };
  const result = await installPluginFile(invoke, file);
  assert.equal(result.accepted, false);
  assert.equal(result.reason, 'plugin.extension_invalid');
  assert.equal(read, false, 'the file is not read');
  assert.deepEqual(calls.map(call => call[0]), ['cad_plugin_status']);
});

test('installPluginFile refuses a payload over 4 MiB before reading the file', async () => {
  const { calls, invoke } = recorder({ cad_plugin_status: statusFrame() });
  let read = false;
  const file = { name: 'big.js', size: MAX_BYTES + 1, arrayBuffer: async () => { read = true; return new ArrayBuffer(0); } };
  const result = await installPluginFile(invoke, file);
  assert.equal(result.accepted, false);
  assert.equal(result.reason, 'plugin.payload_too_large');
  assert.equal(read, false);
  assert.deepEqual(calls.map(call => call[0]), ['cad_plugin_status']);
});

test('installPluginFile validates the module and sends base64 plus meta', async () => {
  const previous = globalThis.__YACR_VENDOR_PLUGIN__;
  delete globalThis.__YACR_VENDOR_PLUGIN__;
  const bytes = new TextEncoder().encode('export const demo = 1;');
  const { calls, invoke } = recorder({
    cad_plugin_status: statusFrame(),
    cad_plugin_install: mutationFrame(),
  });
  const importer = async () => {
    globalThis.__YACR_VENDOR_PLUGIN__ = {
      abiVersion: 1,
      decode() { return new ArrayBuffer(0); },
      id: 'demo',
      name: 'Demo',
    };
  };
  try {
    const result = await installPluginFile(invoke, fakeFile('demo.js', bytes), importer);
    assert.equal(result.accepted, true);
    const install = calls.find(call => call[0] === 'cad_plugin_install');
    assert.ok(install);
    assert.equal(install[1].filename, 'demo.js');
    assert.deepEqual([...bytesFromBase64(install[1].payload)], [...bytes]);
    assert.deepEqual(install[1].meta, { id: 'demo', name: 'Demo' });
    // The validation registration is restored (never deleted) after the import.
    assert.equal(globalThis.__YACR_VENDOR_PLUGIN__, undefined);
  } finally {
    restoreGlobal(previous);
  }
});

test('installPluginFile refuses an ABI mismatch before the shell install command', async () => {
  const bytes = new TextEncoder().encode('export const demo = 1;');
  const { calls, invoke } = recorder({ cad_plugin_status: statusFrame() });
  const importer = async () => {
    globalThis.__YACR_VENDOR_PLUGIN__ = { abiVersion: 3, decode() {} };
  };
  const result = await installPluginFile(invoke, fakeFile('demo.mjs', bytes), importer);
  assert.equal(result.accepted, false);
  assert.equal(result.reason, 'plugin.abi_mismatch: 3');
  assert.equal(calls.some(call => call[0] === 'cad_plugin_install'), false);
});

// ---------------------------------------------------------------------------
// The active-decoder loader
// ---------------------------------------------------------------------------

test('loadVendorPlugin uses the bundled tch script when tch is active', async () => {
  const previous = globalThis.__YACR_VENDOR_PLUGIN__;
  const bundled = { abiVersion: 1, decode: records => records };
  globalThis.__YACR_VENDOR_PLUGIN__ = bundled;
  resetVendorPluginLoaderForTests();
  try {
    const { calls, invoke } = recorder({ cad_plugin_status: statusFrame({ activeDecoder: 'tch' }) });
    assert.equal(await loadVendorPlugin({ invoke }), bundled);
    assert.deepEqual(calls.map(call => call[0]), ['cad_plugin_status']);
  } finally {
    resetVendorPluginLoaderForTests();
    restoreGlobal(previous);
  }
});

test('loadVendorPlugin reads and imports the installed entry for another id', async () => {
  const previous = globalThis.__YACR_VENDOR_PLUGIN__;
  delete globalThis.__YACR_VENDOR_PLUGIN__;
  resetVendorPluginLoaderForTests();
  try {
    const payload = base64FromBytes(new TextEncoder().encode('export const demo = 1;'));
    const { calls, invoke } = recorder({
      cad_plugin_status: statusFrame({ activeDecoder: 'demo', sessionActiveDecoder: 'demo' }),
      cad_plugin_read_entry: { payload, entry: 'index.js' },
    });
    let imported = '';
    const loaded = await loadVendorPlugin({
      invoke,
      importModule: async url => {
        imported = url;
        globalThis.__YACR_VENDOR_PLUGIN__ = { abiVersion: 1, decode: records => records };
        return {};
      },
    });
    assert.equal(typeof loaded.decode, 'function');
    assert.deepEqual(calls.map(call => call[0]), ['cad_plugin_status', 'cad_plugin_read_entry']);
    assert.deepEqual(calls[1][1], { id: 'demo' });
    assert.ok(imported.startsWith('blob:'));
  } finally {
    resetVendorPluginLoaderForTests();
    restoreGlobal(previous);
  }
});

test('no active decoder rejects with vendor.decoder_disabled', async () => {
  resetVendorPluginLoaderForTests();
  try {
    const { invoke } = recorder({
      cad_plugin_status: statusFrame({ activeDecoder: null, sessionActiveDecoder: null }),
    });
    await assert.rejects(loadVendorPlugin({ invoke }), /vendor\.decoder_disabled/);
  } finally {
    resetVendorPluginLoaderForTests();
  }
});

test('an unsupported carrier still loads the bundled tch decoder', async () => {
  // `dom-canvas` / Web have no plugin registry, but the bundled module is still
  // shipped and always-on there. Only a supported shell can disable it.
  const previous = globalThis.__YACR_VENDOR_PLUGIN__;
  const bundled = { abiVersion: 1, decode: records => records };
  globalThis.__YACR_VENDOR_PLUGIN__ = bundled;
  resetVendorPluginLoaderForTests();
  try {
    const { invoke } = recorder({
      cad_plugin_status: () => { throw new Error('unsupported-runtime: cad_plugin_status'); },
    });
    assert.equal(await loadVendorPlugin({ invoke }), bundled);
  } finally {
    resetVendorPluginLoaderForTests();
    restoreGlobal(previous);
  }
});

// ---------------------------------------------------------------------------
// A disabled decoder still settles the pass (the host must not hang)
// ---------------------------------------------------------------------------

/** Read the diagnostic list out of an ABI v1 result frame. */
function resultDiagnostics(bytes) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const count = view.getUint32(32, true);
  const decoder = new TextDecoder();
  const diagnostics = [];
  let offset = 36;
  for (let index = 0; index < count; index += 1) {
    const length = view.getUint16(offset, true);
    offset += 2;
    diagnostics.push(decoder.decode(bytes.subarray(offset, offset + length)));
    offset += length;
  }
  return { status: view.getUint32(24, true), diagnostics };
}

test('runVendorPass posts status=1 vendor.decoder_disabled instead of hanging', async () => {
  resetVendorPluginLoaderForTests();
  try {
    const { calls, invoke } = recorder({
      cad_plugin_status: statusFrame({ activeDecoder: null, sessionActiveDecoder: null }),
      cad_vendor_apply: { accepted: false, applied: 0, diagnostics: ['vendor.decoder_disabled'] },
    });
    const ack = await runVendorPass({ token: 7, generation: 7, entities: 2 }, { invoke });
    assert.equal(ack.accepted, false);
    assert.deepEqual(calls.map(call => call[0]), ['cad_plugin_status', 'cad_vendor_apply']);
    assert.equal(calls.some(call => call[0] === 'cad_vendor_records'), false, 'no records are fetched');
    const payload = calls[1][1].payload;
    const result = resultDiagnostics(bytesFromBase64(payload));
    assert.equal(result.status, 1);
    assert.deepEqual(result.diagnostics, ['vendor.decoder_disabled']);
  } finally {
    resetVendorPluginLoaderForTests();
  }
});

// ---------------------------------------------------------------------------
// vendor-settled memory
// ---------------------------------------------------------------------------

test('recordVendorSettled notifies subscribers and keeps the latest frame', () => {
  resetPluginManagerForTests();
  const seen = [];
  const off = subscribeVendorSettled(settled => seen.push(settled));
  recordVendorSettled(3, ['vendor.unsupported_layout']);
  assert.equal(seen.length, 1);
  assert.equal(seen[0].applied, 3);
  assert.deepEqual(seen[0].diagnostics, ['vendor.unsupported_layout']);
  assert.deepEqual(lastVendorSettled(), seen[0]);

  off();
  recordVendorSettled(4, []);
  assert.equal(seen.length, 1, 'unsubscribed listeners are not called');
  assert.equal(lastVendorSettled().applied, 4);

  // One throwing listener cannot break the pump or the record.
  const bad = subscribeVendorSettled(() => { throw new Error('listener'); });
  recordVendorSettled(5, ['x']);
  assert.equal(lastVendorSettled().applied, 5);
  bad();

  resetPluginManagerForTests();
  assert.equal(lastVendorSettled(), null);
});
