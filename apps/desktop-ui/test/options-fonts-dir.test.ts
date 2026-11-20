// The fonts-directory rows of Options > Files.
//
// The editable row shows the **saved choice** (`fontsDirOverride`, written under
// the key `fontsDir`); the read-only row shows the directory actually in use
// (`fontsDir`). Pinned here: the write shape, "cancel changes nothing", the
// carrier gate (surface mode, never the build target), the unavailable/missing
// rendering for a carrier without a local directory, and the warning — which is
// the host's `fontsDirMissing` boolean and nothing else.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  SETTINGS_ERROR,
  applyPayload,
  dirtyFields,
  draftFromSettings,
  fieldById,
  fieldsForTab,
  normalizeValue,
  validateField,
  writeAll,
} from '../src/options/options.ts';
import {
  PICK_DIRECTORY_COMMAND,
  canPickDirectory,
  fontsDirKnown,
  fontsDirMissing,
  pickDirectory,
  pickerAvailable,
} from '../src/options/fonts-dir.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

/**
 * The host's snapshot shape: `fontsDir` in use, `fontsDirOverride` the saved
 * choice, `fontsDirMissing` its verdict (always present).
 */
function snapshot(overrides = {}) {
  return {
    fontsDir: '/opt/yacr/fonts',
    fontsDirOverride: '/opt/yacr/fonts',
    fontsDirMissing: false,
    ...overrides,
  };
}

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

// --- the row itself --------------------------------------------------------

test('the fonts folder is an editable choice above the effective directory', () => {
  // The host's two snapshot keys are not folded together: `fontsDirOverride` is
  // the value the row shows, `fontsDir` is the directory in use, and the write
  // key is the host's own `fontsDir`.
  const field = fieldById('fontsDir');
  assert.equal(field.kind, 'path');
  assert.equal(field.tab, 'files');
  assert.equal(field.wire, 'fontsDirOverride');
  assert.equal(field.key, 'fontsDir');
  assert.equal(field.labelKey, 'options.field.fontsDir');
  assert.equal(field.helpKey, 'options.help.fontsDir');
  // The effective row sits directly below it and is answered by the host only.
  const ids = fieldsForTab('files').map(entry => entry.id);
  assert.equal(ids.indexOf('fontsDirEffective'), ids.indexOf('fontsDir') + 1);
  const effective = fieldById('fontsDirEffective');
  assert.equal(effective.kind, 'readonly');
  assert.equal(effective.wire, 'fontsDir');
  assert.equal(effective.key, undefined, 'the effective value is never written');
  assert.equal(effective.labelKey, 'options.field.fontsDirEffective');
  assert.equal(effective.helpKey, 'options.help.fontsDirEffective');
});

test('the override seeds, diffs, writes and clears through cad_set_setting', async () => {
  const wire = snapshot();
  assert.equal(draftFromSettings(wire).fontsDir, '/opt/yacr/fonts');
  assert.equal(draftFromSettings(wire).fontsDirEffective, '/opt/yacr/fonts');
  // An omitted `fontsDirOverride` (never chosen, or cleared) is the documented
  // "nothing saved" state: the editable row is empty, not an invented path.
  assert.equal(draftFromSettings(snapshot({ fontsDirOverride: null })).fontsDir, '');
  assert.equal(draftFromSettings({ fontsDir: '/usr/share/fonts' }).fontsDir, '');
  assert.equal(draftFromSettings({ fontsDir: '/usr/share/fonts' }).fontsDirEffective, '/usr/share/fonts');
  assert.equal(draftFromSettings({}).fontsDir, '');
  // The effective row keeps "not reported" as its own state.
  assert.equal(draftFromSettings({}).fontsDirEffective, null);
  // An untouched empty row equals its empty baseline: nothing is written.
  assert.deepEqual(dirtyFields(draftFromSettings({}), {}), []);
  assert.deepEqual(applyPayload(draftFromSettings({}), {}), []);

  // A picked directory stages exactly one trimmed string write.
  const picked = { ...draftFromSettings(wire), fontsDir: '  /opt/yacr/fonts2 ' };
  assert.deepEqual(dirtyFields(picked, wire), ['fontsDir']);
  assert.deepEqual(applyPayload(picked, wire), [
    { field: 'fontsDir', key: 'fontsDir', value: '/opt/yacr/fonts2' },
  ]);
  const granted = recordingInvoke({ accepted: true });
  assert.deepEqual(await writeAll(granted.invoke, applyPayload(picked, wire)), { kind: 'accepted' });
  assert.deepEqual(granted.calls, [
    { command: 'cad_set_setting', args: { key: 'fontsDir', value: '/opt/yacr/fonts2' } },
  ]);

  // Clearing writes the empty override — never a fabricated default path.
  const cleared = { ...draftFromSettings(wire), fontsDir: '' };
  assert.deepEqual(applyPayload(cleared, wire), [
    { field: 'fontsDir', key: 'fontsDir', value: '' },
  ]);
  const clearGranted = recordingInvoke({ accepted: true });
  assert.deepEqual(await writeAll(clearGranted.invoke, applyPayload(cleared, wire)), { kind: 'accepted' });
  assert.deepEqual(clearGranted.calls, [
    { command: 'cad_set_setting', args: { key: 'fontsDir', value: '' } },
  ]);
  // Any text is a legal path (the empty one clears), and only a non-string is
  // refused here: whether the directory exists is the host's answer.
  assert.equal(validateField('fontsDir', '/a b/c'), null);
  assert.equal(validateField('fontsDir', ''), null);
  assert.equal(validateField('fontsDir', 7), SETTINGS_ERROR.invalidValue);
  assert.equal(normalizeValue('fontsDir', '  /x  '), '/x');
  assert.equal(normalizeValue('fontsDir', null), '');
});

