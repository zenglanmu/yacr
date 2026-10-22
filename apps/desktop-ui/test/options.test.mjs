// Options dialog contract: the control table, the client-side mirror of the
// host's setting rules, the staged diff, the two IPCs and every entry point.
//
// The tests are deliberately table-driven: the field table is the one place that
// says which controls exist, so a control that gains or loses its backend key
// (or changes its write key) fails here rather than in a window nobody opened.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createTranslator } from '../src/i18n.mjs';
import { POPUP_KINDS, popupBoot, popupView, readPopupBoot } from '../src/popup.mjs';
import {
  LOCALE_CHOICES,
  OPTIONS_FIELDS,
  OPTIONS_KEYS,
  OPTIONS_TABS,
  PLOT_FORMATS,
  SETTINGS_ERROR,
  SETTINGS_READ_COMMAND,
  SETTINGS_WAITING,
  SETTINGS_WRITE_COMMAND,
  applyPayload,
  dirtyFields,
  draftFromSettings,
  entryVisible,
  fieldById,
  fieldsForTab,
  isOptionsCommand,
  languageChanged,
  normalizeValue,
  optionsErrorView,
  optionsTabs,
  parseIntegerText,
  readSettings,
  settleSettings,
  settingErrorCode,
  tabById,
  validateDraft,
  validateField,
  writeAll,
  writeSetting,
} from '../src/options.mjs';
import { readCatalog, readModule } from './helpers.mjs';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const tEn = createTranslator({ 'zh-CN': zh, en }, 'en');

/** A complete, valid snapshot, in the exact shape `cad_settings` returns. */
function snapshot(overrides = {}) {
  return {
    backgroundModel: '#1b1d21',
    backgroundPaper: '#f4f2ed',
    crosshairPercent: 5,
    grid: true,
    axes: true,
    selectionHighlight: true,
    snapHints: true,
    snapEnabled: true,
    hoverPreview: true,
    pickboxSize: 6,
    apertureSize: 12,
    plotFormat: 'svg',
    fontsDir: '/usr/share/fonts',
    ...overrides,
  };
}

/** Record every IPC the dialog makes, in order. */
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

// --- the control table -----------------------------------------------------

test('the tabs are AutoCAD order and each one owns at least one real control', () => {
  assert.deepEqual(OPTIONS_TABS.map(tab => tab.id), ['display', 'drafting', 'plot', 'files', 'user']);
  assert.deepEqual(tabById('drafting').labelKey, 'options.tab.drafting');
  assert.equal(tabById('nope'), null);
  // Every declared tab has a field, so nothing is rendered as an empty page;
  // a tab that lost its last control would be dropped by `optionsTabs()`.
  const rendered = optionsTabs();
  assert.deepEqual(rendered.map(tab => tab.id), ['display', 'drafting', 'plot', 'files', 'user']);
  for (const tab of rendered) {
    assert.ok(fieldsForTab(tab.id).length > 0, `${tab.id} would be empty`);
  }
  assert.deepEqual(fieldsForTab('no-such-tab'), []);
});

test('every field sits on a declared tab exactly once and nothing else exists', () => {
  const ids = OPTIONS_FIELDS.map(field => field.id);
  assert.equal(new Set(ids).size, ids.length, 'a field id is declared twice');
  for (const field of OPTIONS_FIELDS) {
    assert.ok(tabById(field.tab), `${field.id} points at an undeclared tab`);
    assert.equal(fieldById(field.id), field);
    assert.ok(field.labelKey.startsWith('options.field.'), field.id);
    assert.ok(field.helpKey.startsWith('options.help.'), field.id);
  }
  assert.equal(fieldById('nope'), null);
  assert.deepEqual(fieldsForTab('display').map(field => field.id), [
    'backgroundModel', 'backgroundPaper', 'crosshairSize', 'grid', 'axes', 'selectionHighlight', 'snapHints',
  ]);
  assert.deepEqual(fieldsForTab('drafting').map(field => field.id), [
    'snapEnabled', 'hoverPreview', 'pickboxSize', 'apertureSize',
  ]);
  assert.deepEqual(fieldsForTab('plot').map(field => field.id), ['plotFormat']);
  assert.deepEqual(fieldsForTab('files').map(field => field.id), ['fontsDir']);
  assert.deepEqual(fieldsForTab('user').map(field => field.id), ['language']);
});

