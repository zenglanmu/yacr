// Contract tests for the two new chrome floats (2026-11-12):
// `TOOLPALETTES`/`TOOLPALETTESCLOSE`/`TPNAVIGATE` and
// `PERFANALYZER`/`PERFANALYZERCLOSE`, plus the three File report commands
// (`RECOVER`, `RECOVERALL`, `PURGEAECDATA`).
//
// Everything here is a pure model, store or source-structure assertion: no React
// DOM is mounted, no host runs and no pixels are produced. What is pinned is the
// part that would be wrong silently — a word that leaked to `cad_command`, a
// close that stopped being idempotent, a tab name that opened a panel it should
// refuse, a missing reading rendered as `0`, and a report row added to the
// refusal catalog.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  PERFANALYZER_COMMAND,
  PERFANALYZERCLOSE_COMMAND,
  TOOL_PALETTES_BLOCKS_TAB,
  TOOLPALETTES_COMMAND,
  TOOLPALETTESCLOSE_COMMAND,
  TPNAVIGATE_COMMAND,
} from '../src/commands/chrome-commands.ts';
import { localSelectorFor, toolPalettesTabRefusalKey } from '../src/commands/local-selectors.ts';
import { LOCAL_CHROME_COMMANDS } from '../src/commands/gating.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { REFUSAL_COMMAND_CATALOG } from '../src/commands/catalog-refusals.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';
import { createTranslator } from '../src/i18n.ts';
import {
  chromeSurfaceSnapshot,
  closePerformanceAnalyzer,
  closeToolPalettes,
  openPerformanceAnalyzer,
  openToolPalettes,
  openQuickCalc,
  closeQuickCalc,
  resetChromeSurfaces,
} from '../src/chrome/surfaces.ts';
import {
  PERFORMANCE_NA_KEY,
  documentCountRows,
  performanceMetricRows,
} from '../src/performance/analyzer.ts';
import { readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const t = createTranslator({ 'zh-CN': zh, en }, 'en');

const LOCAL_WORDS = [
  TOOLPALETTES_COMMAND,
  TOOLPALETTESCLOSE_COMMAND,
  TPNAVIGATE_COMMAND,
  PERFANALYZER_COMMAND,
  PERFANALYZERCLOSE_COMMAND,
];

/** Every line this suite expects the chrome — never the router — to answer. */
const LOCAL_LINES = [
  'TOOLPALETTES', 'TOOLPALETTESCLOSE',
  'TPNAVIGATE Blocks', 'TPNAVIGATE TOOLS', 'TPNAVIGATE',
  'PERFANALYZER', 'PERFANALYZERCLOSE',
];

const NEW_KEYS = [
  'toolpalettes.title', 'toolpalettes.close', 'toolpalettes.subtitle',
  'toolpalettes.tab.blocks', 'toolpalettes.tab.only_blocks', 'toolpalettes.empty',
  'toolpalettes.refusal.no_tab', 'toolpalettes.refusal.only_blocks',
  'performance.title', 'performance.close', 'performance.subtitle', 'performance.empty',
  'performance.unavailable', 'performance.section.performance', 'performance.section.counts',
  'performance.na', 'performance.metric.gpuReserved',
  'performance.count.entities', 'performance.count.selection', 'performance.count.blocks',
  'performance.count.layouts', 'performance.count.references',
  'performance.session.frames', 'performance.session.imports', 'performance.hint',
  'draw.kind.toolpalettes', 'draw.kind.toolpalettes.hint',
  'draw.kind.toolpalettesclose', 'draw.kind.toolpalettesclose.hint',
  'draw.kind.tpnavigate', 'draw.kind.tpnavigate.hint',
  'draw.kind.perfanalyzer', 'draw.kind.perfanalyzer.hint',
  'draw.kind.perfanalyzerclose', 'draw.kind.perfanalyzerclose.hint',
  'draw.kind.recover', 'draw.kind.recover.hint',
  'draw.kind.recoverall', 'draw.kind.recoverall.hint',
  'draw.kind.purgeaecdata', 'draw.kind.purgeaecdata.hint',
];

test('the five words are chrome-local on every layout and always resolve locally', () => {
  assert.deepEqual(LOCAL_WORDS, [
    'TOOLPALETTES', 'TOOLPALETTESCLOSE', 'TPNAVIGATE', 'PERFANALYZER', 'PERFANALYZERCLOSE',
  ]);
  for (const word of LOCAL_WORDS) {
    assert.equal(LOCAL_CHROME_COMMANDS.includes(word), true, `${word} is chrome-local`);
    for (const layout of ['desktop', 'mobile']) {
      assert.notEqual(localSelectorFor(word, layout), null, `${word} on ${layout}`);
      // Case and surrounding whitespace are the same selector (the command line
      // normalizes before it looks anything up).
      assert.notEqual(localSelectorFor(`  ${word.toLowerCase()}  `, layout), null, `${word} normalized`);
    }
  }
});

test('no line of this family is forwarded to the router', async () => {
  // Every form — including the bare tab word and a tab this build refuses —
  // resolves to a chrome result, so `runLocalSelector` answers it and the one
  // `cad_command` lane is never reached.
  for (const line of LOCAL_LINES) {
    assert.notEqual(localSelectorFor(line, 'desktop'), null, line);
    assert.notEqual(localSelectorFor(line, 'mobile'), null, line);
  }
  const app = await readModule('App.tsx');
  const interceptor = app.indexOf('if (runLocalSelector(text, options.fromInput === true)) return;');
  const router = app.indexOf('queueCadCommand(invoke, text, status, false,');
  assert.ok(interceptor > 0, 'the one local interceptor runs in the submit path');
  assert.ok(router > interceptor, 'it returns before the router lane is reached');
  assert.equal((app.match(/queueCadCommand\(/g) ?? []).length, 1, 'one router lane only');
});

test('both close words are an idempotent success, and one float closes the other', () => {
  resetChromeSurfaces();
  // Already closed: still a success, not a refusal.
  closeToolPalettes();
  closePerformanceAnalyzer();
  closeToolPalettes();
  assert.equal(chromeSurfaceSnapshot().toolPalettes, false);

  openToolPalettes();
  assert.equal(chromeSurfaceSnapshot().toolPalettes, true);
  const firstFocus = chromeSurfaceSnapshot().toolPalettesFocus;
  openToolPalettes();
  assert.equal(chromeSurfaceSnapshot().toolPalettesFocus, firstFocus + 1, 'a repeat open is a focus request');
  assert.equal(chromeSurfaceSnapshot().toolPalettes, true, 'a repeat open keeps the float up');
  closeToolPalettes();
  closeToolPalettes();
  assert.equal(chromeSurfaceSnapshot().toolPalettes, false, 'TOOLPALETTESCLOSE is idempotent');

  openPerformanceAnalyzer();
  assert.equal(chromeSurfaceSnapshot().performanceAnalyzer, true);
  closePerformanceAnalyzer();
  closePerformanceAnalyzer();
  assert.equal(chromeSurfaceSnapshot().performanceAnalyzer, false, 'PERFANALYZERCLOSE is idempotent');

  // Only one chrome float is ever open: opening the analyzer closes the palette
  // (and the calculator closes both), and closing the other float is a no-op.
  openToolPalettes();
  openPerformanceAnalyzer();
  assert.equal(chromeSurfaceSnapshot().toolPalettes, false);
  assert.equal(chromeSurfaceSnapshot().performanceAnalyzer, true);
  closeToolPalettes();
  assert.equal(chromeSurfaceSnapshot().performanceAnalyzer, true);
  openToolPalettes();
  openQuickCalc();
  assert.equal(chromeSurfaceSnapshot().toolPalettes, false, 'QuickCalc is a float too');
  closeQuickCalc();
  resetChromeSurfaces();
});

test('TPNAVIGATE accepts only Blocks and refuses every other tab in words', () => {
  assert.equal(TOOL_PALETTES_BLOCKS_TAB, 'BLOCKS');
  for (const text of ['TPNAVIGATE Blocks', 'tpnavigate blocks', '  TPNAVIGATE   bLoCkS  ']) {
    assert.deepEqual(localSelectorFor(text, 'desktop'), { kind: 'toolPalettesTab', tab: 'BLOCKS' }, text);
  }
  assert.equal(toolPalettesTabRefusalKey('BLOCKS'), null, 'the tab this build has is not refused');

  for (const tab of ['TOOLS', 'PROPERTIES', 'PATTERNS']) {
    // The line is still the chrome's: it resolves locally and carries the tab it
    // was given, which is what lets the dispatch refuse it instead of routing it.
    assert.deepEqual(localSelectorFor(`TPNAVIGATE ${tab}`, 'mobile'), { kind: 'toolPalettesTab', tab });
    const key = toolPalettesTabRefusalKey(tab);
    assert.equal(typeof key, 'string', `${tab} has a refusal key`);
    const sentence = t(key, { tab });
    assert.match(sentence, /Blocks/, `${tab}: the sentence names the tab that exists`);
    assert.match(sentence, new RegExp(tab), `${tab}: the sentence names the refused tab`);
    assert.equal(/Named refusal/i.test(sentence), false, 'a tab refusal is a clear sentence, not a machine code');
  }
  // The bare word names no tab; it is refused with its own sentence.
  assert.equal(toolPalettesTabRefusalKey(''), 'toolpalettes.refusal.no_tab');
  assert.match(t('toolpalettes.refusal.no_tab'), /Blocks/);
});

test('a performance reading the host did not measure is N/A, never 0', () => {
  assert.equal(PERFORMANCE_NA_KEY, 'performance.na');
  const na = t('performance.na');
  assert.equal(na, 'N/A');

  // No block at all: every metric row is the N/A word and says so.
  const absent = performanceMetricRows(null, t);
  assert.deepEqual(absent.map(row => row.id), ['cpu', 'rss', 'gpu', 'gpuPeak', 'gpuReserved', 'vram']);
  for (const row of absent) {
    assert.equal(row.unavailable, true, row.id);
    assert.equal(row.text, na, row.id);
  }

  // A real zero is a measurement: 0% CPU and 0 bytes are formatted, not N/A.
  const partial = performanceMetricRows({
    cpuPercent: 0,
    processRssBytes: 0,
    processVirtualBytes: null,
    systemTotalBytes: null,
    systemAvailableBytes: null,
    gpuMemoryUsedBytes: null,
    gpuMemoryPeakBytes: null,
    gpuMemoryReservedBytes: 256 * 1024 * 1024,
    vramTotalBytes: 8 * 1024 ** 3,
  }, t);
  const byId = Object.fromEntries(partial.map(row => [row.id, row]));
  assert.equal(byId.cpu.text, '0.0%');
  assert.equal(byId.cpu.unavailable, false, 'a measured 0 is not N/A');
  assert.equal(byId.rss.text, '0 B');
  assert.equal(byId.gpu.text, na);
  assert.equal(byId.gpu.unavailable, true);
  assert.equal(byId.gpuReserved.text, '256 MiB');
  assert.equal(byId.vram.text, '8 GiB');
});

test('the drawing/session counts are the real fields, and a null count is N/A', () => {
  const status = {
    entityCount: null,
    selection: { count: 2 },
    blocks: [
      { name: 'Widget', memberCount: 2, basePoint: [1, 2, 3] },
      { name: null, memberCount: 0, basePoint: null },
    ],
    frameCount: 9,
    importCount: 1,
  };
  const rows = documentCountRows(status, t);
  const byId = Object.fromEntries(rows.map(row => [row.id, row]));
  assert.equal(byId.entities.text, t('performance.na'), 'a null entity count is N/A');
  assert.equal(byId.entities.unavailable, true);
  assert.equal(byId.selection.text, '2');
  assert.equal(byId.blocks.text, '2');
  // An absent list is the documented empty list, so a real 0 is reported as 0.
  assert.equal(byId.layouts.text, '0');
  assert.equal(byId.references.text, '0');
  assert.equal(byId.frames.text, '9');
  assert.equal(byId.imports.text, '1');
  // No status at all: the panel renders its empty state, and the counters the
  // shell cannot know (entities, frames, imports) stay N/A.
  const none = Object.fromEntries(documentCountRows(null, t).map(row => [row.id, row]));
  assert.equal(none.entities.unavailable, true);
  assert.equal(none.frames.unavailable, true);
  assert.equal(none.imports.unavailable, true);
});

test('the block list is one shared surface: same rows, same INSERT flow', async () => {
  const toolPalettes = await readModule('components/ToolPalettesPanel.tsx');
  const classic = await readModule('components/ClassicInsertPanel.tsx');
  for (const [name, source] of [['ToolPalettesPanel', toolPalettes], ['ClassicInsertPanel', classic]]) {
    assert.ok(source.includes("from '../classic/classic-rows.ts'"), `${name} reads the authoritative block rows`);
    assert.ok(source.includes('blockRows('), `${name} projects cad_status.blocks`);
    assert.ok(source.includes("from './BlockList.tsx'"), `${name} renders the shared row list`);
    assert.ok(source.includes('<BlockList'), `${name} mounts the shared row list`);
    assert.ok(source.includes('onInsert={onInsert}'), `${name} arms the same INSERT flow`);
  }
  const insert = await readModule('classic/use-insert-block.ts');
  assert.ok(insert.includes('closeClassicPanel();'), 'the classic panel still closes on insert');
  assert.ok(insert.includes('closeToolPalettes();'), 'the Tool Palettes closes on the same insert');
});

test('the surfaces renderer mounts both floats, and the shell stands down from Escape', async () => {
  const surfaces = await readModule('components/ChromeSurfaces.tsx');
  assert.ok(surfaces.includes('<ToolPalettesPanel'));
  assert.ok(surfaces.includes('focusRevision={state.toolPalettesFocus}'), 'a repeat open re-focuses the panel');
  assert.ok(surfaces.includes('onClose={closeToolPalettes}'));
  assert.ok(surfaces.includes('<PerformanceAnalyzerPanel onClose={closePerformanceAnalyzer} />'));
  const app = await readModule('App.tsx');
  assert.ok(app.includes('openToolPalettes, closeToolPalettes,'));
  assert.ok(app.includes('setPerformanceAnalyzer: (open: boolean) =>'));
  // The floats own Escape (their own listener closes them), so the shell's
  // global handler stands down exactly like it does for Quick View.
  assert.ok(app.includes('chromeSurfaces.toolPalettes || chromeSurfaces.performanceAnalyzer'));
  // Both panels are non-modal chrome floats: the shared focus hook, no scrim
  // element and no focus-trap machinery.
  for (const [name, source] of [
    ['ToolPalettesPanel', await readModule('components/ToolPalettesPanel.tsx')],
    ['PerformanceAnalyzerPanel', await readModule('components/PerformanceAnalyzerPanel.tsx')],
  ]) {
    assert.ok(source.includes('useSurfacePanelFocus(onClose)'), `${name} uses the shared focus/Escape hook`);
    assert.equal(source.includes('focusTrap'), false, `${name} has no focus trap`);
    assert.equal(/className="[^"]*backdrop/.test(source), false, `${name} paints no scrim/backdrop`);
  }
});

test('the three report commands are File catalog rows, never refusals', () => {
  const rows = [
    ['RECOVER', 'diagnostics'],
    ['RECOVERALL', 'open'],
    ['PURGEAECDATA', 'erase'],
  ];
  for (const [id, icon] of rows) {
    const found = COMMAND_CATALOG.filter(row => row.id === id);
    assert.equal(found.length, 1, `${id} is catalogued exactly once`);
    assert.deepEqual(
      { ...found[0] },
      { id, labelKey: `draw.kind.${id.toLowerCase()}`, icon, group: 'file' },
      `${id} is a File-group report with an existing glyph`,
    );
    assert.equal(REFUSAL_COMMAND_CATALOG.some(row => row.id === id), false, `${id} is a real report`);
    assert.equal(typeof en[`draw.kind.${id.toLowerCase()}`], 'string', `${id} has an English label`);
    assert.equal(typeof en[`draw.kind.${id.toLowerCase()}.hint`], 'string', `${id} has an English hint`);
    assert.equal(typeof zh[`draw.kind.${id.toLowerCase()}`], 'string', `${id} has a Chinese label`);
  }
  // SHOWURLS is a command-line-only AutoCAD name and must not be offered here.
  assert.equal(COMMAND_CATALOG.some(row => row.id === 'SHOWURLS'), false);
  assert.equal(typeof en['draw.kind.showurls'], 'undefined');
  assert.equal(typeof zh['draw.kind.showurls'], 'undefined');
  // The File tab's utilities area carries RECOVER only; the other two reports are
  // catalog/command-line entries.
  const file = RIBBON_TABS.find(tab => tab.id === 'file');
  const utilities = file.panels.find(panel => panel.id === 'utilities');
  const ids = utilities.items.map(item => item.id);
  assert.equal(ids.includes('recover'), true, 'RECOVER sits in the File utilities panel');
  assert.equal(utilities.items.find(item => item.id === 'recover').command, 'RECOVER');
  for (const other of ['recoverall', 'purgeaecdata']) {
    assert.equal(ids.includes(other), false, `${other} stays command-line only`);
  }
});

test('every new catalog key exists in both languages with the same placeholders', () => {
  const placeholder = /(?<!\\)\{([A-Za-z_][A-Za-z0-9_]*)\}/g;
  for (const key of NEW_KEYS) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.ok(zh[key].trim().length > 0, `zh-CN empty ${key}`);
    assert.ok(en[key].trim().length > 0, `en empty ${key}`);
    const a = [...zh[key].matchAll(placeholder)].map(match => match[1]).sort();
    const b = [...en[key].matchAll(placeholder)].map(match => match[1]).sort();
    assert.deepEqual(a, b, `placeholders differ for ${key}`);
  }
  // The catalog rows the chrome test's slug rule expects are the keys added here.
  for (const word of LOCAL_WORDS) {
    assert.ok(NEW_KEYS.includes(`draw.kind.${word.toLowerCase()}`), `${word} owns its label key`);
    assert.ok(NEW_KEYS.includes(`draw.kind.${word.toLowerCase()}.hint`), `${word} owns its hint key`);
  }
});
