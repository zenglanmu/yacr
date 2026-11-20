// The one pre-save gate (ADR-0041).
//
// SAVE always writes the native `.ymodel` archive, so the experimental-DWG
// confirmation is gone: a `.dwg`-labelled document is just a label and its SAVE
// goes straight to the host. The only question left before the funnel is the
// optional embed-resources one; it must still fire, cancel cleanly and own the
// single dialog slot. Source-contract style like the other file suites.
import test from 'node:test';
import assert from 'node:assert/strict';
import { admitCommandText, setCommandCatalog } from '../src/commands/commands.ts';
import {
  embedNeedsDialog,
  isSaveFamilyCommand,
  readEmbedPreview,
  resolveSaveEmbed,
  SAVE_FAMILY_COMMANDS,
} from '../src/commands/save-embed.ts';
import { readAppSource, readModule } from './helpers.ts';

setCommandCatalog(['OPEN', 'NEW', 'SAVE', 'SAVEAS', 'QSAVE', 'PLOT', 'EXPORT', 'PRINT', 'CLOSE', 'FIT']);

const dwgDocument = {
  documentLabel: '/tmp/workshop/plan.dwg', entityCount: 3, loading: false,
  canOpen: true, canNew: true, canSave: true, canPlot: true, canFit: true,
};

test('the DWG confirmation is gone and a .dwg-labelled SAVE reaches the host directly', async () => {
  // The design is deleted, not merely unused: the dialog, its model and the
  // old hook name all fail to read.
  for (const gone of [
    'components/DwgSaveConfirmDialog.tsx',
    'components/use-dwg-save-confirm.tsx',
    'commands/save-confirm.ts',
  ]) {
    await assert.rejects(readModule(gone), `${gone} must be deleted`);
  }
  // No shipped module names the removed design (the exact grep list of the
  // removal) — a leftover gate could only be dead code or a second question.
  const source = await readAppSource();
  for (const token of [
    'dwgSaveConfirmed', 'dwgSavePending', 'dwgSaveDialog', 'DwgSaveConfirm',
    'save.dwg_confirm', 'isDwgSaveCandidate', 'dwgExportName', 'picker_waiting_dwg',
  ]) {
    assert.equal(source.includes(token), false, `${token} must not be referenced`);
  }

  // A `.dwg` label changes nothing: the line is admitted by the host vocabulary
  // and the capability booleans, with no chrome-side source inspection.
  assert.deepEqual(admitCommandText('SAVE', { status: dwgDocument }), {
    kind: 'admitted', command: 'SAVE', reasonKey: null,
  });
  // A carrier that cannot preview (or has nothing to embed) answers `null`/an
  // empty list, so the gate raises no question and the line is dispatched.
  assert.equal(await readEmbedPreview(async () => { throw new Error('unsupported-runtime: preview'); }), null);
  assert.equal(embedNeedsDialog(null), false);
  assert.equal(embedNeedsDialog({ policy: true, entries: [] }), false);

  // The shell's funnel takes no "a caution was answered" flag any more: the
  // third parameter is gone and the gate is called once, before `cad_command`.
  const app = await readModule('App.tsx');
  assert.equal(
    app.split('const submitCommand = useCallback(async (text: string, options: SubmitOptions = {}) =>').length - 1,
    1,
    'submitCommand has no DWG bypass parameter',
  );
  const askAt = app.indexOf('await beforeSave(text, status,');
  const queueAt = app.indexOf('await queueCadCommand(invoke, text, status, false');
  assert.ok(askAt > 0 && queueAt > askAt, 'the gate sits before the one host dispatch');
  const hook = await readModule('components/use-save-gate.tsx');
  assert.equal(hook.includes('documentLabel'), false, 'the gate never inspects the source name');
  assert.equal(hook.includes('dwg'), false);
});

test('the embed gate is the only pre-save question and it still fires for SAVE', async () => {
  assert.deepEqual([...SAVE_FAMILY_COMMANDS], ['SAVE', 'SAVEAS', 'QSAVE']);
  for (const text of ['SAVE', 'saveas', ' qsave ']) assert.equal(isSaveFamilyCommand(text), true, text);
  // EXPORT writes a copy elsewhere and embeds nothing: it never asks.
  assert.equal(isSaveFamilyCommand('EXPORT'), false);
  assert.equal(admitCommandText('EXPORT', { status: dwgDocument }).kind, 'admitted');

  // One preview read, one apply, one dispatch site: a second funnel or a second
  // dialog slot would show up as a second occurrence here.
  const source = await readAppSource();
  for (const ipc of ['cad_ymodel_resource_preview', 'cad_set_embed_resources']) {
    assert.equal(source.split(`'${ipc}'`).length - 1, 1, `${ipc} has one call site`);
  }
  const hook = await readModule('components/use-save-gate.tsx');
  assert.equal(hook.split('<SaveEmbedDialog').length - 1, 1, 'one dialog render');
  assert.equal(hook.split('await readEmbedPreview(invoke)').length - 1, 1, 'one preview read');
  assert.ok(hook.includes('if (preview === null || !embedNeedsDialog(preview)) return false;'));
  // A listed resource is what earns the question.
  assert.equal(embedNeedsDialog({ policy: false, entries: [{ kind: 'font', name: 'a.ttf', source: null, bytes: 1, found: true }] }), true);

  // At least one entry: the question opens and a cancel applies nothing.
  let applied = 0;
  let dispatched = 0;
  const cancelled = await resolveSaveEmbed('cancel', true, {
    invoke: async () => { applied += 1; return null; },
    dispatchSave: () => { dispatched += 1; },
  });
  assert.equal(cancelled.kind, 'cancelled');
  assert.equal(applied, 0, 'cancel never applies the answer');
  assert.equal(dispatched, 0, 'cancel never dispatches SAVE');
  // Confirm applies first, then dispatches exactly once; a failed apply keeps
  // the dialog open and dispatches nothing.
  const confirmed = await resolveSaveEmbed('confirm', true, {
    invoke: async () => { applied += 1; return null; },
    dispatchSave: () => { dispatched += 1; },
  });
  assert.equal(confirmed.kind, 'saved');
  assert.deepEqual([applied, dispatched], [1, 1]);
  const failed = await resolveSaveEmbed('confirm', false, {
    invoke: async () => { throw new Error('ymodel.conflict: archive is stale'); },
    dispatchSave: () => { dispatched += 1; },
  });
  assert.equal(failed.kind, 'failed');
  assert.equal(dispatched, 1, 'a refused apply must not save anyway');
});

test('the shell keeps one SAVE funnel and one dialog slot, and shrinks', async () => {
  const app = await readModule('App.tsx');
  assert.equal(app.split('await beforeSave(').length - 1, 1, 'one gate call site');
  assert.equal(app.split('{saveGateDialog}').length - 1, 1, 'one dialog slot');
  assert.equal(app.split('if (saveGatePending()) return;').length - 1, 1, 'the dialog owns Escape');
  assert.equal(app.split('await queueCadCommand(').length - 1, 1, 'one CAD command funnel');
  // The file-size code ceiling (scripts/check-file-size.py) is 1000 lines; the
  // removal of the DWG gate must leave the shell under it with room to spare.
  // `splitlines()` semantics, like the gate itself.
  const lines = app.replace(/\n$/, '').split('\n').length;
  assert.ok(lines <= 1000, `App.tsx is ${lines} lines`);
  assert.ok(lines < 999, `App.tsx must shrink (was 999 lines, is ${lines})`);
});