// --- the picker ------------------------------------------------------------

test('a cancelled picker changes nothing, and a refusal is not a cancel', async () => {
  assert.equal(PICK_DIRECTORY_COMMAND, 'cad_pick_directory');
  // `null`, `undefined` and an empty answer are all "the user cancelled".
  for (const answer of [null, undefined, '', '   ']) {
    const cancelled = recordingInvoke(answer);
    assert.equal(await pickDirectory(cancelled.invoke), null, String(answer));
    assert.deepEqual(cancelled.calls, [{ command: 'cad_pick_directory' }]);
  }
  const chosen = recordingInvoke('/opt/yacr/fonts\n');
  assert.equal(await pickDirectory(chosen.invoke), '/opt/yacr/fonts');
  // A refusal throws: the dialog reports it instead of staging a value.
  const refused = recordingInvoke(() => new Error('unsupported-runtime: cad_pick_directory'));
  await assert.rejects(() => pickDirectory(refused.invoke), /unsupported-runtime/);

  // Source contract: the dialog stages only a non-null answer, so a cancel keeps
  // the previous value exactly as it was.
  const dialog = await readModule('components/OptionsDialog.tsx');
  assert.ok(dialog.includes('pickDirectory(invoke)'));
  assert.match(dialog, /picked === null\) return;/);
  assert.ok(dialog.includes("t('options.action.browse')"));
  assert.ok(dialog.includes("t('options.action.clear')"));
  assert.ok(dialog.includes("update(field.id, '')"), 'clear stages the empty override');
  // The placeholder is the effective directory, so an empty row still says what
  // will be read.
  assert.ok(dialog.includes('placeholder={effective ?? undefined}'));
  assert.ok(!dialog.includes('@tauri-apps/api'), 'the picker goes through host/invoke.ts');
});

// --- the carrier gate ------------------------------------------------------

test('the editable row exists only where a directory picker can open', async () => {
  // The gate is the reported surface mode, never the build target.
  assert.equal(pickerAvailable('native-below'), true);
  assert.equal(pickerAvailable('dom-canvas'), false);
  assert.equal(pickerAvailable('unknown'), false);
  assert.equal(canPickDirectory({ status: { hostProfile: { surfaceMode: 'native-below' } } }), true);
  assert.equal(canPickDirectory({ status: { hostProfile: { surfaceMode: 'dom-canvas' } } }), false);
  assert.equal(canPickDirectory({ status: { hostProfile: { surfaceMode: 'something-new' } } }), false);
  // A host with no profile yet is the historic `native-below` carrier (the same
  // absence rule as every other optional status field).
  assert.equal(canPickDirectory({ status: {} }), true);

  const dialog = await readModule('components/OptionsDialog.tsx');
  assert.ok(dialog.includes('surfaceModeFromShell({ status })'));
  assert.ok(dialog.includes('pickerAvailable(surfaceMode)'));
  // The row always renders; the gate decides whether it is an editor or the
  // disabled/missing shape, and the picker is never called without it.
  assert.ok(dialog.includes('const editable = canPick;'));
  assert.ok(dialog.includes('if (!canPick) return;'));
  // Both faces of the row say why it cannot be edited here.
  assert.ok(dialog.includes("'options.help.fontsDirUnavailable'"));
  assert.ok(!dialog.includes('__YACR_RUNTIME__'), 'the gate is not the build target');
});

