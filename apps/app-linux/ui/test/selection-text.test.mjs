// Editing the selected text entity from the docked properties palette
// (ADR 0020 addendum B), plus the `text_prefill` hand-off into the command
// line's text step.
//
// Two shapes are under test here and they must stay separate:
//
// - the **panel editor**, which edits the host's *current single selection*
//   through `cad_set_selected_text` (no entity id in the payload, because the
//   host resolves and validates the selection), and
// - the **text step** of a text-carrying tool, which is unchanged except that a
//   `TEXTEDIT` step may arrive with the current string prefilled.
//
// The panel is chrome outside the CAD rectangle: the native surface is
// input-transparent and DOM cannot be painted over it, so nothing here may move
// the editor onto the canvas (asserted below, not just documented).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  SELECTED_TEXT_COMMAND,
  SELECTED_TEXT_MAX_LENGTH,
  TEXT_EDIT_INITIAL,
  TEXT_EDIT_KEYS,
  checkSelectedText,
  selectedTextOf,
  setSelectedText,
  textEditApplied,
  textEditDraftChanged,
  textEditHostChanged,
  textEditKeyAction,
  textEditView,
} from '../src/textedit.mjs';
import { TEXT_TOO_LONG_KEY, TOOL_TEXT_MAX_LENGTH, toolTextPrefill } from '../src/text.mjs';
import { createTranslator } from '../src/i18n.mjs';
import { readCatalog, readModule } from './helpers.mjs';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const t = createTranslator({ 'zh-CN': zh, en }, 'zh-CN');
const te = createTranslator({ 'zh-CN': zh, en }, 'en');

// A single selected text entity as the host reports it. `text` is the
// round-trippable string, deliberately outside `rows` (those are display only).
const singleText = text => ({ count: 1, empty: false, rows: [{ key: 'text', value: text }], mixedKeys: [], text });

/** Fold a sequence of host values / keystrokes into the editor state. */
function fold(steps, start = TEXT_EDIT_INITIAL) {
  return steps.reduce((state, step) => (step.host !== undefined
    ? textEditHostChanged(state, step.host)
    : textEditDraftChanged(state, step.draft)), start);
}

test('only a single selected object with a real string is editable', () => {
  assert.equal(selectedTextOf(singleText('Ground floor')), 'Ground floor');
  // A multi-selection never exposes one entity's text, whatever the field says.
  assert.equal(selectedTextOf({ count: 2, empty: false, rows: [], mixedKeys: [], text: 'Ground floor' }), null);
  // Nothing selected.
  for (const selection of [null, undefined, { count: 0, empty: true, rows: [], mixedKeys: [] },
    { count: 0, empty: true, rows: [], mixedKeys: [], text: null }]) {
    assert.equal(selectedTextOf(selection), null, JSON.stringify(selection));
  }
  // A non-text entity, an older host without the field, or a malformed value is
  // "not editable" — never a fabricated string and never a thrown status.
  for (const text of [null, undefined, 42, {}, [], true]) {
    assert.equal(selectedTextOf({ ...singleText(''), text }), null, String(text));
  }
  const { text: _omitted, ...noField } = singleText('');
  assert.equal(selectedTextOf(noField), null);
  for (const malformed of ['x', 1, [], null]) assert.equal(selectedTextOf(malformed), null, String(malformed));
  // An empty string is a real string (the host refuses it on apply, and the
  // editor must show the object rather than hide it).
  assert.equal(selectedTextOf(singleText('')), '');
});

