// Contract test for the collapsed ribbon's menu bar.
//
// Collapsing the ribbon must not leave a dead tab strip over empty space: the
// same 26px row becomes a menu bar whose triggers are the tab names, and each
// dropdown is built from that tab's own `RIBBON_TABS` panels. These assertions
// lock the pure model (panel order, carrier reduction, the inline/submenu
// threshold, variant rows), the single dispatch funnel the component uses, the
// accessibility roles and dismissal wiring, and the CSS geometry that keeps the
// host's CAD partition rect identical across collapse/expand.
//
// Everything is a structural/model assertion, never a pixel or screenshot one.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCatalog, readCss, readModule } from './helpers.ts';
import { PALETTE_ACTIONS, PALETTE_COMMANDS, RIBBON_TABS, offeredItems, offeredPanels } from '../src/ribbon/ribbon-model.ts';
import {
  RIBBON_MENU_HOVER_OPEN_MS,
  RIBBON_MENU_MIN_HEIGHT,
  RIBBON_MENU_SUBMENU_MIN,
  RIBBON_MENU_SURFACE_INSET,
  RIBBON_MENU_TRIGGER_GAP,
  panelStartsGroup,
  placeRibbonMenuLevel,
  ribbonMenuEntries,
  ribbonMenuEntriesAt,
  ribbonMenuEntriesFor,
  ribbonMenuEntryKind,
  ribbonMenuFlyoutAnchor,
  ribbonMenuMaxHeight,
} from '../src/ribbon/ribbon-menu.ts';

function block(css, selector) {
  const match = css.match(new RegExp(`${selector}\\s*\\{([^}]*)\\}`));
  assert.ok(match, `expected a rule for ${selector}`);
  return match[1];
}

/** A payload-free item stub (one dispatch token, as the model requires). */
function commandItem(id) {
  return { id, labelKey: 'draw.kind.line', icon: 'line', command: id };
}

/** A tab stub with the given item counts, one panel each. */
function tabWithCounts(...counts) {
  return {
    id: 'stub',
    labelKey: 'ribbon.home',
    panels: counts.map((count, index) => ({
      id: `panel-${index}`,
      titleKey: 'ribbon.panel.draw',
      items: Array.from({ length: count }, (_, item) => commandItem(`P${index}-${item}`)),
    })),
  };
}

test('every ribbon tab maps to menu entries in panel order', () => {
  for (const tab of RIBBON_TABS) {
    // Both carrier states: unknown/`true` keeps the ribbon as shipped, `false`
    // drops the palette panels. Either way the surviving panels keep their order.
    for (const palettesAvailable of [true, false, undefined]) {
      const entries = ribbonMenuEntriesFor(tab, palettesAvailable);
      const panels = offeredPanels(tab.panels, palettesAvailable)
        .filter(panel => offeredItems(panel.items, palettesAvailable).length > 0);
      assert.deepEqual(entries.map(entry => entry.id), panels.map(panel => panel.id),
        `${tab.id} (palettes=${palettesAvailable}): the menu must keep the ribbon's panel order`);
      assert.equal(entries.length, panels.length, `${tab.id}: no offered panel may be lost`);
      for (const entry of entries) {
        assert.ok(entry.rows.length > 0, `${tab.id}/${entry.id}: an empty panel is dropped, never titled`);
        assert.equal(
          entry.kind,
          entry.rows.length >= RIBBON_MENU_SUBMENU_MIN ? 'panel' : 'section',
          `${tab.id}/${entry.id}: the kind must follow the shared threshold`,
        );
      }
    }
  }
  assert.equal(ribbonMenuEntriesAt(0).length, ribbonMenuEntriesFor(RIBBON_TABS[0]).length);
  assert.deepEqual(ribbonMenuEntriesAt(RIBBON_TABS.length + 5), []);
});

test('the inline/submenu boundary is exact and empty panels are dropped', () => {
  assert.equal(RIBBON_MENU_SUBMENU_MIN, 7);
  assert.equal(ribbonMenuEntryKind(RIBBON_MENU_SUBMENU_MIN - 1), 'section');
  assert.equal(ribbonMenuEntryKind(RIBBON_MENU_SUBMENU_MIN), 'panel');
  assert.equal(ribbonMenuEntryKind(0), 'section');

  const entries = ribbonMenuEntriesFor(tabWithCounts(0, 6, 7));
  assert.deepEqual(entries.map(entry => entry.id), ['panel-1', 'panel-2'],
    'an empty panel is dropped; six items stay inline; seven become a submenu');
  assert.deepEqual(entries.map(entry => entry.kind), ['section', 'panel']);
});

