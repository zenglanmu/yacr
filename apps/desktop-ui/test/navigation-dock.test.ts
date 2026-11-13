// Contract tests for the right-edge navigation dock, its collapse control and
// the icon rail.
//
// The dock is one column inside the workspace row: a **head** (the ViewCube,
// then the one collapse/expand control directly under it) and, below it, the
// rail. The head is what survives both rail states — that is what lets the
// compact layout shrink to a card with a cube and one icon instead of a
// full-height 56px strip of chrome over the drawing.
//
// The rail's expanded state is host truth on the desktop (`navbarVisible`,
// toggled by the real `NAVBAR` command) and local on the compact layout, where it
// starts collapsed. Both live in `layout/nav-rail-state.ts`, never in the rail.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readModule } from './helpers.ts';

/** The declaration block of the first `selector {` rule in `css`. */
function ruleFor(css, selector) {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, `${selector} has a rule`);
  return css.slice(start, css.indexOf('}', start));
}

const zOf = rule => Number(/z-index:\s*(\d+)/.exec(rule)[1]);

test('the dock head is the cube plus its toggle, and the rail comes after it', async () => {
  const dock = await readModule('components/NavigationDock.tsx');
  // DOM order inside the dock: head (cube, toggle) then the rail. The toggle is
  // *not* in the rail: it must exist in both rail states and on both layouts.
  const head = dock.indexOf('className="navdock__head"');
  const cube = dock.indexOf('<ViewCube />');
  const toggle = dock.indexOf('className="navdock__toggle"');
  const rail = dock.indexOf('<NavRail open={open} />');
  assert.ok(head > 0 && cube > head, 'the cube is the head\'s first child');
  assert.ok(toggle > cube, 'the collapse control sits directly under the cube');
  assert.ok(rail > toggle, 'the rail is rendered after the head');
  assert.ok(dock.includes('className="navdock app-region"'), 'one layout region, tagged for metrics');
  // The rail receives the state; it does not own it.
  assert.ok(dock.includes('useNavRailState()'));
  assert.ok(dock.includes("import { useNavRailState } from '../layout/nav-rail-state.ts';"));
  // The state is exposed as data, the switch as a real button.
  assert.ok(dock.includes("data-rail={open ? 'open' : 'closed'}"), 'the dock carries its rail state');
  assert.ok(dock.includes('aria-expanded={open}'), 'the toggle reports its state');
  assert.ok(dock.includes('onClick={toggle}'));
  // A vertical rail: up when open, down when folded away.
  assert.ok(dock.includes("open ? 'chevronUp' : 'chevronDown'"));
  assert.equal(dock.includes('chevronLeft'), false, 'no left/right chevron survives');
  // No hardcoded copy: the sentence comes from the catalog through the hook.
  assert.ok(dock.includes('t(labelKey)'));

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

test('the rail is presentational: it takes `open` and owns no state or copy', async () => {
  const rail = await readModule('components/NavRail.tsx');
  // Only the five view commands plus the gated palette toggles.
  assert.ok(rail.includes("const NAV_ITEMS = ['FIT', 'ZOOMIN', 'ZOOMOUT', 'PAN', 'ORBIT'];"));
  assert.ok(rail.includes('shell.canCommand(id)'));
  assert.ok(rail.includes("t('command.unsupported', { command: t(command.labelKey) })"));
  // The palette gate survives the rewrite: one helper, all four toggles inside it.
  assert.ok(rail.includes("import { palettesOffered } from '../cad/surface.ts';"));
  assert.ok(rail.includes('const palettes = palettesOffered(shell);'));
  assert.ok(rail.includes('{palettes ? ('));
  assert.ok(rail.includes('className="navrail__rule"'), 'the divider stays with the toggles');
  for (const action of ['shell.toggleLayers()', 'shell.setFileInfoOpen(', 'shell.setDiagnosticsOpen(', 'shell.setReferencesOpen(']) {
    assert.ok(rail.includes(action), `${action} survives`);
  }
  // The chevron moved out of the rail: no toggle, no host command, no own state.
  assert.equal(rail.includes('navrail__toggle'), false, 'the rail renders no collapse control');
  assert.equal(rail.includes("shell.runCommand('NAVBAR')"), false, 'the rail never toggles the host setting itself');
  assert.equal(rail.includes('useState'), false, 'the rail is a projection of `open`, not a state owner');
  assert.equal(rail.includes('useLayoutEffect'), false);
  assert.equal(rail.includes('navigationSettings'), false);
  // Closed means absent: no empty toolbar element, so the compact card collapses
  // to the head and the desktop column shows the dock's own fill.
  assert.ok(rail.includes('export function NavRail({ open }: NavRailProps)'));
  assert.ok(rail.includes('if (!open) return null;'), 'a closed rail renders nothing at all');
});

test('the collapse state is host truth on desktop and local on mobile', async () => {
  const hook = await readModule('layout/nav-rail-state.ts');
  // Desktop: the host's navbar setting decides, and the toggle is the real command.
  assert.ok(hook.includes('navigationSettings(shell.status?.navigation)'));
  assert.ok(hook.includes('navigation.navbarVisible'));
  assert.ok(hook.includes("shell.runCommand('NAVBAR')"), 'desktop collapse is the host setting, not local state');
  // Compact: local state, collapsed first, reset only on a breakpoint crossing.
  assert.ok(hook.includes('useState(false)'), 'the compact rail starts collapsed');
  assert.ok(hook.includes('useLayoutEffect'));
  assert.ok(hook.includes('}, [mobile]);'), 'the reset is keyed on the breakpoint, not on every render');
  assert.ok(hook.includes('shell.mobile'));
  // The three catalog sentences, and which state wears which one.
  assert.ok(hook.includes("'navrail.collapse'"));
  assert.ok(hook.includes("'navrail.expand'"));
  assert.ok(hook.includes("'navrail.hidden'"), 'the host-off state has its own sentence');
  assert.ok(
    hook.includes("compactOpen ? 'navrail.collapse' : 'navrail.expand'"),
    'the compact toggle describes what pressing it does',
  );
  assert.ok(
    hook.includes("navigation.navbarVisible ? 'navrail.collapse' : 'navrail.hidden'"),
    'a host-off navbar is reported as off, not as a user collapse',
  );
  // State and copy stay apart: the hook returns keys, the component translates.
  assert.equal(hook.includes('useI18n'), false, 'the state module holds no copy');
});

test('the desktop dock floats over the right edge and takes no width from the partition', async () => {
  const css = await readModule('style.css');
  assert.ok(css.includes('--navrail-w: 56px'), 'one dock width token');
  const dockRule = ruleFor(css, '.navdock');
  assert.ok(dockRule.includes('position: absolute'), 'out of flow: the dock takes no flex width');
  assert.ok(dockRule.includes('top: 0; right: 0; bottom: 0;'), 'pinned to the row\'s right edge, full height');
  assert.ok(dockRule.includes('width: var(--navrail-w)'), 'the one width token sizes the column');
  assert.equal(dockRule.includes('flex:'), false, 'no flex basis/width reservation survives');
  assert.equal(dockRule.includes('pointer-events'), false, 'the whole column hits normally (clicks never fall through)');
  assert.ok(dockRule.includes('background: var(--chrome-3)'), 'the dock keeps an opaque fill');
  assert.ok(dockRule.includes('border-left: 1px solid var(--line)'), 'opaque chrome keeps its edge over the drawing');
  assert.equal(dockRule.includes('border-radius'), false, 'the desktop column is not a card');

  // The head stacks the cube above the toggle, and the toggle is the one control
  // present in both rail states.
  const headRule = ruleFor(css, '.navdock__head');
  assert.ok(headRule.includes('flex-direction: column'));
  assert.equal(headRule.includes('position:'), false, 'the head is ordinary flow inside the dock');
  const toggleRule = ruleFor(css, '.navdock__toggle');
  assert.match(toggleRule, /width:\s*32px/, 'a compact desktop control');
  assert.match(toggleRule, /height:\s*3[0-2]px/, '30-32px tall, not a toolbar-sized button');
  assert.ok(toggleRule.includes('border-radius: var(--radius)'));
  assert.equal(css.includes('.navrail__toggle'), false, 'the old rail chevron rule is gone');

  // The containing block and the stacking order it participates in.
  assert.ok(ruleFor(css, '.workspace').includes('position: relative'), 'the dock is positioned against the workspace row');
  assert.ok(ruleFor(css, '.workspace').includes('background: transparent'), 'the row stays transparent over the native surface');
  assert.ok(ruleFor(css, '.workspace__canvas').includes('flex: 1 1 auto'), 'the drawing area owns the whole row');
  const dockZ = zOf(dockRule);
  assert.ok(dockZ > 0, 'the dock carries its own stacking level');
  // Both CAD carriers keep the default level (`auto`), so the dock's positive
  // level paints over the drawing — that is what makes the opaque column visible.
  assert.equal(ruleFor(css, '.native-input').includes('z-index'), false, 'the CAD input keeps the default stacking level');
  assert.equal(ruleFor(css, '.cad-canvas').includes('z-index'), false, 'the in-DOM carrier keeps the default stacking level');
  // Floating palettes stay above the dock: a window dragged or resized to the
  // right edge must keep painting over it (otherwise its resize grip vanishes).
  assert.ok(zOf(ruleFor(css, '.palette-layer')) > dockZ, 'the floating palette layer paints above the dock');
  assert.ok(zOf(ruleFor(css, '.update-notice-layer')) > dockZ, 'the update banner layer paints above the dock');
  assert.ok(dockZ < zOf(ruleFor(css, '.host-notice-overlay')), 'host notices still paint above the dock');

  // `metrics.test.ts` pins the rail's own fill; the desktop column keeps it.
  assert.ok(ruleFor(css, '.navrail').includes('background: var(--chrome-3)'));
  assert.equal(ruleFor(css, '.navrail').includes('position: fixed'), false);
  // The update banner stays inside the drawing area: the same token moved.
  assert.ok(css.includes('inset: 0 var(--navrail-w) 0 0;'));
});

test('the compact dock is a content-sized card, never a full-height strip', async () => {
  const mobile = await readModule('mobile.css');
  const card = ruleFor(mobile, ':root[data-layout="mobile"] .navdock');
  assert.ok(card.includes('top: 8px'), 'pinned 8px below the drawing row\'s top edge');
  assert.ok(card.includes('right: 8px'), 'and 8px in from its right edge');
  assert.ok(card.includes('bottom: auto'), 'the strip\'s full-height bottom edge is gone');
  assert.ok(card.includes('height: auto'), 'the card is as tall as the head it shows');
  assert.ok(card.includes('width: auto'), 'and as wide as one control column');
  assert.equal(/bottom:\s*0/.test(card), false, 'no full-height mobile strip rule survives');
  assert.equal(card.includes('position: fixed'), false, 'the card stays inside the workspace row, not over the bars');
  assert.ok(card.includes('max-width: calc(100vw - 16px)'), 'it never spans a phone screen');
  assert.ok(mobile.includes('max-height: min(60dvh, calc(100% - 16px))'), 'open, it is capped and scrolls inside');
  assert.ok(card.includes('border: 1px solid'), 'the card has its own frame, not a left-edge strip');
  assert.ok(card.includes('border-radius: 8px'), 'a rounded card');
  assert.ok(card.includes('box-shadow:'), 'it floats above the drawing');
  assert.ok(card.includes('overflow: hidden'), 'the rail scrollbar and hovers stay inside the radius');
  assert.ok(card.includes('background: var(--chrome-3)'), 'the card is opaque chrome');
  assert.equal(card.includes('border-left: 1px solid var(--line)'), false, 'the desktop left-border edge is replaced');

  // The head is the card's definite width and the collapsed footprint: the cube
  // plus one 44px control, nothing else.
  const head = ruleFor(mobile, ':root[data-layout="mobile"] .navdock__head');
  assert.ok(head.includes('width: var(--navrail-w)'), 'one control column wide');
  assert.match(head, /gap:\s*4px/, 'at least 4px between the cube and the toggle');
  const toggle = ruleFor(mobile, ':root[data-layout="mobile"] .navdock__toggle');
  assert.ok(toggle.includes('width: 44px') && toggle.includes('height: 44px'), 'a finger-sized toggle');
  assert.ok(ruleFor(mobile, ':root[data-layout="mobile"] .navrail__button').includes('width: 44px'), '44px rail buttons');
  assert.ok(mobile.includes(':root[data-layout="mobile"] .viewcube__home'), 'Home is finger-sized on mobile too');
  // The rail paints nothing of its own inside the card, so a *collapsed* card (and
  // an open card's column) never shows a full-height rail background. The desktop
  // fill still lives in `style.css`, which is what `metrics.test.ts` pins.
  assert.ok(
    ruleFor(mobile, ':root[data-layout="mobile"] .navrail').includes('background: transparent'),
    'the card owns the fill, not the rail',
  );
  // The compact layout never re-reserves the dock width nor hides the dock.
  assert.equal(mobile.includes('flex-basis: var(--navrail-w)'), false, 'the compact layout never re-reserves the dock width');
  assert.equal(/\[data-layout="mobile"\][^{]*\.navdock\s*\{[^}]*display:\s*none/.test(mobile), false);
  assert.equal(mobile.includes(':root[data-layout="mobile"] .navrail {\n  display: none;'), false);
});
