// Contract test for the fixed-height ribbon band. The band must not change
// height when the active tab changes (or when a variant popover opens), because
// the host samples the CAD partition rect from this chrome. These assertions
// lock the CSS token, the horizontal small-item flow, the in-band variant
// popover and the dismissal wiring, so a future edit cannot silently reintroduce
// the tab-to-tab height jump.
//
// Two reader-facing contracts are locked here as well, both as model/CSS
// assertions rather than screenshots:
//
//   * the standard views stack each semantic pair in one column (top/bottom,
//     front/back, left/right) with isometric as the odd one out;
//   * the panel strip carries an edge cue on exactly the sides that hide
//     content, decided by the pure `ribbonOverflow` predicate.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCatalog, readCss, readModule } from './helpers.ts';
import { columnMajorCells, pairsShareEdge, ribbonTab } from '../src/ribbon-model.ts';
import { STANDARD_VIEW_PAIRS, STANDARD_VIEWS } from '../src/view3d.ts';
import {
  OVERFLOW_EPSILON, RIBBON_CUE_WIDTH, ribbonBandOverflow, ribbonCueLayout, ribbonCueReserve,
  ribbonOverflow, ribbonScrollStep, ribbonScrollTarget, sameOverflow,
} from '../src/ribbon-overflow.ts';

function block(css, selector) {
  const match = css.match(new RegExp(`${selector}\\s*\\{([^}]*)\\}`));
  assert.ok(match, `expected a rule for ${selector}`);
  return match[1];
}

test('the ribbon band height is one token, identical on every tab', async () => {
  const css = await readCss();
  assert.match(css, /--ribbon-h:\s*\d+px/, 'expected a --ribbon-h token');
  const panels = block(css, '\\.ribbon__panels');
  assert.match(panels, /height:\s*var\(--ribbon-h\)/, 'the band must take its height from the token');
  // Regression guard: the old content-driven floor is what let tiles grow the band.
  assert.doesNotMatch(panels, /min-height:\s*80px/);
});

test('small items flow horizontally instead of one tall column', async () => {
  const css = await readCss();
  const small = block(css, '\\.ribbon-panel__group\\.is-small');
  assert.match(small, /display:\s*grid/);
  assert.match(small, /grid-auto-flow:\s*column/);
  assert.match(small, /grid-template-rows:/);
  assert.doesNotMatch(small, /flex-direction:\s*column/);
  // A compact icon is required so a small row fits the fixed band.
  assert.match(css, /\.ribbon-panel__group\.is-small \.ribbon-button \.icon\s*\{[^}]*width:\s*16px/);
});

test('the variant menu is an in-band popover that cannot grow the band', async () => {
  const css = await readCss();
  const menu = block(css, '\\.ribbon-panel__menu');
  assert.match(menu, /position:\s*absolute/);
  assert.match(menu, /max-height:\s*100%/);
  assert.match(menu, /overflow:\s*auto/);
  // It is confined to the panel body, which is the positioning context.
  const body = block(css, '\\.ribbon-panel__body');
  assert.match(body, /position:\s*relative/);
});

test('Ribbon renders the menu inside the panel body and marks the open item', async () => {
  const source = await readModule('components/Ribbon.tsx');
  const bodyAt = source.indexOf('ribbon-panel__body');
  const menuAt = source.indexOf('<VariantMenu');
  const titleAt = source.indexOf('className="ribbon-panel__title"');
  assert.ok(bodyAt !== -1 && menuAt !== -1 && titleAt !== -1);
  assert.ok(menuAt > bodyAt && menuAt < titleAt, 'the menu must render inside the panel body');
  assert.ok(source.includes("menuOpen ? 'is-open' : ''"), 'the invoking item must be marked open');
});

test('the variant popover is dismissible and keeps its accessibility roles', async () => {
  const source = await readModule('components/Ribbon.tsx');
  assert.ok(source.includes("event.key === 'Escape'"), 'Escape must dismiss the popover');
  assert.ok(source.includes("closest?.('.ribbon-panel__menu')"), 'an outside click must dismiss the popover');
  for (const role of ['role="tab"', 'role="tabpanel"', 'role="menu"', 'role="menuitem"']) {
    assert.ok(source.includes(role), `expected ${role} to be preserved`);
  }
  assert.ok(source.includes('aria-expanded={menuOpen}'));
});

