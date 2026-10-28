// Contract for the chrome's transient error banner.
//
// On-device Android verification found a sticky one: a refused
// `cad_standard_view` (a standard-view cell that looked enabled on a carrier with
// no such channel) threw `unsupported-runtime: cad_standard_view` into `setError`,
// and nothing ever cleared it — the red line stayed up through a later successful
// `FIT` and only vanished on app restart. The host's own `status.error` was right
// to persist (the host is the authority on its own failures); the *chrome's* line
// must not outlive the attempt that produced it.
//
// This is a source contract on purpose. `App.tsx` owns that state, has no pure
// model behind it, and the rule is about **where** the write sits (before the
// attempt, and on admission), which a rendered-DOM test could not pin without a
// live host. The suite therefore asserts the ordering in the real source.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readModule } from './helpers.ts';

const source = await readModule('App.tsx');

/** The offset of `needle` at or after `from`; fails loudly when it is missing. */
function at(needle: string, from = 0): number {
  const offset = source.indexOf(needle, from);
  assert.notEqual(offset, -1, `App.tsx must still contain: ${needle}`);
  return offset;
}

test('the banner stays host-first: status.error and pollError are never suppressed', () => {
  // The precedence is the whole point: a host-reported failure outranks the
  // chrome's own line, which outranks a poll failure. The fix clears the middle
  // slot only; it must not touch the host's.
  assert.ok(
    source.includes("const displayError = status?.error || error || pollError || ''"),
    'the banner must keep `status.error` first, then the chrome line, then the poll failure',
  );
  assert.ok(source.includes('displayedError: displayError'), 'the rendered banner reads that one derived value');
  // The chrome never writes the host's error back: it has no channel for it, and
  // a local clear of `status.error` would hide a real host failure.
  assert.equal(/status\.error\s*=/.test(source), false, 'the chrome must never assign status.error');
});

test('a new command attempt clears the chrome line, and admission clears it again', () => {
  const submit = at('const submitCommand = useCallback(');
  const queued = at('await queueCadCommand(', submit);
  const cleared = source.lastIndexOf("setError('')", queued);
  assert.ok(cleared > submit, 'the chrome line must be cleared before the command is queued');
  // Nothing between the clear and the queue call may re-introduce the old line.
  assert.equal(source.slice(cleared + "setError('')".length, queued).includes('setError('), false);

  const accepted = at("if (result.kind === 'accepted')", queued);
  const admission = accepted + "if (result.kind === 'accepted')".length;
  const clearedOnAdmission = source.indexOf("setError('')", admission);
  const historyPush = source.indexOf('setHistory(', admission);
  assert.ok(clearedOnAdmission > admission, 'an admitted command must clear the chrome line');
  assert.ok(clearedOnAdmission < historyPush, 'the admission clear belongs to the same branch');
});

test('a standard-view attempt clears the chrome line before it is sent', () => {
  const runStandard = at('runStandardView: view =>');
  const sent = at('submitStandardView(', runStandard);
  const cleared = source.lastIndexOf("setError('')", sent);
  assert.ok(cleared > runStandard, 'a standard-view attempt must clear the previous sentence first');
  // Exactly one `submitStandardView` call site exists, and it is this one: a
  // second caller without the clear would bring the sticky banner back.
  assert.equal(source.split('submitStandardView({').length - 1, 1, 'one standard-view entry point');
});

test('a refusal still reaches the banner (clearing is not suppression)', () => {
  // `submitStandardView` reports the failure through `report`, which `App.tsx`
  // wires to `setError`: the clear happens first, the real refusal lands after it.
  assert.ok(
    source.includes('void submitStandardView({ invoke, view, t, report: setError })'),
    'a refused standard view must still be reported, not swallowed by the clear',
  );
});
