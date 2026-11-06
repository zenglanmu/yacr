// Contract tests for the read-only Font Manager: the strict `cad_fonts`
// validator, the `FONTS` selector, the row projection and the one-shot store.
// No screenshot and no invented host action.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  createFontsStore,
  FONTS_COMMAND,
  FONTS_PAYLOAD_INVALID,
  FONTS_SELECTOR,
  fontGroup,
  fontSummary,
  fontTone,
  isFontsCommand,
  isFontsWaiting,
  parseFontsAnswer,
  projectFontRows,
} from '../src/fonts/fonts.ts';
import { localSelectorFor } from '../src/commands/local-selectors.ts';
import { readComponentsSource, readModule } from './helpers.ts';

const face = (overrides = {}) => ({
  key: 'romans',
  file: '/fonts/romans.shx',
  kind: 'shx',
  source: 'package',
  status: 'loaded',
  detail: null,
  ...overrides,
});

const report = (overrides = {}) => ({
  available: true,
  reason: null,
  fontsDir: '/opt/yacr/fonts',
  catalogEntries: 4,
  locale: 'latin',
  defaultFace: 'romans',
  localeMissing: null,
  requested: ['romans'],
  textNeeded: true,
  faces: [face()],
  ...overrides,
});

test('the selector is one local word, case-insensitive', () => {
  assert.equal(FONTS_SELECTOR, 'FONTS');
  assert.equal(FONTS_COMMAND, 'cad_fonts');
  assert.equal(isFontsCommand('fonts'), true);
  assert.equal(isFontsCommand('  Fonts '), true);
  assert.equal(isFontsCommand('FONTS 2'), false);
  assert.equal(isFontsCommand(''), false);
  assert.equal(isFontsCommand(null), false);
});

test('FONTS opens the panel on both layouts while the desktop palette pair is refused on mobile', () => {
  assert.deepEqual(localSelectorFor('FONTS', 'desktop'), { kind: 'fonts' });
  assert.deepEqual(localSelectorFor('fonts', 'mobile'), { kind: 'fonts' });
  assert.deepEqual(localSelectorFor('RESOURCES', 'mobile'), { kind: 'resources' });
  assert.deepEqual(localSelectorFor('LAYER', 'desktop'), { kind: 'layers' });
  assert.deepEqual(localSelectorFor('LAYER', 'mobile'), { kind: 'refused', panelKey: 'layers.panel' });
  assert.deepEqual(localSelectorFor('PROPERTIES', 'mobile'), { kind: 'refused', panelKey: 'properties.panel' });
  assert.equal(localSelectorFor('LINE', 'desktop'), null);
});

test('a valid report keeps every host value and invents no row', () => {
  const answer = parseFontsAnswer(report({ faces: [
    face(),
    face({ key: 'hztxt', file: '/fonts/hztxt.shx', kind: 'shx', status: 'failed', detail: 'missing glyph' }),
    face({ key: 'Arial', file: null, kind: 'system', source: 'system', status: 'unresolved' }),
    face({ key: 'old-mesh', kind: 'mesh', source: 'explicit', status: 'unsupported', detail: 'mesh faces are not rendered' }),
  ] }));
  assert.deepEqual(answer, report({ faces: [
    face(),
    face({ key: 'hztxt', file: '/fonts/hztxt.shx', kind: 'shx', status: 'failed', detail: 'missing glyph' }),
    face({ key: 'Arial', file: null, kind: 'system', source: 'system', status: 'unresolved' }),
    face({ key: 'old-mesh', kind: 'mesh', source: 'explicit', status: 'unsupported', detail: 'mesh faces are not rendered' }),
  ] }));
});

test('a report with no faces is a real answer, not an unreadable one', () => {
  const answer = parseFontsAnswer(report({ faces: [], catalogEntries: 0, defaultFace: null }));
  assert.equal(answer.available, true);
  assert.deepEqual(answer.faces, []);
});