test('Apply sends the field verbatim to cad_set_selected_text and nothing else', async () => {
  // MTEXT markup, astral characters, line breaks and edge whitespace all travel
  // exactly as they are: no trim, no normalisation, no markup rendering.
  const verbatim = '  Line one\\PLine two\n第 3 行 😀👩‍🚀  ';
  // The field *shows* that same string, character for character.
  assert.equal(textEditView(fold([{ host: verbatim }])).value, verbatim);
  assert.equal(textEditView(fold([{ host: verbatim }])).length, [...verbatim].length);
  const calls = [];
  const result = await setSelectedText(async (...args) => { calls.push(args); return { accepted: true }; }, verbatim);
  assert.deepEqual(calls, [['cad_set_selected_text', { text: verbatim }]]);
  assert.equal(SELECTED_TEXT_COMMAND, 'cad_set_selected_text');
  assert.deepEqual(result, { kind: 'accepted' });
  // No entity id: the host resolves the selection itself, so the payload has one
  // key only — a second field would be a second source of truth.
  assert.deepEqual(Object.keys(calls[0][1]), ['text']);
  // Acceptance is queue admission, not a completed edit.
  assert.equal('completed' in result, false);
  assert.equal('text' in result, false);
  // A single-line input strips line breaks from `value`; the field must not be one.
  const editor = await readModule('components/SelectionTextEditor.jsx');
  assert.ok(editor.includes('<textarea'), 'the editor field is a textarea');
  assert.ok(!editor.includes('<input'), 'the editor field is not a single-line input');
});

test('the text edit uses the same refusal surface as the text step, and never fakes success', async () => {
  assert.equal(SELECTED_TEXT_MAX_LENGTH, TOOL_TEXT_MAX_LENGTH);
  assert.equal(SELECTED_TEXT_MAX_LENGTH, 1024);
  // The same rules, locally: an obvious refusal never reaches IPC.
  const noIpc = () => assert.fail('a refused edit reached IPC');
  for (const text of ['', '   ', '\n\t ', null, undefined, 42, {}]) {
    const result = await setSelectedText(noIpc, text);
    assert.equal(result.kind, 'refused', String(text));
    assert.ok(result.error);
  }
  assert.equal((await setSelectedText(noIpc, '   ')).reason, 'empty');
  assert.equal((await setSelectedText(noIpc, null)).reason, 'kind');
  assert.equal(checkSelectedText(' ok ').kind, 'ok');
  // A host refusal is explicit, never a silent success.
  for (const failing of [
    async () => { throw Error('selection is not a text entity'); },
    async () => ({ accepted: false }),
    async () => null,
    async () => undefined,
  ]) {
    const result = await setSelectedText(failing, 'replacement');
    assert.equal(result.kind, 'failed');
    assert.ok(result.error);
  }
});

test('an empty or whitespace-only draft is refused, kept verbatim and never clears the selection', async () => {
  // The view keeps the whitespace exactly as typed, and Apply stays clickable so
  // the refusal is *surfaced* instead of leaving a mysteriously dead button.
  const state = fold([{ host: 'Ground floor' }, { draft: '   ' }]);
  const view = textEditView(state);
  assert.equal(view.value, '   ');
  assert.equal(view.empty, true);
  assert.equal(view.canApply, true);
  assert.equal(view.detached, false);
  assert.equal(checkSelectedText(view.value).reason, 'empty');
  // The refusal message is the editor's own catalog entry, and the component
  // routes the `empty` reason to it.
  const editor = await readModule('components/SelectionTextEditor.jsx');
  assert.ok(editor.includes("result.reason === 'empty'"));
  assert.ok(editor.includes('t(TEXT_EDIT_KEYS.empty)'));
  // Nothing here touches the selection: the only IPC is the text, and no
  // selection command is issued anywhere in the editor.
  assert.ok(!/cad_(select|clear|pick)/.test(editor), 'the editor never changes the selection');
  assert.ok(!editor.includes("invoke("), 'the view never talks to the host directly');
});

test('over the bound the limit and the count are shown, the draft is kept and nothing is truncated', async () => {
  const over = 'a'.repeat(SELECTED_TEXT_MAX_LENGTH + 1);
  const view = textEditView(fold([{ host: 'x' }, { draft: over }]));
  assert.equal(view.length, SELECTED_TEXT_MAX_LENGTH + 1);
  assert.equal(view.max, SELECTED_TEXT_MAX_LENGTH);
  assert.equal(view.tooLong, true);
  assert.equal(view.canApply, false, 'an over-long draft is not offered for apply');
  assert.equal(view.value, over, 'the draft is kept verbatim, never truncated');
  // The bound is inclusive, and counted in Unicode scalar values like the host:
  // an astral character is one character, not two UTF-16 code units.
  assert.equal(textEditView(fold([{ host: 'x' }, { draft: '😀'.repeat(SELECTED_TEXT_MAX_LENGTH) }])).tooLong, false);
  assert.equal(textEditView(fold([{ host: 'x' }, { draft: '😀'.repeat(SELECTED_TEXT_MAX_LENGTH + 1) }])).tooLong, true);
  // The limit shown is the text step's message with the real number, so both
  // entry points name the same bound in the same words.
  const editor = await readModule('components/SelectionTextEditor.jsx');
  assert.ok(editor.includes('t(TEXT_TOO_LONG_KEY, { max: SELECTED_TEXT_MAX_LENGTH })'));
  assert.ok(editor.includes('const tooLongText = view.tooLong'));
  assert.ok(!editor.includes('maxLength'), 'the field must not silently truncate');
  assert.ok(editor.includes('role="alert"'), 'the refusal is announced');
});

