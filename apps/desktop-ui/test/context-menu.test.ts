// Contract test for the CAD shortcut menu (right-click inside the partition).
//
// The model, the navigation and the placement rule are executed here, because
// they are pure. The wiring that only exists as source — App mounts the overlay,
// dispatch goes through the shared command funnel, Escape closes — is checked as
// source strings, the same style `defaults.test.mjs` and `ribbon.test.mjs` use.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CONTEXT_MENU_MARGIN,
  CONTEXT_MENU_SECTIONS,
  buildContextMenu,
  edgeEnabledIndex,
  menuItems,
  nextEnabledIndex,
  placeContextMenu,
  selectionCount,
} from '../src/commands/context-menu.ts';
import { readAppSource, readCatalog, readCss, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

/** Every gate satisfied: the only state in which every item is enabled. */
const READY = Object.freeze({
  selection: { count: 1 },
  tool: null,
  lastCommand: 'LINE',
  canFit: true,
  hasDocument: true,
});

const enabledOf = menu => Object.fromEntries(menuItems(menu).map(item => [item.id, item.enabled]));
const disabledIn = overrides => Object.entries(enabledOf(buildContextMenu({ ...READY, ...overrides })))
  .filter(([, enabled]) => !enabled)
  .map(([id]) => id);

test('five sections in a fixed order, stable ids and i18n keys', () => {
  const menu = buildContextMenu(READY);
  assert.deepEqual(menu.sections.map(section => section.id), [
    'repeat', 'clipboard', 'selection', 'view', 'history',
  ]);
  assert.deepEqual(CONTEXT_MENU_SECTIONS, menu.sections.map(section => section.id));
  assert.deepEqual(menu.sections.map(section => section.items.map(item => item.id)), [
    ['repeat.last'],
    ['clipboard.cut', 'clipboard.cutbase', 'clipboard.copy', 'clipboard.copybase', 'clipboard.paste', 'clipboard.pasteorig'],
    ['selection.all', 'selection.erase', 'selection.properties'],
    ['view.fit', 'view.pan'],
    ['edit.undo', 'edit.redo'],
  ]);
  assert.deepEqual(menuItems(menu).map(item => item.key), [
    'menu.repeat',
    'menu.clipboard.cut', 'menu.clipboard.cutbase', 'menu.clipboard.copy', 'menu.clipboard.copybase',
    'menu.clipboard.paste', 'menu.clipboard.pasteorig',
    'menu.selection.all', 'menu.selection.erase', 'menu.selection.properties',
    'menu.view.fit', 'menu.view.pan',
    'menu.edit.undo', 'menu.edit.redo',
  ]);
  assert.equal(menuItems(menu).length, 14);
});

test('every item carries the command the shared funnel would run', () => {
  const commands = Object.fromEntries(menuItems(buildContextMenu(READY)).map(item => [item.id, item.command]));
  assert.deepEqual(commands, {
    'repeat.last': 'LINE',
    'clipboard.cut': 'CUTCLIP',
    'clipboard.cutbase': 'CUTBASE',
    'clipboard.copy': 'COPYCLIP',
    'clipboard.copybase': 'COPYBASE',
    'clipboard.paste': 'PASTECLIP',
    'clipboard.pasteorig': 'PASTEORIG',
    'selection.all': 'SELECTALL',
    'selection.erase': 'ERASE',
    // The chrome-local selector: the app's run path intercepts it exactly like a
    // typed PROPERTIES and opens the docked palette; no router command is sent.
    'selection.properties': 'PROPERTIES',
    'view.fit': 'FIT',
    'view.pan': 'PAN',
    'edit.undo': 'UNDO',
    'edit.redo': 'REDO',
  });
  // The repeated command is normalized the way the command line stores it.
  const repeat = menuItems(buildContextMenu({ ...READY, lastCommand: '  line ' }))[0];
  assert.equal(repeat.command, 'LINE');
});

test('every menu key resolves in both catalogs', () => {
  for (const item of menuItems(buildContextMenu(READY))) {
    assert.equal(typeof zh[item.key], 'string', `zh-CN missing ${item.key}`);
    assert.equal(typeof en[item.key], 'string', `en missing ${item.key}`);
    assert.ok(zh[item.key].trim().length > 0, `${item.key} is empty in zh-CN`);
    assert.ok(en[item.key].trim().length > 0, `${item.key} is empty in en`);
  }
});

test('nothing is hidden: each unavailable item is present and disabled', () => {
  const all = enabledOf(buildContextMenu(READY));
  assert.deepEqual(Object.values(all).every(Boolean), true, 'the ready state enables every item');

  // No selection (absent and count 0 are the same fact). Every copy/cut variant
  // needs one, including the base-point pair.
  assert.deepEqual(disabledIn({ selection: null }), [
    'clipboard.cut', 'clipboard.cutbase', 'clipboard.copy', 'clipboard.copybase', 'selection.erase',
  ]);
  assert.deepEqual(disabledIn({ selection: { count: 0 } }), [
    'clipboard.cut', 'clipboard.cutbase', 'clipboard.copy', 'clipboard.copybase', 'selection.erase',
  ]);
  // No document, and the host reporting it cannot fit.
  assert.deepEqual(disabledIn({ hasDocument: false }), ['view.fit']);
  assert.deepEqual(disabledIn({ canFit: false }), ['view.fit']);
  // No previous command.
  assert.deepEqual(disabledIn({ lastCommand: null }), ['repeat.last']);
  assert.deepEqual(disabledIn({ lastCommand: '   ' }), ['repeat.last']);
  // A capturing tool blocks the clipboard and the destructive item.
  assert.deepEqual(
    disabledIn({ tool: { key: 'LINE', step: 1 } }),
    [
      'clipboard.cut', 'clipboard.cutbase', 'clipboard.copy', 'clipboard.copybase',
      'clipboard.paste', 'clipboard.pasteorig', 'selection.erase',
    ],
  );
});

test('the clipboard section offers both base-point variants and both pastes', () => {
  const section = buildContextMenu(READY).sections.find(candidate => candidate.id === 'clipboard');
  assert.ok(section, 'the clipboard section exists');
  const items = section.items;
  assert.deepEqual(items.map(item => [item.id, item.command, item.key]), [
    ['clipboard.cut', 'CUTCLIP', 'menu.clipboard.cut'],
    // Each base-point variant sits beside its plain sibling: the selection rule is
    // the same, what changes is that the paste that follows is relative to a pick.
    ['clipboard.cutbase', 'CUTBASE', 'menu.clipboard.cutbase'],
    ['clipboard.copy', 'COPYCLIP', 'menu.clipboard.copy'],
    ['clipboard.copybase', 'COPYBASE', 'menu.clipboard.copybase'],
    ['clipboard.paste', 'PASTECLIP', 'menu.clipboard.paste'],
    ['clipboard.pasteorig', 'PASTEORIG', 'menu.clipboard.pasteorig'],
  ]);
  // Both pastes need only "no capture": the chrome holds no clipboard-length fact,
  // so nothing here invents a gate the host does not publish.
  for (const capturing of [false, true]) {
    const enabled = Object.fromEntries(
      buildContextMenu({ ...READY, tool: capturing ? { key: 'LINE', step: 1 } : null }).sections
        .find(candidate => candidate.id === 'clipboard').items
        .map(item => [item.id, item.enabled]),
    );
    assert.equal(enabled['clipboard.paste'], !capturing);
    assert.equal(enabled['clipboard.pasteorig'], !capturing);
  }
  // The menu names a paste that asks for an insertion point and one that puts the
  // geometry back where it came from: the two are not the same claim.
  assert.notEqual(zh['menu.clipboard.paste'], zh['menu.clipboard.pasteorig']);
  assert.notEqual(en['menu.clipboard.paste'], en['menu.clipboard.pasteorig']);
  assert.match(en['menu.clipboard.pasteorig'], /original coordinates/i);
  assert.match(zh['menu.clipboard.pasteorig'], /原坐标/);
  // PASTEBLOCK stays unwired: no menu row may offer it.
  assert.equal(menuItems(buildContextMenu(READY)).some(item => item.command === 'PASTEBLOCK'), false);
});

test('a capturing tool keeps the same menu; only the blocked items change', () => {
  const idle = buildContextMenu(READY);
  const capturing = buildContextMenu({ ...READY, tool: { key: 'DIST', step: 2 } });
  assert.deepEqual(
    capturing.sections.map(section => section.id),
    idle.sections.map(section => section.id),
  );
  assert.deepEqual(
    menuItems(capturing).map(item => [item.id, item.key, item.command]),
    menuItems(idle).map(item => [item.id, item.key, item.command]),
  );
  // The tool's own confirm/cancel path is never mirrored here: no CANCEL item.
  assert.ok(!menuItems(capturing).some(item => /CANCEL|CONFIRM/.test(item.command ?? '')));

  const enabled = enabledOf(capturing);
  for (const id of ['repeat.last', 'selection.all', 'selection.properties', 'view.fit', 'view.pan', 'edit.undo', 'edit.redo']) {
    assert.equal(enabled[id], true, `${id} stays available while the tool captures`);
  }
});

test('selectionCount only accepts a positive integer count', () => {
  assert.equal(selectionCount({ count: 3 }), 3);
  assert.equal(selectionCount({ count: 0 }), 0);
  assert.equal(selectionCount(null), 0);
  assert.equal(selectionCount({ count: '2' }), 0);
  assert.equal(selectionCount({ count: true }), 0);
  assert.equal(selectionCount({ count: 1.5 }), 0);
  assert.equal(selectionCount({ count: -1 }), 0);
});

test('ids are unique and the output is deterministic', async () => {
  const ids = menuItems(buildContextMenu(READY)).map(item => item.id);
  assert.equal(new Set(ids).size, ids.length, 'no duplicate item id');
  assert.deepEqual(buildContextMenu(READY), buildContextMenu({ ...READY }));
  // No clock, no randomness: the same input always renders the same menu.
  const source = await readModule('commands/context-menu.ts');
  assert.doesNotMatch(source, /Math\.random|Date\.now|new Date\(/);
});

test('arrow/Home/End navigation lands on enabled items only, wrapping at the ends', () => {
  const items = [
    { id: 'a', enabled: true },
    { id: 'b', enabled: false },
    { id: 'c', enabled: true },
    { id: 'd', enabled: false },
  ];
  assert.equal(nextEnabledIndex(items, -1, 1), 0, 'Down from nothing starts at the first');
  assert.equal(nextEnabledIndex(items, -1, -1), 2, 'Up from nothing starts at the last');
  assert.equal(nextEnabledIndex(items, 0, 1), 2, 'a disabled item is skipped');
  assert.equal(nextEnabledIndex(items, 2, 1), 0, 'Down wraps');
  assert.equal(nextEnabledIndex(items, 0, -1), 2, 'Up wraps');
  assert.equal(edgeEnabledIndex(items, 'first'), 0);
  assert.equal(edgeEnabledIndex(items, 'last'), 2);
  assert.equal(nextEnabledIndex([{ enabled: false }], -1, 1), -1, 'nothing enabled: no focus');
  assert.equal(nextEnabledIndex([], 0, 1), -1);
  assert.equal(nextEnabledIndex(items, 0, 0), -1);

  // The shipped menu's own order: with a selection and no tool the first Down
  // lands on repeat, and a menu with no selection still reaches every enabled row.
  const flat = menuItems(buildContextMenu({ ...READY, selection: null }));
  const first = nextEnabledIndex(flat, -1, 1);
  assert.equal(flat[first].id, 'repeat.last');
  const visited = new Set();
  let index = -1;
  for (let step = 0; step < flat.length; step += 1) {
    index = nextEnabledIndex(flat, index, 1);
    visited.add(flat[index].id);
  }
  assert.deepEqual(
    [...visited].sort(),
    flat.filter(item => item.enabled).map(item => item.id).sort(),
    'every enabled item is reachable, no disabled one is',
  );
});

test('placement opens down-right, flips near an edge and clamps inside the window', () => {
  const viewport = { width: 800, height: 600 };
  const size = { width: 208, height: 260 };
  assert.deepEqual(placeContextMenu({ anchor: { x: 100, y: 100 }, size, viewport }), { left: 100, top: 100 });
  // Right edge: flips to the left of the press.
  assert.deepEqual(placeContextMenu({ anchor: { x: 700, y: 100 }, size, viewport }), { left: 492, top: 100 });
  // Bottom edge: flips above the press.
  assert.deepEqual(placeContextMenu({ anchor: { x: 100, y: 500 }, size, viewport }), { left: 100, top: 240 });
  // Both edges at once.
  assert.deepEqual(placeContextMenu({ anchor: { x: 790, y: 590 }, size, viewport }), { left: 582, top: 330 });
  // A window smaller than the menu pins it to the margin instead of going off-screen.
  assert.deepEqual(
    placeContextMenu({ anchor: { x: 300, y: 300 }, size, viewport: { width: 200, height: 200 } }),
    { left: CONTEXT_MENU_MARGIN, top: CONTEXT_MENU_MARGIN },
  );
  // A missing viewport still yields a usable position, never NaN.
  assert.deepEqual(placeContextMenu({ anchor: { x: 10, y: 10 }, size }), { left: 8, top: 8 });
});

test('App mounts the overlay, dispatches through the shared funnel and closes on Escape', async () => {
  const app = await readAppSource();
  assert.ok(app.includes("import { ContextMenu } from './components/ContextMenu.tsx';"));
  assert.match(app, /<ContextMenu\b/);
  // One dispatch path: the same `runCommand` the ribbon and command line use.
  assert.match(app, /if \(item\.command\) shell\.runCommand\(item\.command\);/);
  // `PROPERTIES` is not re-implemented in the menu: the intercept stays in the
  // one submit path (exactly one call site in App.jsx).
  const appModule = await readModule('App.tsx');
  assert.equal((appModule.match(/isPropertiesCommand\(/g) ?? []).length, 1, 'the local selector is reused, not duplicated');
  // Repeat reads the command line's own history — there is no second history.
  assert.match(app, /lastCommand: historyRef\.current\.at\(-1\) \?\? null/);
  // The state is still the one hook from `defaults.mjs`.
  assert.match(app, /onContextMenu: detail => \{/);
  assert.match(app, /cadContextMenuRequest\(\{/);

  const component = await readModule('components/ContextMenu.tsx');
  assert.match(component, /role="menu"/);
  assert.match(component, /role="menuitem"/);
  assert.match(component, /role="separator"/);
  assert.match(component, /aria-disabled/);
  assert.match(component, /key === 'Escape'/);
  assert.match(component, /onDismiss\(\{ restoreFocus: true \}\)/);
  // Capture phase: the menu owns Escape, so the shell's tool cancel never fires
  // for the same press.
  assert.match(component, /addEventListener\('keydown', onKeyDown, true\)/);
  // A long menu scrolls inside itself; only a scroll elsewhere dismisses it.
  // Same press-through dismissal; only the handler's parameter annotation is new.
  assert.match(component, /const onScroll = \(event(?:: [\w.]+)?\) => \{ if \(!contains\(event\.target\)\) dismiss\(\);/);
  // No second window, no IPC of its own: this is an overlay in the same tree.
  assert.doesNotMatch(component, /WebviewWindow|createPortal/);
  assert.doesNotMatch(component, /@tauri-apps\/api/);
});

test('the overlay is a fixed, quiet surface in the existing chrome tokens', async () => {
  const css = await readCss();
  const rule = css.match(/\.context-menu\s*\{[^}]*\}/);
  assert.ok(rule, 'the .context-menu rule exists');
  assert.match(rule[0], /position:\s*fixed/);
  assert.match(rule[0], /background:\s*var\(--chrome-2\)/);
  assert.match(rule[0], /border-radius:\s*var\(--radius\)/);
  // Above the ribbon popover (20), below the modal popup layer (60).
  const z = Number(rule[0].match(/z-index:\s*(\d+)/)?.[1]);
  assert.ok(z > 20 && z < 60, `unexpected z-index ${z}`);
  // Disabled items read as disabled, never hidden.
  assert.match(css, /\.context-menu__item\[aria-disabled="true"\]/);
  // No backdrop: the overlay must not intercept presses meant for the CAD
  // partition, so nothing spans the viewport behind the menu.
  assert.doesNotMatch(css, /\.context-menu-backdrop/);
});
