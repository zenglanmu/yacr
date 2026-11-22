// Contract tests for the right-edge navigation dock, its collapse control and
// the icon rail.
//
// The dock is one column inside the workspace row: a **head** (the compact theme
// switch, then the one collapse/expand control directly under it) and, below it,
// the rail. The head is what survives both rail states — that is what lets the
// compact layout shrink to a card with a switch and one icon instead of a
// full-height 56px strip of chrome over the drawing. The orientation ball is a
// separate floating overlay inside the drawing area (`.viewball-layer`), not a
// dock child: it paints above the dock and below the palettes.
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

test('the dock head is the compact theme switch and its toggle; the ball is a separate overlay', async () => {
  const dock = await readModule('components/NavigationDock.tsx');
  // DOM order inside the dock: head (theme, toggle) then the rail. The toggle and
  // the theme switch are *not* in the rail: they must exist in both rail states
  // and on both layouts (the theme one on compact only). The orientation ball is
  // deliberately not a dock child.
  const head = dock.indexOf('className="navdock__head"');
  const theme = dock.indexOf('className="navdock__theme"');
  const toggle = dock.indexOf('className="navdock__toggle"');
  const rail = dock.indexOf('<NavRail open={open} />');
  assert.ok(head > 0);
  assert.equal(dock.includes('<ViewCube'), false, 'the ball is not mounted in the dock');
  assert.equal(dock.includes('ViewCube.tsx'), false, 'the dock does not import the ball');
  assert.equal(dock.includes('viewcube__'), false, 'the dock owns no ball styling hook');
  // The order is deliberate: the chevron acts on the rail, so it stays the last
  // control directly above it; the theme switch is chrome appearance and sits
  // before it, never as the rail's first tool.
  assert.ok(theme > head, 'the theme switch is a head row');
  assert.ok(toggle > theme, 'the collapse control stays the last head control');
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
  // The ball is a workspace overlay: after the native input (so it paints above
  // the CAD surface) and before the palette layer (so palettes stay above it).
  assert.ok(workspace.includes("import { ViewCube } from './ViewCube.tsx';"));
  const area = workspace.indexOf('className="workspace__canvas"');
  const nativeInput = workspace.indexOf("['native-input'");
  const layer = workspace.indexOf('className="viewball-layer"');
  const ball = workspace.indexOf('<ViewCube />', layer);
  const palettes = workspace.indexOf('className="palette-layer"');
  assert.ok(area > 0 && nativeInput > area, 'the drawing area holds the native input');
  assert.ok(layer > nativeInput, 'the ball layer follows the native input');
  assert.ok(ball > layer, 'the ball is mounted in that layer');
  assert.ok(palettes > ball, 'the floating palettes stay above the ball');
  // DOM order is unchanged: the dock follows the drawing area (so it paints over
  // it) and precedes the notice hosts, so focus order still reaches the CAD input
  // before the dock's controls.
  const dockAt = workspace.indexOf('<NavigationDock />');
  assert.ok(dockAt > palettes, 'the dock comes after the drawing area and its ball layer');
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
  // The rail is the dock's `tool bar|导航` landmark and its scroller, so it holds
  // tools only: no theme switch, no theme state.
  assert.equal(rail.includes('ThemeToggle'), false, 'the theme switch is not a rail item');
  assert.equal(rail.includes('navrail__theme'), false, 'no theme group in the column');
  assert.equal(rail.includes('applyTheme('), false, 'no second theme owner');
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

test('the compact theme switch lives in the dock head, outside the scrolling rail landmark', async () => {
  const dock = await readModule('components/NavigationDock.tsx');
  // Compact only: the desktop title row owns its one switch, so the desktop column
  // never grows a second one. `ThemeToggle` still owns every write.
  assert.ok(dock.includes("import { ThemeToggle } from './ThemeToggle.tsx';"));
  assert.ok(dock.includes('{shell.mobile ? ('), 'the mount point is the layout, not a prop');
  assert.ok(dock.includes('className="navdock__theme"'));
  assert.equal(dock.includes('applyTheme('), false, 'no second theme owner');
  // Inside the head, ahead of the rail: the rail is the scroller and the
  // `tool bar|导航` landmark, so a switch there would scroll out of reach.
  const headAt = dock.indexOf('<div className="navdock__head">');
  const themeAt = dock.indexOf('className="navdock__theme"');
  const toggleAt = dock.indexOf('className="navdock__toggle"');
  const railAt = dock.indexOf('<NavRail open={open} />');
  assert.ok(themeAt > headAt && themeAt < toggleAt, 'the switch is a head row, before the chevron');
  assert.ok(themeAt < railAt, 'and never inside the rail');
  const rail = await readModule('components/NavRail.tsx');
  assert.equal(rail.includes('ThemeToggle'), false, 'the rail landmark holds no theme switch');
  // The head is the fixed part: sized by its content and never a scroller, while
  // the rail (which caps and scrolls inside the card) is the only overflow area.
  const desktop = await readModule('style.css');
  const headRule = ruleFor(desktop, '.navdock__head');
  assert.ok(headRule.includes('flex: 0 0 auto'), 'the head never shrinks into the scroller');
  assert.equal(headRule.includes('overflow'), false, 'the head is not a scroller');
  assert.ok(ruleFor(desktop, '.navrail').includes('overflow-y: auto'), 'the rail is the scroller');
  const mobile = await readModule('mobile.css');
  const mobileHead = ruleFor(mobile, ':root[data-layout="mobile"] .navdock__head');
  assert.equal(mobileHead.includes('overflow'), false, 'the compact head does not scroll or clip either');
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

  // The head is ordinary flow inside the dock and holds only the compact theme
  // switch and the toggle; the orientation ball is a separate overlay.
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
  // The floating ball is its own layer inside the drawing area: above the dock,
  // below the palettes and the update banner, so neither control is ever hidden.
  const ballCss = await readModule('components/viewcube.css');
  const ballLayer = ruleFor(ballCss, '.viewball-layer');
  assert.ok(ballLayer.includes('position: absolute'), 'the ball layer is out of flow');
  assert.ok(ballLayer.includes('inset: 0'), 'and spans the drawing area');
  assert.ok(ballLayer.includes('pointer-events: none'), 'the layer lets the drawing through');
  const ballZ = zOf(ballLayer);
  assert.ok(ballZ > dockZ, 'the ball paints above the dock');
  assert.ok(ballZ < zOf(ruleFor(css, '.palette-layer')), 'palettes paint above the ball');
  assert.ok(ballZ < zOf(ruleFor(css, '.update-notice-layer')), 'the update banner paints above the ball');
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
  // The card takes the same cutout inset as the floating menu button and the
  // orientation ball. Without it, an edge-to-edge notched phone puts the card's
  // head under the status bar / notch — where it can neither be read nor tapped.
  assert.ok(
    card.includes('top: calc(8px + env(safe-area-inset-top))'),
    'pinned 8px below the drawing row\'s top edge, clear of the cutout',
  );
  assert.ok(card.includes('right: 8px'), 'and 8px in from its right edge');
  assert.ok(card.includes('bottom: auto'), 'the strip\'s full-height bottom edge is gone');
  assert.ok(card.includes('height: auto'), 'the card is as tall as the head it shows');
  assert.ok(card.includes('width: auto'), 'and as wide as one control column');
  assert.equal(/bottom:\s*0/.test(card), false, 'no full-height mobile strip rule survives');
  assert.equal(card.includes('position: fixed'), false, 'the card stays inside the workspace row, not over the bars');
  assert.ok(card.includes('max-width: calc(100vw - 16px)'), 'it never spans a phone screen');
  // The cap subtracts the inset the card was moved down by, so an open rail can
  // never push the card's bottom past the drawing row on a notched phone.
  assert.ok(
    card.includes('max-height: min(60dvh, calc(100% - 16px - env(safe-area-inset-top)))'),
    'open, it is capped and scrolls inside, inset included',
  );
  assert.ok(card.includes('max-height: 60dvh'), 'the dvh cap is the fallback before the exact one');
  assert.ok(card.includes('border: 1px solid'), 'the card has its own frame, not a left-edge strip');
  assert.ok(card.includes('border-radius: 8px'), 'a rounded card');
  assert.ok(card.includes('box-shadow:'), 'it floats above the drawing');
  assert.ok(card.includes('overflow: hidden'), 'the rail scrollbar and hovers stay inside the radius');
  assert.ok(card.includes('background: var(--chrome-3)'), 'the card is opaque chrome');
  assert.equal(card.includes('border-left: 1px solid var(--line)'), false, 'the desktop left-border edge is replaced');

  // The head is the card's definite width and the collapsed footprint: the compact
  // theme switch plus the one 44px collapse control, nothing else. The ball is not
  // a card child — it floats beside the card (see the compact corner rule below).
  const head = ruleFor(mobile, ':root[data-layout="mobile"] .navdock__head');
  assert.ok(head.includes('width: var(--navrail-w)'), 'one control column wide');
  assert.match(head, /gap:\s*4px/, 'at least 4px between the theme switch and the toggle');
  const toggle = ruleFor(mobile, ':root[data-layout="mobile"] .navdock__toggle');
  assert.ok(toggle.includes('width: 44px') && toggle.includes('height: 44px'), 'a finger-sized toggle');
  assert.ok(ruleFor(mobile, ':root[data-layout="mobile"] .navrail__button').includes('width: 44px'), '44px rail buttons');
  assert.ok(mobile.includes(':root[data-layout="mobile"] .viewcube__home'), 'Home is finger-sized on mobile too');
  // The ball keeps the **desktop corner** (the drawing's top-right) and moves left
  // of the dock card, exactly as the desktop ball sits left of the 56px rail: the
  // offset is the card's own footprint (8px inset + 56px column + 2px frame) plus
  // an 8px gap. It shrinks; the rail width token is untouched, and the top-left
  // corner belongs to the floating menu button.
  const ballCard = ruleFor(mobile, ':root[data-layout="mobile"] .viewcube');
  assert.ok(ballCard.includes('--viewball-size: 96px'), 'a smaller compact ball');
  assert.ok(
    ballCard.includes('right: calc(var(--navrail-w) + 18px)'),
    'left of the dock card by its inset, frame and an 8px gap',
  );
  assert.ok(ballCard.includes('left: auto'), 'not at the top-left corner any more');
  assert.ok(
    ballCard.includes('top: calc(8px + env(safe-area-inset-top))'),
    'the ball, the card and the menu button share one cutout inset, so the top row stays level',
  );
  const ballUnavailable = ruleFor(mobile, ':root[data-layout="mobile"] .viewcube--unavailable');
  assert.ok(
    ballUnavailable.includes('right: calc(var(--navrail-w) + 18px)')
      && ballUnavailable.includes('top: calc(8px + env(safe-area-inset-top))'),
    'the unavailable state holds the same corner',
  );
  assert.ok(ruleFor(mobile, ':root[data-layout="mobile"] .viewcube__home').includes('width: 44px'), 'a 44px Home touch target');
  const theme = ruleFor(mobile, ':root[data-layout="mobile"] .navdock__theme .ui-icon-button');
  assert.ok(theme.includes('width: 44px') && theme.includes('height: 44px'), 'the theme switch is a 44px target in the card head');
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