test('each control is the editor its backend value really needs', () => {
  const kinds = Object.fromEntries(OPTIONS_FIELDS.map(field => [field.id, field.kind]));
  assert.deepEqual(kinds, {
    backgroundModel: 'color',
    backgroundPaper: 'color',
    crosshairSize: 'number',
    grid: 'bool',
    axes: 'bool',
    selectionHighlight: 'bool',
    snapHints: 'bool',
    snapEnabled: 'bool',
    hoverPreview: 'bool',
    pickboxSize: 'number',
    apertureSize: 'number',
    plotFormat: 'select',
    fontsDir: 'readonly',
    language: 'locale',
  });
  // Number controls carry the host's own range and a unit suffix, never a clamp.
  assert.deepEqual(
    OPTIONS_FIELDS.filter(field => field.kind === 'number').map(field => [field.id, field.min, field.max]),
    [['crosshairSize', 1, 100], ['pickboxSize', 1, 20], ['apertureSize', 1, 50]],
  );
  for (const field of OPTIONS_FIELDS.filter(entry => entry.kind === 'number')) {
    assert.ok(field.unitKey, `${field.id} needs a unit`);
  }
  // The plot selector offers exactly the two formats the host can produce.
  assert.deepEqual(fieldById('plotFormat').choices, ['svg', 'pdf']);
  assert.deepEqual(PLOT_FORMATS, ['svg', 'pdf']);
  assert.deepEqual(LOCALE_CHOICES, ['zh-CN', 'en']);
});

test('the wire field and the write key differ only where the host says so', () => {
  // Read from `cad_settings`, written through `cad_set_setting`.
  const differing = OPTIONS_FIELDS.filter(field => field.wire && field.key && field.wire !== field.key);
  assert.deepEqual(differing.map(field => [field.id, field.wire, field.key]), [
    ['crosshairSize', 'crosshairPercent', 'crosshairSize'],
  ]);
  // The two non-settings controls carry no write key at all.
  assert.deepEqual(
    OPTIONS_FIELDS.filter(field => !field.key).map(field => field.id),
    ['fontsDir', 'language'],
  );
});

// --- validation (the client-side mirror of crates/cad-host/src/settings.rs) --

test('integers are parsed strictly and never silently coerced', () => {
  assert.equal(parseIntegerText('40'), 40);
  assert.equal(parseIntegerText(' 40 '), 40);
  assert.equal(parseIntegerText(7), 7);
  for (const bad of ['', ' ', '4 0', '40.0', '-1', '+1', '4e1', '0x10', null, undefined, {}, [], 1.5, NaN]) {
    assert.equal(parseIntegerText(bad), null, `${String(bad)} must not parse`);
  }
});

test('the validation table matches the host ranges and codes', () => {
  const cases = [
    // [field, value, expected]
    ['crosshairSize', '1', null],
    ['crosshairSize', '40', null],
    ['crosshairSize', '100', null],
    ['crosshairSize', '0', SETTINGS_ERROR.invalidValue],
    ['crosshairSize', '101', SETTINGS_ERROR.invalidValue],
    ['crosshairSize', '', SETTINGS_ERROR.invalidValue],
    ['crosshairSize', '4.5', SETTINGS_ERROR.invalidValue],
    ['pickboxSize', '1', null],
    ['pickboxSize', '20', null],
    ['pickboxSize', '21', SETTINGS_ERROR.invalidValue],
    ['apertureSize', '50', null],
    ['apertureSize', '51', SETTINGS_ERROR.invalidValue],
    ['backgroundModel', '#abc', SETTINGS_ERROR.invalidValue],
    ['backgroundModel', '#aabb', SETTINGS_ERROR.invalidValue],
    ['backgroundModel', '#aabbcc', null],
    ['backgroundModel', '#AABBCC', null],
    ['backgroundModel', ' #aabbcc ', null],
    ['backgroundModel', '#aabbccdd', SETTINGS_ERROR.invalidValue],
    ['backgroundModel', 'red', SETTINGS_ERROR.invalidValue],
    ['backgroundPaper', '0x112233', SETTINGS_ERROR.invalidValue],
    ['grid', true, null],
    ['grid', false, null],
    ['grid', 'true', SETTINGS_ERROR.invalidValue],
    ['grid', 1, SETTINGS_ERROR.invalidValue],
    ['snapEnabled', true, null],
    ['hoverPreview', false, null],
    ['plotFormat', 'svg', null],
    ['plotFormat', 'pdf', null],
    ['plotFormat', 'png', SETTINGS_ERROR.invalidPlotFormat],
    ['plotFormat', 'SVG', SETTINGS_ERROR.invalidPlotFormat],
    ['language', 'zh-CN', null],
    ['language', 'en', null],
    ['language', 'fr', SETTINGS_ERROR.invalidValue],
    ['fontsDir', '/usr/share/fonts', null],
    ['nope', '1', SETTINGS_ERROR.unknownKey],
  ];
  for (const [id, value, expected] of cases) {
    assert.equal(validateField(id, value), expected, `${id} = ${JSON.stringify(value)}`);
  }
});

