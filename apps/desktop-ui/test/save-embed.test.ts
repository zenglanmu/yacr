// The optional pre-save "embed external resources" question.
//
// SAVE first asks `cad_ymodel_resource_preview`; only a readable answer that
// lists at least one resource opens the dialog. These are pure-model and
// source-contract tests (no React DOM): the decision, the two calls and their
// order are real logic, and the wiring is asserted against the hook, the dialog
// source and `App.tsx`.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  EMBED_APPLY_COMMAND,
  EMBED_KINDS,
  EMBED_PREVIEW_COMMAND,
  embedGroups,
  embedInitialToggle,
  embedKindKey,
  embedNeedsDialog,
  embedTotal,
  formatByteSize,
  isSaveFamilyCommand,
  parseEmbedPreview,
  readEmbedPreview,
  resolveSaveEmbed,
  SAVE_FAMILY_COMMANDS,
} from '../src/commands/save-embed.ts';
import { readAppSource, readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

function recordingInvoke(handler) {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push(args === undefined ? { command } : { command, args });
    const answer = typeof handler === 'function' ? handler(command, args, calls.length) : handler;
    if (answer instanceof Error) throw answer;
    return answer;
  };
  return { invoke, calls };
}

/** A preview with one found font, one found image and one missing xref. */
function preview(overrides = {}) {
  return {
    policy: true,
    entries: [
      { kind: 'font', name: 'arial.ttf', source: '/fonts/arial.ttf', bytes: 754974, found: true },
      { kind: 'xref', name: 'site.dwg', source: '/proj/site.dwg', bytes: 0, found: false },
      { kind: 'image', name: 'logo.png', source: '/proj/logo.png', bytes: 1468006, found: true },
    ],
    ...overrides,
  };
}

// --- the preview -----------------------------------------------------------

test('the preview is validated and never turned into a fake answer', () => {
  const parsed = parseEmbedPreview(preview());
  assert.equal(parsed.policy, true);
  assert.deepEqual(parsed.entries.map(entry => [entry.kind, entry.name]), [
    ['font', 'arial.ttf'], ['xref', 'site.dwg'], ['image', 'logo.png'],
  ]);
  assert.deepEqual(EMBED_KINDS, ['font', 'xref', 'image']);

  // A row the chrome cannot name honestly is dropped, not rendered as a guess.
  const filtered = parseEmbedPreview({
    policy: false,
    entries: [
      { kind: 'blob', name: 'x', bytes: 1, found: true },
      { kind: 'font', name: '   ', found: true },
      { kind: 'xref' },
      null, 'nope', [],
      { kind: 'font', name: 'ok.ttf', found: true },
    ],
  });
  assert.deepEqual(filtered.entries.map(entry => entry.name), ['ok.ttf']);
  // `found` is only believed when the host said exactly `true`; ambiguity reads
  // as "will not be embedded", never as a promise to embed.
  const loose = parseEmbedPreview({ entries: [{ kind: 'font', name: 'a.ttf', found: 'yes' }] });
  assert.equal(loose.entries[0].found, false);
  // A reported size that is not a usable count is `null` (no size cell), never a
  // fabricated `0 B`.
  const odd = parseEmbedPreview({ entries: [{ kind: 'font', name: 'b.ttf', bytes: '12', found: true }] });
  assert.equal(odd.entries[0].bytes, null);
  // A payload without `policy` keeps it unknown; the toggle then falls back to
  // "on iff something was found" (see `embedInitialToggle`).
  assert.equal(parseEmbedPreview({ entries: [] }).policy, null);
  // Anything without an `entries` array is not a preview at all.
  for (const value of [null, undefined, [], 'nope', 7, {}, { entries: 'nope' }]) {
    assert.equal(parseEmbedPreview(value), null, JSON.stringify(value));
  }
});

test('a refused preview or an empty list opens no dialog at all', async () => {
  assert.equal(EMBED_PREVIEW_COMMAND, 'cad_ymodel_resource_preview');
  assert.equal(EMBED_APPLY_COMMAND, 'cad_set_embed_resources');

  // A carrier whose document is not an archive (Web, `dom-canvas`, no open file)
  // refuses by name: that is "no question", not an error and not "nothing to
  // embed" — the caller keeps today's immediate SAVE.
  const refused = recordingInvoke(() => new Error('unsupported-runtime: cad_ymodel_resource_preview'));
  assert.equal(await readEmbedPreview(refused.invoke), null);
  assert.deepEqual(refused.calls, [{ command: 'cad_ymodel_resource_preview' }]);
  assert.equal(embedNeedsDialog(null), false);

  // Nothing to embed: no extra click for a document that carries no resources.
  const empty = recordingInvoke({ policy: true, entries: [] });
  const emptyPreview = await readEmbedPreview(empty.invoke);
  assert.notEqual(emptyPreview, null);
  assert.equal(embedNeedsDialog(emptyPreview), false);

  const listed = recordingInvoke(preview());
  assert.equal(embedNeedsDialog(await readEmbedPreview(listed.invoke)), true);
});

