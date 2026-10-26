// Options dialog contract: the control table, the client-side mirror of the
// host's setting rules, the staged diff, the two IPCs and every entry point.
//
// The tests are deliberately table-driven: the field table is the one place that
// says which controls exist, so a control that gains or loses its backend key
// (or changes its write key) fails here rather than in a window nobody opened.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createTranslator } from '../src/i18n.ts';
import { POPUP_KINDS, popupView } from '../src/popup/popup.ts';
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
  WGPU_BACKENDS,
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
  parseRealText,
  readSettings,
  settleSettings,
  settingErrorCode,
  tabById,
  validateDraft,
  validateField,
  writeAll,
  writeSetting,
} from '../src/options/options.ts';
import { readCatalog, readModule } from './helpers.ts';

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
    // `SNAPUNIT`: a real, in drawing units (the host's default is 10.0).
    snapSpacing: 10,
    pickboxSize: 6,
    apertureSize: 12,
    gpuBudgetPercent: 90,
    plotFormat: 'svg',
    fontsDir: '/usr/share/fonts',
    // The WGPU backend is a shared wire key: snake_case in the snapshot too.
    wgpu_backend: 'auto',
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
    'backgroundModel', 'backgroundPaper', 'crosshairSize', 'grid', 'axes', 'selectionHighlight',
    'snapHints', 'gpuBudgetPercent',
  ]);
  assert.deepEqual(fieldsForTab('drafting').map(field => field.id), [
    'snapEnabled', 'hoverPreview', 'snapSpacing', 'pickboxSize', 'apertureSize',
  ]);
  assert.deepEqual(fieldsForTab('plot').map(field => field.id), ['plotFormat']);
  assert.deepEqual(fieldsForTab('files').map(field => field.id), ['fontsDir']);
  assert.deepEqual(fieldsForTab('user').map(field => field.id), ['language', 'wgpuBackend']);
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
    snapSpacing: 'real',
    pickboxSize: 'number',
    apertureSize: 'number',
    gpuBudgetPercent: 'number',
    plotFormat: 'select',
    fontsDir: 'readonly',
    language: 'locale',
    wgpuBackend: 'select',
  });
  // Number controls carry the host's own range and a unit suffix, never a clamp.
  assert.deepEqual(
    OPTIONS_FIELDS.filter(field => field.kind === 'number').map(field => [field.id, field.min, field.max]),
    [['crosshairSize', 1, 100], ['gpuBudgetPercent', 10, 100], ['pickboxSize', 1, 20], ['apertureSize', 1, 50]],
  );
  for (const field of OPTIONS_FIELDS.filter(entry => entry.kind === 'number' || entry.kind === 'real')) {
    assert.ok(field.unitKey, `${field.id} needs a unit`);
  }
  // The plot selector offers exactly the two formats the host can produce.
  assert.deepEqual(fieldById('plotFormat').choices, ['svg', 'pdf']);
  assert.deepEqual(PLOT_FORMATS, ['svg', 'pdf']);
  assert.deepEqual(LOCALE_CHOICES, ['zh-CN', 'en']);
  // The WGPU selector offers exactly the shared enum, with its own copy.
  assert.deepEqual(WGPU_BACKENDS, ['auto', 'vulkan', 'dx12']);
  assert.deepEqual(fieldById('wgpuBackend').choices, WGPU_BACKENDS);
  assert.equal(fieldById('wgpuBackend').choiceKeyPrefix, 'options.wgpuBackend');
  // The plot format keeps the host's dedicated refusal; the backend is a plain
  // enum mismatch.
  assert.equal(fieldById('plotFormat').errorCode, SETTINGS_ERROR.invalidPlotFormat);
  assert.equal(fieldById('wgpuBackend').errorCode, undefined);
});

