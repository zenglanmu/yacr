// Contracts for the second pixel carrier (`hostProfile.surfaceMode: dom-canvas`,
// ADR 0024 D3).
//
// Scope, stated honestly: these are **pure-function** and **source-structure**
// tests. There is no DOM mount, no browser run and no pixel evidence here. What
// they pin is (a) the nested-profile reads and the fallback policy for a host
// that reports no profile, and (b) the shape of the conditional that mounts the
// carrier.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  SURFACE_MODE_DOM_CANVAS,
  SURFACE_MODE_NATIVE_BELOW,
  SURFACE_MODE_UNKNOWN,
  palettesOffered,
  surfaceModeFromShell,
} from '../src/cad/surface.ts';
import { readAppSource, readModule } from './helpers.ts';

test('the two modelled carriers have the wire names the host reports', () => {
  assert.equal(SURFACE_MODE_NATIVE_BELOW, 'native-below');
  assert.equal(SURFACE_MODE_DOM_CANVAS, 'dom-canvas');
  assert.equal(SURFACE_MODE_UNKNOWN, 'unknown');
});

test('an explicit host report selects exactly that carrier', () => {
  assert.equal(
    surfaceModeFromShell({ status: { hostProfile: { surfaceMode: 'native-below' } } }),
    SURFACE_MODE_NATIVE_BELOW,
  );
  assert.equal(
    surfaceModeFromShell({ status: { hostProfile: { surfaceMode: 'dom-canvas' } } }),
    SURFACE_MODE_DOM_CANVAS,
  );
  // The field is read from the live shell status, not from a launch option or a
  // platform hint: an unrelated field next to it changes nothing.
  assert.equal(
    surfaceModeFromShell({
      status: {
        hostProfile: { surfaceMode: 'dom-canvas' },
        crosshair: false,
        documentLabel: 'x',
      },
    }),
    SURFACE_MODE_DOM_CANVAS,
  );
  // The raw status and the wrapped shell value are the same reading.
  assert.equal(
    surfaceModeFromShell({ hostProfile: { surfaceMode: 'dom-canvas' } }),
    SURFACE_MODE_DOM_CANVAS,
  );
});

test('a missing profile never guesses: it is the native-below fact fallback', () => {
  // The same policy as `shell.status?.crosshair`: a pre-P1 host reports no
  // `hostProfile`, and `native-below` is the only carrier any shipped host has.
  assert.equal(surfaceModeFromShell({}), SURFACE_MODE_NATIVE_BELOW);
  assert.equal(surfaceModeFromShell({ status: null }), SURFACE_MODE_NATIVE_BELOW);
  assert.equal(surfaceModeFromShell({ status: {} }), SURFACE_MODE_NATIVE_BELOW);
  assert.equal(surfaceModeFromShell({ status: { hostProfile: {} } }), SURFACE_MODE_NATIVE_BELOW);
  assert.equal(surfaceModeFromShell({ status: { hostProfile: null } }), SURFACE_MODE_NATIVE_BELOW);
  assert.equal(surfaceModeFromShell(undefined), SURFACE_MODE_NATIVE_BELOW);
  assert.equal(surfaceModeFromShell(null), SURFACE_MODE_NATIVE_BELOW);
});

test('a present-but-unrecognised value is explicit, never a native-below guess', () => {
  for (const value of ['DOM-CANVAS', 'Dom-Canvas', 'dom-canvas ', ' dom-canvas', 'dom_canvas', 'canvas', '', 0, 1, true, {}, [], null]) {
    assert.equal(
      surfaceModeFromShell({ status: { hostProfile: { surfaceMode: value } } }),
      SURFACE_MODE_UNKNOWN,
      `surfaceMode ${JSON.stringify(value)} must not select a modelled carrier`,
    );
  }
});

test('palettesOffered reads the nested capability and keeps absent => true', () => {
  assert.equal(palettesOffered({ status: { hostProfile: { palettesAvailable: false } } }), false);
  assert.equal(palettesOffered({ status: { hostProfile: { palettesAvailable: true } } }), true);
  // Raw status and the shared shell value are both accepted.
  assert.equal(palettesOffered({ hostProfile: { palettesAvailable: false } }), false);
  assert.equal(palettesOffered({ status: {} }), true);
  assert.equal(palettesOffered({ status: { hostProfile: {} } }), true);
  assert.equal(palettesOffered({ status: { hostProfile: { palettesAvailable: undefined } } }), true);
  assert.equal(palettesOffered({ status: { hostProfile: { palettesAvailable: 'false' } } }), true);
  assert.equal(palettesOffered({}), true);
  assert.equal(palettesOffered(null), true);
  assert.equal(palettesOffered(undefined), true);
});

test('the workspace mounts the carrier from the helper, never from a local mirror', async () => {
  const workspace = await readModule('components/Workspace.tsx');
  assert.ok(workspace.includes("from '../cad/surface.ts'"));
  assert.ok(workspace.includes('surfaceModeFromShell(shell)'));
  assert.ok(workspace.includes('SURFACE_MODE_DOM_CANVAS'));
  // No local state, no platform sniffing: the mode is a data flag only.
  assert.ok(!workspace.includes('useState'));
  assert.ok(!workspace.includes('platform'));
});

test('the shipped tree has exactly one canvas: the dom-canvas carrier', async () => {
  // A second canvas anywhere would be a second pixel surface. Prose mentions of
  // ``<canvas>`` (comments/docs) carry no attributes, so only real elements
  // match here; the stacking regression fixture stays canvas-free on purpose.
  const app = await readAppSource();
  const elements = [...app.matchAll(/<canvas\s+[a-zA-Z][^>]*>/g)];
  assert.equal(elements.length, 1);
  assert.ok(elements[0][0].includes('id="cad-canvas"'));
});
