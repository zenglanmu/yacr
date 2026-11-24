// Contract tests for the width-based layout switch and the compact chrome.
//
// Five contracts are locked here, none of them a screenshot:
//
//   * the breakpoint is one constant and the resolver agrees with it (`1024` is
//     mobile — phone portrait, phone landscape and every tablet — and `1025` is
//     the first desktop width; an unreadable width is desktop);
//   * the drawer's open/close transition is pure, including its no-op identity;
//   * the drawer's ribbon grouping is the real model minus exactly the items a
//     phone cannot offer (no docked palettes, no chrome-collapse switches), and
//     its refusals collapse **by reason key**, not by rendered sentence;
//   * the compact chrome is minimal: one **floating** menu button over the
//     drawing's top-left corner (no top strip takes height from the drawing), the
//     drawing's name and the theme switch live in the drawer and the navigation
//     rail, and the command bar is hidden by default behind a floating trigger
//     while still auto-revealing for a prompt, a text step, a host failure or the
//     history sheet;
//   * the app renders the compact regions on mobile only, keeps the desktop
//     regions gated on it, and hides — rather than unmounts — the docked chrome
//     inside `<Workspace />`, so a live breakpoint crossing never remounts the
//     CAD session.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { readCatalog, readMobileCss, readModule } from './helpers.ts';
import {
  LAYOUT_ATTRIBUTE, MOBILE_MAX_WIDTH, MOBILE_MEDIA_QUERY, layoutForWidth, layoutFromMatches,
} from '../src/layout/layout-mode.ts';
import { MOBILE_DRAWER_CLOSED, mobileDrawerNext } from '../src/layout/mobile-drawer.ts';
import {
  MOBILE_COMMAND_BAR_CLOSED, commandBarMustShow, mobileCommandBarNext, mobileCommandBarVisible,
} from '../src/layout/mobile-commandbar.ts';
import {
  MOBILE_HIDDEN_ACTIONS, mobileRibbonDomId, mobileRibbonSections, mobileRibbonTabs, panelRefusalKey, sharedRefusal,
} from '../src/layout/mobile-ribbon.ts';
import {
  PALETTE_ACTIONS, PALETTE_COMMANDS, RIBBON_TABS, SHEET_PALETTE_ACTIONS, SHELL_ACTIONS,
} from '../src/ribbon/ribbon-model.ts';