test('validateDraft reports every bad value and skips what is not sent', () => {
  const draft = {
    crosshairSize: '101',
    pickboxSize: '6',
    backgroundModel: '#1b1d21',
    plotFormat: 'png',
    grid: null, // not reported by this host: nothing to validate
    fontsDir: '/usr/share/fonts', // read-only
    language: 'fr', // not a settings key, but still must be a real locale
  };
  assert.deepEqual(validateDraft(draft), {
    crosshairSize: SETTINGS_ERROR.invalidValue,
    plotFormat: SETTINGS_ERROR.invalidPlotFormat,
  });
  assert.deepEqual(validateDraft(null), {});
  // A clean draft has no errors at all.
  assert.deepEqual(validateDraft(draftFromSettings(snapshot(), 'en')), {});
});

// --- the staged draft ------------------------------------------------------

test('the draft is seeded from the host snapshot, never from local defaults', () => {
  const draft = draftFromSettings(snapshot(), 'zh-CN');
  assert.equal(draft.crosshairSize, '5');
  assert.equal(draft.pickboxSize, '6');
  assert.equal(draft.apertureSize, '12');
  assert.equal(draft.grid, true);
  assert.equal(draft.plotFormat, 'svg');
  assert.equal(draft.fontsDir, '/usr/share/fonts');
  assert.equal(draft.language, 'zh-CN');
  // Colours are canonicalised to lower case for comparison and display.
  assert.equal(draftFromSettings(snapshot({ backgroundModel: '#AABBCC' })).backgroundModel, '#aabbcc');

  // An older/partial host reports less: those controls stay explicitly absent.
  const empty = draftFromSettings({}, null);
  assert.equal(empty.crosshairSize, null);
  assert.equal(empty.grid, null);
  assert.equal(empty.plotFormat, null);
  assert.equal(empty.fontsDir, null);
  assert.equal(empty.language, null);
  // A malformed value is not repaired into something it never was.
  const malformed = draftFromSettings({
    grid: 'yes', crosshairPercent: -3, backgroundModel: 'grey', plotFormat: 'dxf', fontsDir: 7,
  });
  assert.equal(malformed.grid, null);
  // A real number outside the range is kept as reported and refused by the
  // validator, not silently clamped into a "valid" value.
  assert.equal(malformed.crosshairSize, '-3');
  assert.equal(validateField('crosshairSize', malformed.crosshairSize), SETTINGS_ERROR.invalidValue);
  assert.equal(malformed.backgroundModel, null);
  assert.equal(malformed.plotFormat, null);
  assert.equal(malformed.fontsDir, null);
});

test('an unedited draft is clean in both value shapes the wire can use', () => {
  const wire = snapshot();
  assert.deepEqual(dirtyFields(draftFromSettings(wire, 'en'), wire, 'en'), []);
  // Integer-vs-text and colour case are canonicalised, not "changes".
  const draft = draftFromSettings(snapshot({ backgroundModel: '#AABBCC', pickboxSize: 6 }), 'en');
  assert.deepEqual(dirtyFields(draft, snapshot({ backgroundModel: '#aabbcc' }), 'en'), []);
  assert.deepEqual(applyPayload(draft, snapshot({ backgroundModel: '#aabbcc' })), []);
});

