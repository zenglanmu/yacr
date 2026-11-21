// BIM browse panel: the one selection request.
//
// A press on a row sends every geometry id of the component. The module never
// paints a "selected" state of its own: a resolved acknowledgement only means the
// host took the request, and a refusal is returned in the host's words.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  BIM_SELECT_COMMAND,
  bimSelectionPayload,
  selectBimEntities,
} from '../src/bim/selection.ts';

test('the selection payload keeps every id once and drops non-ids', () => {
  assert.deepEqual(bimSelectionPayload([7, 8, 8, 9]), { entityIds: [7, 8, 9] });
  assert.deepEqual(bimSelectionPayload([-1, 1.5, '2', null, 3]), { entityIds: [3] });
  assert.equal(bimSelectionPayload([]), null);
  assert.equal(bimSelectionPayload(null), null);
});

test('one selection request carries exactly the payload the host expects', async () => {
  const calls = [];
  const result = await selectBimEntities(async (command, args) => {
    calls.push({ command, args });
    return { accepted: true };
  }, [7, 8]);
  assert.deepEqual(result, { kind: 'ok', count: 2 });
  assert.deepEqual(calls, [{ command: BIM_SELECT_COMMAND, args: { entityIds: [7, 8] } }]);
});

test('an ack that says accepted:false is a refusal with its own reason', async () => {
  const result = await selectBimEntities(async () => ({ accepted: false, error: 'selection.no_document' }), [1]);
  assert.deepEqual(result, { kind: 'error', reason: 'selection.no_document' });
  // A refusal without a reason still names the failure honestly.
  const bare = await selectBimEntities(async () => ({ accepted: false }), [1]);
  assert.deepEqual(bare, { kind: 'error', reason: 'CAD_COMMAND_NOT_ACCEPTED' });
});

test('a thrown refusal (the carrier marker included) is returned verbatim', async () => {
  const result = await selectBimEntities(async () => {
    throw new Error('unsupported-runtime: cad_select_entities');
  }, [1]);
  assert.deepEqual(result, { kind: 'error', reason: 'unsupported-runtime: cad_select_entities' });
});

test('an ack without a negative flag counts as admitted, and no ids sends nothing', async () => {
  let called = false;
  const ack = await selectBimEntities(async () => { called = true; return undefined; }, [1]);
  assert.deepEqual(ack, { kind: 'ok', count: 1 });
  assert.equal(called, true);
  const none = await selectBimEntities(async () => { throw new Error('must not be called'); }, []);
  assert.deepEqual(none, { kind: 'invalid' });
});