test('the layout breakpoint is one constant, and the resolver agrees with it', () => {
  // 1024 is the last phone/tablet width; the layout is chosen by width only.
  assert.equal(MOBILE_MAX_WIDTH, 1024);
  assert.equal(MOBILE_MEDIA_QUERY, '(max-width: 1024px)');
  assert.equal(LAYOUT_ATTRIBUTE, 'data-layout');
  assert.equal(layoutForWidth(390), 'mobile', 'a phone in portrait');
  assert.equal(layoutForWidth(768), 'mobile', 'a tablet in portrait');
  assert.equal(layoutForWidth(1024), 'mobile', 'max-width is inclusive');
  assert.equal(layoutForWidth(914), 'mobile', 'a phone in landscape (2400x1080 class screen)');
  assert.equal(layoutForWidth(1025), 'desktop', 'the first desktop width');
  assert.equal(layoutForWidth(1280), 'desktop');
  assert.equal(layoutForWidth(914.5), 'mobile', 'a fractional landscape width is still a phone');
  // An unreadable measurement never invents the compact chrome.
  for (const width of [null, undefined, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.equal(layoutForWidth(width), 'desktop', String(width));
  }
  // Only a `true` match is mobile.
  assert.equal(layoutFromMatches(true), 'mobile');
  for (const matches of [false, null, undefined]) assert.equal(layoutFromMatches(matches), 'desktop');
});

test('the layout hook subscribes to the one query and writes the one attribute', async () => {
  const hook = await readModule('layout/use-layout-mode.ts');
  assert.ok(hook.includes('matchMedia(MOBILE_MEDIA_QUERY)'), 'one query, from the constant');
  assert.ok(hook.includes("query.addEventListener('change'"), 'a live resize flips the layout');
  assert.ok(hook.includes("query.removeEventListener('change'"), 'and the subscription is removed');
  // WebKitGTK builds that only ship the deprecated form still update.
  assert.ok(hook.includes('query.addListener(onChange)'));
  assert.ok(hook.includes('document.documentElement.setAttribute(LAYOUT_ATTRIBUTE, next)'));
  // The first paint already has the right layout (no desktop flash on a phone).
  assert.ok(hook.includes('useState<LayoutMode>(initialLayoutMode)'));
  assert.ok(hook.includes('export function initialLayoutMode()'));
  // No reload path: the hook never touches `location` or the document's content.
  assert.equal(hook.includes('location.reload'), false);
  assert.equal(hook.includes('location.'), false);
});

test('the drawer transition is pure and returns the same state when nothing changes', () => {
  const closed = MOBILE_DRAWER_CLOSED;
  assert.equal(closed.open, false);
  const opened = mobileDrawerNext(closed, 'toggle');
  assert.equal(opened.open, true);
  assert.notEqual(opened, closed, 'a real change is a new object');
  assert.equal(mobileDrawerNext(opened, 'toggle').open, false);
  assert.equal(mobileDrawerNext(opened, 'close').open, false);
  assert.equal(mobileDrawerNext(closed, 'open').open, true);
  // No-ops keep identity, so a redundant close (Escape after an item ran) does
  // not re-render the tree.
  assert.equal(mobileDrawerNext(closed, 'close'), closed);
  assert.equal(mobileDrawerNext(opened, 'open'), opened);
});

test('the command bar starts hidden, toggles purely, and reveals for must-see facts', () => {
  // Default closed: the drawing gets the row below the top bar.
  assert.equal(MOBILE_COMMAND_BAR_CLOSED.open, false);
  const opened = mobileCommandBarNext(MOBILE_COMMAND_BAR_CLOSED, 'toggle');
  assert.equal(opened.open, true);
  assert.notEqual(opened, MOBILE_COMMAND_BAR_CLOSED, 'a real change is a new object');
  assert.equal(mobileCommandBarNext(opened, 'toggle').open, false);
  assert.equal(mobileCommandBarNext(opened, 'close').open, false);
  assert.equal(mobileCommandBarNext(MOBILE_COMMAND_BAR_CLOSED, 'open').open, true);
  // No-ops keep identity, so a redundant close does not re-render the tree.
  assert.equal(mobileCommandBarNext(MOBILE_COMMAND_BAR_CLOSED, 'close'), MOBILE_COMMAND_BAR_CLOSED);
  assert.equal(mobileCommandBarNext(opened, 'open'), opened);

  const quiet = { promptMode: false, textMode: false, notice: false, sheet: false };
  assert.equal(commandBarMustShow(quiet), false);
  assert.equal(mobileCommandBarVisible(MOBILE_COMMAND_BAR_CLOSED, quiet), false, 'hidden with nothing to say');
  assert.equal(mobileCommandBarVisible(opened, quiet), true, 'the user\'s own open wins');
  // Every must-see fact reveals the bar on its own, without the user asking, and
  // none of them can be closed away while it holds.
  for (const condition of ['promptMode', 'textMode', 'notice', 'sheet']) {
    const reveal = { ...quiet, [condition]: true };
    assert.equal(commandBarMustShow(reveal), true, `${condition} must show the bar`);
    assert.equal(mobileCommandBarVisible(MOBILE_COMMAND_BAR_CLOSED, reveal), true, `${condition} reveals it while closed`);
    assert.equal(
      mobileCommandBarVisible(mobileCommandBarNext(opened, 'close'), reveal),
      true,
      `${condition} keeps it revealed after a close`,
    );
  }
});

test('the compact surface owns the bar state and wires the trigger to the bar it controls', async () => {
  const app = await readModule('App.tsx');
  // One region renders both halves; nothing else reads the flag.
  assert.ok(app.includes('{mobile ? <DomainMobileCommandSurface /> : commandLineOpen ? <DomainCommandLine /> : null}'));
  const surface = await readModule('components/MobileCommandSurface.tsx');
  assert.ok(surface.includes('{bar.open ? <MobileCommandBar /> : null}'), 'the bar is mounted only while revealed');
  assert.ok(surface.includes('barId={MOBILE_COMMAND_BAR_ID}'), 'one bar id, shared by element and trigger');
  assert.ok(surface.includes('MOBILE_COMMAND_BAR_ID') && surface.includes("from './MobileCommandBar.tsx'"), 'the id is the bar\'s export');

  const hook = await readModule('layout/use-mobile-commandbar.ts');
  assert.ok(hook.includes('useState<MobileCommandBarState>(MOBILE_COMMAND_BAR_CLOSED)'), 'hidden by default');
  assert.ok(
    hook.includes("mobileCommandBarNext(value, mobileCommandBarVisible(value, reveal) ? 'close' : 'open')"),
    'the trigger acts on what is on screen, not on the stale flag',
  );
  // The reveal facts are the bar's own: the same text-step/prompt readers, the
  // host banner and the history sheet this bar owns.
  assert.ok(hook.includes('shell.pluginPrompt !== null'));
  assert.ok(hook.includes('awaitingToolText(shell.status)'));
  assert.ok(hook.includes("notice: shell.displayedError !== ''"));
  assert.ok(hook.includes('sheet: shell.historyOpen'));
  assert.ok(hook.includes('mobileCommandBarVisible(state, reveal)'));
  // Closing the bar dismisses the sheet it owns (the sheet is shell state).
  assert.ok(hook.includes('setHistoryOpen(false);'));
  // No reset effect: the region itself unmounts on a breakpoint crossing.
  assert.equal(hook.includes('useEffect'), false, 'the state resets by unmounting');

  const bar = await readModule('components/MobileCommandBar.tsx');
  assert.ok(bar.includes("export const MOBILE_COMMAND_BAR_ID = 'cad-mobile-commandbar';"));
  // Mounted means open, so the class is constant and the reserved column engages;
  // without it the fixed trigger would sit on top of Send.
  assert.ok(bar.includes('className="mobile-commandbar is-open app-region"'));

  const toggle = await readModule('components/MobileCommandBarToggle.tsx');
  assert.ok(toggle.includes('aria-expanded={expanded}'), 'the trigger reports what is on screen');
  assert.ok(toggle.includes('aria-controls={barId}'), 'and names the bar it reveals');
  assert.ok(toggle.includes("t(expanded ? 'mobile.commandbar.hide' : 'mobile.commandbar.show')"), 'catalog copy');
  assert.ok(toggle.includes("expanded ? 'chevronDown' : 'terminal'"), 'down when open, a terminal when hidden');
  assert.ok(toggle.includes("t('mobile.commandbar.pinned')"), 'a forced reveal says why it stays');
  assert.ok(toggle.includes('className={[\'mobile-commandbar-toggle\''), 'one class family');
  // A chrome overlay: the button is the only painted node (no wrapper, no
  // full-size layer that would swallow the drawing's presses around it).
  assert.equal((toggle.match(/className=/g) ?? []).length, 1, 'exactly one painted node');
  assert.ok(toggle.includes('<button'), 'the trigger is a real button');
  const css = await readMobileCss();
  const trigger = css.slice(css.indexOf('.mobile-commandbar-toggle {'), css.indexOf('.mobile-commandbar-toggle:hover'));
  assert.ok(trigger.includes('position: fixed'), 'it takes no flex space from the drawing');
  assert.ok(trigger.includes('width: 44px') && trigger.includes('height: 44px'), 'a 44px touch target');
  assert.ok(trigger.includes('--yacr-safe-bottom'), 'it clears the host gesture bar like the bar');
  assert.ok(css.includes('.mobile-commandbar-toggle.is-pinned'), 'a forced reveal has its own state');
  assert.ok(css.includes('.mobile-commandbar.is-open'), 'the open bar reserves the trigger\'s column');
});

test('the drawer offers the real ribbon model minus the two chrome-collapse switches', () => {
  // The hidden switches are exactly the two that collapse desktop chrome; both
  // are documented in `layout/mobile-ribbon.ts`.
  assert.deepEqual([...MOBILE_HIDDEN_ACTIONS], [SHELL_ACTIONS.cleanScreen, SHELL_ACTIONS.commandLine]);
  // The chips are the ribbon's own tabs, in ribbon order; a tab whose panels are
  // all pending has nothing to tap and is not a chip.
  const expectedTabs = RIBBON_TABS
    .filter(tab => mobileRibbonSections(tab.id, false).length > 0)
    .map(tab => tab.id);
  assert.deepEqual(mobileRibbonTabs().map(tab => tab.id), expectedTabs);
  assert.equal(expectedTabs.includes('parametric'), false, 'the pending-only tab is desktop structure');

  for (const tab of mobileRibbonTabs()) {
    const sections = mobileRibbonSections(tab.id);
    assert.ok(sections.length > 0, `${tab.id} must offer something to tap`);
    for (const section of sections) {
      assert.ok(section.items.length > 0, `${tab.id}/${section.id} must not be a titled empty box`);
      assert.equal(typeof section.titleKey, 'string');
    }
    // Section order is the ribbon's panel order: the drawer never reorders tools.
    const order = sections.map(section => section.id);
    const expected = tab.panels.map(panel => panel.id).filter(id => order.includes(id));
    assert.deepEqual(order, expected, `${tab.id} keeps its panel order`);
    // No dead affordance: nothing that opens a *docked* palette (never rendered on
    // mobile), nothing that collapses chrome sizes the drawer does not have. The
    // properties inspector is the one palette action with a compact surface (its
    // own sheet), so it is the only palette action allowed through — and it must
    // really be there, or the sheet would have no entry from the drawer.
    for (const section of sections) {
      for (const item of section.items) {
        if (typeof item.action === 'string') {
          if (PALETTE_ACTIONS.includes(item.action)) {
            assert.ok(SHEET_PALETTE_ACTIONS.includes(item.action), `${tab.id}: ${item.action} has no compact surface`);
          }
          assert.equal(MOBILE_HIDDEN_ACTIONS.includes(item.action), false, `${tab.id}: hidden action ${item.action}`);
        }
        if (typeof item.command === 'string') {
          assert.equal(PALETTE_COMMANDS.includes(item.command), false, `${tab.id}: palette command ${item.command}`);
        }
      }
    }
  }

  // The Home tab keeps its tool panels and drops the two palette-only ones.
  assert.deepEqual(mobileRibbonSections('home').map(section => section.id), ['draw', 'modify', 'annotation', 'block', 'properties', 'groups', 'utilities', 'clipboard', 'bim']);
  // The one palette action with a compact surface is actually reachable there:
  // the properties entry survives the carrier filter, as an action token (never a
  // router command).
  assert.deepEqual([...SHEET_PALETTE_ACTIONS], [SHELL_ACTIONS.properties]);
  const propertiesEntries = RIBBON_TABS
    .flatMap(tab => mobileRibbonSections(tab.id).flatMap(section => section.items))
    .filter(item => item.action === SHELL_ACTIONS.properties);
  assert.ok(propertiesEntries.length > 0, 'the properties inspector is reachable from the drawer');
  for (const item of propertiesEntries) assert.equal(item.command, undefined, 'a sheet entry is chrome-local');
  // Every other palette action is still dropped, whatever panel it lives in: the
  // drawer keeps no toggle for a window the compact layout cannot render.
  const drawerItems = RIBBON_TABS.flatMap(tab => mobileRibbonSections(tab.id).flatMap(section => section.items));
  for (const obsolete of [SHELL_ACTIONS.fileinfo, SHELL_ACTIONS.diagnostics]) {
    assert.equal(drawerItems.some(item => item.action === obsolete), false, `${obsolete} has no compact surface`);
  }
  // The File tab keeps both panels: the utilities cluster still offers Audit,
  // the recovery report, About, Options and the direct Plugins page after its
  // palette and chrome switches are gone. Plugins is a dialog entry, so the
  // compact layout keeps it too.
  assert.deepEqual(mobileRibbonSections('file').map(section => section.id), ['file', 'utilities']);
  assert.deepEqual(
    mobileRibbonSections('file').at(-1).items.map(item => item.id),
    ['audit', 'recover', 'about', 'options', 'plugins'],
  );
  // The View tab keeps its `palettes` panel only because STATUS is a router
  // command; the palette toggles, the chrome-collapse switches and the
  // workspace words are dropped from the interface panel, leaving the two
  // surfaces a phone can really use (the layout overview and QuickCalc).
  // Coordinates/Viewports now hold real router commands (UCSICON, VPORTS…) and
  // stay in the drawer like every other wired panel.
  assert.deepEqual(mobileRibbonSections('view').map(section => section.id), ['navigate', 'view3d', 'coordinates', 'viewports', 'standard', 'viewstate', 'display', 'palettes', 'interface']);
  assert.deepEqual(
    mobileRibbonSections('view').find(section => section.id === 'palettes').items.map(item => item.id),
    ['STATUS'],
  );
  assert.deepEqual(
    mobileRibbonSections('view').find(section => section.id === 'interface').items.map(item => item.id),
    ['QVLAYOUT', 'QUICKCALC'],
  );
  // An unknown tab id resolves like `ribbonTab` (Home), never to an empty drawer.
  assert.deepEqual(mobileRibbonSections('nope').map(section => section.id), ['draw', 'modify', 'annotation', 'block', 'properties', 'groups', 'utilities', 'clipboard', 'bim']);
});

test('drawer DOM ids survive multi-word ribbon ids', () => {
  assert.equal(mobileRibbonDomId('CHAMFER POLYLINE'), 'mobile-ribbon-CHAMFER-POLYLINE');
  assert.equal(mobileRibbonDomId('  ZOOM WINDOW '), 'mobile-ribbon-ZOOM-WINDOW');
  assert.ok(!mobileRibbonDomId('AREA OBJECT').includes(' '), 'an id is never a token list');
});

test('one shared refusal is printed once per panel, and mixed reasons stay per cell', () => {
  // The comparison is over **reason keys**, never over rendered sentences: the
  // sentence names the command it belongs to, so on the web carrier (empty
  // vocabulary) every cell used to read differently and nothing ever collapsed.
  assert.equal(sharedRefusal(['command.unsupported', 'command.unsupported']), 'command.unsupported');
  // One enabled tool: the cells explain themselves (an enabled cell has no reason).
  assert.equal(sharedRefusal(['command.unsupported', null]), null);
  // Different reasons: never collapsed into a sentence that is only half true.
  assert.equal(sharedRefusal(['linux.busy', 'command.unsupported']), null);
  // An all-enabled panel needs no note.
  assert.equal(sharedRefusal([null, null]), null);
  assert.equal(sharedRefusal([]), null);
});

test('a shared refusal gets command-free panel wording, or keeps the per-cell copy', () => {
  // The catalog sentence for an unsupported command carries `{command}`, so a
  // panel-level note needs its own words.
  assert.equal(panelRefusalKey('command.unsupported'), 'mobile.refusal.unsupported');
  // These two already read as panel sentences and are reused verbatim.
  assert.equal(panelRefusalKey('plot.no_document'), 'plot.no_document');
  assert.equal(panelRefusalKey('linux.busy'), 'linux.busy');
  // A reason with no panel wording falls back to the per-cell sentences; an
  // absent reason is not a reason.
  assert.equal(panelRefusalKey('view.orbit.gesture'), null);
  assert.equal(panelRefusalKey(null), null);
  assert.equal(panelRefusalKey(undefined), null);
});

test('App renders the compact regions on mobile only and keeps the desktop regions gated', async () => {
  const app = await readModule('App.tsx');
  // The single layout source. The hook owns the only `data-layout` write (on the
  // document element), so the shell element carries no second copy.
  assert.ok(app.includes('const layout = useLayoutMode();'));
  assert.ok(app.includes("const mobile = layout === 'mobile';"));
  assert.equal(app.includes('data-layout={'), false, 'one writer, on the document element');
  // Compact regions.
  assert.ok(app.includes('<MobileMenuButton menuOpen={mobileDrawer.open}'));
  assert.ok(app.includes('<MobileRibbonDrawer open={mobileDrawer.open}'));
  assert.ok(app.includes('{mobile ? <DomainMobileCommandSurface /> : commandLineOpen ? <DomainCommandLine /> : null}'));
  // Desktop regions are gated on the compact layout, not rendered alongside it.
  // The ribbon and the drawing tabs carry the finer-grained chrome visibility on
  // top of the layout/clean-screen gates; LayoutTabs and the status bar keep the
  // layout-only gate.
  assert.ok(app.includes('{mobile || cleanScreen || !chromeVisibility.ribbon ? null : <DomainRibbon />}'));
  assert.ok(app.includes('{mobile || cleanScreen || !chromeVisibility.fileTabs ? null : <DocTabs />}'));
  assert.ok(app.includes('{mobile || cleanScreen ? null : <LayoutTabs />}'));
  assert.ok(app.includes('{mobile ? null : <StatusBar />}'));
  // One CAD partition in the tree, at the same position in both layouts: the
  // session must never be remounted by a breakpoint crossing.
  assert.equal((app.match(/^\s*<Workspace \/>$/gm) ?? []).length, 1, 'exactly one Workspace element');
  // Leaving the compact layout closes the drawer instead of leaving it latched.
  assert.ok(app.includes("setMobileDrawer(state => mobileDrawerNext(state, 'close'));"));
});

test('the compact menu is one floating button: no bar, no reserved height, no second control', async () => {
  const menu = await readModule('components/MobileMenuButton.tsx');
  // One control: the menu button. The drawing's name moved into the drawer, and
  // the theme switch moved into the navigation dock's head, so neither competes
  // with the drawing for a strip of its height.
  assert.equal((menu.match(/<button\b/g) ?? []).length, 1, 'exactly one button in the menu region');
  assert.equal(menu.includes('role="banner"'), false, 'no whole-row banner is announced or rendered');
  assert.equal(menu.includes('mobile-topbar__title'), false, 'no document label in the button');
  assert.equal(menu.includes('documentLabel'), false, 'the button does not read the document label');
  assert.equal(menu.includes('ThemeToggle'), false, 'no theme switch beside the button');
  assert.ok(menu.includes('aria-expanded={menuOpen}'));
  assert.ok(menu.includes('aria-controls={drawerId}'));
  assert.ok(menu.includes('aria-label={t(\'mobile.menu\')}'));
  assert.ok(menu.includes('shell.openProgress'), 'the host open-progress strip stays');
  // The wrapper is the button's own anchored box. `.mobile-topbar` stays as the
  // compact chrome's selector for the web smoke harness and the Android layout log;
  // it no longer paints a row.
  assert.ok(menu.includes('className="mobile-topbar app-region"'), 'one anchored region, tagged for metrics');

  const css = await readMobileCss();
  const rule = selector => {
    const start = css.indexOf(`${selector} {`);
    assert.ok(start > 0, `${selector} must exist`);
    return css.slice(start, css.indexOf('}', start));
  };
  // No top strip: the anchor is fixed over the drawing, paints nothing, and is
  // never wider than the button, so the drawing keeps every pixel around it.
  const anchor = rule('.mobile-topbar');
  assert.ok(anchor.includes('position: fixed'), 'it takes no flex height from the drawing');
  assert.equal(/\bflex:/.test(anchor), false, 'a fixed anchor reserves no flex basis');
  assert.equal(/min-height/.test(anchor), false, 'no strip height survives');
  assert.equal(anchor.includes('background'), false, 'nothing is painted behind the button');
  assert.equal(anchor.includes('border'), false, 'and no divider separates the drawing from a bar');
  assert.ok(anchor.includes('top: calc(8px + env(safe-area-inset-top))'), 'clear of the cutout');
  assert.ok(anchor.includes('left: max(8px, env(safe-area-inset-left))'), 'and of the left inset');
  // The button itself: a 44px target with its own surface, because it now floats
  // over the drawing instead of over a chrome fill.
  const button = rule('.mobile-topbar__menu');
  assert.ok(button.includes('width: 44px') && button.includes('height: 44px'), 'a 44px touch target');
  assert.ok(button.includes('background: var(--chrome-2)'), 'it carries its own surface');
  assert.ok(button.includes('box-shadow:'), 'and floats above the drawing');
  assert.ok(button.includes('border-radius: 999px'), 'round like the other compact floating controls');
  // The import strip reports on the drawing's own top edge and takes no press.
  const progress = rule('.mobile-topbar__progress');
  assert.ok(progress.includes('position: fixed') && progress.includes('pointer-events: none'), 'a report, never a hit area');
  assert.ok(progress.includes('top: env(safe-area-inset-top, 0px)'), 'pinned below the cutout');
  // The drawer starts below the button band (8 + 44 + 8), so the button stays
  // visible and tappable while the drawer is open — the band is the overlay's
  // padding, not a row in the shell.
  const layer = rule('.mobile-drawer-layer');
  assert.ok(
    layer.includes('padding-top: calc(var(--mobile-menubar-h) + env(safe-area-inset-top))'),
    'the drawer clears the floating button',
  );
  assert.ok(css.includes('--mobile-menubar-h: 60px'), 'one band height, 8 + 44 + 8');

  const drawer = await readModule('components/MobileRibbonDrawer.tsx');
  assert.ok(drawer.includes('role="dialog"'));
  assert.ok(drawer.includes('aria-modal="true"'));
  assert.ok(drawer.includes('mobileRibbonSections('), 'the real model, reduced for touch');
  assert.ok(drawer.includes('mobileRibbonTabs()'), 'the real tab list');
  assert.ok(drawer.includes('MOBILE_DRAWER_ID'));
  // The drawer's header carries the label the removed strip used to show, one
  // line, with the full name in `title`.
  assert.ok(drawer.includes("const documentLabel = shell.documentLabel ?? t('start.title');"), 'the label lives here now');
  assert.ok(drawer.includes('className="mobile-drawer__title" title={documentLabel}'), 'one ellipsized line with the full name');
  // Same gating and same dispatch as the band: imported, never re-spelled.
  assert.ok(drawer.includes("import { itemEnabled, runItem, unavailable } from './Ribbon.tsx';"));
  assert.ok(drawer.includes('runItem(shell, item)'));
  // Close paths: the close button, the backdrop and Escape.
  assert.ok(drawer.includes("className=\"mobile-drawer__backdrop\""), 'a backdrop tap closes');
  assert.ok(drawer.includes("window.addEventListener('keydown', onKeyDown)"));
  assert.ok(drawer.includes("event.key !== 'Escape'"));
  assert.ok(drawer.includes('runItem(shell, item); onClose();'), 'running an item closes it');
  // The refusal collapse is by **reason key**: the same gate (`itemEnabled` +
  // `unavailable`) is read through a key-only translator, and the panel note is
  // the one command-free sentence from `panelRefusalKey`.
  assert.ok(drawer.includes('const keyOnly = ((key: string) => key) as Translator;'));
  assert.ok(drawer.includes('unavailable(keyOnly, item, shell)'), 'the key is read through the one gate');
  assert.ok(drawer.includes('panelRefusalKey(sharedRefusal('), 'the panel note comes from the shared helper');
  assert.ok(drawer.includes('showReason={note === null}'), 'a shared reason is printed once, per cell otherwise');

  const bar = await readModule('components/MobileCommandBar.tsx');
  assert.ok(bar.includes('const line = useCommandLine();'), 'the shared command-line controller');
  assert.ok(bar.includes('<CommandField id="cad-command"'), 'the shared field');
  assert.ok(bar.includes('mobileHistoryRows(shell.historyEntries)'), 'a capped history of the one log');
  assert.ok(bar.includes('role="alert"'), 'refusals stay visible without a status bar');
  assert.ok(bar.includes('shell.displayedError'), 'the status bar\'s error slot moved here');
  assert.ok(bar.includes("releaseFocusOnOutsidePress("), 'the same focus hand-off as the docked field');
  assert.ok(bar.includes('id={MOBILE_COMMAND_BAR_ID}'), 'the trigger\'s aria-controls names this element');
});

test('no compact component binds the CAD surface or mounts a second session', async () => {
  for (const name of [
    'components/MobileMenuButton.tsx',
    'components/MobileRibbonDrawer.tsx',
    'components/MobileCommandBar.tsx',
  ]) {
    // Comments may *describe* the funnel; code may not touch it.
    const source = (await readModule(name))
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/\/\/.*$/gm, '');
    assert.equal(source.includes('native-input'), false, `${name} must not touch the CAD input funnel`);
    assert.equal(source.includes('cad-canvas'), false, `${name} must not touch the pixel carrier`);
    assert.equal(source.includes("addEventListener('pointerdown'"), false, `${name} must not listen on the drawing`);
    assert.equal(source.includes('createCanvasTransport'), false, `${name} must not own a session`);
  }
});