test('the dirty list is table-ordered and compares against the right baseline', () => {
  const wire = snapshot();
  const draft = { ...draftFromSettings(wire, 'en'), plotFormat: 'pdf', grid: false, pickboxSize: '8' };
  assert.deepEqual(dirtyFields(draft, wire, 'en'), ['grid', 'pickboxSize', 'plotFormat']);
  // The language is compared against the running locale, not the wire.
  assert.deepEqual(dirtyFields({ ...draft, language: 'zh-CN' }, wire, 'en'), [
    'grid', 'pickboxSize', 'plotFormat', 'language',
  ]);
  // A value that is still being typed (or unreported) is never "dirty".
  const typing = { ...draftFromSettings(wire, 'en'), crosshairSize: '', apertureSize: null };
  assert.deepEqual(dirtyFields(typing, wire, 'en'), []);
  assert.equal(languageChanged({ language: 'zh-CN' }, 'en'), true);
  assert.equal(languageChanged({ language: 'en' }, 'en'), false);
  assert.equal(languageChanged({ language: 'fr' }, 'en'), false);
});

test('the apply payload carries write keys, typed values and nothing else', () => {
  const wire = snapshot();
  const draft = {
    ...draftFromSettings(wire, 'en'),
    crosshairSize: '40',
    backgroundModel: '#AABBCC',
    grid: false,
    plotFormat: 'pdf',
    comfort: 'ignored',
  };
  assert.deepEqual(applyPayload(draft, wire), [
    { field: 'backgroundModel', key: 'backgroundModel', value: '#aabbcc' },
    // The wire field is `crosshairPercent`; the write key is `crosshairSize`.
    { field: 'crosshairSize', key: 'crosshairSize', value: 40 },
    { field: 'grid', key: 'grid', value: false },
    { field: 'plotFormat', key: 'plotFormat', value: 'pdf' },
  ]);
  // The read-only fonts folder and the UI language are never written.
  const onlyLocal = { ...draftFromSettings(wire, 'en'), fontsDir: '/tmp/fonts', language: 'zh-CN' };
  assert.deepEqual(applyPayload(onlyLocal, wire), []);
});

test('normalizeValue is the single canonical form used by diff and settle', () => {
  assert.equal(normalizeValue('crosshairSize', ' 7 '), 7);
  assert.equal(normalizeValue('grid', true), true);
  assert.equal(normalizeValue('grid', 'true'), null);
  assert.equal(normalizeValue('backgroundModel', '#AABBCC'), '#aabbcc');
  assert.equal(normalizeValue('plotFormat', 'pdf'), 'pdf');
  assert.equal(normalizeValue('plotFormat', 'png'), null);
  assert.equal(normalizeValue('language', 'en'), 'en');
  assert.equal(normalizeValue('fontsDir', '/x'), '/x');
  assert.equal(normalizeValue('nope', 'x'), null);
});

// --- the two IPCs ----------------------------------------------------------

test('reading the snapshot separates "no session yet" from a real failure', async () => {
  assert.equal(SETTINGS_READ_COMMAND, 'cad_settings');
  assert.equal(SETTINGS_WRITE_COMMAND, 'cad_set_setting');
  assert.equal(SETTINGS_WAITING, 'cad_waiting');

  const ok = recordingInvoke({ grid: true });
  assert.deepEqual(await readSettings(ok.invoke), { kind: 'ok', settings: { grid: true } });
  assert.deepEqual(ok.calls, [{ command: 'cad_settings' }]);

  // Tauri rejects with a plain string, which is what the model reads.
  const waiting = recordingInvoke(() => { throw 'cad_waiting: no session is open yet'; });
  assert.deepEqual(await readSettings(waiting.invoke), { kind: 'waiting' });

  const failed = recordingInvoke(() => { throw 'cad_unavailable: the session is closed'; });
  assert.deepEqual(await readSettings(failed.invoke), {
    kind: 'failed', error: 'cad_unavailable: the session is closed',
  });

  // A non-object answer is a failure, never an empty snapshot full of defaults.
  for (const payload of [null, undefined, [], 'nope', 7]) {
    const wrong = recordingInvoke(payload);
    assert.deepEqual(await readSettings(wrong.invoke), { kind: 'failed', error: 'SETTINGS_INVALID_PAYLOAD' });
  }
});