test('a carrier without palettes loses the palette panels and their rows', () => {
  const home = RIBBON_TABS.find(tab => tab.id === 'home');
  assert.ok(home, 'the Home tab must exist');
  const full = ribbonMenuEntriesFor(home, true);
  const reduced = ribbonMenuEntriesFor(home, false);
  const fullIds = full.map(entry => entry.id);
  assert.ok(fullIds.includes('layers'), 'the palette panel is part of the full menu');
  assert.ok(!reduced.some(entry => entry.id === 'layers'), 'a palette-only panel is dropped whole');
  // A panel that keeps any non-palette row survives, minus the palette toggles.
  const properties = reduced.find(entry => entry.id === 'properties');
  assert.ok(properties, 'the Properties panel keeps its inquiry rows');
  assert.ok(properties.rows.length < full.find(entry => entry.id === 'properties').rows.length);
  assert.deepEqual(reduced.map(entry => entry.id), fullIds.filter(id => id !== 'layers'),
    'every other panel keeps its order');

  // The surviving rows carry no palette toggle, on every tab: a menu entry that
  // opens a panel the carrier never renders would be a dead affordance.
  for (const tab of RIBBON_TABS) {
    for (const entry of ribbonMenuEntriesFor(tab, false)) {
      for (const row of entry.rows) {
        const item = row.item;
        assert.ok(!(typeof item.action === 'string' && PALETTE_ACTIONS.includes(item.action)), `${item.id} is a palette toggle`);
        assert.ok(!(typeof item.command === 'string' && PALETTE_COMMANDS.includes(item.command)), `${item.id} is a palette command`);
      }
    }
  }
});

test('an item keeps its variants as one more menu level', () => {
  const draw = ribbonMenuEntries('home').find(entry => entry.id === 'draw');
  assert.ok(draw, 'the Home tab must carry the Draw panel');
  assert.equal(draw.kind, 'panel', 'the Draw panel is far past the inline threshold');
  const circle = draw.rows.find(row => row.id === 'CIRCLE');
  assert.ok(circle, 'the Circle split button must be a menu row');
  assert.deepEqual(circle.children.map(child => child.id), ['CIRCLE', 'CIRCLE 2P', 'CIRCLE 3P', 'CIRCLE D'],
    'the variant flyout is the model list verbatim, never a re-invented one');
  assert.ok(circle.children.every(child => child.item.command === child.id),
    'variant rows keep their own dispatch tokens');
  // A plain item has no deeper level.
  const line = draw.rows.find(row => row.id === 'LINE');
  assert.deepEqual(line.children, []);
});

test('a section caption that repeats the menu title is suppressed', async () => {
  const fileTab = RIBBON_TABS.find(tab => tab.id === 'file');
  assert.ok(fileTab, 'the File tab must exist');
  const file = ribbonMenuEntriesFor(fileTab, true);
  const fileEntry = file.find(entry => entry.id === 'file');
  assert.ok(fileEntry, 'the File panel is the first section');
  assert.equal(fileEntry.titleKey, fileTab.labelKey, 'the panel repeats the trigger words');
  assert.equal(fileEntry.caption, false, 'File > File must not print a header');
  // Only the caption goes: every offered item still renders. Counted from the
  // model, not spelled out, so a command added to the File panel (CLOSE,
  // currently) cannot be silently swallowed by a suppressed caption.
  const filePanel = fileTab.panels.find(panel => panel.id === 'file');
  assert.ok(filePanel, 'the File tab must carry the File panel');
  assert.equal(fileEntry.rows.map(row => row.id).join(','),
    offeredItems(filePanel.items, true).map(item => item.id).join(','),
    'the rows render without the caption, one per offered item');

  // A distinct panel title keeps its caption (and a big panel is never a caption).
  const home = RIBBON_TABS.find(tab => tab.id === 'home');
  const entries = ribbonMenuEntriesFor(home, true);
  assert.equal(entries.find(entry => entry.id === 'clipboard').caption, true);
  assert.equal(entries.find(entry => entry.id === 'draw').caption, false, 'a submenu row is its own label');

  // The same emptiness rule covers the caption that repeats the **first row**
  // of its own section: Home > Properties printed 属性 over an 属性 item.
  const properties = entries.find(entry => entry.id === 'properties');
  assert.ok(properties, 'the Properties section must exist');
  assert.equal(properties.rows[0].item.labelKey, properties.titleKey, 'the first row carries the panel name');
  assert.equal(properties.caption, false, 'a caption repeating its own first row is dropped');
  assert.equal(properties.rows.length, 3, 'every row still renders');
  // A caption that names something the first row does not keeps its header.
  const navigate = entries.find(entry => entry.id === 'navigate');
  assert.notEqual(navigate.rows[0].item.labelKey, navigate.titleKey);
  assert.equal(navigate.caption, true);

  // The renderer consumes the projection flag instead of re-deriving the rule.
  const source = await readModule('components/RibbonMenuBar.tsx');
  assert.ok(source.includes('if (entry.caption)'), 'the caption row is conditional on the projection');
  assert.ok(source.includes('!entry.caption && position === 0'), 'a suppressed caption still opens its group');
});