test('the mobile stylesheets are guarded by the one layout attribute and fit a phone', async () => {
  // Both halves of the compact bundle: the command bar moved to its own file at
  // the code-file ceiling, so a rule that lives there must stay covered too.
  const css = await readMobileCss();
  // One guard, no second media query: the breakpoint exists in `layout-mode.ts`
  // only, so the stylesheets cannot drift from the hook.
  assert.equal(css.includes('@media'), false, 'the compact bundle must not add a second breakpoint');
  assert.ok(css.includes(':root[data-layout="mobile"] .shell'));
  assert.ok(css.includes('height: 100dvh'), 'the drawing follows the visual viewport');
  // Safe areas on both ends: the floating menu button is inset from the cutout,
  // and the command bar takes the larger of the platform inset and the host's own
  // variable. There is no top strip left to pad.
  assert.ok(css.includes('top: calc(8px + env(safe-area-inset-top))'), 'the menu button clears the cutout');
  assert.ok(css.includes('left: max(8px, env(safe-area-inset-left))'), 'and the left inset');
  assert.equal(css.includes('padding-top: env(safe-area-inset-top)'), false, 'no bar-level top padding survives');
  assert.ok(
    css.includes('padding-bottom: calc(8px + max(env(safe-area-inset-bottom, 0px), var(--yacr-safe-bottom, 0px)))'),
    'the command bar honours the host bottom inset',
  );
  assert.ok(css.includes('--yacr-safe-bottom'), 'the host variable is named in the stylesheet');
  // The desktop chrome inside `<Workspace />` is hidden, not unmounted: the start
  // column, the floating palette layer (the compact layout keeps its own sheets)
  // and the nav rail.
  assert.ok(css.includes(':root[data-layout="mobile"] .workspace__left'));
  assert.ok(css.includes(':root[data-layout="mobile"] .palette-layer'));
  assert.ok(css.includes(':root[data-layout="mobile"] .navrail'));
  // The properties inspector is the one palette the compact layout keeps, so its
  // sheet belongs to the compact bundle (layer, scrim, surface).
  assert.ok(css.includes(':root[data-layout="mobile"] .properties-layer'));
  assert.ok(css.includes(':root[data-layout="mobile"] .properties-layer__backdrop'));
  assert.ok(css.includes(':root[data-layout="mobile"] .properties--floating'));
  // 44px touch targets on the menu button and the command bar controls.
  const menu = css.slice(css.indexOf('.mobile-topbar__menu {'), css.indexOf('.mobile-topbar__menu {') + 400);
  assert.ok(menu.includes('width: 44px') && menu.includes('height: 44px'));
  // The removed strip keeps no styling for a label or a theme switch.
  assert.equal(css.includes('.mobile-topbar__title'), false, 'no title rule survives');
  assert.equal(css.includes('.mobile-topbar .ui-icon-button'), false, 'no theme-switch rule survives beside the button');
  // The sheet is capped near half the drawing and opens upward from the bar.
  assert.ok(css.includes('--mobile-sheet-h: 45dvh'));
  assert.ok(css.includes('bottom: calc(100% + 6px)'));
  // Dialogs become full-width sheets that never exceed the viewport.
  assert.ok(css.includes(':root[data-layout="mobile"] .popup-dialog'));
  assert.ok(css.includes('max-height: 100dvh'));
  assert.ok(css.includes(':root[data-layout="mobile"] .options-rail'), 'the Options rail becomes a chip row');
  assert.ok(css.includes(':root[data-layout="mobile"] .context-menu__item'), 'the shortcut menu keeps 44px rows');
  // The desktop stylesheet keeps no compact breakpoint of its own.
  const desktop = await readModule('style.css');
  assert.equal(desktop.includes('768px'), false, 'the compact breakpoint lives in one place');
  assert.equal(desktop.includes('1024px'), false, 'the compact breakpoint lives in one place');
});