// --- the lists and the summary --------------------------------------------

test('found and missing rows are grouped, counted and sized separately', () => {
  const entries = parseEmbedPreview(preview()).entries;
  const groups = embedGroups(entries);
  assert.deepEqual(groups.found.map(entry => entry.name), ['arial.ttf', 'logo.png']);
  assert.deepEqual(groups.missing.map(entry => entry.name), ['site.dwg']);
  // The total counts only what would really travel.
  const total = embedTotal(entries);
  assert.deepEqual(total, { count: 2, bytes: 754974 + 1468006 });
  assert.equal(formatByteSize(total.bytes), '2.1 MB');
  assert.equal(formatByteSize(754974), '737 KB');
  assert.equal(formatByteSize(0), '0 B');
  assert.equal(formatByteSize(1024), '1 KB');
  assert.equal(formatByteSize(64 * 1024 * 1024), '64 MB');
  assert.equal(formatByteSize(1536), '1.5 KB');
  assert.equal(formatByteSize(null), '', 'no reported count is no size cell');
  assert.equal(formatByteSize(-1), '');
  // Every kind reads through its own catalog key, never the raw wire word.
  for (const kind of EMBED_KINDS) {
    assert.equal(embedKindKey(kind), `save.embed.kind.${kind}`);
    assert.equal(typeof en[embedKindKey(kind)], 'string', kind);
    assert.equal(typeof zh[embedKindKey(kind)], 'string', kind);
  }
});

test('the toggle starts from the host policy, else from what was found', () => {
  assert.equal(embedInitialToggle(parseEmbedPreview(preview({ policy: true }))), true);
  assert.equal(embedInitialToggle(parseEmbedPreview(preview({ policy: false }))), false,
    'a reported policy wins even when files were found');
  const unknown = parseEmbedPreview(preview({ policy: undefined }));
  assert.equal(unknown.policy, null);
  assert.equal(embedInitialToggle(unknown), true, 'defaults on when something can travel');
  const noneFound = parseEmbedPreview({
    entries: [{ kind: 'font', name: 'gone.ttf', found: false }],
  });
  assert.equal(embedInitialToggle(noneFound), false, 'never pre-embeds nothing');
  assert.equal(embedInitialToggle(null), false);
});

// --- the two calls ---------------------------------------------------------

test('confirm applies the choice and only then sends cad_command SAVE', async () => {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push(args === undefined ? { command } : { command, args });
    return null;
  };
  const outcome = await resolveSaveEmbed('confirm', true, {
    invoke,
    dispatchSave: () => { void invoke('cad_command', { command: 'SAVE' }); },
  });
  assert.deepEqual(outcome, { kind: 'saved' });
  assert.deepEqual(calls, [
    { command: 'cad_set_embed_resources', args: { embed: true } },
    { command: 'cad_command', args: { command: 'SAVE' } },
  ]);
  // The answer travels as a real boolean, both ways.
  const off = [];
  await resolveSaveEmbed('confirm', false, {
    invoke: async (command, args) => { off.push({ command, args }); return null; },
    dispatchSave: () => {},
  });
  assert.deepEqual(off, [{ command: 'cad_set_embed_resources', args: { embed: false } }]);
});

test('cancel sends nothing, and a failed apply never falls through to a save', async () => {
  let dispatched = false;
  const untouched = recordingInvoke(() => { throw new Error('must not be called'); });
  assert.deepEqual(
    await resolveSaveEmbed('cancel', true, { invoke: untouched.invoke, dispatchSave: () => { dispatched = true; } }),
    { kind: 'cancelled' },
  );
  assert.deepEqual(untouched.calls, [], 'cancel makes no IPC call');
  assert.equal(dispatched, false);

  const refused = await resolveSaveEmbed('confirm', true, {
    invoke: async () => { throw new Error('ymodel.unsupported_version: archive is newer'); },
    dispatchSave: () => { dispatched = true; },
  });
  assert.equal(refused.kind, 'failed');
  assert.match(refused.error, /ymodel\.unsupported_version/);
  assert.equal(dispatched, false, 'the save must not silently ignore the choice');
});

// --- the wiring ------------------------------------------------------------

