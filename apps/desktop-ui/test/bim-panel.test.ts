// BIM browse panel: the integration contracts that are not pure functions.
//
// Source scans and model reads only — no DOM mount. They pin the entry point
// (ribbon action, Home panel, App dispatch), both mounts (palette window and
// compact sheet), the catalog keys every dynamic lookup needs, the two carriers'
// stylesheet rules, and the two constants the contract fixes (150 ms debounce,
// the exact unsupported marker).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCatalog, readCss, readMobileCss, readModule } from './helpers.ts';
import {
  BIM_PAGE_SIZE,
  BIM_UNSUPPORTED_MARKER,
} from '../src/bim/bim-query.ts';
import { BIM_DEBOUNCE_MS } from '../src/bim/bim-store.ts';
import { BIM_ACTION } from '../src/ribbon/bim-panel.ts';
import {
  PALETTE_ACTIONS,
  RIBBON_TABS,
  offeredItems,
  offeredPanels,
} from '../src/ribbon/ribbon-model.ts';
import { BIM_PANEL } from '../src/ribbon/bim-panel.ts';

test('the panel is one Home ribbon entry, and it survives a palette-less carrier', () => {
  assert.equal(BIM_ACTION, 'bim');
  const home = RIBBON_TABS.find(tab => tab.id === 'home');
  assert.ok(home, 'the Home tab exists');
  const bim = home.panels.find(panel => panel.id === 'bim');
  assert.ok(bim, 'the Home tab carries a BIM panel');
  assert.equal(bim.titleKey, 'ribbon.panel.bim');
  assert.equal(bim.items.length, 1);
  assert.equal(bim.items[0].action, 'bim');
  assert.equal(bim.items[0].labelKey, 'bim.panel');
  assert.equal(bim.items[0].icon, 'bim');
  // Deliberately not a palette action: a carrier without a BIM query lane must
  // still show the entry so the panel can explain the refusal.
  assert.equal(PALETTE_ACTIONS.includes('bim'), false);
  assert.equal(offeredItems(bim.items, false).length, 1);
  assert.ok(offeredPanels(home.panels, false).some(panel => panel.id === 'bim'));
  // The panel object is defined once, in its own module.
  assert.equal(BIM_PANEL.id, 'bim');
});

test('App dispatches the shell action to the panel store, not to a router command', async () => {
  const app = await readModule('App.tsx');
  assert.match(app, /import \{ openBimPanel \} from '\.\/bim\/panel\.ts';/);
  assert.match(app, /action === 'bim'\) openBimPanel\(\)/);
  // The action is chrome-local: nothing about it rides `cad_command`.
  assert.doesNotMatch(app, /action === 'bim'\)[^\n]*runCommand/);
});

test('Workspace mounts one panel per layout: palette window on desktop, sheet on compact', async () => {
  const workspace = await readModule('components/Workspace.tsx');
  assert.match(workspace, /import \{ BimPanel \} from '\.\/BimPanel\.tsx';/);
  assert.match(workspace, /const bimOpen = useBimPanelOpen\(\);/);
  assert.match(workspace, /const showBim = palettesAvailable && bimOpen;/);
  assert.match(workspace, /windowKey="bim"/);
  assert.match(workspace, /showBim && !shell\.mobile/);
  assert.match(workspace, /showBim && shell\.mobile/);
  assert.match(workspace, /className="bim-layer"/);
  assert.match(workspace, /className="bim-layer__backdrop"/);
});