test('consecutive panel submenus are one visual block', async () => {
  const home = RIBBON_TABS.find(tab => tab.id === 'home');
  const entries = ribbonMenuEntriesFor(home, true);
  const at = id => {
    const index = entries.findIndex(entry => entry.id === id);
    assert.ok(index !== -1, `the Home tab must carry the ${id} panel`);
    return index;
  };
  assert.equal(entries[at('clipboard')].kind, 'section');
  assert.equal(panelStartsGroup(entries, at('draw')), true,
    'the first submenu row opens the block, because a section came before it');
  assert.equal(panelStartsGroup(entries, at('modify')), false,
    'a panel after a panel is a sibling submenu, not a new section');
  assert.equal(panelStartsGroup(entries, at('layers')), false);
  assert.equal(panelStartsGroup(entries, 0), false, 'the first row never needs a rule');

  // The Draw tab is two panels and no section: one block, no hairlines at all.
  const drawTab = RIBBON_TABS.find(tab => tab.id === 'draw');
  const drawEntries = ribbonMenuEntriesFor(drawTab, true);
  assert.deepEqual(drawEntries.map(entry => entry.kind), ['panel', 'panel']);
  assert.equal(panelStartsGroup(drawEntries, 0), false);
  assert.equal(panelStartsGroup(drawEntries, 1), false, 'two sibling submenus keep the block open');
  // The File tab is a section followed by a panel: that boundary still rules.
  const fileTab = RIBBON_TABS.find(tab => tab.id === 'file');
  const fileEntries = ribbonMenuEntriesFor(fileTab, true);
  assert.deepEqual(fileEntries.map(entry => entry.kind), ['section', 'panel']);
  assert.equal(panelStartsGroup(fileEntries, 1), true);

  // The renderer asks the shared rule instead of hard-coding `startsGroup: true`.
  const source = await readModule('components/RibbonMenuBar.tsx');
  assert.ok(source.includes('startsGroup: panelStartsGroup(entries, entryIndex)'),
    'the panel row takes its group rule from the projection');
  assert.doesNotMatch(source.slice(source.indexOf('kind: \'panel\','), source.indexOf('kind: \'panel\'') + 320),
    /startsGroup:\s*true/, 'no unconditional hairline above a panel row');

  // A no-icon panel row is heavier than a caption, so it reads as a submenu.
  const css = await readCss();
  assert.match(block(css, '\\.ribbon-menu__row\\.is-panel'), /font-weight:\s*500/);
});

test('the dropdown hangs from the trigger bottom-left and never flips upward', () => {
  const viewport = { width: 1280, height: 800 };
  const anchor = { left: 100, right: 168, top: 0, bottom: 26 };
  const placed = placeRibbonMenuLevel({ anchor, depth: 0, size: { width: 240, height: 300 }, viewport });
  assert.deepEqual(placed, { left: anchor.left, top: anchor.bottom + RIBBON_MENU_TRIGGER_GAP });

  // With the height budget applied, a tall dropdown still opens downward: the
  // top stays on the trigger's bottom edge instead of flipping over the bar.
  const tall = placeRibbonMenuLevel({
    anchor,
    depth: 0,
    viewport,
    size: { width: 240, height: ribbonMenuMaxHeight({ anchor, depth: 0, viewport }) },
  });
  assert.equal(tall.top, anchor.bottom + RIBBON_MENU_TRIGGER_GAP);
  assert.ok(tall.top + ribbonMenuMaxHeight({ anchor, depth: 0, viewport }) <= viewport.height - 8,
    'the budget leaves the bottom margin intact');
});

test('a flyout opens flush right, or flips beside the parent row, never over it', () => {
  const viewport = { width: 1280, height: 800 };
  const size = { width: 240, height: 300 };

  const room = { left: 300, right: 520, top: 100, bottom: 124 };
  const beside = placeRibbonMenuLevel({ anchor: room, depth: 1, size, viewport });
  assert.equal(beside.left, room.right, 'flush to the parent row\'s right edge');
  assert.equal(beside.top, room.top, 'top-aligned with the row');

  // The row sits near the right edge: the flyout must flip to the row's left
  // edge, i.e. its right side touches `anchor.left`. Anchoring at `anchor.left`
  // would instead paint it back over the parent dropdown.
  const crowded = { left: 1000, right: 1272, top: 100, bottom: 124 };
  const flipped = placeRibbonMenuLevel({ anchor: crowded, depth: 1, size, viewport });
  assert.equal(flipped.left, crowded.left - size.width);
  assert.ok(flipped.left + size.width <= crowded.left, 'the flyout never overlaps the parent');
  assert.ok(flipped.left >= 8, 'and stays inside the window margin');
});

test('a menu wider than the window is clamped to the margin', () => {
  const viewport = { width: 1280, height: 800 };
  const placed = placeRibbonMenuLevel({
    anchor: { left: 100, right: 168, top: 0, bottom: 26 },
    depth: 0,
    size: { width: 2000, height: 300 },
    viewport,
  });
  assert.equal(placed.left, 8, 'pinned to the 8px margin');
});