// --- the two host facts ----------------------------------------------------

test('the warning is the host flag alone, never a comparison of the two paths', async () => {
  // `fontsDirMissing` is the host's verdict and is always present.
  assert.equal(fontsDirMissing({ fontsDir: '/a', fontsDirOverride: '/b', fontsDirMissing: true }), true);
  assert.equal(fontsDirMissing({ fontsDir: '/a', fontsDirOverride: '/b', fontsDirMissing: false }), false);
  // The critical case this rule exists for: `--fonts-dir` wins over a valid saved
  // choice, so the two paths differ without anything being missing.
  assert.equal(fontsDirMissing({ fontsDir: '/cli/fonts', fontsDirOverride: '/saved/fonts', fontsDirMissing: false }), false);
  // A stale choice the host did diagnose.
  assert.equal(fontsDirMissing({ fontsDir: '/usr/share/fonts', fontsDirOverride: '/gone', fontsDirMissing: true }), true);
  // Nothing saved is nothing missing; an unreadable snapshot is no warning.
  assert.equal(fontsDirMissing({ fontsDir: '/usr/share/fonts', fontsDirMissing: false }), false);
  for (const value of [{}, { fontsDirMissing: 'yes' }, null, undefined]) {
    assert.equal(fontsDirMissing(value), false, JSON.stringify(value));
  }
  // "Can the host name a directory at all" is key absence, not string equality.
  assert.equal(fontsDirKnown(snapshot()), true);
  assert.equal(fontsDirKnown({ fontsDirOverride: '/saved' }), false);
  assert.equal(fontsDirKnown({ fontsDir: '  ' }), false);
  assert.equal(fontsDirKnown(null), false);

  // The warning is inline on the row (a role=status line), never a modal, and it
  // uses the published catalog sentence.
  const dialog = await readModule('components/OptionsDialog.tsx');
  assert.ok(dialog.includes('fontsDirMissing(settings)'));
  assert.ok(dialog.includes("t('options.status.fontsDirMissing')"));
  assert.ok(!/role="alertdialog"/.test(dialog), 'a missing directory is not a modal');
  assert.ok(!dialog.includes('fontsDir !=='), 'no path comparison decides the warning');
});

test('a carrier without a local directory renders the rows as unavailable', async () => {
  // Web/`dom-canvas` omits both path keys and refuses the pick and the write, so
  // the editable row must be a disabled field with the "not reported" badge — not
  // an empty editable box, and not a missing Browse button on its own.
  const dialog = await readModule('components/OptionsDialog.tsx');
  assert.ok(dialog.includes('const editable = canPick;'));
  assert.ok(dialog.includes('disabled={!editable}'));
  assert.ok(dialog.includes('{editable ? ('));
  // The action pair exists only on the editable path.
  const browseAt = dialog.indexOf("t('options.action.browse')");
  const editableAt = dialog.indexOf('{editable ? (');
  assert.ok(editableAt > 0 && browseAt > editableAt, 'Browse lives inside the editable branch');
  assert.ok(
    dialog.includes(') : fontsAvailable ? (') && dialog.includes('<MissingBadge t={t} />'),
    'no directory reported keeps the missing badge',
  );
  // The read-only row keeps the same pattern: absent value => "not reported".
  assert.match(dialog, /\{missing\s*\n\s*\? <MissingBadge t=\{t\} \/>/);
  // And both rows drop the "pick a directory" sentence there.
  assert.ok(dialog.includes("fontsRow && (!canPick || !fontsAvailable)"));
  assert.ok(dialog.includes("'options.help.fontsDirUnavailable'"));

  // The same absence on the read-only value side is the draft's "not reported".
  assert.equal(draftFromSettings({ fontsDirMissing: false }).fontsDirEffective, null);
  assert.equal(draftFromSettings({ fontsDirMissing: false }).fontsDir, '');
});

test('every fonts-directory sentence exists in both catalogs', () => {
  for (const key of [
    'options.field.fontsDir',
    'options.help.fontsDir',
    'options.field.fontsDirEffective',
    'options.help.fontsDirEffective',
    'options.help.fontsDirUnavailable',
    'options.action.browse',
    'options.action.clear',
    'options.status.fontsDirMissing',
  ]) {
    assert.equal(typeof en[key], 'string', `en is missing ${key}`);
    assert.equal(typeof zh[key], 'string', `zh-CN is missing ${key}`);
    assert.ok(en[key].trim().length > 0, key);
    assert.ok(zh[key].trim().length > 0, key);
  }
});