test('the standard views stack each semantic pair in one column', () => {
  const panel = ribbonTab('view').panels.find(entry => entry.id === 'standard');
  assert.ok(panel, 'the View tab must carry the standard-view panel');
  // The layout contract lives on the model, so a reader sees it in the code too.
  assert.equal(panel.rows, 2, 'the standard panel must flow column-major over two rows');
  assert.equal(panel.items.length, STANDARD_VIEWS.length);
  assert.deepEqual(
    panel.items.map(item => item.standardView),
    STANDARD_VIEWS.map(item => item.standardView),
    'the reflow must not reorder the views',
  );
  for (const pair of STANDARD_VIEW_PAIRS) {
    for (const name of pair) {
      assert.ok(panel.items.some(item => item.standardView === name), `missing ${name}`);
    }
  }
  assert.equal(
    pairsShareEdge(panel.items, panel.rows, STANDARD_VIEW_PAIRS, item => item.standardView),
    true,
    'top/bottom, front/back and left/right must each share an edge',
  );
  // Meaningful guard: the three-row flow this replaced split every pair, so a
  // regression back to `rows: 3` fails the property above rather than passing it
  // vacuously.
  assert.equal(pairsShareEdge(panel.items, 3, STANDARD_VIEW_PAIRS, item => item.standardView), false);
});

test('isometric is the odd one out in its own column', () => {
  const panel = ribbonTab('view').panels.find(entry => entry.id === 'standard');
  const cells = columnMajorCells(panel.items, panel.rows);
  const columns = new Map();
  for (const cell of cells) columns.set(cell.column, [...(columns.get(cell.column) ?? []), cell]);
  const last = columns.get(Math.max(...columns.keys()));
  assert.equal(last.length, 1, 'the last column holds the single odd view');
  assert.equal(last[0].item.standardView, 'isometric');
  for (const [column, entries] of columns) {
    if (column === Math.max(...columns.keys())) continue;
    assert.equal(entries.length, 2, `column ${column} must hold one pair`);
    assert.deepEqual(entries.map(entry => entry.row), [0, 1], `column ${column} must fill both rows`);
  }
});

test('the overflow cue appears only on a side that really hides panels', () => {
  assert.equal(ribbonOverflow({ scrollWidth: 900, clientWidth: 900 }).overflowing, false);
  // Sub-pixel layout slack is not overflow: no cue on a maximised window.
  assert.equal(ribbonOverflow({ scrollWidth: 901, clientWidth: 900 }).overflowing, false);
  assert.equal(ribbonOverflow({ scrollWidth: 900 + OVERFLOW_EPSILON, clientWidth: 900 }).overflowing, false);
  assert.deepEqual(ribbonOverflow({ scrollWidth: 1800, clientWidth: 960, scrollLeft: 0 }),
    { overflowing: true, start: false, end: true });
  assert.deepEqual(ribbonOverflow({ scrollWidth: 1800, clientWidth: 960, scrollLeft: 420 }),
    { overflowing: true, start: true, end: true });
  assert.deepEqual(ribbonOverflow({ scrollWidth: 1800, clientWidth: 960, scrollLeft: 840 }),
    { overflowing: true, start: true, end: false });
  // Overscroll (negative offset, as WebKit reports on RTL/rubber-band) is not a
  // reason to hide the cue for the end the reader has not reached.
  assert.deepEqual(ribbonOverflow({ scrollWidth: 1800, clientWidth: 960, scrollLeft: -40 }),
    { overflowing: true, start: false, end: true });
  // No strip (first paint, collapsed ribbon, jsdom): nothing to page.
  assert.deepEqual(ribbonOverflow(), { overflowing: false, start: false, end: false });
});