test('the drawer caps its height, scrolls its body and never clips a row silently', async () => {
  const css = await readModule('mobile.css');
  // The cap is two limits: the design cap (68dvh, not the 70% that left a 144px
  // strip of drawing) and a floor for a short viewport (a phone in landscape),
  // so a strip of the drawing is always visible under the drawer.
  const capStart = css.indexOf('--mobile-drawer-h:');
  assert.ok(capStart > 0, 'the drawer cap is one variable');
  const cap = css.slice(capStart, css.indexOf(';', capStart));
  assert.ok(cap.includes('68dvh'), 'the panel is capped at 68dvh');
  assert.ok(cap.includes('min('), 'the cap is the smaller of the design cap and the viewport limit');
  assert.ok(cap.includes('- 96px'), 'the floating trigger plus a strip of drawing always stay visible');
  const panel = css.slice(css.indexOf('.mobile-drawer {'), css.indexOf('.mobile-drawer__backdrop {'));
  assert.ok(panel.includes('max-height: var(--mobile-drawer-h)'), 'the panel uses the cap');
  // The body is the scroller: momentum scrolling, contained overscroll, a visible
  // thin scrollbar, and a bottom padding that matches the fade so the last row is
  // fully reachable instead of sliced at the edge.
  const body = css.slice(css.indexOf('.mobile-drawer__body {'), css.indexOf('.mobile-drawer__section {'));
  assert.ok(body.includes('overflow-y: auto'));
  assert.ok(body.includes('-webkit-overflow-scrolling: touch'), 'momentum scrolling in the WebViews');
  assert.ok(body.includes('overscroll-behavior: contain'));
  assert.ok(body.includes('scrollbar-width: thin'), 'the scroller is visible');
  assert.ok(css.includes('.mobile-drawer__body::-webkit-scrollbar-thumb'));
  assert.ok(body.includes('padding: 2px 10px calc(var(--mobile-drawer-fade) + 10px)'), 'a reachable last row');
  // The visible affordance at the bottom edge, and the pinned head/chips.
  assert.ok(css.includes('.mobile-drawer::after'), 'a bottom fade says "there is more below"');
  assert.ok(css.includes('- 96px'), 'the floor no longer reserves a hidden command bar');
  const head = css.slice(css.indexOf('.mobile-drawer__head {'), css.indexOf('.mobile-drawer__head {') + 260);
  assert.ok(head.includes('flex: 0 0 auto'), 'the head stays pinned above the scroller');
  // The document label's row is exactly the close button's 44px: no vertical
  // padding, so it costs a short landscape viewport nothing extra.
  assert.ok(head.includes('padding: 0 4px 0 14px'), 'the label row adds no vertical padding');
  assert.ok(css.includes('--mobile-drawer-fade: 24px'), 'the fade is one variable');
});