test('resting on a submenu parent opens its flyout, and only a parent', async () => {
  assert.equal(RIBBON_MENU_HOVER_OPEN_MS, 220, 'the intent delay is the shared constant');
  const source = await readModule('components/RibbonMenuBar.tsx');
  assert.ok(source.includes('RIBBON_MENU_HOVER_OPEN_MS'), 'the renderer uses the shared delay');

  // The timer: armed by `scheduleSubmenu`, opened after the delay, and every
  // cancellation point clears it.
  const scheduleAt = source.indexOf('const scheduleSubmenu = (depth: number, index: number) => {');
  assert.ok(scheduleAt !== -1, 'hover opens are scheduled in one place');
  const schedule = source.slice(scheduleAt, scheduleAt + 420);
  assert.ok(schedule.includes('cancelHover();'), 'a new schedule clears the previous one');
  assert.ok(schedule.includes('hoverTimer.current = window.setTimeout('), 'the open is delayed');
  assert.ok(schedule.includes('}, RIBBON_MENU_HOVER_OPEN_MS);'), 'the delay is the shared constant');
  assert.ok(schedule.includes('openSubmenu(depth, index, false);'), 'the timer opens the same flyout');

  const cancelAt = source.indexOf('const cancelHover = () => {');
  assert.ok(cancelAt !== -1, 'one cancellation helper');
  assert.ok(source.slice(cancelAt, cancelAt + 220).includes('window.clearTimeout(hoverTimer.current)'));
  // Unmount clears a pending open.
  const cleanupAt = source.indexOf('useEffect(() => () => {');
  assert.ok(cleanupAt !== -1, 'an unmount cleanup exists');
  assert.ok(source.slice(cleanupAt, cleanupAt + 220).includes('window.clearTimeout(hoverTimer.current)'),
    'a pending open never fires after unmount');

  // Hover dispatch: a parent row schedules; a leaf only clears; the row leaving
  // and the caption both clear.
  const hoverAt = source.indexOf('const hoverRow = (depth: number, index: number) => {');
  assert.ok(hoverAt !== -1, 'row hover has one handler');
  const hover = source.slice(hoverAt, hoverAt + 900);
  assert.ok(hover.includes('if (!view.enabled || !view.hasSubmenu) return;'),
    'a plain leaf row never arms a timer');
  assert.ok(hover.includes('scheduleSubmenu(depth, index);'), 'a parent row arms one');
  assert.ok(hover.includes('closeDeeper(depth);'), 'a foreign row retires the open flyout');
  assert.ok(hover.includes('state.path.length > depth && openParent !== index'),
    'only a row that is not the open flyout\'s parent collapses deeper levels');
  assert.ok(source.includes('onMouseEnter={() => hoverRow(depth, index)}'), 'rows are wired to the hover handler');
  assert.ok(source.includes('onMouseLeave={() => cancelHover()}'), 'leaving a row cancels a pending open');
  assert.ok(source.includes('onMouseEnter={() => closeDeeper(depth)}'), 'a caption retires a foreign flyout');

  // Explicit activation still opens immediately and cancels a pending hover.
  const openAt = source.indexOf('const openSubmenu = (depth: number, index: number, activateFirst: boolean) => {');
  assert.ok(source.slice(openAt, openAt + 420).includes('cancelHover();'),
    'click/Enter/ArrowRight cancel the hover timer');
  // Close paths cancel too.
  const closeAt = source.indexOf('const closeAll = (restoreFocus: boolean) => {');
  assert.ok(source.slice(closeAt, closeAt + 160).includes('cancelHover();'), 'closing the menu clears the timer');
  const deepestAt = source.indexOf('const closeDeepest = () => {');
  assert.ok(source.slice(deepestAt, deepestAt + 160).includes('cancelHover();'), 'Escape clears the timer');
  const dismissAt = source.indexOf('const onPointerDown = (event: PointerEvent) =>');
  assert.ok(source.slice(dismissAt, dismissAt + 160).includes('cancelHover();'), 'dismissal clears the timer');
});

test('each level is capped to the space below its anchor', async () => {
  const viewport = { width: 1280, height: 800 };
  const trigger = { left: 0, right: 68, top: 0, bottom: 26 };
  assert.equal(
    ribbonMenuMaxHeight({ anchor: trigger, depth: 0, viewport }),
    viewport.height - (trigger.bottom + RIBBON_MENU_TRIGGER_GAP) - 8,
  );
  const row = { left: 300, right: 520, top: 100, bottom: 124 };
  assert.equal(ribbonMenuMaxHeight({ anchor: row, depth: 1, viewport }), viewport.height - row.top - 8);
  // A short window keeps a usable, scrolling menu instead of a sliver.
  assert.equal(
    ribbonMenuMaxHeight({ anchor: { left: 0, right: 0, top: 0, bottom: 600 }, depth: 0, viewport: { width: 800, height: 640 } }),
    RIBBON_MENU_MIN_HEIGHT,
  );
  // The component writes the computed budget and holds no placement math itself.
  const source = await readModule('components/RibbonMenuBar.tsx');
  assert.ok(source.includes('maxHeight: `${maxHeight}px`'), 'the level writes its computed max-height');
  assert.ok(source.includes('ribbonMenuMaxHeight('), 'the budget comes from the pure helper');
  assert.ok(source.includes('placeRibbonMenuLevel('), 'placement comes from the pure helper');
  assert.ok(!source.includes('placeContextMenu('), 'no placement arithmetic in the component');
});

test('Ribbon mounts the menubar instead of the band when collapsed', async () => {
  const source = await readModule('components/Ribbon.tsx');
  assert.ok(source.includes("import { RibbonMenuBar } from './RibbonMenuBar.tsx';"));
  const branchAt = source.indexOf('{shell.ribbonCollapsed ? (');
  const menubarAt = source.indexOf('<RibbonMenuBar');
  assert.ok(branchAt !== -1, 'the collapsed branch must be explicit');
  assert.ok(menubarAt > branchAt, 'the menubar is what the collapsed branch mounts');
  // The tab strip lives in the expanded branch; the band is not mounted collapsed.
  assert.ok(source.includes('{shell.ribbonCollapsed ? null : ('), 'the band stays behind the expanded guard');
  assert.ok(source.indexOf('className="ribbon__band"') > source.indexOf('{shell.ribbonCollapsed ? null : ('));
  // The region keeps its collapsed flag and gains the menu stacking class.
  assert.ok(source.includes('shell.ribbonCollapsed ? \'is-collapsed\' : \'\''));
  assert.ok(source.includes("menubarOpen ? 'is-menu-open' : ''"));
});