test('the count is shown next to the field from the catalog', async () => {
  const view = textEditView(fold([{ host: 'Ground floor' }]));
  assert.equal(view.length, 12);
  assert.equal(view.max, 1024);
  const editor = await readModule('components/SelectionTextEditor.jsx');
  assert.ok(editor.includes('t(TEXT_EDIT_KEYS.count, { used: view.length, max: view.max })'));
});

test('a selection change under focus keeps the draft and says why Apply is off', () => {
  // The user is editing "A" and the host reports a different selection.
  const editing = fold([{ host: 'A original' }, { draft: 'A edited' }]);
  assert.equal(textEditView(editing).canApply, true);
  // Deselected (or multi-selected): the draft is kept verbatim, detached and
  // blocked — it is never silently discarded and never applied to nothing.
  const detached = textEditHostChanged(editing, null);
  assert.equal(detached.draft, 'A edited');
  assert.equal(detached.base, 'A original');
  assert.equal(detached.detached, true);
  const view = textEditView(detached);
  assert.equal(view.visible, true, 'the parked draft stays on screen');
  assert.equal(view.canApply, false);
  assert.equal(view.value, 'A edited');
  // Selecting the same text entity again re-attaches the draft (no data loss and
  // no need for a discard button).
  const reattached = textEditHostChanged(detached, 'A original');
  assert.equal(reattached.detached, false);
  assert.equal(reattached.draft, 'A edited');
  assert.equal(textEditView(reattached).canApply, true);
  // Another text entity while the draft is edited: also parked, not applied to
  // an object the user never edited.
  const other = textEditHostChanged(editing, 'B');
  assert.equal(other.detached, true);
  assert.equal(other.draft, 'A edited');
  // An untouched field simply follows the new object (there is nothing to lose).
  const untouched = textEditHostChanged(fold([{ host: 'A original' }]), 'B');
  assert.deepEqual(untouched, { base: 'B', draft: 'B', detached: false });
  // Untouched and deselected: nothing to show at all.
  const hidden = textEditHostChanged(fold([{ host: 'A original' }]), null);
  assert.equal(textEditView(hidden).visible, false);
  // No draft, no host text: the editor renders nothing.
  assert.equal(textEditView(TEXT_EDIT_INITIAL).visible, false);
});

test('an accepted edit becomes the new baseline so the host echo does not park the draft', () => {
  const editing = fold([{ host: 'A original' }, { draft: 'A replacement' }]);
  const applied = textEditApplied(editing, 'A replacement');
  assert.deepEqual(applied, { base: 'A replacement', draft: 'A replacement', detached: false });
  // The next status echoes the same string: not a selection change.
  assert.equal(textEditHostChanged(applied, 'A replacement'), applied);
  assert.equal(textEditView(applied).canApply, true);
  // Typing during the in-flight edit is kept when the baseline moves.
  const typing = fold([{ host: 'A original' }, { draft: 'A repl' }, { draft: 'A replacement 2' }]);
  const raced = textEditApplied(typing, 'A replacement');
  assert.equal(raced.base, 'A replacement');
  assert.equal(raced.draft, 'A replacement 2');
  assert.equal(raced.detached, false);
  // The host is authoritative: an untouched draft follows a normalised echo.
  assert.deepEqual(textEditHostChanged(applied, 'A replacement!'), { base: 'A replacement!', draft: 'A replacement!', detached: false });
});