test('one cue click pages most of a viewport and never past either end', () => {
  const strip = { scrollWidth: 1800, clientWidth: 960, scrollLeft: 0 };
  assert.equal(ribbonScrollStep(960), 768);
  assert.equal(ribbonScrollStep(40), 120, 'a narrow strip still takes a visible step');
  assert.equal(ribbonScrollStep(0), 0);
  assert.equal(ribbonScrollTarget(strip, 'end'), 768);
  assert.equal(ribbonScrollTarget({ ...strip, scrollLeft: 700 }, 'end'), 840, 'clamped at the end');
  assert.equal(ribbonScrollTarget({ ...strip, scrollLeft: 300 }, 'start'), 0, 'clamped at the start');
  assert.equal(ribbonScrollTarget({ ...strip, scrollLeft: 300 }, 'sideways'), 300);
});

test('the cue is measured against the band, so it cannot latch on', () => {
  // While the cue is shown the strip is 30px narrower. Measuring inside that
  // narrowed box would keep reporting overflow after the window grew back,
  // because clientWidth lost the reserve; the band width is what the reader sees.
  const content = 950;
  const band = 960;
  assert.equal(ribbonOverflow({ scrollWidth: content, clientWidth: band - RIBBON_CUE_WIDTH }).overflowing, true,
    'the narrowed strip alone would keep the cue alive');
  assert.equal(ribbonBandOverflow({ scrollWidth: content, bandWidth: band }).overflowing, false,
    'measured against the band, the content fits and no cue is shown');
  // Still a real overflow when the content genuinely exceeds the band.
  assert.equal(ribbonBandOverflow({ scrollWidth: 1200, bandWidth: band }).end, true);
});

test('an unchanged overflow state reuses the previous object', () => {
  const overflowing = ribbonOverflow({ scrollWidth: 1800, clientWidth: 960, scrollLeft: 0 });
  // A sub-pixel scroll nudge still reports the same three booleans, so the
  // renderer can keep its previous object and skip a re-render on every frame of
  // a smooth scroll.
  const same = ribbonOverflow({ scrollWidth: 1800, clientWidth: 960, scrollLeft: 0.5 });
  assert.equal(sameOverflow(overflowing, same), true);
  const flipped = ribbonOverflow({ scrollWidth: 1800, clientWidth: 960, scrollLeft: 840 });
  assert.equal(sameOverflow(overflowing, flipped), false);
  assert.equal(sameOverflow(overflowing, null), false);
});

