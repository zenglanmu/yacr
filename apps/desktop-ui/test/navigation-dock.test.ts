// Contract tests for the right-edge navigation dock and the collapsible rail.
//
// The dock is the one structural change here: the ViewCube and the icon rail
// share one column that floats over the right edge of the drawing, so the CAD
// partition rect spans the whole workspace row and never depends on the rail
// state. The rail's expanded state is host truth on the desktop
// (`navbarVisible`, toggled by the real `NAVBAR` command) and local on the
// compact layout, where it starts collapsed.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readModule } from './helpers.ts';

test('the dock renders the cube above the rail, in one right-edge column', async () => {
  const dock = await readModule('components/NavigationDock.tsx');
  const cube = dock.indexOf('<ViewCube />');
  const rail = dock.indexOf('<NavRail />');
  assert.ok(cube > 0 && rail > cube, 'the cube is docked above the icon rail');
  assert.ok(dock.includes('className="navdock app-region"'), 'one layout region, tagged for metrics');
  const workspace = await readModule('components/Workspace.tsx');
  assert.ok(workspace.includes("import { NavigationDock } from './NavigationDock.tsx';"));
  assert.ok(workspace.includes('<NavigationDock />'));
  assert.equal(workspace.includes('<NavRail />'), false, 'the workspace renders the dock, not the bare rail');
  // DOM order is unchanged: the dock follows the drawing area (so it paints over
  // it) and precedes the notice hosts, so focus order still reaches the CAD input
  // before the dock's controls.
  const dockAt = workspace.indexOf('<NavigationDock />');
  assert.ok(dockAt > workspace.indexOf('className="workspace__canvas"'), 'the dock comes after the drawing area');
  assert.ok(dockAt < workspace.indexOf('<PluginNoticeHost />'), 'the dock stays ahead of the notice hosts');
});

test('the rail collapse is a real control: NAVBAR on desktop, local on mobile', async () => {
  const rail = await readModule('components/NavRail.tsx');
  assert.ok(rail.includes('aria-expanded={open}'), 'the chevron reports its state');
  assert.ok(rail.includes("t('navrail.collapse')"));
  assert.ok(rail.includes("t('navrail.expand')"));
  assert.ok(rail.includes("t('navrail.hidden')"), 'the host-off state has its own sentence');
  assert.ok(rail.includes("shell.runCommand('NAVBAR')"), 'desktop collapse is the host setting, not local state');
  assert.ok(rail.includes('navigationSettings(shell.status?.navigation)'));
  assert.ok(rail.includes('shell.mobile'));
  assert.ok(rail.includes('useState(false)'), 'the compact rail starts collapsed');
  assert.ok(rail.includes('useLayoutEffect'), 'a breakpoint crossing returns it to the collapsed default');
  // The palette gate survives the rewrite: one helper, both/all toggles inside it.
  assert.ok(rail.includes("import { palettesOffered } from '../cad/surface.ts';"));
  assert.ok(rail.includes('const palettes = palettesOffered(shell);'));
  assert.ok(rail.includes('{palettes ? ('));
  // Only the chevron survives a collapsed rail.
  const collapsed = rail.slice(rail.indexOf('{open ? ('), rail.indexOf(') : null}', rail.indexOf('{open ? (')));
  assert.ok(collapsed.includes('NAV_ITEMS.map'), 'the tool buttons are inside the expanded branch');
});

test('the dock floats over the right edge and takes no width from the partition', async () => {
  const css = await readModule('style.css');
  assert.ok(css.includes('--navrail-w: 56px'), 'one dock width token');
  const ruleFor = (selector) => {
    const start = css.indexOf(`${selector} {`);
    assert.ok(start >= 0, `${selector} has a rule`);
    return css.slice(start, css.indexOf('}', start));
  };
  const dockRule = ruleFor('.navdock');
  assert.ok(dockRule.includes('position: absolute'), 'out of flow: the dock takes no flex width');
  assert.ok(dockRule.includes('top: 0; right: 0; bottom: 0;'), 'pinned to the row\'s right edge, full height');
  assert.ok(dockRule.includes('width: var(--navrail-w)'), 'the one width token sizes the column');
  assert.equal(dockRule.includes('flex:'), false, 'no flex basis/width reservation survives');
  assert.equal(dockRule.includes('pointer-events'), false, 'the whole column hits normally (clicks never fall through)');
  assert.ok(dockRule.includes('background: var(--chrome-3)'), 'the dock keeps an opaque fill');
  assert.ok(dockRule.includes('border-left: 1px solid var(--line)'), 'opaque chrome keeps its edge over the drawing');

  // The containing block and the stacking order it participates in.
  assert.ok(ruleFor('.workspace').includes('position: relative'), 'the dock is positioned against the workspace row');
  assert.ok(ruleFor('.workspace').includes('background: transparent'), 'the row stays transparent over the native surface');
  assert.ok(ruleFor('.workspace__canvas').includes('flex: 1 1 auto'), 'the drawing area owns the whole row');
  const zOf = (rule) => Number(/z-index:\s*(\d+)/.exec(rule)[1]);
  const dockZ = zOf(dockRule);
  assert.ok(dockZ > 0, 'the dock carries its own stacking level');
  // Both CAD carriers keep the default level (`auto`), so the dock's positive
  // level paints over the drawing — that is what makes the opaque column visible.
  assert.equal(ruleFor('.native-input').includes('z-index'), false, 'the CAD input keeps the default stacking level');
  assert.equal(ruleFor('.cad-canvas').includes('z-index'), false, 'the in-DOM carrier keeps the default stacking level');
  // Floating palettes stay above the dock: it used to be an unpositioned flex
  // item, and a window dragged or resized to the right edge must keep painting
  // over it (otherwise the palette's right-edge resize grip vanishes).
  const paletteZ = zOf(ruleFor('.palette-layer'));
  assert.ok(paletteZ > dockZ, 'the floating palette layer paints above the dock');
  // The banner layer is above the dock too; its `--navrail-w` right inset is what
  // keeps the two from meeting.
  assert.ok(zOf(ruleFor('.update-notice-layer')) > dockZ, 'the update banner layer paints above the dock');
  assert.ok(dockZ < zOf(ruleFor('.host-notice-overlay')), 'host notices still paint above the dock');

  const railRule = ruleFor('.navrail');
  assert.ok(railRule.includes('background: var(--chrome-3)'));
  assert.equal(railRule.includes('position: fixed'), false);
  // The update banner stays inside the drawing area: the same token moved.
  assert.ok(css.includes('inset: 0 var(--navrail-w) 0 0;'));

  const mobile = await readModule('mobile.css');
  assert.equal(mobile.includes(':root[data-layout="mobile"] .navrail {\n  display: none;'), false);
  const compactRail = mobile.slice(mobile.indexOf(':root[data-layout="mobile"] .navrail {'), mobile.indexOf('}', mobile.indexOf(':root[data-layout="mobile"] .navrail {')));
  assert.ok(compactRail.includes('display: flex'));
  assert.equal(mobile.includes('flex-basis: var(--navrail-w)'), false, 'the compact layout never re-reserves the dock width');
  assert.ok(mobile.includes(':root[data-layout="mobile"] .navrail__button'), '44px touch targets are declared');
  assert.ok(mobile.includes(':root[data-layout="mobile"] .viewcube__home'), 'Home is finger-sized on mobile too');
  // The dock is never hidden on mobile: the cube is the compact 3D cue.
  assert.equal(/\[data-layout="mobile"\][^{]*\.navdock\s*\{[^}]*display:\s*none/.test(mobile), false);
});
