// The command line suspends the CAD pointer lane while it has focus, so a press
// on the drawing area must release that focus *before* the same press reaches
// `.native-input`. These tests model the browser's dispatch order (window
// capture listeners, then the target's listeners) so the one-click contract is
// exercised end to end with the real input binding.
import test from 'node:test';
import assert from 'node:assert/strict';
import { bindProbeInput } from '../src/cad/input.ts';
import { releaseFocusOnOutsidePress } from '../src/focus-release.ts';
import { readModule } from './helpers.ts';

class Target {
  constructor() { this.listeners = new Map(); }
  addEventListener(type, callback, options) {
    const capture = options === true || options?.capture === true;
    const list = this.listeners.get(type) ?? [];
    list.push({ callback, capture });
    this.listeners.set(type, list);
  }
  removeEventListener(type, callback, options) {
    const capture = options === true || options?.capture === true;
    this.listeners.set(type, (this.listeners.get(type) ?? [])
      .filter(entry => entry.callback !== callback || entry.capture !== capture));
  }
  emit(type, event = {}) {
    for (const entry of [...(this.listeners.get(type) ?? [])]) entry.callback(event);
  }
  count(type, capture) {
    return (this.listeners.get(type) ?? []).filter(entry => entry.capture === capture).length;
  }
}

function environment() {
  const env = new Target();
  Object.assign(env, { innerWidth: 800, innerHeight: 600, devicePixelRatio: 1, document: { documentElement: {} } });
  env.requestAnimationFrame = () => 1;
  env.cancelAnimationFrame = () => {};
  env.setTimeout = () => 1;
  env.clearTimeout = () => {};
  env.matchMedia = () => new Target();
  env.ResizeObserver = class { observe() {} disconnect() {} };
  return env;
}

function surface() {
  const element = new Target();
  element.rect = { left: 0, top: 0, width: 800, height: 600 };
  element.getBoundingClientRect = () => element.rect;
  let captured = null;
  element.setPointerCapture = id => { captured = id; };
  element.hasPointerCapture = id => captured === id;
  element.releasePointerCapture = () => { captured = null; };
  return element;
}

// The command line's own focus wiring: `focus` suspends the lane and `blur`
// restores it — exactly what `onFocus` / `onBlur` call in the component.
function commandLine(binding) {
  const suspensions = [];
  const input = {
    focused: false,
    focus() { this.focused = true; suspensions.push(true); binding.suspend(true); },
    blur() { if (!this.focused) return; this.focused = false; suspensions.push(false); binding.suspend(false); },
  };
  const root = { contains: node => node === input };
  return { input, root, suspensions };
}

// One press as the browser dispatches it: the window's capture listeners fire
// first, then the listeners on the event target.
function press(env, target, event) {
  env.emit('pointerdown', event);
  if (event.target === target) target.emit('pointerdown', event);
}

function setup() {
  const env = environment();
  const element = surface();
  const inputs = [];
  const binding = bindProbeInput(element, {
    canStart: () => true,
    push: input => { inputs.push(input); return true; },
  }, env);
  return { env, element, inputs, binding };
}

test('a press outside releases focus first and still reaches the suspended lane', () => {
  const { env, element, inputs, binding } = setup();
  const { input, root, suspensions } = commandLine(binding);
  input.focus();
  assert.deepEqual(suspensions, [true]);

  // Control: without the hand-off the press is dropped, which is the defect.
  press(env, element, { target: element, pointerId: 1, clientX: 200, clientY: 200, button: 0, shiftKey: false });
  assert.deepEqual(inputs.map(entry => entry.kind), ['resize'], 'a suspended lane drops the press');

  const dispose = releaseFocusOnOutsidePress(root, () => input.blur(), env);
  let prevented = 0;
  let stopped = 0;
  const event = {
    target: element,
    pointerId: 2,
    clientX: 210,
    clientY: 220,
    button: 0,
    shiftKey: false,
    preventDefault() { prevented += 1; },
    stopPropagation() { stopped += 1; },
  };
  press(env, element, event);

  assert.equal(input.focused, false, 'the press blurs the focused command line');
  assert.deepEqual(suspensions, [true, false], 'blur unsuspends the lane');
  assert.deepEqual(inputs.map(entry => entry.kind), ['resize', 'pointer_down'], 'the same press reaches the lane exactly once');
  assert.deepEqual(
    { x: inputs[1].x, y: inputs[1].y, button: inputs[1].button },
    { x: 210, y: 220, button: 0 },
  );
  assert.equal(prevented, 0, 'the press is never cancelled');
  assert.equal(stopped, 0, 'the press is never stopped');
  dispose();
});