test('the WGPU backend uses the shared snake_case wire key on both directions', () => {
  const field = fieldById('wgpuBackend');
  // The config key another lane reads is `wgpu_backend`: never camelCased.
  assert.equal(field.wire, 'wgpu_backend');
  assert.equal(field.key, 'wgpu_backend');
  assert.equal(field.tab, 'user');
  assert.equal(field.labelKey, 'options.field.wgpuBackend');
  assert.equal(field.helpKey, 'options.help.wgpuBackend');
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

test('reals are parsed exactly, and the host positive rule is mirrored', () => {
  assert.equal(parseRealText('2.5'), 2.5);
  assert.equal(parseRealText(' 0.5 '), 0.5);
  assert.equal(parseRealText('10'), 10);
  assert.equal(parseRealText(2.5), 2.5);
  assert.equal(parseRealText('.5'), 0.5);
  assert.equal(parseRealText('2.'), 2);
  for (const bad of [
    '', ' ', '0', '0.0', '-1', '-0.5', 'abc', '2,5', '0x10', 'Infinity', 'NaN', '2 5',
    null, undefined, {}, [], NaN, Infinity, -Infinity, 0, -1,
  ]) {
    assert.equal(parseRealText(bad), null, `${String(bad)} must not parse`);
  }
  // A field passes its own bounds in; the seeder does not (see `draftFromSettings`).
  const field = fieldById('snapSpacing');
  assert.equal(parseRealText('1000000', field), 1_000_000);
  assert.equal(parseRealText('2000001', field), null);
  // The snapshot's text form round-trips, including exponent spellings.
  for (const value of [0.5, 2.5, 10, 1e-7, 1234.5678]) {
    assert.equal(parseRealText(String(value)), value, `${value} must round-trip`);
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
    // The snap spacing is a real: a fraction is legal (the host's
    // `snap_spacing_is_valid` is `finite && > 0`), zero and negatives are not.
    ['snapSpacing', '2.5', null],
    ['snapSpacing', '0.5', null],
    ['snapSpacing', '10', null],
    ['snapSpacing', 2.5, null],
    ['snapSpacing', '1000000', null],
    ['snapSpacing', '0', SETTINGS_ERROR.invalidValue],
    ['snapSpacing', 0, SETTINGS_ERROR.invalidValue],
    ['snapSpacing', '-1', SETTINGS_ERROR.invalidValue],
    ['snapSpacing', '-0.5', SETTINGS_ERROR.invalidValue],
    ['snapSpacing', 'abc', SETTINGS_ERROR.invalidValue],
    ['snapSpacing', '', SETTINGS_ERROR.invalidValue],
    ['snapSpacing', '2000001', SETTINGS_ERROR.invalidValue],
    ['snapSpacing', NaN, SETTINGS_ERROR.invalidValue],
    // The GPU budget is a percentage of the card: the floor and the whole card
    // are accepted, anything outside is a refusal (never clamped).
    ['gpuBudgetPercent', '10', null],
    ['gpuBudgetPercent', '90', null],
    ['gpuBudgetPercent', '100', null],
    ['gpuBudgetPercent', '9', SETTINGS_ERROR.invalidValue],
    ['gpuBudgetPercent', '101', SETTINGS_ERROR.invalidValue],
    ['gpuBudgetPercent', '', SETTINGS_ERROR.invalidValue],
    ['gpuBudgetPercent', '90.5', SETTINGS_ERROR.invalidValue],
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
    ['wgpuBackend', 'auto', null],
    ['wgpuBackend', 'vulkan', null],
    ['wgpuBackend', 'dx12', null],
    // A backend outside the shared enum is a plain invalid value (the host's own
    // `settings.invalid_value`), never the plot-format sentence and never clamped
    // to `auto`.
    ['wgpuBackend', 'Vulkan', SETTINGS_ERROR.invalidValue],
    ['wgpuBackend', 'metal', SETTINGS_ERROR.invalidValue],
    ['wgpuBackend', '', SETTINGS_ERROR.invalidValue],
    ['wgpuBackend', null, SETTINGS_ERROR.invalidValue],
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
  assert.equal(draft.snapSpacing, '10');
  assert.equal(draft.gpuBudgetPercent, '90');
  assert.equal(draft.grid, true);
  assert.equal(draft.plotFormat, 'svg');
  assert.equal(draft.fontsDir, '/usr/share/fonts');
  assert.equal(draft.language, 'zh-CN');
  assert.equal(draft.wgpuBackend, 'auto');
  // Colours are canonicalised to lower case for comparison and display.
  assert.equal(draftFromSettings(snapshot({ backgroundModel: '#AABBCC' })).backgroundModel, '#aabbcc');

  // An older/partial host reports less: those controls stay explicitly absent.
  const empty = draftFromSettings({}, null);
  assert.equal(empty.crosshairSize, null);
  assert.equal(empty.gpuBudgetPercent, null);
  assert.equal(empty.snapSpacing, null);
  assert.equal(empty.grid, null);
  assert.equal(empty.plotFormat, null);
  assert.equal(empty.fontsDir, null);
  assert.equal(empty.language, null);
  // A host that does not report the backend shows the row as unreported (and
  // unwritable) rather than seeding the `auto` default on its behalf.
  assert.equal(empty.wgpuBackend, null);
  // A malformed value is not repaired into something it never was.
  const malformed = draftFromSettings({
    grid: 'yes', crosshairPercent: -3, backgroundModel: 'grey', plotFormat: 'dxf', fontsDir: 7,
    wgpu_backend: 'metal', snapSpacing: 0,
  });
  // A spacing the host's own rule refuses is reported as absent, never seeded as
  // a "valid" default; a legal real that merely exceeds this row's typo guard is
  // kept as reported and then refused by the validator.
  assert.equal(malformed.snapSpacing, null);
  assert.equal(draftFromSettings({ snapSpacing: 2_000_000 }).snapSpacing, '2000000');
  assert.equal(validateField('snapSpacing', '2000000'), SETTINGS_ERROR.invalidValue);
  assert.equal(malformed.grid, null);
  assert.equal(malformed.wgpuBackend, null);
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
    gpuBudgetPercent: '25',
    plotFormat: 'pdf',
    wgpuBackend: 'vulkan',
    comfort: 'ignored',
  };
  assert.deepEqual(applyPayload(draft, wire), [
    { field: 'backgroundModel', key: 'backgroundModel', value: '#aabbcc' },
    // The wire field is `crosshairPercent`; the write key is `crosshairSize`.
    { field: 'crosshairSize', key: 'crosshairSize', value: 40 },
    { field: 'grid', key: 'grid', value: false },
    // The GPU budget is the same key in the snapshot, the write and the config.
    { field: 'gpuBudgetPercent', key: 'gpuBudgetPercent', value: 25 },
    { field: 'plotFormat', key: 'plotFormat', value: 'pdf' },
    // The backend keeps the shared snake_case key in both directions.
    { field: 'wgpuBackend', key: 'wgpu_backend', value: 'vulkan' },
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
  assert.equal(normalizeValue('wgpuBackend', 'dx12'), 'dx12');
  assert.equal(normalizeValue('wgpuBackend', 'DX12'), null);
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
    ...WGPU_BACKENDS.map(choice => `options.wgpuBackend.${choice}`),
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

test('the WGPU backend rows read like the language row, in both catalogs', async () => {
  const field = fieldById('wgpuBackend');
  // The label/help copy follows the field-table convention (both catalogs).
  assert.equal(zh[field.labelKey], 'WGPU 后端');
  assert.equal(en[field.labelKey], 'WGPU backend');
  assert.ok(zh[field.helpKey].trim().length > 0);
  assert.ok(en[field.helpKey].trim().length > 0);
  // Each choice reads through its own catalog key, exactly like a language name
  // does: the wire spelling is never shown raw.
  const tzh = createTranslator({ 'zh-CN': zh, en }, 'zh-CN');
  assert.deepEqual(WGPU_BACKENDS.map(choice => en[`${field.choiceKeyPrefix}.${choice}`]), [
    'Auto', 'Vulkan', 'DirectX 12',
  ]);
  assert.deepEqual(WGPU_BACKENDS.map(choice => zh[`${field.choiceKeyPrefix}.${choice}`]), [
    '自动', 'Vulkan', 'DirectX 12',
  ]);
  for (const choice of WGPU_BACKENDS) {
    assert.notEqual(tzh(`${field.choiceKeyPrefix}.${choice}`), `${field.choiceKeyPrefix}.${choice}`,
      `${choice} has no copy, so the raw key would be rendered`);
  }
  // The selector is a native select, the same control the language row uses.
  const dialog = await readModule('components/OptionsDialog.tsx');
  assert.ok(dialog.includes('choiceKeyPrefix'), 'a copy-carrying select must resolve through the catalog');
});

test('the snap spacing is a decimal drafting row, written through cad_set_setting', async () => {
  const field = fieldById('snapSpacing');
  assert.equal(field.kind, 'real');
  assert.equal(field.tab, 'drafting');
  // The host's `cad_settings` field and its `cad_set_setting` key agree here, and
  // both are the camelCase wire spelling of `SNAPUNIT` (`snap_spacing`).
  assert.equal(field.wire, 'snapSpacing');
  assert.equal(field.key, 'snapSpacing');
  // The range mirrors `snap_spacing_is_valid` (finite, strictly positive) plus a
  // typo guard; the host re-validates and its refusal is authoritative.
  assert.equal(field.min, Number.MIN_VALUE);
  assert.equal(field.max, 1_000_000);
  assert.equal(field.unitKey, 'options.unit.drawing_units');
  assert.equal(field.labelKey, 'options.field.snapSpacing');
  assert.equal(field.helpKey, 'options.help.snapSpacing');
  assert.ok(fieldsForTab('drafting').includes(field));

  // Seeded from the snapshot (a fraction survives as text), validated, diffed.
  const wire = snapshot({ snapSpacing: 2.5 });
  assert.equal(draftFromSettings(wire).snapSpacing, '2.5');
  assert.equal(validateField('snapSpacing', draftFromSettings(wire).snapSpacing), null);
  // A textually different equal value is not a change.
  assert.deepEqual(dirtyFields({ ...draftFromSettings(wire), snapSpacing: '2.50' }, wire), []);
  const draft = { ...draftFromSettings(wire), snapSpacing: '0.5' };
  assert.deepEqual(applyPayload(draft, wire), [
    { field: 'snapSpacing', key: 'snapSpacing', value: 0.5 },
  ]);

  // The very same staged write path as every other writable field: one typed
  // `cad_set_setting` call, a JSON **number** (never a string the host would
  // read as text and refuse).
  const granted = recordingInvoke({ accepted: true });
  assert.deepEqual(await writeAll(granted.invoke, applyPayload(draft, wire)), { kind: 'accepted' });
  assert.deepEqual(granted.calls, [
    { command: 'cad_set_setting', args: { key: 'snapSpacing', value: 0.5 } },
  ]);
  // A refusal keeps the host's code and names the field it came from.
  const refused = recordingInvoke(() => { throw 'settings.invalid_value: spacing must be > 0'; });
  assert.deepEqual(await writeAll(refused.invoke, applyPayload(draft, wire)), {
    kind: 'failed', field: 'snapSpacing', key: 'snapSpacing', code: 'settings.invalid_value',
  });
  // And the re-read check recognises the written real, so settle cannot claim a
  // value the host does not have.
  const entry = { field: 'snapSpacing', key: 'snapSpacing', value: 0.5 };
  assert.equal(entryVisible(snapshot({ snapSpacing: 0.5 }), entry), true);
  assert.equal(entryVisible(snapshot({ snapSpacing: 2.5 }), entry), false);

  // Both catalogs carry the row's copy, and it reads as drawing units.
  assert.equal(zh[field.labelKey], '捕捉间距');
  assert.equal(en[field.labelKey], 'Snap spacing');
  assert.equal(zh[field.unitKey], '单位');
  assert.equal(en[field.unitKey], 'units');
  assert.ok(zh[field.helpKey].trim().length > 0);
  assert.ok(en[field.helpKey].trim().length > 0);

  // Rendered by the shared numeric editor, not a bespoke control.
  const dialog = await readModule('components/OptionsDialog.tsx');
  assert.ok(!dialog.includes('snapSpacing'), 'no special case for this field');
  assert.ok(dialog.includes("case 'real':"));
  assert.ok(dialog.includes("inputMode={decimal ? 'decimal' : 'numeric'}"));
  assert.ok(dialog.includes("step={decimal ? 'any' : undefined}"));
  assert.ok(dialog.includes('t(field.unitKey)'));
});

test('the GPU budget is a generic numeric row, never a bespoke control', async () => {
  const field = fieldById('gpuBudgetPercent');
  assert.equal(field.kind, 'number');
  assert.equal(field.tab, 'display', 'it sits with the other graphics settings');
  assert.equal(field.wire, 'gpuBudgetPercent');
  assert.equal(field.key, 'gpuBudgetPercent');
  assert.equal(field.min, 10);
  assert.equal(field.max, 100);
  assert.equal(field.unitKey, 'options.unit.percent');
  assert.equal(field.labelKey, 'options.field.gpuBudgetPercent');
  assert.equal(field.helpKey, 'options.help.gpuBudgetPercent');
  assert.ok(fieldsForTab('display').includes(field));
  // The dialog renders every number field through the same row/editor path, so
  // this control needs (and has) no per-field branch of its own.
  const dialog = await readModule('components/OptionsDialog.tsx');
  assert.ok(!dialog.includes('gpuBudgetPercent'), 'no special case for this field');
  assert.ok(dialog.includes("case 'number':"));
  assert.ok(dialog.includes('t(field.unitKey)'));
  // Both catalogs carry the copy the descriptor points at.
  assert.equal(zh[field.labelKey], 'GPU 内存预算');
  assert.equal(en[field.labelKey], 'GPU memory budget');
  assert.ok(zh[field.helpKey].trim().length > 0);
  assert.ok(en[field.helpKey].trim().length > 0);
  // The help says it is a share of the card's memory and a budget, not the total.
  assert.match(en[field.helpKey], /graphics card's memory/i);
  assert.match(en[field.helpKey], /budget/i);
  assert.match(en[field.helpKey], /not the card's total/i);
});

test('the options popup kind is allowlisted and carries no payload', () => {
  assert.ok(POPUP_KINDS.includes('options'));
  assert.deepEqual(popupView('options', undefined), {
    kind: 'options', requestedKind: 'options',
  });
  // A payload with settings-shaped fields is ignored: the dialog reads the
  // snapshot itself, so an injected one can never become the baseline.
  const injected = popupView('options', { kind: 'options', grid: false, crosshairPercent: 99 });
  assert.deepEqual(injected, { kind: 'options', requestedKind: 'options' });
});

test('the model is pure and the dialog owns no IPC text of its own', async () => {
  const model = await readModule('options/options.ts');
  // No JSX/React/DOM in the model: it is importable from a node test.
  for (const forbidden of ["from 'react'", 'React.', 'document.', 'window.', '<div']) {
    assert.ok(!model.includes(forbidden), `options.mjs must stay pure: ${forbidden}`);
  }
  // Both IPC names are declared once, in the model.
  assert.ok(model.includes(`'cad_settings'`));
  assert.ok(model.includes(`'cad_set_setting'`));

  const dialog = await readModule('components/OptionsDialog.tsx');
  assert.ok(dialog.includes("from '../options/options.ts'"));
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
  // Keyboard closing is PopupOverlay's (Escape), not a second local handler.
  assert.ok(!/['"]Escape['"]/.test(dialog), 'the dialog must not handle Escape itself');
});

test('all three entry points open the same host-owned popup', async () => {
  const app = await readModule('App.tsx');
  // The typed word is intercepted before the command is admitted.
  const intercept = app.indexOf('isOptionsCommand(text)');
  assert.ok(intercept > 0, 'the command line must intercept OPTIONS');
  assert.ok(intercept < app.indexOf('submitting.current = true'), 'the intercept runs first');
  assert.ok(app.includes("requestOpenPopup('options')"));
  assert.ok(app.includes("openOptionsPopup: () => requestOpenPopup('options')"));
  assert.ok(app.includes("action === 'options'"));

  const ribbon = await readModule('ribbon/ribbon-model.ts');
  assert.ok(ribbon.includes("options: 'options'"));
  assert.ok(ribbon.includes('labelKey: OPTIONS_KEYS.ribbonEntry'));
  assert.ok(ribbon.includes("action: 'options'"));

  const status = await readModule('components/StatusBar.tsx');
  assert.ok(status.includes('shell.openOptionsPopup'));
  assert.ok(status.includes('OPTIONS_KEYS.title'));
  assert.ok(!/statusbar__workspace"\s+disabled/.test(status), 'the settings button is enabled now');
});

test('the dialog card size is UI-owned, not a mirror of a host window', async () => {
  const layout = await readModule('popup/popup-layout.ts');
  assert.ok(layout.includes('export const OPTIONS_WIDTH = 720;'));
  assert.ok(layout.includes('export const OPTIONS_HEIGHT = 560;'));
  assert.ok(layout.includes('export const POPUP_CARD_SIZE = Object.freeze({'));
  // No host window exists any more: the former Rust constants must be gone.
  const rust = await readFile(new URL('../../../crates/cad-ui-core/src/popup/mod.rs', import.meta.url), 'utf8');
  assert.equal(rust.includes('pub const OPTIONS_WIDTH'), false);
  assert.equal(rust.includes('pub const OPTIONS_HEIGHT'), false);
  assert.ok(rust.includes('PopupKind::Options'));
  assert.ok(rust.includes('"options" => Ok(Self::Options)'));
});