test('a write is only accepted when the host says so, and refusals keep their code', async () => {
  const accepted = recordingInvoke({ accepted: true });
  assert.deepEqual(await writeSetting(accepted.invoke, 'grid', false), { kind: 'accepted' });
  assert.deepEqual(accepted.calls, [{ command: 'cad_set_setting', args: { key: 'grid', value: false } }]);

  // Anything other than `accepted: true` is not a success.
  for (const response of [{}, { accepted: false }, { accepted: 'true' }, null, undefined]) {
    const wrong = recordingInvoke(response);
    assert.deepEqual(await writeSetting(wrong.invoke, 'grid', false), {
      kind: 'failed', code: SETTINGS_ERROR.invalidValue,
    });
  }

  const refused = recordingInvoke(() => { throw 'settings.invalid_value: grid expects a boolean'; });
  assert.deepEqual(await writeSetting(refused.invoke, 'grid', 'yes'), {
    kind: 'failed', code: 'settings.invalid_value',
  });
  assert.equal(settingErrorCode('settings.invalid_plot_format: expected svg or pdf'), 'settings.invalid_plot_format');
  assert.equal(settingErrorCode('settings.unknown_key: "comfort"'), 'settings.unknown_key');
  assert.equal(settingErrorCode('cad_unavailable: closed'), 'cad_unavailable: closed');
});

test('writeAll is sequential and stops at the first refusal, naming the field', async () => {
  const entries = [
    { field: 'crosshairSize', key: 'crosshairSize', value: 40 },
    { field: 'plotFormat', key: 'plotFormat', value: 'pdf' },
    { field: 'grid', key: 'grid', value: false },
  ];
  const grantAll = recordingInvoke({ accepted: true });
  assert.deepEqual(await writeAll(grantAll.invoke, entries), { kind: 'accepted' });
  assert.deepEqual(grantAll.calls.map(call => call.args.key), ['crosshairSize', 'plotFormat', 'grid']);

  const refuseSecond = recordingInvoke((command, args) => {
    if (args.key === 'plotFormat') throw 'settings.invalid_plot_format: no dxf';
    return { accepted: true };
  });
  assert.deepEqual(await writeAll(refuseSecond.invoke, entries), {
    kind: 'failed', field: 'plotFormat', key: 'plotFormat', code: 'settings.invalid_plot_format',
  });
  assert.deepEqual(refuseSecond.calls.map(call => call.args.key), ['crosshairSize', 'plotFormat'],
    'the queue stops at the refusal instead of pretending the rest landed');
});

test('settle waits for the queued writes to become visible, and never claims more', async () => {
  const draft = { ...draftFromSettings(snapshot(), 'en'), grid: false };
  const entries = applyPayload(draft, snapshot());
  assert.deepEqual(entries, [{ field: 'grid', key: 'grid', value: false }]);
  assert.equal(entryVisible(snapshot(), entries[0]), false, 'the old snapshot does not show it yet');
  assert.equal(entryVisible(snapshot({ grid: false }), entries[0]), true);

  // The first two reads are the previous tick's snapshot, the third shows it.
  let reads = 0;
  const late = recordingInvoke(() => {
    reads += 1;
    return snapshot({ grid: reads >= 3 ? false : true });
  });
  const slept = [];
  const settled = await settleSettings(late.invoke, entries, {
    attempts: 5, delayMs: 7, sleep: async ms => { slept.push(ms); },
  });
  assert.equal(settled.kind, 'ok');
  assert.equal(settled.settings.grid, false);
  assert.equal(reads, 3);
  assert.deepEqual(slept, [7, 7]);

  // A snapshot that never shows the write is `stale` with the last read, not a
  // manufactured success.
  const never = recordingInvoke(() => snapshot());
  const stale = await settleSettings(never.invoke, entries, {
    attempts: 2, delayMs: 0, sleep: async () => {},
  });
  assert.equal(stale.kind, 'stale');
  assert.equal(stale.settings.grid, true);

  // `cad_waiting` keeps trying instead of failing the dialog.
  let rounds = 0;
  const booting = recordingInvoke(() => {
    rounds += 1;
    if (rounds < 2) throw new Error('cad_waiting: no session');
    return snapshot({ grid: false });
  });
  const waited = await settleSettings(booting.invoke, entries, {
    attempts: 4, delayMs: 0, sleep: async () => {},
  });
  assert.equal(waited.kind, 'ok');
  assert.equal(waited.settings.grid, false);

  // A broken read is reported as a failure, not as a stale snapshot.
  const broken = recordingInvoke(() => { throw 'cad_unavailable: closed'; });
  assert.deepEqual(await settleSettings(broken.invoke, entries, { attempts: 1, sleep: async () => {} }), {
    kind: 'failed', error: 'cad_unavailable: closed',
  });

  // Nothing to wait for: one read settles it.
  const empty = recordingInvoke(snapshot());
  assert.equal((await settleSettings(empty.invoke, [])).kind, 'ok');
});

