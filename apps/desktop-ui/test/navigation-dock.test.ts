// Contract tests for the right-edge navigation dock and the collapsible rail.
//
// The dock is the one structural change here: the ViewCube and the icon rail
// share a fixed-width column so the reserved width holds in both rail states (the
// CAD partition rect never moves). The rail's expanded state is host truth on the
// desktop (`navbarVisible`, toggled by the real `NAVBAR` command) and local on the
// compact layout, where it starts collapsed.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readModule } from './helpers.ts';

test('the dock renders the cube above the rail, in one fixed-width column', async () => {
  const dock = await readModule('components/NavigationDock.tsx');
  const cube = dock.indexOf('<ViewCube />');
  const rail = dock.indexOf('<NavRail />');
  assert.ok(cube > 0 && rail > cube, 'the cube is docked above the icon rail');
  assert.ok(dock.includes('className="navdock app-region"'), 'one layout region, tagged for metrics');
  const workspace = await readModule('components/Workspace.tsx');
  assert.ok(workspace.includes("import { NavigationDock } from './NavigationDock.tsx';"));
  assert.ok(workspace.includes('<NavigationDock />'));
  assert.equal(workspace.includes('<NavRail />'), false, 'the workspace renders the dock, not the bare rail');
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

test('the dock width is reserved and the compact rail is not hidden any more', async () => {
  const css = await readModule('style.css');
  assert.ok(css.includes('--navrail-w: 56px'), 'one widened dock, not a floating cube');
  const dockRule = css.slice(css.indexOf('.navdock {'), css.indexOf('}', css.indexOf('.navdock {')));
  assert.ok(dockRule.includes('flex: 0 0 var(--navrail-w)'), 'the width is taken from the drawing in both states');
  assert.ok(dockRule.includes('background: var(--chrome-3)'), 'the dock keeps an opaque fill');
  const railRule = css.slice(css.indexOf('.navrail {'), css.indexOf('}', css.indexOf('.navrail {')));
  assert.ok(railRule.includes('background: var(--chrome-3)'));
  assert.equal(railRule.includes('position: fixed'), false);
  // The update banner stays inside the drawing area: the same token moved.
  assert.ok(css.includes('inset: 0 var(--navrail-w) 0 0;'));

  const mobile = await readModule('mobile.css');
  assert.equal(mobile.includes(':root[data-layout="mobile"] .navrail {\n  display: none;'), false);
  const compactRail = mobile.slice(mobile.indexOf(':root[data-layout="mobile"] .navrail {'), mobile.indexOf('}', mobile.indexOf(':root[data-layout="mobile"] .navrail {')));
  assert.ok(compactRail.includes('display: flex'));
  assert.ok(mobile.includes(':root[data-layout="mobile"] .navrail__button'), '44px touch targets are declared');
  assert.ok(mobile.includes(':root[data-layout="mobile"] .viewcube__home'), 'Home is finger-sized on mobile too');
  // The dock is never hidden on mobile: the cube is the compact 3D cue.
  assert.equal(/\[data-layout="mobile"\][^{]*\.navdock\s*\{[^}]*display:\s*none/.test(mobile), false);
});