test('the menubar dispatches only through the band\'s own funnel', async () => {
  const source = await readModule('components/RibbonMenuBar.tsx');
  assert.ok(source.includes("import { itemEnabled, runItem, unavailable } from './Ribbon.tsx';"),
    'the shared helpers are the only dispatch path');
  assert.ok(source.includes('runItem(shell, view.item)'), 'running a row uses `runItem`');
  assert.ok(source.includes('itemEnabled(shell, row.item)'), 'enablement uses `itemEnabled`');
  assert.ok(source.includes('unavailable(t, item, shell)'), 'the refusal sentence uses `unavailable`');
  // No second admission rule may grow here.
  assert.ok(!source.includes('canCommand('), 'admission must not be re-derived locally');
  assert.ok(!source.includes('DOCUMENT_SCOPED_COMMANDS'), 'document gating belongs to `unavailable`');
  assert.ok(!source.includes('panelLauncherCommand'), 'the band launcher has no menu equivalent');
  // The same model, one trigger per tab.
  assert.ok(source.includes('RIBBON_TABS.map'), 'the triggers are the model\'s tabs');
  assert.ok(source.includes('ribbonMenuEntriesFor('), 'the menus come from the shared projection');
});

test('the menubar keeps the menu roles, the chevron and the dismissal wiring', async () => {
  const source = await readModule('components/RibbonMenuBar.tsx');
  for (const marker of [
    'role="menubar"',
    'role="menu"',
    'role="menuitem"',
    'aria-haspopup="menu"',
    'aria-expanded',
    'aria-disabled',
    'aria-activedescendant',
  ]) {
    assert.ok(source.includes(marker), `expected ${marker}`);
  }
  // The collapsed row keeps the expand affordance at its right end.
  assert.ok(source.includes('icon="chevronDown"'));
  assert.ok(source.includes("t('ribbon.expand')"));
  // Escape is owned in the capture phase, and it restores the trigger's focus.
  assert.ok(source.includes("window.addEventListener('keydown', onKeyDown, true)"), 'Escape must be capture-phase');
  assert.ok(source.includes("case 'Escape'"));
  assert.ok(source.includes('focusTrigger(tabIndex)'), 'closing returns focus to the invoking trigger');
  // Outside press / blur / resize all dismiss.
  assert.ok(source.includes("document.addEventListener('pointerdown', onPointerDown, true)"));
  assert.ok(source.includes("window.addEventListener('blur', dismiss)"));
  assert.ok(source.includes("window.addEventListener('resize', dismiss)"));
  // Multi-level keyboard rules.
  for (const key of ["case 'ArrowDown'", "case 'ArrowUp'", "case 'Home'", "case 'End'", "case 'ArrowRight'", "case 'ArrowLeft'", "case 'Tab'"]) {
    assert.ok(source.includes(key), `expected ${key}`);
  }
});

test('the tab strip and the menubar share one 26px row token', async () => {
  const css = await readCss();
  assert.match(css, /--ribbon-bar-h:\s*26px/, 'the row height is one token');
  assert.match(block(css, '\\.ribbon__tabs'), /height:\s*var\(--ribbon-bar-h\)/);
  assert.match(block(css, '\\.ribbon__menubar'), /height:\s*var\(--ribbon-bar-h\)/);
  // Neither row may fall back to a literal height or to the band height.
  assert.doesNotMatch(block(css, '\\.ribbon__tabs'), /height:\s*\d+px/);
  assert.doesNotMatch(block(css, '\\.ribbon__menubar'), /height:\s*\d+px/);
  assert.doesNotMatch(block(css, '\\.ribbon__menubar'), /var\(--ribbon-h\)/);
});

test('the dropdown is an overlay above the document tabs, not part of the band', async () => {
  const css = await readCss();
  const menu = block(css, '\\.ribbon-menu');
  assert.match(menu, /position:\s*fixed/, 'the menu is viewport-anchored like the shortcut menu');
  assert.doesNotMatch(menu, /var\(--ribbon-h\)/, 'the overlay never takes the band height');
  const menuZ = Number((menu.match(/z-index:\s*(\d+)/) ?? [])[1]);
  assert.ok(Number.isFinite(menuZ) && menuZ > 0, 'the overlay carries a z-index');
  // The region lifts itself only while a menu is open, so the overlay paints
  // above the regions that follow it (document tabs, workspace).
  const lifted = block(css, '\\.ribbon\\.is-menu-open');
  assert.match(lifted, /position:\s*relative/);
  const regionZ = Number((lifted.match(/z-index:\s*(\d+)/) ?? [])[1]);
  assert.ok(Number.isFinite(regionZ) && regionZ > 0, 'the open ribbon outranks the later regions');
  assert.ok(menuZ > regionZ, 'the overlay sits above its own region');
  // A collapsed trigger must not look like a selected tab: no underline, no fill.
  const trigger = block(css, '\\.ribbon-menubar__item');
  assert.doesNotMatch(trigger, /border-bottom/, 'no tab underline');
  assert.doesNotMatch(trigger, /background:\s*var\(--chrome\)/, 'no raised tab fill');
});