test('every compact control keeps a >= 44px touch target', async () => {
  const css = await readMobileCss();
  const block = (selector) => {
    const start = css.indexOf(`${selector} {`);
    assert.ok(start > 0, `${selector} must exist`);
    return css.slice(start, css.indexOf('}', start));
  };
  // The menu button, the drawer's close button and its tab chips.
  assert.match(block('.mobile-topbar__menu'), /width: 44px[\s\S]*height: 44px/);
  assert.match(block('.mobile-drawer__head .ui-icon-button'), /width: 44px[\s\S]*height: 44px/);
  const tab = block('.mobile-drawer__tab');
  assert.ok(tab.includes('min-width: 44px') && tab.includes('min-height: 44px'), 'chips are 44x44 hit areas');
  // The command bar's send/history controls and the sheet's own actions.
  assert.match(block('.mobile-commandbar .ui-icon-button'), /width: 44px[\s\S]*height: 44px/);
  assert.match(block('.mobile-sheet__actions .ui-icon-button'), /width: 44px[\s\S]*height: 44px/);
  // The history and completion rows.
  assert.ok(block('.mobile-sheet__row').includes('min-height: 44px'));
  assert.ok(block('.mobile-sheet .completion').includes('min-height: 44px'));
  // ...and the drawer's tool cells, variant rows and options toggles.
  assert.ok(block('.mobile-drawer__item').includes('min-height: 44px'));
  assert.ok(block('.mobile-drawer__variant').includes('min-height: 44px'));
  assert.ok(block('.mobile-drawer__variants-toggle').includes('min-height: 44px'));
  assert.ok(block(':root[data-layout="mobile"] .context-menu__item').includes('min-height: 44px'));
});