test('malformed payloads are refused by name instead of coerced', () => {
  const bad = [
    null,
    [],
    {},
    report({ available: 'true' }),
    report({ available: true, reason: 'host.fonts_unavailable' }),
    report({ available: false, reason: null }),
    report({ available: false, reason: 'not a catalog key' }),
    report({ fontsDir: 3 }),
    report({ catalogEntries: -1 }),
    report({ catalogEntries: 1.5 }),
    report({ locale: 'en' }),
    report({ defaultFace: 4 }),
    report({ localeMissing: 7 }),
    report({ faces: {} }),
    report({ faces: [face({ key: '' })] }),
    report({ faces: [face({ key: 3 })] }),
    report({ faces: [face({ file: 3 })] }),
    report({ faces: [face({ kind: 'ttf' })] }),
    report({ faces: [face({ source: 'user' })] }),
    report({ faces: [face({ status: 'ok' })] }),
    report({ faces: [face({ detail: 9 })] }),
    report({ faces: [face(), face()] }),
  ];
  for (const payload of bad) {
    assert.throws(() => parseFontsAnswer(payload), new RegExp(FONTS_PAYLOAD_INVALID), JSON.stringify(payload));
  }
  // An unavailable report with a named reason is valid, and so is an empty faces
  // array: neither is malformed.
  assert.equal(parseFontsAnswer(report({
    available: false, reason: 'host.fonts_unavailable', faces: [], defaultFace: null,
  })).reason, 'host.fonts_unavailable');
});

test('rows group as loaded / failed / unresolved with stable badges and tones', () => {
  const groups = projectFontRows([
    face(),
    face({ key: 'broken', status: 'failed' }),
    face({ key: 'unsupported', status: 'unsupported' }),
    face({ key: 'missing', status: 'unresolved' }),
  ]);
  assert.deepEqual(groups.loaded.map(row => row.key), ['romans']);
  assert.deepEqual(groups.failed.map(row => row.key), ['broken', 'unsupported']);
  assert.deepEqual(groups.unresolved.map(row => row.key), ['missing']);
  assert.equal(groups.loaded[0].statusKey, 'fonts.status.loaded');
  assert.equal(groups.loaded[0].sourceKey, 'fonts.source.package');
  assert.equal(groups.loaded[0].tone, 'ok');
  assert.equal(groups.failed[0].tone, 'error');
  assert.equal(groups.unresolved[0].tone, 'muted');
  assert.equal(fontGroup('unsupported'), 'failed');
  assert.equal(fontTone('unresolved'), 'muted');
  assert.deepEqual(fontSummary(groups), { total: 4, loaded: 1, failed: 2, unresolved: 1 });
  // An absent list is an empty projection, never a placeholder row.
  assert.deepEqual(projectFontRows(null), { loaded: [], failed: [], unresolved: [] });
  assert.deepEqual(fontSummary(null), { total: 0, loaded: 0, failed: 0, unresolved: 0 });
});

test('cad_waiting is an explicit loading state with one bounded follow-up read', async () => {
  const calls = [];
  const store = createFontsStore(async command => {
    calls.push(command);
    throw new Error('cad_waiting');
  }, { wait: async () => {} });
  await store.load();
  assert.equal(store.getState().phase, 'waiting');
  assert.equal(store.getState().answer, null);
  assert.equal(store.getState().error, null);
  assert.deepEqual(calls, [FONTS_COMMAND, FONTS_COMMAND], 'one read plus one follow-up, never a loop');
  assert.equal(isFontsWaiting('cad_waiting'), true);
  assert.equal(isFontsWaiting('unsupported-runtime: cad_fonts'), false);
});

test('available:false keeps the named reason and is not the empty state', async () => {
  const store = createFontsStore(async () => report({
    available: false, reason: 'host.fonts_unavailable', faces: [], defaultFace: null,
  }));
  await store.load();
  const state = store.getState();
  assert.equal(state.phase, 'unavailable');
  assert.equal(state.answer.reason, 'host.fonts_unavailable');
  assert.notEqual(state.phase, 'empty');
  assert.deepEqual(projectFontRows(state.answer.faces), { loaded: [], failed: [], unresolved: [] });
});