test('collapsed triggers are uniform, and the open one merges with its menu', async () => {
  const css = await readCss();
  // 1. No trigger carries a stored selection: 文件 is not accent-tinted while
  // the bar is collapsed. The expanded tab strip keeps its accent tab.
  assert.doesNotMatch(css, /\.ribbon-menubar__item\.is-accent/, 'a collapsed trigger has no accent tint');
  assert.match(css, /\.ribbon-tab\.is-accent\s*\{[^}]*color:\s*var\(--accent-text\)/,
    'the expanded tab strip keeps its accent tab');
  const source = await readModule('components/RibbonMenuBar.tsx');
  assert.ok(!source.includes('is-accent'), 'the menubar applies no accent class');
  assert.ok(source.includes('className="ribbon-menubar__item"'), 'every trigger is the plain class');

  // 2. The dropdown is one step above the bar, and the trigger that owns it
  // takes the same fill, so title and menu are one shape. Hover stays --hover.
  const menu = block(css, '\\.ribbon-menu');
  assert.match(menu, /background-color:\s*var\(--chrome\)/, 'the dropdown is raised above the bar');
  assert.doesNotMatch(menu, /--chrome-2/, 'not another bar-coloured panel');
  const open = block(css, "\\.ribbon-menubar__item\\[aria-expanded='true'\\]");
  assert.match(open, /background:\s*var\(--chrome\)/, 'the open trigger takes the dropdown fill');
  assert.doesNotMatch(open, /var\(--pressed\)/, 'no recessed hole under the merged title');
  assert.match(css, /\.ribbon-menubar__item:hover:not\(\[aria-expanded='true'\]\)\s*\{[^}]*background:\s*var\(--hover\)/,
    'hover keeps the hover fill');

  // The seam: a level-0 dropdown drops its top border, and the open trigger
  // paints a 1px strip of the dropdown's fill over the bar's own hairline. The
  // bar keeps its dark delimiter everywhere else — the merge is scoped to the
  // trigger's box, not to the whole 1440px row. Nothing moves a box, so the
  // partition rect the host samples is untouched.
  const levelZero = block(css, "\\.ribbon-menu\\[data-depth='0'\\]");
  assert.match(levelZero, /border-top:\s*0/);
  assert.doesNotMatch(levelZero, /box-shadow/, 'one elevation for every level');
  assert.doesNotMatch(css, /\.ribbon__menubar\.is-open/, 'the bar is never recoloured whole');
  assert.match(css, /\.ribbon__menubar\s*\{[^}]*border-bottom:\s*1px solid var\(--line\)/,
    'the bar keeps its dark bottom delimiter');
  const strip = block(css, "\\.ribbon-menubar__item\\[aria-expanded='true'\\]::after");
  assert.match(strip, /bottom:\s*-1px/, 'the strip lies exactly over the bar hairline');
  assert.match(strip, /height:\s*1px/);
  assert.match(strip, /background:\s*var\(--chrome\)/, 'and takes the dropdown fill');
  assert.match(strip, /pointer-events:\s*none/, 'a 1px strip must not become a click target');
  assert.match(block(css, '\\.ribbon-menubar__item'), /position:\s*relative/,
    'the strip is positioned against its own trigger');
  assert.ok(source.includes('data-depth={depth}'), 'the surface publishes its level');
  assert.ok(source.includes('className="ribbon__menubar"'), 'the bar carries no open-state class');
});

test('the expand chevron is a button, not a toggle', async () => {
  const source = await readModule('components/RibbonMenuBar.tsx');
  const at = source.indexOf('icon="chevronDown"');
  assert.ok(at !== -1, 'the collapsed row keeps its expand affordance');
  const expand = source.slice(at, at + 420);
  assert.doesNotMatch(expand, /\bpressed\b/,
    'no `pressed` prop: a permanent is-on fill would make the chevron a toggle');
  assert.ok(expand.includes("label={t('ribbon.expand')}"), 'still one accessible button');
  assert.ok(expand.includes('setRibbonCollapsed(false)'), 'it still expands the band');
});

test('a flyout anchors to the parent surface, never to the row inside it', () => {
  // The surface inset is the CSS box the flyout has to compensate for.
  assert.equal(RIBBON_MENU_SURFACE_INSET, 5, '1px border + 4px padding');
  const viewport = { width: 1280, height: 900 };
  const size = { width: 240, height: 300 };

  // The row ends inside the parent's padding; its surface is 20px wider.
  const row = { left: 308, right: 500, top: 100, bottom: 124 };
  const surface = { left: 300, right: 520, top: 56, bottom: 560 };
  const anchor = ribbonMenuFlyoutAnchor(row, surface);
  assert.equal(anchor.left, surface.left, 'the horizontal edge is the surface\'s');
  assert.equal(anchor.right, surface.right);
  assert.equal(anchor.top, row.top - RIBBON_MENU_SURFACE_INSET, 'lifted by the surface inset');
  assert.equal(anchor.bottom, row.bottom - RIBBON_MENU_SURFACE_INSET);

  const right = placeRibbonMenuLevel({ anchor, depth: 1, size, viewport });
  assert.equal(right.left, surface.right, 'flush to the surface outer right edge');
  assert.ok(right.left >= surface.right, 'the flyout never overlaps the parent surface');
  assert.equal(right.top + RIBBON_MENU_SURFACE_INSET, row.top,
    'the flyout\'s first row lands level with the parent row');

  // Near the right edge the flip rule measures from the surface's left edge, so
  // the flyout's right edge abuts the surface instead of covering it.
  const crowdedSurface = { left: 1032, right: 1272, top: 56, bottom: 560 };
  const crowdedRow = { left: 1040, right: 1264, top: 100, bottom: 124 };
  const flipped = placeRibbonMenuLevel({
    anchor: ribbonMenuFlyoutAnchor(crowdedRow, crowdedSurface),
    depth: 1,
    size,
    viewport,
  });
  assert.equal(flipped.left + size.width, crowdedSurface.left, 'the flyout\'s right edge is the surface\'s left');
  assert.ok(flipped.left + size.width <= crowdedSurface.left, 'and it never paints over the parent');

  // The height budget is measured from the same lifted anchor, so placement and
  // budget cannot disagree by the inset.
  assert.equal(
    ribbonMenuMaxHeight({ anchor, depth: 1, viewport }),
    viewport.height - (row.top - RIBBON_MENU_SURFACE_INSET) - 8,
  );
});

