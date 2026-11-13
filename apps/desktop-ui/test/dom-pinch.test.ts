// The DOM pinch arithmetic: `ctrl`+wheel (WebView2/Chromium) and the WebKit
// gesture sequence both have to produce one **incremental** CAD zoom
// multiplier, never `NaN`, never a division by zero, and never the cumulative
// value (the host would zoom twice).
import test from 'node:test';
import assert from 'node:assert/strict';
import { GesturePinch, PINCH_SCALE_PER_PIXEL, wheelPinchScale } from '../src/cad/dom-pinch.ts';

test('the pinch constant matches the host wheel constant', () => {
  assert.equal(PINCH_SCALE_PER_PIXEL, 0.0015);
});

test('wheelPinchScale zooms in for a negative delta and out for a positive one', () => {
  // Chromium reports a pinch-out (spread) as a negative deltaY.
  assert.ok(wheelPinchScale(-100) > 1);
  assert.ok(wheelPinchScale(100) < 1);
  // Strictly positive across the range a real wheel/pinch sample can produce.
  assert.ok(wheelPinchScale(1000) > 0);
  assert.equal(wheelPinchScale(0), 1);
});

test('wheelPinchScale never returns NaN for a non-finite delta', () => {
  for (const delta of [NaN, Infinity, -Infinity]) {
    assert.equal(wheelPinchScale(delta), 1);
  }
});

test('successive wheel pinch samples compose by multiplication', () => {
  // The lane merges two queued pinch records as `tail.scale * input.scale`, so
  // one 60 px sample must equal two 30 px samples.
  const one = wheelPinchScale(-60);
  const two = wheelPinchScale(-30) * wheelPinchScale(-30);
  assert.ok(Math.abs(one - two) < 1e-12);
});

test('GesturePinch reports incremental ratios, not the cumulative scale', () => {
  const pinch = new GesturePinch();
  pinch.start(1);
  assert.equal(pinch.change(1.5), 1.5);
  // A cumulative `2.25` after an applied `1.5` is one more 1.5× step.
  assert.equal(pinch.change(2.25), 1.5);
  pinch.end();
});

test('a gesture without start works from a baseline of 1', () => {
  const pinch = new GesturePinch();
  assert.equal(pinch.change(2), 2);
  assert.equal(pinch.change(1), 0.5);
});

test('a no-op sample reports null instead of a 1× record', () => {
  const pinch = new GesturePinch();
  pinch.start(1.25);
  assert.equal(pinch.change(1.25), null);
  pinch.start(1);
  assert.equal(pinch.change(1), null);
});

test('invalid input is refused without moving the baseline', () => {
  const pinch = new GesturePinch();
  pinch.start(2);
  for (const scale of [0, -1, NaN, Infinity, -Infinity]) {
    assert.equal(pinch.change(scale), null);
  }
  // The refused samples left the 2× baseline intact, so the next real sample is
  // still relative to it.
  assert.equal(pinch.change(3), 1.5);
});

test('an invalid start falls back to 1 and end resets the baseline', () => {
  const pinch = new GesturePinch();
  for (const scale of [0, -1, NaN, Infinity]) {
    pinch.start(scale);
    assert.equal(pinch.change(1), null, 'baseline is 1, so 1 is no zoom');
  }
  pinch.start(4);
  pinch.end();
  // `end` forgot the 4× baseline: the next gesture starts over at 1.
  assert.equal(pinch.change(5), 5);
});