test('refusal copy is readable: 11.5px on a compliant contrast token', async () => {
  const css = await readModule('mobile.css');
  for (const selector of ['.mobile-drawer__reason', '.mobile-drawer__note']) {
    const start = css.indexOf(`${selector} {`);
    assert.ok(start > 0, `${selector} must exist`);
    const blockCss = css.slice(start, css.indexOf('}', start));
    assert.ok(blockCss.includes('font-size: 11.5px'), `${selector} is at least 11px`);
    assert.ok(blockCss.includes('color: var(--text-dim)'), `${selector} is not the muted token`);
  }
  // `--text-dim` (#c3ccda) on the cell fill (#333c48) is 6.9:1; the muted token
  // these replaced was 5.4:1 at 10.5px, which is the part that read as noise.
  const desktop = await readModule('style.css');
  assert.ok(desktop.includes('--text-dim: #c3ccda'));
});

test('the status-bar chip cannot wrap, overlap a neighbour or overflow its own border', async () => {
  const css = await readModule('style.css');
  const block = (selector) => {
    const start = css.indexOf(`${selector} {`);
    assert.ok(start > 0, `${selector} must exist`);
    return css.slice(start, css.indexOf('}', start));
  };
  // The bar is one fixed 26px row: it cannot wrap, so nothing may paint outside it.
  assert.ok(block('.statusbar').includes('overflow: hidden'));
  // A 20px chip whose label wraps paints the second line across its own border
  // (the "struck through" label) and over its neighbour.
  const toggle = block('.status-toggle');
  assert.ok(toggle.includes('flex: 0 0 auto'), 'a chip never shrinks into its neighbour');
  assert.ok(toggle.includes('white-space: nowrap'));
  assert.ok(toggle.includes('min-width: 0') && toggle.includes('max-width: 100%'));
  assert.ok(toggle.includes('overflow: hidden') && toggle.includes('text-overflow: ellipsis'));
  // The one chip with a label *and* a value field gives up the field first.
  assert.ok(block('.status-stepper').includes('min-width: 0'));
  assert.ok(block('.status-stepper').includes('overflow: hidden'));
  assert.ok(block('.status-stepper__label').includes('flex: 0 0 auto'));
  const input = block('.status-stepper__input');
  assert.ok(input.includes('flex: 0 1 auto'), 'the field shrinks instead of overflowing');
  assert.ok(input.includes('max-width: 100%'));
  assert.ok(block('.status-choices').includes('min-width: 0'));
  assert.ok(block('.status-choices__label').includes('flex: 0 0 auto'));
  // Dropping labels is the sanctioned escape hatch when the row is still too
  // wide (the chip keeps its accessible name).
  assert.ok(css.includes('@media (max-width: 1180px)'));
  assert.ok(css.includes('.status-stepper__label { display: none; }'));
  assert.ok(css.includes('.status-choices__label { display: none; }'));
  // The state word and the active-tool chip give up their text first.
  assert.ok(block('.statusbar__state').includes('text-overflow: ellipsis'));
  assert.ok(block('.statusbar__tool').includes('white-space: nowrap'));
});

