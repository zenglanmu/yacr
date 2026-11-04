// The browser transport's **vendor pass** contract: the three `cad_vendor_*`
// commands against the wasm session's exports (`apps/app-web/src/vendor.rs`).
//
// The pass itself (`vendor/vendor-pass.ts`) is covered by `vendor-plugin.test.ts`;
// this suite pins the transport half only: the command names, the `u64` ->
// `bigint` boundary wasm-bindgen requires, the binary answer as an `ArrayBuffer`,
// and the refusal-by-name for a module that has no such export (never an empty
// frame that would look like "a document with no vendor objects").
//
// `.ts` by filename only: Node's type stripping runs it, `tsc` never sees it.
import test from 'node:test';
import assert from 'node:assert/strict';

import {
  WEB_VENDOR_APPLY_COMMAND,
  WEB_VENDOR_RECORDS_COMMAND,
  WEB_VENDOR_STATUS_COMMAND,
  createWebAdapter,
} from '../src/host/web-adapter.ts';

/**
 * A stand-in session module with just the vendor exports a test gives it.
 *
 * `createWebAdapter` reads the module lazily, so a bare object with one export is
 * enough; a missing key is exactly the "older artifact" case the transport must
 * refuse by name.
 */
function vendorModule(exports) {
  const calls = [];
  const module = {};
  for (const [name, answer] of Object.entries(exports)) {
    module[name] = (...args) => {
      calls.push([name, ...args]);
      return answer(...args);
    };
  }
  return { calls, module };
}

test('vendor_records converts the session\'s typed array to an ArrayBuffer', async () => {
  const bytes = new Uint8Array([0x44, 0x4e, 0x41, 0x59, 0x02]);
  const fake = vendorModule({ vendor_records: () => bytes });
  const adapter = createWebAdapter(fake.module);

  const frame = await adapter.invoke(WEB_VENDOR_RECORDS_COMMAND, { token: 7 });
  assert.ok(frame instanceof ArrayBuffer, 'the plugin contract is ArrayBuffer');
  assert.deepEqual([...new Uint8Array(frame)], [0x44, 0x4e, 0x41, 0x59, 0x02]);
  // The pass number crosses as the `bigint` wasm-bindgen requires for a `u64`
  // parameter (the wire event carries a plain number).
  assert.deepEqual(fake.calls, [['vendor_records', 7n]]);
});

test('vendor_records accepts an ArrayBuffer or a byte array too', async () => {
  const buffer = new Uint8Array([1, 2, 3]).buffer;
  const fromBuffer = vendorModule({ vendor_records: () => buffer });
  const same = await createWebAdapter(fromBuffer.module).invoke(WEB_VENDOR_RECORDS_COMMAND, { token: 1 });
  assert.equal(same, buffer, 'an ArrayBuffer is handed on as-is');
  assert.deepEqual([...new Uint8Array(same)], [1, 2, 3]);

  const fromArray = vendorModule({ vendor_records: () => [4, 5] });
  const converted = await createWebAdapter(fromArray.module).invoke(WEB_VENDOR_RECORDS_COMMAND, { token: 1 });
  assert.ok(converted instanceof ArrayBuffer);
  assert.deepEqual([...new Uint8Array(converted)], [4, 5]);

  // Anything that is not bytes is loud: a coerced frame would be a corrupt one.
  const broken = vendorModule({ vendor_records: () => 'not bytes' });
  await assert.rejects(
    createWebAdapter(broken.module).invoke(WEB_VENDOR_RECORDS_COMMAND, { token: 1 }),
    /unsupported-runtime: cad_vendor_records: .*not binary records/,
  );
  const badBytes = vendorModule({ vendor_records: () => [1, 300] });
  await assert.rejects(
    createWebAdapter(badBytes.module).invoke(WEB_VENDOR_RECORDS_COMMAND, { token: 1 }),
    /the record frame is not a byte array/,
  );
});

test('a module without the vendor exports refuses by name, never with an empty frame', async () => {
  const adapter = createWebAdapter({});
  await assert.rejects(
    adapter.invoke(WEB_VENDOR_RECORDS_COMMAND, { token: 3 }),
    /^Error: unsupported-runtime: cad_vendor_records: the canvas module has no vendor_records export$/,
  );
  await assert.rejects(
    adapter.invoke(WEB_VENDOR_APPLY_COMMAND, { token: 3, generation: 3, payload: 'AAAA' }),
    /unsupported-runtime: cad_vendor_apply: .*no vendor_apply export/,
  );
  await assert.rejects(
    adapter.invoke(WEB_VENDOR_STATUS_COMMAND),
    /unsupported-runtime: cad_vendor_status: .*no vendor_status export/,
  );
});

test('a malformed pass number is refused before the export is called', async () => {
  const fake = vendorModule({ vendor_records: () => new Uint8Array() });
  const adapter = createWebAdapter(fake.module);
  for (const token of [undefined, -1, 1.5, Number.MAX_SAFE_INTEGER + 2, '7']) {
    await assert.rejects(
      adapter.invoke(WEB_VENDOR_RECORDS_COMMAND, { token }),
      /token must be a non-negative safe integer/,
      `token: ${String(token)}`,
    );
  }
  assert.deepEqual(fake.calls, [], 'nothing reaches the session');
});

test('vendor_apply forwards token, generation and payload, and parses the ack', async () => {
  const ack = { accepted: true, applied: 0, diagnostics: ['vendor.unsupported_layout'] };
  const fake = vendorModule({ vendor_apply: () => JSON.stringify(ack) });
  const adapter = createWebAdapter(fake.module);

  const answer = await adapter.invoke(WEB_VENDOR_APPLY_COMMAND, {
    token: 7,
    generation: 9,
    payload: 'AAAA',
  });
  assert.deepEqual(answer, ack);
  assert.deepEqual(fake.calls, [['vendor_apply', 7n, 9n, 'AAAA']]);

  // A session refusal (a bare string from the wasm `Result::Err`) surfaces as the
  // host's own words, never as a settled pass.
  const refusing = vendorModule({ vendor_apply: () => { throw new Error('vendor.stale_token'); } });
  await assert.rejects(
    createWebAdapter(refusing.module).invoke(WEB_VENDOR_APPLY_COMMAND, { token: 7, generation: 9, payload: 'AAAA' }),
    /vendor\.stale_token/,
  );
  // A payload that is not base64 text cannot be forwarded at all.
  await assert.rejects(
    adapter.invoke(WEB_VENDOR_APPLY_COMMAND, { token: 7, generation: 9 }),
    /payload must be the base64 result frame/,
  );
});

test('vendor_status answers the session\'s parsed descriptor', async () => {
  const fake = vendorModule({
    vendor_status: () => JSON.stringify({ token: 7, generation: 7, entities: 2 }),
  });
  const adapter = createWebAdapter(fake.module);
  assert.deepEqual(await adapter.invoke(WEB_VENDOR_STATUS_COMMAND), {
    token: 7,
    generation: 7,
    entities: 2,
  });
  assert.deepEqual(fake.calls, [['vendor_status']]);

  const broken = vendorModule({ vendor_status: () => 'null' });
  await assert.rejects(
    createWebAdapter(broken.module).invoke(WEB_VENDOR_STATUS_COMMAND),
    /expected a JSON object/,
  );
});
