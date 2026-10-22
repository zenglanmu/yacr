// The webview-default guards: the native context menu (except in text fields),
// the browser reload/print/page-zoom accelerators, Ctrl+wheel page zoom, and the
// single inert CAD shortcut-menu hook.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CAD_PARTITION_SELECTOR,
  bindBrowserDefaults,
  browserCommandForKey,
  cadContextMenuRequest,
  isBrowserZoomWheel,
  isTextEntryTarget,
} from '../src/defaults.mjs';
import { bindProbeInput } from '../src/input.mjs';
import { readAppSource } from './helpers.mjs';

function fakeTarget() {
  class Target {
    handlers = new Map();
    addEventListener(type, callback) { this.handlers.set(type, callback); }
    removeEventListener(type) { this.handlers.delete(type); }
    emit(type, event = {}) { this.handlers.get(type)?.(event); }
  }
  return new Target();
}

function fakeEnvironment() {
  const env = fakeTarget();
  Object.assign(env, {
    innerWidth: 800,
    innerHeight: 600,
    devicePixelRatio: 1,
    requestAnimationFrame: () => 1,
    cancelAnimationFrame: () => {},
    setTimeout: () => 1,
    clearTimeout: () => {},
    matchMedia: () => fakeTarget(),
    ResizeObserver: class { observe() {} disconnect() {} },
  });
  return env;
}

function contextMenuEvent(target, clientX, clientY) {
  const event = { target, clientX, clientY, prevented: 0, preventDefault() { event.prevented++; } };
  return event;
}

function cadTarget(rect = { left: 100, top: 50, width: 800, height: 600 }) {
  return {
    rect,
    closest(selector) { return selector === CAD_PARTITION_SELECTOR ? this : null; },
    getBoundingClientRect() { return this.rect; },
  };
}

function chromeTarget() {
  return { closest: () => null };
}

test('the context menu is suppressed on chrome and the CAD partition, but not in text fields', () => {
  const env = fakeEnvironment();
  const calls = [];
  bindBrowserDefaults(env, { onContextMenu: detail => calls.push(detail) });

  const outside = contextMenuEvent(chromeTarget(), 10, 10);
  env.emit('contextmenu', outside);
  assert.equal(outside.prevented, 1, 'chrome right-click is prevented');
  assert.equal(calls.length, 0, 'only the CAD partition owns the hook');

  const cad = contextMenuEvent(cadTarget(), 130, 70);
  env.emit('contextmenu', cad);
  assert.equal(cad.prevented, 1, 'the CAD right-click is prevented');
  assert.equal(calls.length, 1, 'the CAD hook fires once');

  // The command line (and the tool text step) keep WebKit's native menu so
  // copy/paste works. The CAD hook never fires for a text target.
  for (const target of [
    { tagName: 'INPUT' },
    { tagName: 'TEXTAREA' },
    { tagName: 'SPAN', closest: selector => (selector === '[contenteditable]' ? {} : null) },
    { tagName: 'DIV', isContentEditable: true },
  ]) {
    const event = contextMenuEvent(target, 130, 70);
    env.emit('contextmenu', event);
    assert.equal(event.prevented, 0, `native menu stays for ${target.tagName}`);
  }
  assert.equal(calls.length, 1, 'no text-entry right-click reaches the CAD hook');
});

test('the CAD hook carries the point relative to the partition plus selection and tool', () => {
  const env = fakeEnvironment();
  const selection = { count: 1, empty: false, rows: [{ handle: 7 }], mixedKeys: [] };
  const tool = { key: 'LINE', step: 2 };
  const calls = [];
  bindBrowserDefaults(env, {
    context: () => ({ selection, tool }),
    onContextMenu: detail => calls.push(detail),
  });

  const event = contextMenuEvent(cadTarget({ left: 100, top: 50, width: 800, height: 600 }), 130, 70);
  env.emit('contextmenu', event);

  assert.equal(event.prevented, 1);
  assert.equal(calls.length, 1, 'exactly one call per right-click');
  assert.deepEqual(calls[0], {
    point: { x: 30, y: 20 },
    selection,
    tool,
  });

  // The shipped hook is inert: no menu, no panel, no host mutation.
  assert.equal(cadContextMenuRequest(calls[0]), undefined);
  assert.equal(cadContextMenuRequest(), undefined);
});

test('reload, print and page-zoom accelerators are blocked even with a text field focused', () => {
  const blocked = [
    [{ key: 'r', ctrlKey: true }, 'reload'],
    [{ key: 'R', ctrlKey: true, shiftKey: true }, 'reload'],
    [{ key: 'F5' }, 'reload'],
    [{ key: 'F5', ctrlKey: true, shiftKey: true }, 'reload'],
    [{ key: '=', ctrlKey: true }, 'zoom-in'],
    [{ key: '+', ctrlKey: true, shiftKey: true }, 'zoom-in'],
    [{ key: '=', ctrlKey: true, code: 'NumpadAdd' }, 'zoom-in'],
    [{ key: '-', ctrlKey: true }, 'zoom-out'],
    [{ key: '_', ctrlKey: true, shiftKey: true }, 'zoom-out'],
    [{ key: '0', ctrlKey: true }, 'zoom-reset'],
    [{ key: ')', ctrlKey: true, shiftKey: true }, 'zoom-reset'],
    [{ key: 'p', ctrlKey: true }, 'print'],
    [{ key: 'P', ctrlKey: true, shiftKey: true }, 'print'],
  ];
  for (const [props, expected] of blocked) {
    const env = fakeEnvironment();
    const seen = [];
    bindBrowserDefaults(env, { onBlocked: command => seen.push(command) });
    const event = { target: { tagName: 'INPUT' }, preventDefault() { event.prevented = true; }, ...props };
    env.emit('keydown', event);
    assert.equal(event.prevented, true, `${props.key} default is prevented`);
    assert.deepEqual(seen, [expected], `${props.key} reports ${expected}`);
  }
});