test('an IME composition never applies the edit, and Enter is a line break', () => {
  // The two IME signals: a standard `isComposing` and the legacy `keyCode` 229
  // that some IMEs send for the Enter that commits the composition.
  assert.equal(textEditKeyAction({ key: 'Enter', ctrlKey: true, isComposing: true }), 'none');
  assert.equal(textEditKeyAction({ key: 'Enter', ctrlKey: true, keyCode: 229 }), 'none');
  assert.equal(textEditKeyAction({ key: 'Enter', metaKey: true, isComposing: true }), 'none');
  // The submit is the explicit accelerator, once per press.
  assert.equal(textEditKeyAction({ key: 'Enter', ctrlKey: true }), 'submit');
  assert.equal(textEditKeyAction({ key: 'Enter', metaKey: true }), 'submit');
  assert.equal(textEditKeyAction({ key: 'Enter', ctrlKey: true, repeat: true }), 'none');
  // A plain Enter stays a newline: the text is edited verbatim, so a block keeps
  // its lines and Enter must not commit half an edit.
  assert.equal(textEditKeyAction({ key: 'Enter' }), 'none');
  for (const key of ['a', ' ', 'Escape', 'Tab', 'ArrowDown']) assert.equal(textEditKeyAction({ key }), 'none', key);
  assert.equal(textEditKeyAction(), 'none');
});

test('the editor routes its keys and Apply through the shell, never through IPC or the canvas', async () => {
  const editor = await readModule('components/SelectionTextEditor.jsx');
  const app = await readModule('App.jsx');
  const palette = await readModule('components/PropertiesPalette.jsx');
  // IME-safe key path.
  assert.ok(editor.includes('if (textEditKeyAction(event) !== \'submit\') return;'));
  assert.ok(editor.includes('event.preventDefault();'));
  // The text always comes from the field as it last reported it, so a click that
  // lands with an IME compositionend in the same task sends the real value.
  assert.ok(editor.includes('const sending = draftRef.current;'));
  assert.ok(editor.includes('draftRef.current = event.target.value;'));
  // One edit IPC, through the shell's admission funnel.
  assert.ok(editor.includes('shell.applySelectedText(sending)'));
  assert.ok(editor.includes('if (!view.canApply) return;'), 'a detached/over-long Ctrl+Enter sends nothing');
  assert.ok(app.includes('setSelectedText(invoke, text)'));
  assert.ok(app.includes('applySelectedText,'));
  assert.ok(!app.includes("invoke('cad_set_selected_text'"), 'the IPC lives in textedit.mjs only');
  // The editor is mounted in every selection state, so a selection change cannot
  // unmount (and thereby discard) a draft; the palette shows it below the rows.
  assert.ok(palette.includes('<SelectionTextEditor />'));
  assert.ok(palette.indexOf('<SelectionTextEditor />') > palette.indexOf("view.kind === 'multi'"));
  // Chrome, not canvas: no overlay over the native CAD rectangle, and the
  // native partition stays childless (`Workspace.jsx` keeps `.native-input`).
  assert.ok(!editor.includes('native-input'));
  assert.ok(!/position:\s*'absolute'/.test(editor));
  assert.ok(!editor.includes('zIndex'));
  // User-visible copy comes from the catalog only — every `t(...)` here is fed by
  // an exported key constant, so there is no prose (and no literal key) in JSX.
  const calls = [...editor.matchAll(/\bt\(([^,)]*)/g)].map(match => match[1].trim());
  assert.ok(calls.length >= 5, `expected the editor to translate its copy, saw ${calls.length}`);
  for (const source of calls) {
    assert.ok(source.startsWith('TEXT_EDIT_KEYS.') || source === 'TEXT_TOO_LONG_KEY',
      `unexpected copy source: t(${source}`);
  }
  assert.ok(!/[\u4e00-\u9fff]/.test(editor), 'no hardcoded CJK');
});

