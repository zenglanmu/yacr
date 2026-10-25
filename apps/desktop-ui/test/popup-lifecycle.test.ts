// Popup lifecycle contract (ADR 0021 D2, WP3): the DOM half.
//
// The host owns the modal gate and its four clear triggers; the Rust contract
// tests live next to the state machine (`cad_ui_core::popup`, and
// `cad_desktop_shell::popup` for the shared host lifecycle). This suite locks
// the parts that live in the UI document:
//
//   1. explicit close — Close/Escape call `cad_close_popup`;
//   2. reload/navigation — the document cannot keep an overlay across a reload;
//   3. renderer crash — the overlay is not assumed alive after a crash;
//   4. process exit — nothing in the UI may hold the gate on its own;
//
// and the reachability rule that makes ① true even when the body fails: the
// chrome (title + close) is outside the body boundary, and Escape is handled at
// the overlay level.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { readModule } from './helpers.ts';

/** Repository root, three levels above this file (`test/` -> `desktop-ui/` -> `apps/`). */
const REPO = new URL('../../../', import.meta.url);

/** Read a Rust source file relative to the repository root. */
async function readRust(relative) {
  return readFile(new URL(relative, REPO), 'utf8');
}

test('explicit close is the overlay close path (trigger 1)', async () => {
  const overlay = await readModule('components/PopupOverlay.tsx');
  assert.ok(overlay.includes('closePopup(invoke)'), 'Close/Escape must call cad_close_popup');
  assert.ok(overlay.includes("if (event.key !== 'Escape'"), 'Escape is the overlay accelerator');
  assert.ok(overlay.includes('if (event.isComposing || event.keyCode === 229) return;'),
    'an IME composition keeps its Escape');
  assert.ok(overlay.startsWith('//'), 'the overlay documents its ownership');
});

test('close and Escape survive a body that fails to render', async () => {
  const overlay = await readModule('components/PopupOverlay.tsx');
  const chromeAt = overlay.indexOf('<header className="palette__head"');
  const boundaryAt = overlay.indexOf('<PopupBodyBoundary');
  assert.ok(chromeAt !== -1, 'the persistent chrome must render a header');
  assert.ok(boundaryAt !== -1, 'the body must sit behind an error boundary');
  assert.ok(chromeAt < boundaryAt, 'chrome must be rendered outside/before the body boundary');
  assert.ok(overlay.includes('componentDidCatch'), 'body failures must be caught');
  assert.ok(overlay.includes('POPUP_KEYS.bodyFailed'), 'a failed body gets explicit catalog copy');
  // The Escape listener is registered by the overlay component, not by the body.
  const listenerAt = overlay.indexOf("window.addEventListener('keydown', onKeyDown)");
  assert.ok(listenerAt !== -1);
  assert.ok(listenerAt < boundaryAt, 'the Escape listener is installed above the body boundary');
});

test('the shell accelerator stands down while the overlay owns Escape', async () => {
  const app = await readModule('App.tsx');
  // Only the parameter annotation is new: the shell handler still owns the shared
  // keydown hook, and the popup guard must sit before its plain-key branches.
  const shellShortcut = /const onKeyDown = \(event(?:: [\w.]+)?\) => \{/.exec(app);
  assert.ok(shellShortcut);
  const shellShortcutAt = shellShortcut.index;
  const guardAt = app.indexOf('if (popupOpen) return;', shellShortcutAt);
  assert.ok(guardAt !== -1 && guardAt - shellShortcutAt < 400,
    'the shell handler must return before any plain-key branch while a popup is open');
  assert.ok(app.includes('[popupOpen, submitCommand, toolActive, gripDragActive]'),
    'the guard must re-install when the popup, tool or armed-drag state changes');
});

test('reload and renderer crash are wired to the shared lifecycle (triggers 2 and 3)', async () => {
  const init = await readRust('apps/app-linux/src/host/init.rs');
  assert.ok(init.includes('cad_desktop_shell::popup::on_reload('),
    'on_page_load must reset the gate on a reload/navigation');
  assert.ok(init.includes('PageLoadEvent::Started'),
    'the reload trigger is the Started page-load event');
  assert.ok(init.includes('install_renderer_crash_hook'),
    'a renderer crash must be wired to the shared lifecycle');

  const hostPopup = await readRust('apps/app-linux/src/host/popup.rs');
  assert.ok(hostPopup.includes('"web-process-crashed"'),
    'the hook subscribes to WebKitGTK web-process-crashed');
  assert.ok(hostPopup.includes('on_renderer_crash('),
    'the crash hook clears the gate and records the diagnostic');
});

test('process exit clears the gate (trigger 4)', async () => {
  const entry = await readRust('apps/app-linux/src/entry.rs');
  const exits = entry.split('RunEvent::ExitRequested');
  assert.ok(exits.length > 1, 'the entry must handle ExitRequested');
  assert.ok(entry.includes('cad_desktop_shell::popup::on_process_exit(&run_shared);'),
    'the process-exit trigger must call the shared lifecycle');
  assert.ok(entry.split('on_process_exit').length > 2,
    'both ExitRequested and Exit must clear (idempotently)');
});

test('the shared host lifecycle owns the four triggers, not the app', async () => {
  const shared = await readRust('crates/cad-desktop-shell/src/popup.rs');
  for (const fn of ['pub fn close(', 'pub fn on_reload(', 'pub fn on_renderer_crash(', 'pub fn on_process_exit(']) {
    assert.ok(shared.includes(fn), `cad-desktop-shell::popup must expose ${fn}`);
  }
  // The crash diagnostic is technical English, not catalog copy.
  assert.ok(shared.includes('RENDERER_CRASH_ERROR'));
  assert.ok(shared.includes('HostErrorKind::Renderer'));

  const app = await readRust('apps/app-linux/src/host/popup.rs');
  // The app module no longer owns a gate of its own; it maps onto the shared
  // lifecycle and keeps only the main-thread payload derivation.
  assert.ok(app.includes('lifecycle::open('));
  assert.ok(app.includes('lifecycle::close('));
  assert.equal(app.includes('struct SharedGate'), false, 'the app must not re-implement the gate');
});

test('the D1 second-window mechanism is gone', async () => {
  const app = await readRust('apps/app-linux/src/host/popup.rs');
  for (const forbidden of [
    'create_popup_window',
    'TauriPopupOps',
    'POPUP_WEBVIEW_LABEL',
    'popup_window_for_probe',
  ]) {
    assert.equal(app.includes(forbidden), false, `the app popup module must not contain ${forbidden}`);
  }
  const ui = await readModule('popup.ts');
  for (const forbidden of ['POPUP_GLOBAL', 'readPopupBoot', 'popupBoot', '__YACR_POPUP__']) {
    assert.equal(ui.includes(forbidden), false, `popup.mjs must not keep ${forbidden}`);
  }
  const overlay = await readModule('components/PopupOverlay.tsx');
  assert.equal(overlay.includes('__YACR_POPUP__'), false);
});