test('a press inside the command line keeps focus and the lane suspended', () => {
  const { env, inputs, binding } = setup();
  const { input, root, suspensions } = commandLine(binding);
  input.focus();
  const dispose = releaseFocusOnOutsidePress(root, () => input.blur(), env);

  env.emit('pointerdown', { target: input, pointerId: 1, clientX: 10, clientY: 10, button: 0 });

  assert.equal(input.focused, true, 'its own controls do not take focus away');
  assert.deepEqual(suspensions, [true], 'the lane stays suspended');
  assert.deepEqual(inputs.map(entry => entry.kind), ['resize'], 'no canvas press is forwarded');
  dispose();
});

test('the capture listener is installed once and removed on teardown', () => {
  const env = environment();
  const root = { contains: () => false };
  let releases = 0;
  const release = () => { releases += 1; };

  // Model the effect: it runs only when `focused` changes, and each run returns
  // its own disposer (React runs the previous one first).
  let focused = false;
  let teardown = null;
  const render = next => {
    if (next === focused) return;
    focused = next;
    teardown?.();
    teardown = focused ? releaseFocusOnOutsidePress(root, release, env) : null;
  };

  render(true);
  render(true);
  render(true);
  assert.equal(env.count('pointerdown', true), 1, 're-renders never duplicate the listener');
  assert.equal(env.count('pointerdown', false), 0, 'the listener is capture-phase');

  env.emit('pointerdown', { target: {} });
  assert.equal(releases, 1);

  render(false);
  assert.equal(env.count('pointerdown', true), 0, 'blur removes it');
  env.emit('pointerdown', { target: {} });
  assert.equal(releases, 1, 'a removed listener never fires');

  // Mount/unmount on its own: the effect's cleanup is the only removal path.
  render(true);
  assert.equal(env.count('pointerdown', true), 1);
  teardown();
  assert.equal(env.count('pointerdown', true), 0, 'unmount removes it');
});

test('the command line wires the hand-off for exactly the focused period', async () => {
  const source = await readModule('components/CommandLine.tsx');
  assert.ok(source.includes("import { releaseFocusOnOutsidePress } from '../focus-release.ts';"));
  // Only the type argument is new: the ref is still created without an initial node.
  assert.match(source, /const rootRef = useRef(?:<[^>]*>)?\(null\);/);
  assert.ok(source.includes('ref={rootRef}'));
  assert.ok(source.includes('if (!focused) return undefined;'));
  // The helper still gets the live node; the cast only satisfies its narrow root type.
  assert.match(source, /return releaseFocusOnOutsidePress\(rootRef\.current(?:\s+as\s+[^,]+)?, \(\) => input\?\.blur\(\)\);/);
  assert.ok(source.includes('}, [focused]);'));
  // The press must keep travelling: no preventDefault/stopPropagation in the helper.
  const helper = await readModule('focus-release.ts');
  assert.ok(helper.includes("env.addEventListener('pointerdown', handler, true)"));
  assert.ok(helper.includes("env.removeEventListener('pointerdown', handler, true)"));
  assert.ok(!helper.includes('event.preventDefault'));
  assert.ok(!helper.includes('event.stopPropagation'));
});