test('the embedded model is asked once, before the save funnel, for SAVE lines only', async () => {
  assert.deepEqual([...SAVE_FAMILY_COMMANDS], ['SAVE', 'SAVEAS', 'QSAVE']);
  for (const text of ['SAVE', ' save ', 'SaveAs', 'QSAVE']) {
    assert.equal(isSaveFamilyCommand(text), true, text);
  }
  for (const text of ['PLOT', 'OPEN', 'SAVEX', '', null, undefined, 0]) {
    assert.equal(isSaveFamilyCommand(text), false, String(text));
  }

  const hook = await readModule('components/use-save-gate.tsx');
  // Only SAVE-family lines reach the preview, and the question is asked before
  // anything is dispatched.
  assert.ok(hook.includes('if (!isSaveFamilyCommand(text)) return false;'));
  const previewAt = hook.indexOf('await readEmbedPreview(invoke)');
  const openAt = hook.indexOf('setActive(preview)');
  const dispatchAt = hook.indexOf('queued.redispatch()');
  assert.ok(previewAt > 0 && openAt > previewAt, 'the dialog opens only from a preview');
  assert.ok(dispatchAt > openAt, 'dispatch happens after the answer, never before');
  // A failed/empty preview returns without touching the dialog state.
  assert.ok(hook.includes('if (preview === null || !embedNeedsDialog(preview)) return false;'));
  // The failure path keeps the dialog and reports the host's sentence.
  assert.ok(hook.includes('resolveSaveEmbed('));
  assert.ok(hook.includes("if (outcome.kind === 'failed')"));
  // Cancel aborts the save: no apply, no command.
  assert.ok(hook.includes('pendingEmbed.current = null;'));

  // One SAVE funnel in the shell: the gate sits before `queueCadCommand`.
  const app = await readModule('App.tsx');
  const askAt = app.indexOf('await beforeSave(text, status,');
  const queueAt = app.indexOf('await queueCadCommand(invoke, text, status, false');
  assert.ok(askAt > 0 && queueAt > askAt, 'the question is asked before cad_command is sent');
  assert.ok(app.includes('{saveGateDialog}'), 'the hook owns the only pre-save dialog slot');
  assert.ok(app.includes('if (saveGatePending()) return;'), 'the dialog owns Escape while it is open');
  // Both command names are declared exactly once, in the model.
  const source = await readAppSource();
  assert.equal(source.split(`'${EMBED_PREVIEW_COMMAND}'`).length - 1, 1);
  assert.equal(source.split(`'${EMBED_APPLY_COMMAND}'`).length - 1, 1);
});

test('the dialog is one accessible card with the published copy', async () => {
  const keys = [
    'save.embed.title',
    'save.embed.toggle',
    'save.embed.benefit',
    'save.embed.cost',
    'save.embed.total',
    'save.embed.groupFound',
    'save.embed.groupMissing',
    'save.embed.empty',
    'save.embed.ymodelOnly',
  ];
  for (const key of keys) {
    assert.equal(typeof en[key], 'string', `en is missing ${key}`);
    assert.equal(typeof zh[key], 'string', `zh-CN is missing ${key}`);
    assert.ok(en[key].trim().length > 0, key);
  }
  // `{count}`/`{size}` are the total line's only placeholders, identical in both
  // catalogs (a mismatch would print a literal brace).
  const placeholder = /\{([A-Za-z_][A-Za-z0-9_]*)\}/g;
  for (const catalog of [zh, en]) {
    assert.deepEqual(
      [...catalog['save.embed.total'].matchAll(placeholder)].map(match => match[1]),
      ['count', 'size'],
    );
  }

  const dialog = await readModule('components/SaveEmbedDialog.tsx');
  // Its own confirm label (`save.embed.confirm`) and the funnel's cancel.
  assert.equal(en['save.embed.confirm'], 'Save and embed');
  assert.equal(zh['save.embed.confirm'], '保存并嵌入');
  for (const key of [...keys, 'save.embed.confirm', 'tool.cancel']) {
    assert.ok(dialog.includes(`t('${key}'`), key);
  }
  assert.ok(!dialog.includes("save.dwg_confirm"), 'no deleted DWG copy is reused');
  assert.ok(dialog.includes('embedKindKey(entry.kind)'), 'kind labels come from the catalog');
  // One accessible modal from the shared dialog family, focus on the safe answer.
  assert.ok(dialog.includes('role="dialog"'));
  assert.ok(dialog.includes('aria-modal="true"'));
  assert.ok(dialog.includes('initial: cancelRef'));
  assert.ok(dialog.includes('createPortal'));
  // Found and missing rows cannot look alike: the missing group carries its own
  // class and no size cell.
  assert.ok(dialog.includes('is-found'));
  assert.ok(dialog.includes('is-missing'));
  assert.ok(dialog.includes("const size = entry.found ? formatByteSize(entry.bytes) : '';"));
  // The dialog never sends anything itself: the hook owns the two calls.
  assert.ok(!dialog.includes('invoke('), 'no IPC from the dialog');
  assert.ok(!dialog.includes('cad_command'), 'no direct command dispatch');
});
