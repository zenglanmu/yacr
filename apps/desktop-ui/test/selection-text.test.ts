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
  selectedTextMultilineOf,
  selectedTextOf,
  setSelectedText,
  textEditApplied,
  textEditDraftChanged,
  textEditHostChanged,
  textEditKeyAction,
  textEditView,
} from '../src/tools/textedit.ts';
import { POPUP_TEXT_KEYS, textEditKeyAction as popupTextEditKeyAction } from '../src/popup/popup-textedit.ts';
import { TEXT_TOO_LONG_KEY, TOOL_TEXT_MAX_LENGTH, toolTextPrefill } from '../src/tools/text.ts';
import { createTranslator } from '../src/i18n.ts';
import { readCatalog, readModule } from './helpers.ts';

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
  const editor = await readModule('components/SelectionTextEditor.tsx');
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
  const editor = await readModule('components/SelectionTextEditor.tsx');
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
  const editor = await readModule('components/SelectionTextEditor.tsx');
  assert.ok(editor.includes('t(TEXT_TOO_LONG_KEY, { max: SELECTED_TEXT_MAX_LENGTH })'));
  assert.ok(editor.includes('const tooLongText = view.tooLong'));
  assert.ok(!editor.includes('maxLength'), 'the field must not silently truncate');
  assert.ok(editor.includes('role="alert"'), 'the refusal is announced');
});