test('TEXTEDIT prefill seeds the text step; a fresh TEXT still starts empty', () => {
  // The reader is defensive: a missing/malformed field means "no prefill".
  assert.equal(toolTextPrefill({ key: 'draw.text', awaiting_text: true, text_prefill: 'Existing\\Ptext' }), 'Existing\\Ptext');
  assert.equal(toolTextPrefill({ text_prefill: '' }), '', 'an empty prefill is a real value');
  for (const tool of [null, undefined, 'draw.text', [], { }, { text_prefill: null }, { text_prefill: 42 }, { text_prefill: {} }]) {
    assert.equal(toolTextPrefill(tool), null, JSON.stringify(tool));
  }
});

test('the command line seeds the field from the prefill and never overwrites typing', async () => {
  const commandline = await readModule('components/CommandLine.jsx');
  // The step transition seeds from the prefill, so `TEXTEDIT` starts on the
  // current string and `TEXT` (no prefill) starts empty, exactly as before.
  assert.ok(commandline.includes('setTextDraft(toolTextPrefill(tool) ?? \'\');'));
  assert.ok(commandline.includes('awaitingToolText(shell.status)'));
  // A prefill that arrives later fills an empty field only: idempotent, one-way.
  assert.ok(commandline.includes('if (!textMode || !prefill) return;'));
  assert.ok(commandline.includes("setTextDraft(current => (current === '' ? prefill : current));"));
  // The existing text-mode contract is untouched.
  assert.ok(commandline.includes('inputRef.current?.focus();'));
  assert.ok(commandline.includes('textEscapeAction({ awaitingText: textMode, focused: true }) === \'dismiss\''));
  assert.ok(commandline.includes('handleCommandKey(event, submitText);'));
  assert.ok(commandline.includes('if (draft.empty || draft.tooLong) return;'));
  assert.ok(commandline.includes("value={textMode ? textDraft : shell.command}"));
});

// Every key the editor can render. The catalogs live in `crates/**`, which is the
// backend lane's half of this slice: when they have not landed yet the resolution
// check is **reported as skipped with the exact missing keys** — never faked as a
// pass, and it becomes a real pass as soon as the keys land (no test edit needed).
const EDIT_KEYS = [...Object.values(TEXT_EDIT_KEYS), TEXT_TOO_LONG_KEY];
const MISSING_EDIT_KEYS = EDIT_KEYS.filter(key => typeof zh[key] !== 'string' || typeof en[key] !== 'string');

test('every text-edit catalog key resolves in both locales with matching placeholders',
  { skip: MISSING_EDIT_KEYS.length > 0 && `catalog keys not landed by the backend lane yet: ${MISSING_EDIT_KEYS.join(', ')}` },
  () => {
    const placeholder = /(?<!\\)\{([A-Za-z_][A-Za-z0-9_]*)\}/g;
    for (const key of EDIT_KEYS) {
      assert.ok(zh[key].trim().length > 0, `empty zh-CN ${key}`);
      assert.ok(en[key].trim().length > 0, `empty en ${key}`);
      const a = [...zh[key].matchAll(placeholder)].map(match => match[1]).sort();
      const b = [...en[key].matchAll(placeholder)].map(match => match[1]).sort();
      assert.deepEqual(a, b, `placeholders differ for ${key}`);
      // A concrete key must resolve, not fall back to the key itself.
      assert.notEqual(t(key), key, `zh-CN does not resolve ${key}`);
      assert.notEqual(te(key), key, `en does not resolve ${key}`);
    }
    assert.equal(t(TEXT_EDIT_KEYS.count, { used: 3, max: 1024 }).includes('3'), true);
  });

test('the text-edit keys are catalog-shaped, so the catalogs can carry them', () => {
  const shape = /^[a-z][a-z0-9_]*(\.[a-z0-9_]+)+$/;
  for (const key of EDIT_KEYS) assert.match(key, shape, key);
});

test('the text-edit key constants stay the documented names', () => {
  // Pinned so a rename is a contract change, not a silent catalog miss.
  assert.deepEqual(TEXT_EDIT_KEYS, {
    field: 'properties.text.field',
    apply: 'properties.text.apply',
    count: 'properties.text.count',
    empty: 'properties.text.empty',
    detached: 'properties.text.detached',
    failed: 'properties.text.failed',
  });
  assert.equal(SELECTED_TEXT_COMMAND, 'cad_set_selected_text');
});