test('the renderer anchors flyouts to the surface and to its depth', async () => {
  const source = await readModule('components/RibbonMenuBar.tsx');
  assert.ok(source.includes("rowNode?.closest?.('.ribbon-menu')"),
    'the flyout anchor reads the parent surface, not the row');
  assert.ok(source.includes('ribbonMenuFlyoutAnchor(rectOf(rowNode), rectOf(surfaceNode))'),
    'the surface-anchored rectangle comes from the shared helper');
  assert.ok(!source.includes('anchors: [...previous.anchors.slice(0, depth + 1), rectOf(node)]'),
    'a raw row rectangle is no longer stored as a flyout anchor');

  // The CSS box the constant compensates for.
  const css = await readCss();
  const menu = block(css, '\\.ribbon-menu');
  const padding = Number((menu.match(/padding:\s*(\d+)px/) ?? [])[1]);
  const border = Number((menu.match(/border:\s*(\d+)px/) ?? [])[1]);
  assert.equal(border + padding, RIBBON_MENU_SURFACE_INSET,
    'RIBBON_MENU_SURFACE_INSET must stay the surface\'s border + padding');
});

test('every menu row reserves the chevron column', async () => {
  const css = await readCss();
  const chevron = block(css, '\\.ribbon-menu__chevron');
  assert.match(chevron, /width:\s*13px/, 'one fixed slot, glyph or not');
  assert.match(chevron, /flex:\s*none/);
  assert.match(chevron, /justify-content:\s*center/);
  const source = await readModule('components/RibbonMenuBar.tsx');
  assert.ok(source.includes('<span className="ribbon-menu__chevron" aria-hidden="true">'),
    'the slot is rendered unconditionally');
  assert.ok(source.includes('{view.hasSubmenu ? <Icon name="chevronRight" size={13} /> : null}'),
    'only the glyph inside it is conditional');
});

test('a disabled row is dimmer than a caption', async () => {
  const css = await readCss();
  const disabled = block(css, '\\.ribbon-menu__row\\.is-disabled');
  assert.match(disabled, /opacity:\s*\.55/, 'a disabled row is dimmed, not merely muted');
  assert.match(disabled, /cursor:\s*not-allowed/);
  const caption = block(css, '\\.ribbon-menu__caption');
  assert.doesNotMatch(caption, /opacity/, 'a caption keeps its own tone');
  assert.match(caption, /color:\s*var\(--text-muted\)/);
  // The refusal still reaches the row, unchanged.
  const source = await readModule('components/RibbonMenuBar.tsx');
  assert.ok(source.includes('title={rowTitle(t, shell, view)}'));
  assert.ok(source.includes("aria-disabled={view.enabled ? undefined : 'true'}"));
});

test('an open submenu parent marks its chevron glyph with the accent', async () => {
  const css = await readCss();
  // The glyph is a nested `<svg>`: `.ribbon-menu__row .icon` sets its own
  // colour, so a span-only rule paints nothing at all (the bug this locks).
  assert.match(css,
    /\.ribbon-menu__row\[aria-expanded='true'\]\s+\.ribbon-menu__chevron\s+\.icon\s*\{[^}]*color:\s*var\(--accent\)/,
    'the nested glyph takes the accent, not just its slot');
  // And the cascade has to resolve to it: the generic row-icon rule and the
  // hover/active icon rule are equal-or-stronger neighbours, so the accent rule
  // must come last (equal specificity, later wins).
  const generic = css.indexOf('.ribbon-menu__row .icon {');
  const hoverIcons = css.indexOf('.ribbon-menu__row:hover:not(.is-disabled) .icon,');
  const accentAt = css.indexOf(".ribbon-menu__row[aria-expanded='true'] .ribbon-menu__chevron .icon");
  assert.ok(generic !== -1 && hoverIcons !== -1 && accentAt !== -1, 'all three icon rules exist');
  assert.ok(accentAt > hoverIcons && hoverIcons > generic,
    'the accent rule must be declared after the generic and hover icon rules');
  // The slot keeps its width; only its colour moves.
  assert.doesNotMatch(block(css, '\\.ribbon-menu__chevron'), /var\(--accent\)/,
    'the resting chevron stays muted');
  assert.match(block(css, '\\.ribbon-menu__chevron'), /width:\s*13px/);
});