test('the count is shown next to the field from the catalog', async () => {
  const view = textEditView(fold([{ host: 'Ground floor' }]));
  assert.equal(view.length, 12);
  assert.equal(view.max, 1024);
  const editor = await readModule('components/SelectionTextEditor.tsx');
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

/** A single selected text entity whose host reports a line-break mode. */
const textWithMode = (text, multiline) => ({ ...singleText(text), text_multiline: multiline });

/**
 * Feed keydowns through the editor's key map, sending only on `'submit'`.
 *
 * The component does exactly this: `'none'` is left to the textarea (which
 * inserts a line break), `'block'` calls `preventDefault()` so neither a break
 * nor a send happens, and `'submit'` calls `apply()` once.
 */
async function pressKeys(keys, multiline, text = 'Ground floor') {
  const calls = [];
  const invoke = async (command, payload) => { calls.push([command, payload]); return { accepted: true }; };
  for (const key of keys) {
    if (textEditKeyAction(key, multiline) === 'submit') await setSelectedText(invoke, text);
  }
  return calls;
}

test('a single-line entity applies on Enter, and Shift/Alt+Enter neither send nor break', async () => {
  // TEXT: the host refuses a newline by name, so offering the break here would
  // only earn that refusal on Apply. Enter applies, like the command line.
  assert.equal(selectedTextMultilineOf(textWithMode('Ground floor', false)), false);
  assert.equal(textEditKeyAction({ key: 'Enter' }, false), 'submit');
  assert.equal(textEditKeyAction({ key: 'Enter', ctrlKey: true }, false), 'submit');
  assert.equal(textEditKeyAction({ key: 'Enter', metaKey: true }, false), 'submit');
  // `block` (not `none`): the component still prevents the default, so the
  // textarea inserts nothing at all.
  assert.equal(textEditKeyAction({ key: 'Enter', shiftKey: true }, false), 'block');
  assert.equal(textEditKeyAction({ key: 'Enter', altKey: true }, false), 'block');
  // One press, one send — a held key repeats but applies once.
  const one = await pressKeys([{ key: 'Enter' }, { key: 'Enter', repeat: true }, { key: 'Enter', repeat: true }], false);
  assert.deepEqual(one, [['cad_set_selected_text', { text: 'Ground floor' }]]);
  // Shift/Alt+Enter send nothing, and the block is what stops the line break.
  assert.deepEqual(await pressKeys([
    { key: 'Enter', shiftKey: true },
    { key: 'Enter', altKey: true },
  ], false), []);
});

test('the single-line affordance promises Enter and explains the no-break rule', async () => {
  const editor = await readModule('components/SelectionTextEditor.tsx');
  // The button title follows the mode: `(Enter)` where Enter applies, the
  // Ctrl+Enter chord elsewhere (multi-line and unknown, unchanged).
  assert.ok(editor.includes('`${applyLabel} (Enter)`'));
  assert.ok(editor.includes('`${applyLabel} (Ctrl+Enter)`'));
  // The hint is the popup's single-line line, so both editors word the same
  // rule the same way; it is announced with the field and only shown for TEXT.
  assert.ok(editor.includes('t(POPUP_TEXT_KEYS.hintSingle)'));
  assert.ok(editor.includes('hint ? hintId : null'));
  assert.ok(t(POPUP_TEXT_KEYS.hintSingle).length > 0);
  assert.ok(te(POPUP_TEXT_KEYS.hintSingle).length > 0);
});

test('a multi-line entity keeps Enter as a line break and never sends it', async () => {
  assert.equal(selectedTextMultilineOf(textWithMode('Line one\\PLine two', true)), true);
  // `none` lets the textarea insert the break; it is data, not a confirmation.
  assert.equal(textEditKeyAction({ key: 'Enter' }, true), 'none');
  assert.equal(textEditKeyAction({ key: 'Enter', shiftKey: true }, true), 'none');
  assert.deepEqual(await pressKeys([{ key: 'Enter' }, { key: 'Enter', shiftKey: true }], true), []);
  // The explicit accelerator still applies here too.
  assert.equal(textEditKeyAction({ key: 'Enter', ctrlKey: true }, true), 'submit');
  assert.equal(textEditKeyAction({ key: 'Enter', metaKey: true }, true), 'submit');
});

test('without `text_multiline` the keys stay exactly as they were', async () => {
  // An older host does not send the field. Nothing is guessed: Enter is a line
  // break and Ctrl/Cmd+Enter applies, which is the pre-`text_multiline` contract.
  assert.equal(selectedTextMultilineOf(singleText('Ground floor')), null);
  assert.equal(textEditKeyAction({ key: 'Enter' }), 'none');
  assert.equal(textEditKeyAction({ key: 'Enter' }, null), 'none');
  assert.equal(textEditKeyAction({ key: 'Enter', shiftKey: true }), 'none');
  assert.equal(textEditKeyAction({ key: 'Enter', altKey: true }), 'none');
  assert.deepEqual(await pressKeys([{ key: 'Enter' }, { key: 'Enter', shiftKey: true }], null), []);
  assert.equal(textEditKeyAction({ key: 'Enter', ctrlKey: true }), 'submit');
  assert.equal(textEditKeyAction({ key: 'Enter', metaKey: true }), 'submit');
  // A held accelerator is still one action.
  assert.equal(textEditKeyAction({ key: 'Enter', ctrlKey: true, repeat: true }), 'none');
  for (const key of ['a', ' ', 'Escape', 'Tab', 'ArrowDown']) assert.equal(textEditKeyAction({ key }), 'none', key);
  assert.equal(textEditKeyAction(), 'none');
});

test('an IME composition never applies the edit, in either mode', () => {
  // The two IME signals: a standard `isComposing` and the legacy `keyCode` 229
  // that some IMEs send for the Enter that commits the composition.
  for (const mode of [true, false, null]) {
    assert.equal(textEditKeyAction({ key: 'Enter', isComposing: true }, mode), 'none', `isComposing mode=${mode}`);
    assert.equal(textEditKeyAction({ key: 'Enter', ctrlKey: true, isComposing: true }, mode), 'none', `ctrl+compose mode=${mode}`);
    assert.equal(textEditKeyAction({ key: 'Enter', keyCode: 229 }, mode), 'none', `keyCode 229 mode=${mode}`);
    assert.equal(textEditKeyAction({ key: 'Enter', metaKey: true, isComposing: true }, mode), 'none', `meta+compose mode=${mode}`);
    assert.equal(textEditKeyAction({ key: 'Enter', repeat: true, ctrlKey: true }, mode), 'none', `repeated chord mode=${mode}`);
  }
});

test('the docked field and the popup agree on every key, in both known modes', () => {
  // The panel must mirror the popup exactly; only the unknown mode is its own
  // (legacy) contract, because the popup always receives an explicit boolean.
  const keys = [
    { key: 'Enter' },
    { key: 'Enter', shiftKey: true },
    { key: 'Enter', altKey: true },
    { key: 'Enter', ctrlKey: true },
    { key: 'Enter', metaKey: true },
    { key: 'Enter', ctrlKey: true, shiftKey: true },
    { key: 'Enter', repeat: true },
    { key: 'Enter', isComposing: true },
    { key: 'Enter', keyCode: 229 },
    { key: 'a' },
    { key: 'Escape' },
  ];
  for (const mode of [true, false]) {
    for (const event of keys) {
      assert.equal(
        textEditKeyAction(event, mode),
        popupTextEditKeyAction(event, mode),
        `${JSON.stringify(event)} mode=${mode}`,
      );
    }
  }
});

test('the edit mode comes from `text_multiline`, never from a display row type', async () => {
  // The display row is deliberately misleading: the mode must ignore it.
  const asMtextRow = {
    count: 1, empty: false, mixedKeys: [], text: 'A', text_multiline: true,
    rows: [{ key: 'type', value: 'TEXT' }],
  };
  assert.equal(selectedTextMultilineOf(asMtextRow), true);
  const asTextRow = { ...asMtextRow, text_multiline: false, rows: [{ key: 'type', value: 'MTEXT' }] };
  assert.equal(selectedTextMultilineOf(asTextRow), false);
  // The field is absent: unknown, however suggestive the row is.
  const { text_multiline: _omitted, ...olderHost } = asMtextRow;
  assert.equal(selectedTextMultilineOf(olderHost), null);
  // A non-boolean value is never coerced into a mode.
  for (const value of ['true', 1, 0, {}, [], null]) {
    assert.equal(selectedTextMultilineOf({ ...asMtextRow, text_multiline: value }), null, String(value));
  }
  // Only a single selection with real text has an editable mode.
  assert.equal(selectedTextMultilineOf({ ...asMtextRow, count: 2 }), null);
  assert.equal(selectedTextMultilineOf({ ...asMtextRow, text: null }), null);
  for (const malformed of [null, undefined, [], 'A']) assert.equal(selectedTextMultilineOf(malformed), null, String(malformed));
  // The component binds the keys to that reader — no row value reaches them.
  const editor = await readModule('components/SelectionTextEditor.tsx');
  assert.ok(editor.includes('const multiline = selectedTextMultilineOf(shell.status?.selection ?? null);'));
  assert.ok(editor.includes('const action = textEditKeyAction(event, multiline);'));
  assert.ok(!/\.type\b|\[['"]type['"]\]/.test(editor), 'the keys never read a display type');
});

test('the editor routes its keys and Apply through the shell, never through IPC or the canvas', async () => {
  const editor = await readModule('components/SelectionTextEditor.tsx');
  const app = await readModule('App.tsx');
  const palette = await readModule('components/PropertiesPalette.tsx');
  // IME-safe, mode-aware key path: a `block` still prevents the default, and
  // only `submit` applies.
  assert.ok(editor.includes('const action = textEditKeyAction(event, multiline);'));
  assert.ok(editor.includes("if (action === 'submit') apply();"));
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
    // The popup entry's label is popup copy (`POPUP_TEXT_KEYS.entry`), mounted
    // here because this is where the field it extends lives.
    assert.ok(source.startsWith('TEXT_EDIT_KEYS.') || source.startsWith('POPUP_TEXT_KEYS.') || source === 'TEXT_TOO_LONG_KEY',
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
  const commandline = await readModule('components/CommandLine.tsx');
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