// --- copy and entry points -------------------------------------------------

test('a refusal is shown as catalog copy, with the code kept for the tooltip', () => {
  const known = optionsErrorView('settings.invalid_value', tEn);
  assert.equal(known.code, 'settings.invalid_value');
  assert.equal(known.text, en['options.error.settings.invalid_value']);
  assert.ok(!known.text.includes('settings.'), 'the visible sentence must not be a machine code');
  assert.equal(optionsErrorView('settings.invalid_plot_format', tEn).text, en['options.error.settings.invalid_plot_format']);
  assert.equal(optionsErrorView('settings.unknown_key', tEn).text, en['options.error.settings.unknown_key']);
  // Anything the bundle does not know falls back to the generic sentence, and
  // the raw code is still reported.
  for (const code of ['cad_bug: exploded', 'settings.apply_not_visible', '']) {
    const view = optionsErrorView(code, tEn);
    assert.equal(view.code, code);
    assert.equal(view.text, en['options.error.generic']);
  }
});

test('only the typed OPTIONS word opens the dialog; nothing else is intercepted', () => {
  assert.equal(isOptionsCommand('OPTIONS'), true);
  assert.equal(isOptionsCommand('  options '), true);
  assert.equal(isOptionsCommand('Options'), true);
  for (const text of ['PROPERTIES', 'OPTIONSX', 'OP', 'OPTIONS 1', '', null, undefined, 0]) {
    assert.equal(isOptionsCommand(text), false, `${JSON.stringify(text)} must reach the router`);
  }
});

test('every key the dialog asks for exists in both catalogs with real copy', () => {
  const keys = new Set([
    ...Object.values(OPTIONS_KEYS),
    ...OPTIONS_TABS.map(tab => tab.labelKey),
    ...OPTIONS_FIELDS.flatMap(field => [field.labelKey, field.helpKey, field.unitKey].filter(Boolean)),
    ...LOCALE_CHOICES.map(choice => `options.language.${choice}`),
    'options.error.generic',
    'options.error.load_failed',
    'options.error.not_visible',
    ...Object.values(SETTINGS_ERROR).map(code => `options.error.${code}`),
  ]);
  for (const key of keys) {
    for (const [locale, catalog] of [['zh-CN', zh], ['en', en]]) {
      assert.equal(typeof catalog[key], 'string', `${locale} is missing ${key}`);
      assert.ok(catalog[key].trim().length > 0, `${locale} has empty copy for ${key}`);
    }
  }
  // The staged-change note keeps its {count} placeholder in both languages.
  assert.match(en['dialog.options.changed'], /\{count\}/);
  assert.match(zh['dialog.options.changed'], /\{count\}/);
});