test('an overflowing level shows a scrollbar and keeps the active row reachable', async () => {
  const css = await readCss();
  // The level still scrolls inside its budget, and the budget is untouched:
  // nothing here may make a level taller or flip it upward.
  const menu = block(css, '\\.ribbon-menu');
  assert.match(menu, /overflow-y:\s*auto/, 'the level scrolls inside its budget');
  assert.doesNotMatch(menu, /overflow-y:\s*hidden/);
  assert.match(menu, /max-height:\s*calc\(100vh - 16px\)/, 'the height budget is unchanged');

  // The affordance: a compact bar on the surface's own edge. 6px is narrower
  // than the rows' 8px right padding, so it never sits on top of a label.
  assert.match(css, /\.ribbon-menu\s*\{[^}]*scrollbar-width:\s*thin/, 'a thin bar, not the OS default');
  assert.match(css, /\.ribbon-menu\s*\{[^}]*scrollbar-color:\s*var\(--line-soft\) transparent/);
  assert.match(block(css, '\\.ribbon-menu::-webkit-scrollbar'), /width:\s*6px/);
  assert.match(css, /\.ribbon-menu::-webkit-scrollbar-thumb\s*\{[^}]*background:\s*var\(--line-soft\)/,
    'the thumb belongs to the chrome, not the platform');
  const rowPadding = Number((block(css, '\\.ribbon-menu__row').match(/padding:\s*\d+px\s+(\d+)px/) ?? [])[1]);
  assert.ok(6 < rowPadding, 'the scrollbar never overlaps a row\'s right padding');

  // The cue that does not depend on the engine: the two `local` covers scroll
  // with the content, so a shadow is only visible at an edge that is actually
  // cut off — a level that fits shows none at all.
  assert.match(menu, /background-attachment:\s*local, local, scroll, scroll/,
    'the covers scroll with the content');
  assert.match(menu, /radial-gradient\(farthest-side at 50% 100%/, 'a shadow at the cut edge');
  assert.match(menu, /background-size:\s*100% 26px, 100% 26px, 100% 12px, 100% 12px/);

  // Keyboard moves reveal the active row inside the surface; hover does not
  // (the pointer can already see what it is pointing at).
  const source = await readModule('components/RibbonMenuBar.tsx');
  assert.ok(source.includes("node?.scrollIntoView?.({ block: 'nearest' })"),
    'the active row is scrolled into view, nearest edge only');
  assert.ok(source.includes("if (view.enabled) setActive(depth, index);"),
    'hover keeps the plain setter, so resting on a row never scrolls the level');
  assert.ok(source.includes('revealActive.current = true;'), 'the reveal is armed by keyboard moves only');
  for (const step of [
    'moveActive(depth, nextEnabledIndex(rows, activeIndex, 1))',
    'moveActive(depth, nextEnabledIndex(rows, activeIndex, -1))',
    "moveActive(depth, edgeEnabledIndex(rows, 'first'))",
    "moveActive(depth, edgeEnabledIndex(rows, 'last'))",
  ]) {
    assert.ok(source.includes(step), `keyboard uses the revealing setter: ${step}`);
  }
  assert.ok(source.includes('useLayoutEffect(() => {\n    if (!revealActive.current) return;'),
    'the reveal runs after layout, so the row exists at its final geometry');
});

test('a flyout\'s shadow falls away from the parent seam', async () => {
  const css = await readCss();
  const shadowOf = selector => {
    const match = block(css, selector)
      .match(/box-shadow:\s*(-?[\d.]+)(?:px)?\s+(-?[\d.]+)(?:px)?\s+(-?[\d.]+)(?:px)?\s+(-?[\d.]+)(?:px)?/);
    assert.ok(match, `expected an elevation on ${selector}`);
    const [x, y, blur, spread] = match.slice(1).map(Number);
    return { x, y, blur, spread };
  };

  // The base elevation is downward-biased: a shadow that climbed above the
  // surface would smear the bar a dropdown hangs from.
  const base = shadowOf('\\.ribbon-menu');
  assert.equal(base.x, 0, 'a dropdown casts straight down');
  assert.ok(base.blur - base.y - Math.abs(base.spread) <= 0, 'no shadow above the surface');

  // A flyout abuts a sibling surface: the horizontal offset has to cancel the
  // inset and the blur together, or the shadow lands on the parent's 1px border
  // and its 4px padding (lateral reach = |spread| + |offsetX| - blur).
  for (const [side, sign] of [['right', 1], ['left', -1]]) {
    const rule = shadowOf(`\\.ribbon-menu\\[data-side='${side}'\\]`);
    assert.equal(Math.sign(rule.x), sign, `the ${side} flyout casts away from the parent`);
    assert.equal(rule.y, base.y, 'same vertical drop as the base elevation');
    assert.equal(rule.blur, base.blur);
    assert.equal(rule.spread, base.spread);
    assert.ok(Math.abs(rule.x) + Math.abs(rule.spread) >= rule.blur,
      `the ${side} shadow must not reach back over the parent's border`);
  }

  // The surface publishes which side of the parent it opened on.
  const source = await readModule('components/RibbonMenuBar.tsx');
  assert.ok(source.includes("placed.left >= anchor.right ? 'right' : 'left'"),
    'the side comes from the placement, not from the model');
  assert.match(source, /data-side=\{depth > 0 && placed \?/, 'a top-level level has no seam side');
});

test('both catalogs name the collapsed menu bar', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  for (const key of ['ribbon.menubar_label']) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.ok(zh[key].trim() && en[key].trim(), `${key} must not be empty`);
  }
  assert.notEqual(zh['ribbon.menubar_label'], en['ribbon.menubar_label']);
  assert.deepEqual(Object.keys(zh).sort(), Object.keys(en).sort(), 'the catalogs keep identical key sets');
});
