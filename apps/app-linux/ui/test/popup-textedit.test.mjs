// Text-edit popup contract (ADR 0021 D4), UI half.
//
// The dialog edits one text entity's **raw** DXF string. The host derives the
// payload from the current single selection (`{text, multiline}`, both always
// present), owns the `\n` ⇄ `\P` mapping per source class, and owns every
// refusal; this bundle edits the string verbatim and sends it back through
// `cad_set_selected_text`.
//
// These tests lock what must stay true without a renderer:
//   * the boot predicate accepts exactly `{text: string, multiline: boolean}`
//     and falls back to the normal shell for anything else;
//   * the raw string is shown and returned verbatim (markup, braces, astral
//     characters) and nothing in the UI converts a line break;
//   * the key rules per mode: Enter = line break (multiline) / apply (single
//     line), Ctrl/Cmd+Enter = apply, Shift+Enter inserts nothing single-line,
//     and an IME composition never submits;
//   * the live count uses Unicode scalar values, the 1024 bound refuses without
//     truncating, and an empty draft is refused with the catalog's own line;
//   * a refusal (local, host, or busy) keeps the draft; only a successful apply
//     closes the window;
//   * the palette entry is offered only for a real `selection.text` and sends
//     `{kind:'textEdit'}`; the host's `popup.no_text_selection` reaches the
//     error banner as catalog copy;
//   * the dialog document still starts none of the shell's global effects;
//   * every key used is a declared contract key and resolves in both locales.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCatalog, readCss, readModule } from './helpers.mjs';
import {
  DIALOG_TEXT_BODY,
  DIALOG_TITLE,
  TEXT_FIELD,
  TEXT_FIELD_FOCUS,
  TEXT_FIELD_FOCUSED,
  TEXT_HEAD,
  TEXT_META,
} from '../src/popup-layout.mjs';
import {
  openPopup,
  POPUP_GLOBAL,
  POPUP_KINDS,
  POPUP_OPEN_COMMAND,
  popupBoot,
  popupView,
  readPopupBoot,
  textEditPayload,
} from '../src/popup.mjs';
import {
  POPUP_TEXT_KEYS,
  POPUP_TEXT_MAX,
  canOpenTextEditPopup,
  createTextEditSubmitter,
  textEditApplyOutcome,
  textEditDraftView,
  textEditKeyAction,
  textEditLocalRefusal,
  textEditOpenError,
} from '../src/popup-textedit.mjs';
import { createTranslator } from '../src/i18n.mjs';

const ZH = await readCatalog('zh-CN');
const EN = await readCatalog('en');
const T_ZH = createTranslator({ 'zh-CN': ZH, en: EN }, 'zh-CN');
const T_EN = createTranslator({ 'zh-CN': ZH, en: EN }, 'en');

// A string that is raw DXF content and nothing else: a `\P` paragraph break, the
// grouping braces MTEXT uses, an `\W` line-spacing code, and an astral character
// (one Unicode scalar value, two UTF-16 code units).
const RAW_MTEXT = '\\W1.5;First line\\P{Second} line \u{1F600}';

/** A valid textEdit literal exactly as the host injects it. */
function textEditLiteral(payload = { text: RAW_MTEXT, multiline: true }, overrides = {}) {
  return {
    [POPUP_GLOBAL]: { kind: 'textEdit', modal: true, label: 'popup-1', payload, ...overrides },
  };
}

/** An `invoke` stand-in that records calls and answers `response`. */
function recordingInvoke(response = { accepted: true }) {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    return typeof response === 'function' ? response(command, args) : response;
  };
  return { invoke, calls };
}

test('the textEdit kind is implemented alongside about', () => {
  assert.deepEqual(POPUP_KINDS, ['about', 'textEdit']);
});

test('a well-formed textEdit literal boots the dialog, both modes', () => {
  for (const multiline of [true, false]) {
    const boot = popupBoot(textEditLiteral({ text: RAW_MTEXT, multiline }));
    assert.ok(boot, `multiline=${multiline} must boot`);
    assert.equal(boot.kind, 'textEdit');
    assert.equal(boot.modal, true);
    assert.ok(Object.isFrozen(boot));
    const view = popupView(boot);
    assert.deepEqual(view, {
      kind: 'textEdit', requestedKind: 'textEdit', modal: true, text: RAW_MTEXT, multiline,
    });
    // The raw payload object never reaches the view; only its two fields do.
    assert.equal('payload' in view, false);
  }
});