test('the options popup kind is allowlisted and needs no boot payload', () => {
  assert.ok(POPUP_KINDS.includes('options'));
  const boot = readPopupBoot({
    __YACR_POPUP__: { kind: 'options', modal: true, label: 'popup-1', payload: {} },
  });
  assert.deepEqual(boot, { kind: 'options', modal: true, label: 'popup-1', payload: {} });
  assert.ok(popupBoot({ __YACR_POPUP__: boot }), 'an empty payload must boot the dialog');
  assert.deepEqual(popupView(boot), { kind: 'options', requestedKind: 'options', modal: true });
  // A payload with settings-shaped fields is ignored: the dialog reads the
  // snapshot itself, so an injected one can never become the baseline.
  const injected = popupView({
    kind: 'options', modal: true, label: 'popup-1', payload: { grid: false, crosshairPercent: 99 },
  });
  assert.deepEqual(injected, { kind: 'options', requestedKind: 'options', modal: true });
});

test('the model is pure and the dialog owns no IPC text of its own', async () => {
  const model = await readModule('options.mjs');
  // No JSX/React/DOM in the model: it is importable from a node test.
  for (const forbidden of ["from 'react'", 'React.', 'document.', 'window.', '<div']) {
    assert.ok(!model.includes(forbidden), `options.mjs must stay pure: ${forbidden}`);
  }
  // Both IPC names are declared once, in the model.
  assert.ok(model.includes(`'cad_settings'`));
  assert.ok(model.includes(`'cad_set_setting'`));

  const dialog = await readModule('components/OptionsDialog.jsx');
  assert.ok(dialog.includes("from '../options.mjs'"));
  assert.ok(dialog.includes('setLocale'));
  assert.ok(dialog.includes('storeLocale'));
  // The dialog never builds the write IPC or the payload itself: it calls the
  // helpers above, so the wire contract has exactly one owner.
  assert.ok(!dialog.includes('SETTINGS_WRITE_COMMAND'));
  assert.ok(dialog.includes('writeAll('));
  assert.ok(dialog.includes('applyPayload('));
  // Cancel/Escape close without writing: only Apply and OK call `commit`.
  assert.match(dialog, /onClick=\{onClose\}>\{t\(OPTIONS_KEYS\.cancel\)\}/);
  assert.match(dialog, /commit\('apply'\)/);
  assert.match(dialog, /commit\('close'\)/);
  // Keyboard closing is PopupRoot's (Escape), not a second local handler.
  assert.ok(!/['"]Escape['"]/.test(dialog), 'the dialog must not handle Escape itself');
});

test('all three entry points open the same host-owned popup', async () => {
  const app = await readModule('App.jsx');
  // The typed word is intercepted before the command is admitted.
  const intercept = app.indexOf('isOptionsCommand(text)');
  assert.ok(intercept > 0, 'the command line must intercept OPTIONS');
  assert.ok(intercept < app.indexOf('submitting.current = true'), 'the intercept runs first');
  assert.ok(app.includes("openPopup(invoke, 'options')"));
  assert.ok(app.includes("openOptionsPopup: openOptions"));
  assert.ok(app.includes("action === 'options'"));

  const ribbon = await readModule('ribbon-model.mjs');
  assert.ok(ribbon.includes("options: 'options'"));
  assert.ok(ribbon.includes('labelKey: OPTIONS_KEYS.ribbonEntry'));
  assert.ok(ribbon.includes("action: 'options'"));

  const status = await readModule('components/StatusBar.jsx');
  assert.ok(status.includes('shell.openOptionsPopup'));
  assert.ok(status.includes('OPTIONS_KEYS.title'));
  assert.ok(!/statusbar__workspace"\s+disabled/.test(status), 'the settings button is enabled now');
});

test('the dialog size agrees with the Rust window size', async () => {
  const layout = await readModule('popup-layout.mjs');
  assert.ok(layout.includes('export const OPTIONS_WIDTH = 720;'));
  assert.ok(layout.includes('export const OPTIONS_HEIGHT = 560;'));
  // The popup kinds and their window sizes are platform-independent UI logic, so
  // they live in `cad-ui-core` (P1a moved them out of the Linux host).
  const rust = await readFile(new URL('../../../crates/cad-ui-core/src/popup/mod.rs', import.meta.url), 'utf8');
  assert.ok(rust.includes('pub const OPTIONS_WIDTH: f64 = 720.0;'));
  assert.ok(rust.includes('pub const OPTIONS_HEIGHT: f64 = 560.0;'));
  assert.ok(rust.includes('PopupKind::Options'));
  assert.ok(rust.includes('"options" => Ok(Self::Options)'));
});