test('the panel owns every state the task names, plus Escape and the raw marker', async () => {
  const panel = await readModule('components/BimPanel.tsx');
  // First read skeleton, later busy bar, both driven by the store phase.
  assert.match(panel, /bim__skeleton/);
  assert.match(panel, /bim__progress/);
  assert.match(panel, /aria-busy=\{busy \|\| undefined\}/);
  // The unsupported state shows the host's raw marker and renders no table.
  assert.match(panel, /state\.phase === 'unsupported'/);
  assert.match(panel, /state\.error\?\.reason \?\? BIM_UNSUPPORTED_MARKER/);
  // Waiting, first-read error, empty and filtered-empty are separate branches.
  assert.match(panel, /state\.phase === 'waiting'/);
  assert.match(panel, /bimEmptyCopy/);
  // Stale is a banner with a refresh, never a blanked table.
  assert.match(panel, /state\.page\?\.stale/);
  assert.match(panel, /bim\.stale/);
  // Selection refusals are shown verbatim and can be dismissed.
  assert.match(panel, /bim\.select_failed/);
  assert.match(panel, /clearSelectionError/);
  // Escape closes the panel while its own tree has the keyboard.
  assert.match(panel, /event\.key !== 'Escape'/);
  assert.match(panel, /closeBimPanel\(\)/);
  // Facet chips are real toggles; rows are real buttons.
  assert.match(panel, /aria-pressed=\{active\}/);
  assert.match(panel, /className="bim__row"/);
  assert.match(panel, /store\.toggleFacet\(group\.kind, facet\.key\)/);
});

test('the contract constants are the fixed ones', () => {
  assert.equal(BIM_UNSUPPORTED_MARKER, 'unsupported-runtime: bimQuery');
  assert.equal(BIM_PAGE_SIZE, 100);
  assert.equal(BIM_DEBOUNCE_MS, 150);
});

test('both catalogs carry every BIM key, with matching placeholders', async () => {
  const en = await readCatalog('en');
  const zh = await readCatalog('zh-CN');
  const keys = Object.keys(en).filter(key => key.startsWith('bim.'));
  assert.ok(keys.length >= 45, `expected the BIM surface keys, saw ${keys.length}`);
  for (const key of keys) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.ok(zh[key].trim().length > 0, key);
  }
  assert.equal(en['ribbon.panel.bim'], 'BIM');
  assert.equal(en['bim.value.unset'], 'Unspecified');
  assert.equal(zh['bim.value.unset'], '未标注');
  // Dynamic lookups the static checker cannot see:
  for (const key of [
    'bim.evidence.declared',
    'bim.evidence.heuristic',
    'bim.property.kind.text',
    'bim.property.kind.real',
    'bim.property.kind.integer',
    'bim.property.kind.bool',
    'bim.property.op.eq',
    'bim.property.op.ne',
    'bim.property.op.contains',
    'bim.facet.category',
    'bim.facet.level',
    'bim.facet.type',
    'bim.facet.material',
    'bim.empty',
    'bim.empty_hint',
    'bim.no_match',
    'bim.no_match_hint',
  ]) {
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
  }
  // Placeholders are identical in both languages (the translator's contract).
  for (const key of ['bim.property.applied', 'bim.page.range', 'bim.page.index', 'bim.row.parts', 'bim.row.parts_hint', 'bim.facet.filter_hint', 'bim.summary.total', 'bim.summary.matched']) {
    const names = text => [...text.matchAll(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g)].map(match => match[1]).sort();
    assert.deepEqual(names(en[key]), names(zh[key]), key);
  }
});

test('the table never overflows sideways: grid columns are minmax(0, …) and chips wrap', async () => {
  const css = await readCss();
  assert.match(css, /\.bim__row \{[^}]*grid-template-columns:\s*minmax\(0, 1\.1fr\) minmax\(0, 1fr\)/s);
  assert.match(css, /\.bim__chips \{[^}]*flex-wrap:\s*wrap/s);
  assert.match(css, /\.bim__row-main \{[^}]*min-width:\s*0/s);
  assert.match(css, /\.bim__name \{[^}]*text-overflow:\s*ellipsis/s);
  // The filter strip scrolls vertically at most; nothing asks for a sideways
  // scroller inside the panel.
  assert.doesNotMatch(css, /\.bim__chips \{[^}]*overflow-x/s);
});

test('the compact sheet reuses the references/fonts sheet rules, full width', async () => {
  const mobile = await readMobileCss();
  assert.match(mobile, /\.bim-layer \{/);
  assert.match(mobile, /\.bim-layer__backdrop \{/);
  assert.match(mobile, /\.bim--floating \{/);
  assert.match(mobile, /\.bim--floating[^{]*\{[^}]*width:\s*100%/s);
  assert.match(mobile, /\.bim--floating \.bim__input \{[^}]*height:\s*30px/s);
});