test('the textEdit payload keeps the raw string and the empty string is real data', () => {
  const payload = textEditPayload({ text: '', multiline: false, extra: 'ignored' });
  assert.deepEqual(payload, { text: '', multiline: false });
  assert.ok(Object.isFrozen(payload));
  // An envelope field the dialog does not use is simply not copied.
  assert.equal('extra' in payload, false);
  assert.equal(textEditPayload({ text: '  spaced  ', multiline: true }).text, '  spaced  ');
});

test('a textEdit payload that is not exactly {text: string, multiline: boolean} is the normal shell', () => {
  const rejected = [
    // text
    { multiline: true },
    { text: null, multiline: true },
    { text: 7, multiline: true },
    { text: ['a'], multiline: true },
    { text: { value: 'a' }, multiline: true },
    // multiline
    { text: 'a' },
    { text: 'a', multiline: 'true' },
    { text: 'a', multiline: 1 },
    { text: 'a', multiline: null },
    { text: 'a', multiline: {} },
  ];
  for (const payload of rejected) {
    const scope = textEditLiteral(payload);
    // The envelope is still readable — the literal itself is well formed…
    assert.equal(readPopupBoot(scope)?.kind, 'textEdit', JSON.stringify(payload));
    // …but the strict boot switch must not half-open an editor.
    assert.equal(popupBoot(scope), null, JSON.stringify(payload));
    // And the view degrades to the explicit unknown state, never to an empty
    // editor with invented content.
    assert.deepEqual(popupView(readPopupBoot(scope)), { kind: 'unknown', requestedKind: 'textEdit' });
  }
});

test('the textEdit boot rejection is per kind: an about literal keeps booting', () => {
  const about = { [POPUP_GLOBAL]: { kind: 'about', modal: true, label: 'popup-1', payload: { version: '1', build: 'release' } } };
  assert.equal(popupBoot(about)?.kind, 'about');
  // A textEdit envelope missing `multiline` never replaces the shell, even when
  // the rest of the envelope is perfect.
  assert.equal(popupBoot(textEditLiteral({ text: RAW_MTEXT }, { label: 'popup-2' })), null);
});