test('ordinary typing, Enter/Escape, F2/F3 and other chords are not swallowed', () => {
  const env = fakeEnvironment();
  const seen = [];
  bindBrowserDefaults(env, { onBlocked: command => seen.push(command) });
  const allowed = [
    { key: 'a' },
    { key: 'r' },
    { key: 'p' },
    { key: 'Enter' },
    { key: 'Escape', isComposing: true },
    { key: 'F2' },
    { key: 'F3' },
    { key: '0', ctrlKey: false },
    { key: '9', ctrlKey: true },
    { key: 'z', ctrlKey: true, shiftKey: true },
    { key: 'r', altKey: true },
  ];
  for (const props of allowed) {
    const event = { target: { tagName: 'INPUT' }, preventDefault() { event.prevented = true; }, ...props };
    env.emit('keydown', event);
    assert.notEqual(event.prevented, true, `${props.key} is left alone`);
  }
  assert.deepEqual(seen, [], 'no browser command was reported');
});

test('Ctrl+wheel page zoom is blocked; a plain wheel is not', () => {
  const env = fakeEnvironment();
  const seen = [];
  bindBrowserDefaults(env, { onBlocked: command => seen.push(command) });

  const zoom = { ctrlKey: true, preventDefault() { zoom.prevented = true; } };
  env.emit('wheel', zoom);
  assert.equal(zoom.prevented, true);
  assert.deepEqual(seen, ['zoom']);
  assert.equal(isBrowserZoomWheel(zoom), true);

  const pan = { ctrlKey: false, shiftKey: true, preventDefault() { pan.prevented = true; } };
  env.emit('wheel', pan);
  assert.notEqual(pan.prevented, true);
  assert.equal(isBrowserZoomWheel(pan), false);
  assert.deepEqual(seen, ['zoom']);
});

test('a right-click still forwards pointer_down as button 2 and a drag as button 1', () => {
  const env = fakeEnvironment();
  const element = fakeTarget();
  element.rect = { left: 0, top: 0, width: 800, height: 600 };
  element.getBoundingClientRect = () => element.rect;
  element.closest = () => null;
  element.parentElement = null;
  let captured = null;
  element.setPointerCapture = id => { captured = id; };
  element.hasPointerCapture = id => captured === id;
  element.releasePointerCapture = id => { captured = null; };
  const inputs = [];
  const lane = { canStart: () => true, push: input => inputs.push(input) };
  const binding = bindProbeInput(element, lane, env);
  bindBrowserDefaults(env, {});

  element.emit('pointerdown', { pointerId: 1, clientX: 5, clientY: 6, button: 2 });
  element.emit('pointerup', { pointerId: 1, clientX: 5, clientY: 6, button: 2 });
  element.emit('pointerdown', { pointerId: 2, clientX: 7, clientY: 8, button: 1 });
  element.emit('pointerup', { pointerId: 2, clientX: 7, clientY: 8, button: 1 });

  const downs = inputs.filter(input => input.kind === 'pointer_down');
  assert.deepEqual(downs.map(input => input.button), [2, 1]);
  assert.deepEqual(downs[0], { kind: 'pointer_down', x: 5, y: 6, button: 2, shift: false });

  // The context-menu guard never touches the input lane.
  const before = inputs.length;
  const cad = contextMenuEvent(cadTarget(), 10, 10);
  env.emit('contextmenu', cad);
  assert.equal(cad.prevented, 1);
  assert.equal(inputs.length, before, 'right-click suppression adds no input');
  binding.dispose();
});

test('disposing the guards removes every listener', () => {
  const env = fakeEnvironment();
  const guard = bindBrowserDefaults(env, {});
  assert.deepEqual([...env.handlers.keys()].sort(), ['contextmenu', 'keydown', 'wheel']);
  guard.dispose();
  assert.equal(env.handlers.size, 0);
});

test('the app shell installs the guards after its shortcuts, with status context', async () => {
  const app = await readAppSource();
  assert.ok(app.includes("import { bindBrowserDefaults } from './defaults.mjs';"));
  assert.ok(app.includes('bindBrowserDefaults(window, {'));
  assert.ok(app.includes('selection: current?.selection ?? null, tool: current?.tool ?? null'));
  // Ctrl+0 is both the app's clean-screen toggle and WebKit's zoom reset: the
  // app owns it before its `defaultPrevented` check, so listener order with the
  // guard cannot disable clean screen.
  assert.ok(app.indexOf("!event.altKey && event.key === '0'")
    < app.indexOf('bindBrowserDefaults(window, {'));
  assert.ok(browserCommandForKey({ key: '0', ctrlKey: true }) === 'zoom-reset');
  assert.ok(isTextEntryTarget({ tagName: 'INPUT' }));
});