test('the sheet is the only command-line surface on a phone, and it never reserves a gap', async () => {
  const css = await readMobileCss();
  const block = (selector) => {
    const start = css.indexOf(`${selector} {`);
    assert.ok(start > 0, `${selector} must exist`);
    return css.slice(start, css.indexOf('}', start));
  };
  const mobile = await readModule('components/MobileCommandBar.tsx');
  // The sheet opens upward from the bar (the desktop panel uses the same shape),
  // and it is capped so the drawing above stays visible.
  assert.ok(css.includes('--mobile-sheet-h: 45dvh'));
  assert.ok(block('.mobile-sheet').includes('position: absolute'));
  assert.ok(block('.mobile-sheet').includes('bottom: calc(100% + 6px)'));
  // The refusal line under the field is gone, so the bar must not keep a slot for
  // it: the bar is a plain stacked flex column and only the host banner (its one
  // remaining message element) can add height.
  const bar = block('.mobile-commandbar');
  assert.equal(/min-height/.test(bar), false, 'no reserved strip for a removed line');
  assert.equal(/height:\s*\d+px/.test(bar), false, 'the bar is sized by its content');
  assert.equal(mobile.split('mobile-commandbar__message').length - 1, 1, 'only the host banner remains');
  assert.ok(mobile.includes('id="cad-command-message"') === false, 'no message element id survives');
});