test('the draft is the host string verbatim and nothing converts a line break', async () => {
  const view = popupView(popupBoot(textEditLiteral()));
  assert.equal(view.text, RAW_MTEXT);
  // The live draft is the same string: no trim, no markup parsing, and the
  // `\P` the file holds is still two characters.
  assert.equal(textEditDraftView(view.text, true).value, RAW_MTEXT);
  assert.ok(textEditDraftView(view.text, true).value.includes('\\P'));

  const dialog = await readModule('components/TextEditDialog.jsx');
  const module = await readModule('popup-textedit.mjs');
  // Comments may *name* the codes; code must not touch them.
  const code = source => source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
  for (const [file, source] of [['TextEditDialog.jsx', dialog], ['popup-textedit.mjs', module]]) {
    const body = code(source);
    // No conversion in either direction.
    assert.equal(/\\P/.test(body), false, `${file} must not rewrite \\P`);
    assert.equal(/replace\(\s*\/\\r?\\n/.test(body), false, `${file} must not map newlines`);
  }
  // The field is seeded from the payload and is controlled by the draft only.
  assert.ok(dialog.includes('useState(() => (typeof view.text === \'string\' ? view.text : \'\'))'));
  assert.ok(dialog.includes('value={draft}'));
  // No truncation: a bound in the input would silently cut the raw string.
  assert.equal(dialog.includes('maxLength'), false);
});

test('Enter is a line break for multiline and an apply for single line', () => {
  // multiline: a plain Enter is data (the textarea inserts it); the submit is
  // the explicit chord.
  assert.equal(textEditKeyAction({ key: 'Enter' }, true), 'none');
  assert.equal(textEditKeyAction({ key: 'Enter', shiftKey: true }, true), 'none');
  assert.equal(textEditKeyAction({ key: 'Enter', ctrlKey: true }, true), 'submit');
  assert.equal(textEditKeyAction({ key: 'Enter', metaKey: true }, true), 'submit');
  // single line: Enter applies, exactly like the command line. A break is never
  // offered, so Shift+Enter is swallowed and Alt+Enter too.
  assert.equal(textEditKeyAction({ key: 'Enter' }, false), 'submit');
  assert.equal(textEditKeyAction({ key: 'Enter', ctrlKey: true }, false), 'submit');
  assert.equal(textEditKeyAction({ key: 'Enter', metaKey: true }, false), 'submit');
  assert.equal(textEditKeyAction({ key: 'Enter', shiftKey: true }, false), 'block');
  assert.equal(textEditKeyAction({ key: 'Enter', altKey: true }, false), 'block');
  // Everything else belongs to the field.
  for (const key of ['a', ' ', 'Escape', 'Tab', 'ArrowDown', 'Backspace']) {
    for (const multiline of [true, false]) {
      assert.equal(textEditKeyAction({ key }, multiline), 'none', `${key} multiline=${multiline}`);
    }
  }
  assert.equal(textEditKeyAction(), 'none');
  assert.equal(textEditKeyAction(null, true), 'none');
});

test('an IME composition never submits, in either mode', () => {
  for (const multiline of [true, false]) {
    // The Enter that commits a composition carries `isComposing`.
    assert.equal(textEditKeyAction({ key: 'Enter', isComposing: true }, multiline), 'none');
    assert.equal(textEditKeyAction({ key: 'Enter', isComposing: true, ctrlKey: true }, multiline), 'none');
    // Legacy IMEs report it as keyCode 229 without `isComposing`.
    assert.equal(textEditKeyAction({ key: 'Enter', keyCode: 229 }, multiline), 'none');
    // A held Enter is one action, never a burst.
    assert.equal(textEditKeyAction({ key: 'Enter', repeat: true }, multiline), 'none');
    assert.equal(textEditKeyAction({ key: 'Enter', repeat: true, ctrlKey: true }, multiline), 'none');
  }
});

test('the count is Unicode scalar values, and the 1024 bound refuses without truncating', () => {
  // 1024 astral characters are 1024 scalars but 2048 UTF-16 code units.
  const atBound = '\u{1F600}'.repeat(POPUP_TEXT_MAX);
  const overBound = `${atBound}\u{1F600}`;
  assert.equal(POPUP_TEXT_MAX, 1024);
  assert.equal(atBound.length, 2048);
  assert.equal(textEditDraftView(atBound, true).length, 1024);
  assert.equal(textEditDraftView(atBound, true).tooLong, false);
  assert.equal(textEditDraftView(atBound, true).canApply, true);

  const over = textEditDraftView(overBound, true);
  assert.equal(over.length, 1025);
  assert.equal(over.tooLong, true);
  assert.equal(over.canApply, false);
  // The value is never truncated to fit: what the user typed is what stays.
  assert.equal(over.value, overBound);

  // A single-line draft carrying a break cannot be sent, and is kept as typed.
  const broken = textEditDraftView('one\ntwo', false);
  assert.equal(broken.singleLineBreak, true);
  assert.equal(broken.canApply, false);
  assert.equal(broken.value, 'one\ntwo');
  // The same string is a legal multi-line draft.
  assert.equal(textEditDraftView('one\ntwo', true).canApply, true);
});

test('an empty or whitespace-only draft is refused with the catalog line and kept', () => {
  for (const draft of ['', '   ', '\n\t ', '\r\n']) {
    const view = textEditDraftView(draft, true);
    assert.equal(view.empty, true, JSON.stringify(draft));
    assert.equal(view.canApply, false);
    assert.equal(view.value, draft, 'the draft is never cleared or trimmed');
    assert.equal(textEditLocalRefusal(view, T_ZH), T_ZH(POPUP_TEXT_KEYS.empty));
    assert.equal(textEditLocalRefusal(view, T_EN), T_EN(POPUP_TEXT_KEYS.empty));
  }
  // A sendable draft has no refusal at all.
  assert.equal(textEditLocalRefusal(textEditDraftView('ok', true), T_EN), null);
  // Over the bound the refusal names the real bound from the catalog.
  const tooLong = textEditDraftView('a'.repeat(POPUP_TEXT_MAX + 1), true);
  assert.equal(textEditLocalRefusal(tooLong, T_EN), T_EN(POPUP_TEXT_KEYS.tooLong, { max: POPUP_TEXT_MAX }));
  assert.ok(textEditLocalRefusal(tooLong, T_EN).includes(String(POPUP_TEXT_MAX)));
  // A single-line break is explained by the single-line rule itself.
  const broken = textEditDraftView('one\ntwo', false);
  assert.equal(textEditLocalRefusal(broken, T_EN), T_EN(POPUP_TEXT_KEYS.hintSingle));
});

test('a refused apply keeps the draft and shows why; only success closes', () => {
  const sent = 'Ground floor';
  const draft = textEditDraftView(sent, false);
  assert.equal(draft.value, sent);

  // Success: the one outcome that closes the window.
  const done = textEditApplyOutcome({ kind: 'accepted' }, T_EN);
  assert.deepEqual(done, { done: true, message: null });

  // Host refusal, named: the message is the shared "the drawing refused" line
  // with the host's own reason in it, and nothing is discarded.
  const refused = textEditApplyOutcome({ kind: 'failed', error: 'textedit.newline_text' }, T_EN);
  assert.equal(refused.done, false);
  assert.ok(refused.message.includes('textedit.newline_text'), refused.message);
  assert.equal(refused.message, T_EN(POPUP_TEXT_KEYS.refused, { error: 'textedit.newline_text' }));

  // Local refusals surface the same copy as the field's own guard.
  assert.equal(textEditApplyOutcome({ kind: 'refused', reason: 'empty' }, T_EN).message, T_EN(POPUP_TEXT_KEYS.empty));
  assert.equal(
    textEditApplyOutcome({ kind: 'refused', reason: 'too_long' }, T_EN).message,
    T_EN(POPUP_TEXT_KEYS.tooLong, { max: POPUP_TEXT_MAX }),
  );
  // A missing answer is an explicit failure, never a silent success.
  assert.equal(textEditApplyOutcome(null, T_EN).done, false);
  assert.ok(textEditApplyOutcome(undefined, T_ZH).message.length > 0);
});

test('a second apply while one is in flight reports busy instead of sending twice', async () => {
  const calls = [];
  let release;
  const gate = new Promise(resolve => { release = resolve; });
  const submitter = createTextEditSubmitter(async text => {
    calls.push(text);
    await gate;
    return { kind: 'accepted' };
  });
  const first = submitter.submit('one');
  assert.equal(submitter.isBusy(), true);
  const second = await submitter.submit('one');
  assert.deepEqual(second, { kind: 'busy' });
  assert.equal(calls.length, 1, 'the same text must not be sent twice');
  release();
  assert.deepEqual(await first, { kind: 'accepted' });
  assert.equal(submitter.isBusy(), false);
  // Once idle, the next apply is sent normally.
  assert.deepEqual(await submitter.submit('two'), { kind: 'accepted' });
  assert.deepEqual(calls, ['one', 'two']);
  // Busy is reported with the app's existing "another operation is in progress"
  // line, not with words invented in the component.
  assert.equal(textEditApplyOutcome({ kind: 'busy' }, T_EN).message, T_EN(POPUP_TEXT_KEYS.busy));
  assert.ok(T_EN(POPUP_TEXT_KEYS.busy).trim().length > 0);
});

test('the dialog closes only on success, and a failed close never traps the user', async () => {
  const dialog = await readModule('components/TextEditDialog.jsx');
  // One Apply path, one submitter, one outcome interpreter.
  assert.ok(dialog.includes('createTextEditSubmitter(text => setSelectedText(invoke, text))'));
  assert.ok(dialog.includes('if (submitter.isBusy()) {'));
  assert.ok(dialog.includes('setNotice(t(POPUP_TEXT_KEYS.busy));'));
  assert.ok(dialog.includes('const outcome = textEditApplyOutcome(result, t);'));
  assert.ok(dialog.includes('if (outcome.done) {'));
  const doneAt = dialog.indexOf('if (outcome.done) {');
  const closeAt = dialog.indexOf('onClose();', doneAt);
  assert.ok(doneAt !== -1 && closeAt > doneAt, 'onClose is called from the success branch');
  // The refusal branch sets the notice and nothing else: no draft reset, no
  // close, no truncation. Both are pinned globally: `setDraft` has exactly one
  // call site (typing) and `onClose()` exactly one (success).
  assert.ok(dialog.includes('setNotice(outcome.message);'));
  assert.equal((dialog.match(/setDraft\(/g) ?? []).length, 1, 'only typing touches the draft');
  assert.equal((dialog.match(/onClose\(\)/g) ?? []).length, 1, 'only a successful apply closes');
  // Esc and Close both go through the root's idempotent close; a failure is
  // shown in the footer and both buttons stay live for a retry.
  assert.ok(dialog.includes('label={t(POPUP_TEXT_KEYS.close)} onClick={onClose}'));
  assert.ok(dialog.includes('<Button onClick={onClose}>{t(POPUP_TEXT_KEYS.close)}</Button>'));
  assert.ok(dialog.includes('const failure = notice ?? closeFailure ?? null;'));
  assert.ok(dialog.includes('role="alert"'));
  const root = await readModule('components/PopupRoot.jsx');
  assert.ok(root.includes('<TextEditDialog view={view} onClose={requestClose} closeFailure={failure} />'));
  // Escape stays the only global key, and it never closes the window while an
  // IME composition is in progress (that press belongs to the IME).
  assert.ok(root.includes("if (event.isComposing || event.keyCode === 229) return;"));
  assert.ok(dialog.includes('title={failure}'), 'a truncated host error stays readable on hover');
  // The keyboard path is the same `apply`, and a blocked key inserts nothing.
  assert.ok(dialog.includes('const action = textEditKeyAction(event, multiline);'));
  assert.ok(dialog.includes("if (action === 'submit') apply();"));
  // The local refusal runs before IPC, so an unsendable draft never leaves.
  const localAt = dialog.indexOf('const local = textEditLocalRefusal(current, t);');
  const busyAt = dialog.indexOf('if (submitter.isBusy()) {');
  const submitAt = dialog.indexOf('submitter.submit(draftRef.current)');
  assert.ok(localAt !== -1 && busyAt > localAt && submitAt > busyAt, 'local refusal, then busy, then IPC');
});

// --- Dialog chrome after the 640×480 pixel review -------------------------
// The reviewed frame showed an unfocused-looking field, an 11px hint at ~4.0:1
// (its anti-aliased core), 9px of air above the title against ~20px below it,
// and the field's text 9px to the right of the hint's edge. These pin the fixes
// as data, so they cannot drift out of the build silently.

test('the field shows its focus, and the ring costs no layout', async () => {
  // The ring is the focus token, drawn inside the border box (no shift, no
  // clipping at the body edge) and it replaces the shared keyboard-only outline
  // instead of doubling it.
  assert.equal(TEXT_FIELD_FOCUS.borderColor, 'var(--focus)');
  assert.equal(TEXT_FIELD_FOCUS.boxShadow, 'inset 0 0 0 1px var(--focus)');
  assert.equal(TEXT_FIELD_FOCUS.outline, 'none');
  // Focused is the field plus the ring: every metric (padding, border width,
  // radius, font) is inherited, so focusing the field moves nothing.
  assert.deepEqual({ ...TEXT_FIELD_FOCUSED }, { ...TEXT_FIELD, ...TEXT_FIELD_FOCUS });
  const dialog = await readModule('components/TextEditDialog.jsx');
  assert.ok(dialog.includes('style={focused ? TEXT_FIELD_FOCUSED : TEXT_FIELD}'));
  assert.ok(dialog.includes('onFocus={() => setFocused(true)}'));
  assert.ok(dialog.includes('onBlur={() => setFocused(false)}'));
  // The field really is focused on boot, and the caret is after the text — the
  // reviewed frame caught it neither focused nor with a caret at the end.
  assert.ok(dialog.includes('field.focus();'));
  assert.ok(dialog.includes('field.setSelectionRange(end, end);'));
});

test('the header centres the title instead of stacking the body padding under it', async () => {
  // 42px header with a 13px title: ~14.5px of air above and below the ink. The
  // body's top padding is 0, so the 42px above the field is all header — the
  // reviewed frame had 9px above and ~20px below (30px header + 12px padding).
  assert.equal(TEXT_HEAD.height, 42);
  assert.equal(TEXT_HEAD.padding, '0 12px 0 14px');
  assert.equal(DIALOG_TEXT_BODY.padding, '0 16px 12px');
  const ink = Number.parseFloat(DIALOG_TITLE.fontSize);
  const airAbove = (TEXT_HEAD.height - ink) / 2;
  // No body padding below the header means the ink's bottom gap is the same:
  // `height - (airAbove + ink)` = `airAbove`.
  const airBelow = TEXT_HEAD.height - (airAbove + ink);
  assert.equal(airBelow, airAbove);
  assert.ok(airAbove >= 14 && airAbove <= 15, `air around the title: ${airAbove}px`);
  const dialog = await readModule('components/TextEditDialog.jsx');
  assert.ok(dialog.includes('style={TEXT_HEAD}'), 'the editor uses the balanced header');
  assert.equal(dialog.includes('DIALOG_HEAD'), false, 'no stale 30px header import');
});

test('the title keeps the About header’s 13px/600 step above the meta copy', async () => {
  // The pixel review asked this dialog to match About; both read the same
  // object, so they cannot drift. 13px semibold against 11px body copy is a two
  // pixel step and stays a title, not body text.
  assert.equal(DIALOG_TITLE.fontSize, '13px');
  assert.equal(DIALOG_TITLE.fontWeight, 600);
  assert.ok(Number.parseFloat(DIALOG_TITLE.fontSize) - 11 >= 2);
  for (const file of ['components/AboutDialog.jsx', 'components/TextEditDialog.jsx']) {
    const source = await readModule(file);
    assert.ok(source.includes('style={DIALOG_TITLE}'), `${file} must use the shared title style`);
  }
});

test('the hint and count clear 4.5:1 from the stronger secondary token', async () => {
  const css = await readCss();
  const token = name => {
    const match = css.match(new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6})`));
    assert.ok(match, `stylesheet must define --${name}`);
    return match[1].slice(1);
  };
  const luminance = hex => {
    const rgb = hex.match(/\w\w/g).map(pair => parseInt(pair, 16) / 255)
      .map(channel => channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4);
    return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
  };
  const contrast = (a, b) => {
    const values = [luminance(a), luminance(b)].sort((x, y) => y - x);
    return (values[0] + 0.05) / (values[1] + 0.05);
  };
  const surface = token('chrome-2');
  const dim = contrast(token('text-dim'), surface);
  // The reviewer measured the hint's blended core at ~4.0:1; the muted token is
  // 5.4:1 on paper but loses to 11px anti-aliasing. `--text-dim` is 6.9:1, which
  // leaves the same margin after blending.
  assert.ok(dim >= 6, `--text-dim on --chrome-2 = ${dim.toFixed(2)}`);
  assert.ok(dim > contrast(token('text-muted'), surface));
  const dialog = await readModule('components/TextEditDialog.jsx');
  assert.equal((dialog.match(/color: 'var\(--text-dim\)'/g) ?? []).length, 2, 'hint and count both use it');
  assert.equal(dialog.includes("color: 'var(--text-muted)'"), false, 'the 11px meta copy is not muted');
});

test('the meta row gives the corrected hint room on one line at 640px', async () => {
  // 640 − 2×16 = 608px of content. Measured at 11px: zh-CN hint 366px + 12px gap
  // + count 93px = 471px, one row. The English hint is 578px, so it stays on one
  // line by wrapping the count to a second, right-aligned line rather than
  // breaking mid-sentence; the row keeps a 3px row gap for that case.
  assert.equal(TEXT_META.flexWrap, 'wrap');
  assert.equal(TEXT_META.gap, '3px 12px');
  assert.equal(TEXT_META.justifyContent, 'space-between');
  const dialog = await readModule('components/TextEditDialog.jsx');
  assert.ok(dialog.includes("marginLeft: 'auto'"), 'a wrapped count keeps the right edge');
});

test('the field’s text sits within a few pixels of the hint’s edge', () => {
  // 16px body inset + 1px border + 4px padding = 21px box; the mono bearing puts
  // the ink at ~23px against the hint's 18px. It was 8px of padding (ink at 27).
  assert.equal(TEXT_FIELD.padding, '6px 4px');
});

test('the palette entry is offered only for a real selection text and asks for textEdit', async () => {
  // The predicate: a non-null string, and nothing else.
  const withText = { count: 1, empty: false, rows: [], mixedKeys: [], text: 'Lobby' };
  assert.equal(canOpenTextEditPopup(withText), true);
  assert.equal(canOpenTextEditPopup({ ...withText, text: '' }), true, 'an empty string is still a string');
  for (const selection of [
    null,
    undefined,
    {},
    { count: 0, empty: true, rows: [], mixedKeys: [] },
    { ...withText, text: null },
    { ...withText, text: 7 },
    { ...withText, text: {} },
    { ...withText, count: 2 },
    'Lobby',
  ]) {
    assert.equal(canOpenTextEditPopup(selection), false, JSON.stringify(selection));
  }

  const editor = await readModule('components/SelectionTextEditor.jsx');
  assert.ok(editor.includes('const canOpenInWindow = canOpenTextEditPopup(shell.status?.selection ?? null);'));
  assert.ok(editor.includes('disabled={!canOpenInWindow}'));
  assert.ok(editor.includes('onClick={() => shell.openTextEditPopup()}'));
  assert.ok(editor.includes('{t(POPUP_TEXT_KEYS.entry)}'));
  assert.equal(/[\u3400-\u9fff\uf900-\ufaff]/.test(editor), false, 'no hardcoded CJK');
  // The fast paths are untouched: the inline field still sends through the
  // shell, and TEXTEDIT still prefills the command line's text step.
  assert.ok(editor.includes('shell.applySelectedText(sending)'));

  const app = await readModule('App.jsx');
  assert.ok(app.includes("openPopup(invoke, 'textEdit')"));
  assert.ok(app.includes('openTextEditPopup: () => {'));
  assert.ok(app.includes('const failure = textEditOpenError(result, t);'));
  assert.ok(app.includes('if (failure) setError(failure);'), 'the refusal reaches the error banner');
  // The host refusal opens no window: the request is a single cad_open_popup.
  const invoke = recordingInvoke({ accepted: true, kind: 'textEdit', popup_label: 'popup-2', existing: false });
  assert.deepEqual(await openPopup(invoke.invoke, 'textEdit'), {
    kind: 'accepted', requestedKind: 'textEdit', popupLabel: 'popup-2', existing: false,
  });
  assert.deepEqual(invoke.calls, [{ command: POPUP_OPEN_COMMAND, args: { kind: 'textEdit' } }]);
});

test('a host refusal to open is shown as catalog copy, other failures verbatim', async () => {
  // The host names the refusal with a catalog key; the banner shows the line,
  // not the machine name.
  const named = { kind: 'failed', error: 'popup.no_text_selection' };
  assert.equal(textEditOpenError(named, T_EN), T_EN(POPUP_TEXT_KEYS.noSelection));
  // Also when the IPC wraps it in a message.
  assert.equal(
    textEditOpenError({ kind: 'failed', error: 'cad_open_popup: popup.no_text_selection' }, T_ZH),
    T_ZH(POPUP_TEXT_KEYS.noSelection),
  );
  // An unrelated failure is never swallowed nor translated into something else.
  assert.equal(textEditOpenError({ kind: 'failed', error: 'ipc down' }, T_EN), 'ipc down');
  // A malformed result still names itself instead of reporting nothing.
  assert.equal(textEditOpenError({ kind: 'failed' }, T_EN), 'failed');
  // Success (including "already open, focused") has nothing to report.
  assert.equal(textEditOpenError({ kind: 'accepted', existing: true }, T_EN), '');
  assert.equal(textEditOpenError(null, T_EN), '');
  // And the popup layer never refuses the kind itself any more.
  const untouched = recordingInvoke();
  const local = await openPopup(untouched.invoke, 'textEdit');
  assert.equal(local.kind, 'accepted', 'textEdit is an implemented kind');
});

test('the textEdit document starts none of the shell global effects', async () => {
  const shared = [
    'startCadPolling(',
    "'cad_status'",
    'bindBrowserDefaults(',
    'bindProbeInput(',
    'createInputLane(',
    'setCommandCatalog(',
    'probe_input',
    './cad.mjs',
    './defaults.mjs',
    './input.mjs',
    './commands.mjs',
    'useShell()',
  ];
  // The dialog adds no listener of its own: the window's one key (Escape) stays
  // in `PopupRoot`.
  const dialogOnly = [...shared, 'addEventListener'];
  for (const [file, forbiddenList] of [
    ['components/TextEditDialog.jsx', dialogOnly],
    ['components/PopupRoot.jsx', shared],
  ]) {
    const source = await readModule(file);
    for (const forbidden of forbiddenList) {
      assert.equal(source.includes(forbidden), false, `${file} must not start ${forbidden}`);
    }
  }
  // The dialog talks to the host through exactly one existing command.
  const dialog = await readModule('components/TextEditDialog.jsx');
  assert.equal(dialog.includes('setSelectedText(invoke, text)'), true);
  assert.equal(dialog.includes("invoke('cad_"), false, 'no hand-rolled IPC name');
  const root = await readModule('components/PopupRoot.jsx');
  // Escape remains the document's one global key, and it still closes.
  assert.ok(root.includes("event.key !== 'Escape'"));
  assert.ok(root.includes('closePopup(invoke)'));
});

test('every key the text-edit dialog uses is a declared contract key', async () => {
  // Pinned so a rename is a contract change, not a silent catalog miss.
  assert.deepEqual(POPUP_TEXT_KEYS, {
    title: 'dialog.textedit.title',
    apply: 'dialog.textedit.apply',
    close: 'dialog.textedit.close',
    count: 'dialog.textedit.count',
    hintMultiline: 'dialog.textedit.hint_multiline',
    hintSingle: 'dialog.textedit.hint_single',
    entry: 'properties.edit_in_window',
    noSelection: 'popup.no_text_selection',
    empty: 'properties.text.empty',
    tooLong: 'status.tool_text.too_long',
    refused: 'properties.text.failed',
    busy: 'linux.busy',
  });
  // Declared once, as data, and addressed through the registry. The dialog's own
  // copy is a literal here; the reused bound is the shared `TEXT_TOO_LONG_KEY`
  // constant, so the dialog and the text step cannot drift apart.
  const source = await readModule('popup-textedit.mjs');
  for (const key of NEW_KEYS) {
    assert.equal(source.includes(`'${key}'`), true, `popup-textedit.mjs must declare ${key}`);
  }
  assert.equal(POPUP_TEXT_KEYS.tooLong, 'status.tool_text.too_long');
  assert.equal(source.includes('TEXT_TOO_LONG_KEY, textLength'), true, 'the bound key is imported, not repeated');
  const shape = /^[a-z][a-z0-9_]*(\.[a-z0-9_]+)+$/;
  for (const key of Object.values(POPUP_TEXT_KEYS)) assert.match(key, shape, key);
});

// The keys the Rust lane is adding (the reused ones already resolve). A key that
// is neither in the catalog nor in this list is a typo, and fails below even
// while the catalogs are still landing.
const NEW_KEYS = [
  'dialog.textedit.title',
  'dialog.textedit.apply',
  'dialog.textedit.close',
  'dialog.textedit.count',
  'dialog.textedit.hint_multiline',
  'dialog.textedit.hint_single',
  'properties.edit_in_window',
  'popup.no_text_selection',
];
const USED_KEYS = [...new Set(Object.values(POPUP_TEXT_KEYS))];
const MISSING_KEYS = USED_KEYS.filter(key => typeof ZH[key] !== 'string' || typeof EN[key] !== 'string');

test('no key outside the contract is used, and every reused key resolves today', () => {
  for (const key of USED_KEYS) {
    if (NEW_KEYS.includes(key)) continue;
    assert.equal(typeof ZH[key], 'string', `zh-CN missing reused key ${key}`);
    assert.equal(typeof EN[key], 'string', `en missing reused key ${key}`);
  }
  // The dialog's own copy is exactly the eight contract keys — nothing invented.
  assert.deepEqual([...NEW_KEYS].sort(), [
    'dialog.textedit.apply',
    'dialog.textedit.close',
    'dialog.textedit.count',
    'dialog.textedit.hint_multiline',
    'dialog.textedit.hint_single',
    'dialog.textedit.title',
    'popup.no_text_selection',
    'properties.edit_in_window',
  ]);
  for (const key of USED_KEYS) {
    assert.ok(NEW_KEYS.includes(key) || typeof ZH[key] === 'string', `invented key used: ${key}`);
  }
});

test('every text-edit dialog catalog key resolves in both locales with matching placeholders',
  { skip: MISSING_KEYS.length > 0 && `catalog keys not landed by the backend lane yet: ${MISSING_KEYS.join(', ')}` },
  () => {
    const placeholder = /(?<!\\)\{([A-Za-z_][A-Za-z0-9_]*)\}/g;
    for (const key of USED_KEYS) {
      assert.equal(typeof ZH[key], 'string', `zh-CN missing ${key}`);
      assert.equal(typeof EN[key], 'string', `en missing ${key}`);
      assert.ok(ZH[key].trim().length > 0, `empty zh-CN ${key}`);
      assert.ok(EN[key].trim().length > 0, `empty en ${key}`);
      const a = [...ZH[key].matchAll(placeholder)].map(match => match[1]).sort();
      const b = [...EN[key].matchAll(placeholder)].map(match => match[1]).sort();
      assert.deepEqual(a, b, `placeholders differ for ${key}`);
      // A concrete key must resolve, not fall back to the key itself.
      assert.notEqual(T_ZH(key), key, `zh-CN does not resolve ${key}`);
      assert.notEqual(T_EN(key), key, `en does not resolve ${key}`);
    }
    // The count template carries both placeholders and renders them.
    for (const template of [ZH[POPUP_TEXT_KEYS.count], EN[POPUP_TEXT_KEYS.count]]) {
      for (const name of ['{used}', '{max}']) assert.ok(template.includes(name), `${template} missing ${name}`);
    }
    const filled = T_EN(POPUP_TEXT_KEYS.count, { used: 3, max: 1024 });
    assert.ok(filled.includes('3') && filled.includes('1024'), filled);
    assert.equal(/\{[A-Za-z_]/.test(filled), false, filled);
  });