test('a populated report becomes ready, and an available empty report becomes empty', async () => {
  const store = createFontsStore(async () => report());
  await store.load();
  const state = store.getState();
  assert.equal(state.phase, 'ready');
  assert.equal(state.answer.faces.length, 1);
  assert.equal(store.getState().error, null);

  const emptyStore = createFontsStore(async () => report({ faces: [], catalogEntries: 0, defaultFace: null }));
  await emptyStore.load();
  assert.equal(emptyStore.getState().phase, 'empty');
  assert.notEqual(emptyStore.getState().phase, 'unavailable');
});

test('a malformed payload is an error, never an empty success', async () => {
  const store = createFontsStore(async () => ({ available: true, faces: [] }));
  await store.load();
  const state = store.getState();
  assert.equal(state.phase, 'error');
  assert.equal(state.answer, null);
  assert.match(state.error.reason, new RegExp(FONTS_PAYLOAD_INVALID));
});

test('a transport failure is reported in the host words', async () => {
  const store = createFontsStore(async () => { throw new Error('unsupported-runtime: cad_fonts'); });
  await store.load();
  assert.equal(store.getState().phase, 'error');
  assert.equal(store.getState().error.reason, 'unsupported-runtime: cad_fonts');
});

test('the panel renders the named unavailable reason before the empty state', async () => {
  const panel = await readModule('components/FontsPanel.tsx');
  assert.ok(panel.includes("t('fonts.title')"));
  assert.ok(panel.includes("t('fonts.loading')"));
  assert.ok(panel.includes("t('fonts.empty')"));
  assert.ok(panel.includes("t('fonts.read_failed', { reason: state.error?.reason ?? '' })"));
  // The named reason is spoken, through the catalog, from the answer's own key.
  assert.ok(panel.includes("t('fonts.unavailable', { reason: t(answer.reason ?? '') })"));
  const unavailableAt = panel.indexOf("state.phase === 'unavailable'");
  const emptyAt = panel.indexOf("state.phase === 'empty'");
  assert.ok(unavailableAt > 0, 'unavailable has its own branch');
  assert.ok(emptyAt > unavailableAt, 'the empty state can never swallow the unavailable reason');
  // Read-only: no reload/edit/delete affordance, and no invented action.
  assert.equal(panel.includes('RELOAD'), false);
  assert.equal(panel.includes("invoke('cad_fonts')"), false, 'the panel reads through the store, not with its own IPC call');
  const buttons = panel.match(/onClick=/g) ?? [];
  assert.equal(buttons.length, 1, 'the only control is closing the panel');
});

test('the panel is accessible in both variants and the window reuses the references language', async () => {
  const panel = await readModule('components/FontsPanel.tsx');
  assert.ok(panel.includes('aria-label={t(\'fonts.title\')}'));
  assert.ok(panel.includes("role={floating ? 'dialog' : undefined}"));
  assert.ok(panel.includes('aria-modal={floating ? true : undefined}'));
  assert.ok(panel.includes('references__row'));
  assert.ok(panel.includes('references__badge'));
  // The sections are the three closed groups, in the fixed order.
  assert.ok(panel.includes("t('fonts.section.loaded'"));
  assert.ok(panel.includes("t('fonts.section.failed'"));
  assert.ok(panel.includes("t('fonts.section.unresolved'"));
  assert.ok(panel.indexOf("groups.loaded") < panel.indexOf("groups.failed"));
  assert.ok(panel.indexOf("groups.failed") < panel.indexOf("groups.unresolved"));
  const all = await readComponentsSource();
  assert.ok(all.includes('fonts__warning'), 'the locale-missing warning has a visible surface');
});