test('the overflow cue is an overlay button inside the fixed band', async () => {
  const css = await readCss();
  const band = block(css, '\\.ribbon__band');
  assert.match(band, /position:\s*relative/, 'the band must position the cues');
  assert.doesNotMatch(band, /height:/, 'the band wrapper must not add height');
  const cue = block(css, '\\.ribbon__overflow');
  assert.match(cue, /position:\s*absolute/);
  assert.match(cue, /top:\s*0/);
  assert.match(cue, /bottom:\s*0/, 'the cue must stay inside the band');
  assert.match(css, /\.ribbon__overflow\.is-start\s*\{[^}]*left:\s*0/, 'start cue hugs the left edge');
  assert.match(css, /\.ribbon__overflow\.is-end\s*\{[^}]*right:\s*0/, 'end cue hugs the right edge');
  // The cue is only as wide as the band the strip gave up for it.
  assert.match(css, /\.ribbon__overflow\.is-end\s*\{[^}]*width:\s*var\(--ribbon-cue-end/, 'the end cue takes the reserved width');
  assert.match(css, /\.ribbon__overflow\.is-start\s*\{[^}]*width:\s*var\(--ribbon-cue-start/, 'the start cue takes the reserved width');
  // Nothing is painted behind or beside the chevron: the old edge fade ghosted
  // whole items and, over flat band, laid 1-LSB banding columns across the full
  // band height. The reserve replaced both.
  assert.doesNotMatch(cue, /gradient/, 'no gradient under the chevron: nothing is behind it');
  assert.doesNotMatch(css, /\.ribbon__fade/, 'no fade overlay in the band');
  // The idle overlay scrollbar drew nothing here and covered the panel titles
  // when it did; the cue replaces it.
  assert.match(block(css, '\\.ribbon__panels'), /scrollbar-width:\s*none/);
  assert.match(block(css, '\\.ribbon__panels'), /margin-right:\s*var\(--ribbon-cue-end/, 'the strip gives up the cue width');
});

test('the cue reserves its own width out of the strip, so it covers no item', () => {
  const band = { clientWidth: 960, scrollWidth: 1800, scrollLeft: 0 };
  const layout = ribbonCueLayout(band);
  assert.equal(layout.reserve.end, RIBBON_CUE_WIDTH, 'the hidden side reserves exactly the cue width');
  assert.equal(layout.reserve.start, 0, 'a side with no hidden content reserves nothing');
  // The strip clips at the inner edge of the reserve: the last visible item ends
  // at or before this edge, so nothing is painted under the cue.
  assert.equal(layout.strip.right, band.clientWidth - RIBBON_CUE_WIDTH);
  assert.equal(layout.hit.end.left, layout.strip.right, 'the hit box starts where the strip ends');
  assert.equal(layout.hit.end.right, band.clientWidth);
  assert.equal(layout.hit.start.left, layout.hit.start.right, 'no start hit box without start overflow');
  // Both sides: same contract mirrored.
  const both = ribbonCueLayout({ clientWidth: 960, scrollWidth: 1800, scrollLeft: 400 });
  assert.deepEqual(both.reserve, { start: RIBBON_CUE_WIDTH, end: RIBBON_CUE_WIDTH });
  assert.equal(both.strip.left, RIBBON_CUE_WIDTH);
  assert.equal(both.strip.right, 960 - RIBBON_CUE_WIDTH);
  assert.equal(both.hit.start.right, both.strip.left);
  assert.equal(both.hit.end.left, both.strip.right);
  // Nothing hidden: full-width strip, no reserve, no hit box anywhere.
  const idle = ribbonCueLayout({ clientWidth: 1853, scrollWidth: 1853 });
  assert.deepEqual(idle.reserve, { start: 0, end: 0 });
  assert.deepEqual(idle.strip, { left: 0, right: 1853, width: 1853 });
});

test('no cue hit box ever intersects the strip content box', () => {
  const states = [
    { clientWidth: 960, scrollWidth: 1800, scrollLeft: 0 },
    { clientWidth: 960, scrollWidth: 1800, scrollLeft: 400 },
    { clientWidth: 960, scrollWidth: 1800, scrollLeft: 840 },
    { clientWidth: 1853, scrollWidth: 1853, scrollLeft: 0 },
    // Sub-pixel slack: no cue, so nothing is reserved for one.
    { clientWidth: 900, scrollWidth: 901, scrollLeft: 0 },
  ];
  for (const metrics of states) {
    const layout = ribbonCueLayout(metrics);
    for (const side of ['start', 'end']) {
      const box = layout.hit[side];
      const overlaps = box.right > layout.strip.left && box.left < layout.strip.right;
      assert.equal(overlaps, false, `${side} cue over the strip for ${JSON.stringify(metrics)}`);
    }
    // The reserve is all-or-nothing per side: either the cue's width or nothing.
    for (const side of ['start', 'end']) {
      assert.ok([0, RIBBON_CUE_WIDTH].includes(layout.reserve[side]), `${side} reserve must be 0 or the cue width`);
    }
  }
});


test('the ribbon binds the cue to a real measurement, not to a fixed state', async () => {
  const source = await readModule('components/Ribbon.tsx');
  assert.ok(source.includes('ribbonBandOverflow({'), 'the cue state must come from the strip metrics');
  assert.ok(source.includes('node.parentElement'), 'the measurement must use the band, not the narrowed strip');
  assert.ok(source.includes("overflow.start ? <OverflowCue side=\"start\""), 'start cue renders conditionally');
  assert.ok(source.includes("overflow.end ? <OverflowCue side=\"end\""), 'end cue renders conditionally');
  assert.ok(source.includes('requestAnimationFrame'), 'late layout (fonts) must be re-measured');
  assert.ok(source.includes('ResizeObserver'), 'a window resize must re-measure');
  assert.ok(source.includes("panel.rows === 2 ? 'is-rows-2'"), 'the two-row layout is driven by the model');
});

test('both catalogs name the two overflow cues', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  for (const key of ['ribbon.scroll_left', 'ribbon.scroll_right']) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
  }
  assert.notEqual(zh['ribbon.scroll_left'], en['ribbon.scroll_left']);
});