test('the document reports a device-width viewport, so the breakpoint is CSS px', async () => {
  // Without `width=device-width` a phone lays the document out at ~980px and the
  // layout hook would resolve `desktop` on a 390px screen. `viewport-fit=cover`
  // is what makes the safe-area insets the bars pad with non-zero.
  const html = await readFile(new URL('../index.html', import.meta.url), 'utf8');
  const meta = html.match(/<meta name="viewport" content="([^"]*)"/);
  assert.ok(meta, 'the document must declare a viewport');
  assert.match(meta[1], /width=device-width/);
  assert.match(meta[1], /initial-scale=1\.0/);
  assert.match(meta[1], /viewport-fit=cover/);
  assert.equal(meta[1].includes('maximum-scale'), false, 'pinch zoom must stay available');
});

test('the compact copy is catalog copy in both languages', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const keys = [
    'mobile.menu', 'mobile.drawer', 'mobile.close', 'mobile.send',
    // The panel-level wording for a reason every tool in a panel shares.
    'mobile.refusal.unsupported',
    // The floating trigger's two actions and the reason a must-see condition
    // keeps the bar revealed.
    'mobile.commandbar.show', 'mobile.commandbar.hide', 'mobile.commandbar.pinned',
  ];
  for (const key of keys) {
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.ok(en[key].trim().length > 0 && zh[key].trim().length > 0, `empty ${key}`);
  }
  assert.equal(en['mobile.menu'], 'Menu');
  // Panel-level wording must not name a command: it stands for a whole panel.
  assert.equal(en['mobile.refusal.unsupported'].includes('{command}'), false);
  assert.equal(zh['mobile.refusal.unsupported'].includes('{command}'), false);
  assert.deepEqual(Object.keys(zh).sort(), Object.keys(en).sort(), 'the catalogs keep identical key sets');
  // No CJK literal may leak into the compact components: every word is a key.
  for (const name of [
    'components/MobileMenuButton.tsx',
    'components/MobileRibbonDrawer.tsx',
    'components/MobileCommandBar.tsx',
    'components/MobileCommandBarToggle.tsx',
    'components/CommandField.tsx',
  ]) {
    const source = await readModule(name);
    assert.equal(/[\u4e00-\u9fff]/.test(source), false, `${name} must not hardcode Chinese copy`);
  }
});

test('the host bottom inset and the breakpoint are documented for the other lanes', async () => {
  const readme = await readFile(new URL('../README.md', import.meta.url), 'utf8');
  // The Android lane sets the variable, so the contract has to be written down
  // where it will look (README + the stylesheet header).
  assert.ok(readme.includes('--yacr-safe-bottom'), 'README documents the host bottom inset');
  assert.ok(readme.includes('max(env(safe-area-inset-bottom'), 'README states the precedence');
  assert.ok(readme.includes('≤1024'), 'README names the current breakpoint');
  assert.equal(readme.includes('768px'), false, 'no README breakpoint sentence still names 768px');
  assert.equal(readme.includes('≤768'), false, 'no README breakpoint sentence still names ≤768');
  const css = await readMobileCss();
  assert.ok(css.includes('<= 1024 CSS px'), 'the stylesheet header names the current breakpoint');
});
