// Contract tests for the frontend vendor-plugin host (ADR 0028 D3/D4/D5).
//
// These drive the real host module through its test seam, with a fake plugin
// module: no `plugins/tch.js` load, no Rust session, no DOM.
import test from 'node:test';
import assert from 'node:assert/strict';

import {
  currentPluginNoticeForTests,
  currentPluginPanelForTests,
  dispatchCadStatus,
  dismissPluginNotice,
  pluginCommand,
  registerPluginModuleForTests,
  requestProperties,
  resetPluginHostForTests,
  showPluginNotice,
} from '../src/plugins/host.ts';

const PANEL = Object.freeze({
  kind: 'TCH_WALL',
  handle: '1A2B',
  groups: [
    {
      key: 'identity',
      rows: [
        { key: 'identity.handle', value: '1A2B' },
        { key: 'identity.layer', value: 'WALL' },
      ],
    },
  ],
});

/** A `cad_status.selection`-shaped block; only `count`/`rows` matter here. */
function selection(handle = '1A2B', count = 1) {
  return {
    count,
    empty: count === 0,
    rows: [{
      key: 'handle', value: handle, group: 'entity',
      mixed: false, editable: false, kind: 'text', options: [],
    }],
    mixedKeys: [],
    text: null,
    text_multiline: false,
  };
}

/** A module whose only real behaviour is the registration hook. */
function pluginModule(register) {
  return { abiVersion: 1, decode: () => new ArrayBuffer(0), register };
}

test('properties asks providers in order: first non-null panel wins, null clears', () => {
  resetPluginHostForTests();
  let panel = PANEL;
  registerPluginModuleForTests(pluginModule(host => {
    host.on('properties', () => null);
    host.on('properties', () => panel);
  }));

  requestProperties(selection());
  assert.equal(currentPluginPanelForTests(), PANEL);

  panel = null;
  requestProperties(selection());
  assert.equal(currentPluginPanelForTests(), null);
});

test('a module without register contributes no handlers and is not an error', () => {
  resetPluginHostForTests();
  registerPluginModuleForTests({ abiVersion: 1, decode: () => new ArrayBuffer(0) });

  requestProperties(selection());
  assert.equal(currentPluginPanelForTests(), null);
  assert.equal(pluginCommand('MOVE', selection()), null);
});

test('command interception returns the first handled decision and ignores null', () => {
  resetPluginHostForTests();
  registerPluginModuleForTests(pluginModule(host => {
    host.on('command', () => null);
    host.on('command', request => (
      request.command === 'MOVE' ? { handled: true, messageKey: 'vendor.editing.unsupported' } : null
    ));
  }));
  assert.deepEqual(pluginCommand('MOVE', selection()), {
    handled: true,
    messageKey: 'vendor.editing.unsupported',
  });
  assert.equal(pluginCommand('LINE', selection()), null);

  resetPluginHostForTests();
  registerPluginModuleForTests(pluginModule(host => {
    host.on('command', () => ({ handled: true, messageKey: 'first' }));
    host.on('command', () => ({ handled: true, messageKey: 'second' }));
  }));
  assert.deepEqual(pluginCommand('MOVE', selection()), { handled: true, messageKey: 'first' });
});

test('handler exceptions pass through to the next handler and to the router', () => {
  resetPluginHostForTests();
  const statuses = [];
  registerPluginModuleForTests(pluginModule(host => {
    host.on('properties', () => { throw new Error('properties failed'); });
    host.on('properties', () => PANEL);
    host.on('command', () => { throw new Error('command failed'); });
    host.on('command', () => ({ handled: true, messageKey: 'vendor.editing.unsupported' }));
    host.on('cad-status', () => { throw new Error('status failed'); });
    host.on('cad-status', status => statuses.push(status.documentLabel));
  }));

  requestProperties(selection());
  assert.equal(currentPluginPanelForTests(), PANEL);
  assert.deepEqual(pluginCommand('MOVE', selection()), {
    handled: true,
    messageKey: 'vendor.editing.unsupported',
  });
  dispatchCadStatus({ documentLabel: 'fixture.dwg', selection: selection() });
  assert.deepEqual(statuses, ['fixture.dwg']);
});

test('a malformed panel is rejected as null, never repaired into a guessed panel', () => {
  resetPluginHostForTests();
  registerPluginModuleForTests(pluginModule(host => {
    host.on('properties', () => ({
      kind: 'TCH_WALL',
      handle: '1A2B',
      groups: [{ key: 'identity', rows: [{ key: 'identity.handle', value: 42 }] }],
    }));
  }));
  requestProperties(selection());
  assert.equal(currentPluginPanelForTests(), null);
});

test('a panel returned for a multi-selection is rejected with the diagnostic notice', () => {
  resetPluginHostForTests();
  registerPluginModuleForTests(pluginModule(host => {
    host.on('properties', () => PANEL);
  }));

  requestProperties(selection('mixed', 2));
  assert.equal(currentPluginPanelForTests(), null);
  assert.equal(currentPluginNoticeForTests()?.key, 'vendor.plugin.multi_selection_unsupported');
});

test('the notice store reports, replaces and dismisses one notice', () => {
  resetPluginHostForTests();
  showPluginNotice('vendor.editing.unsupported', { command: 'MOVE' });
  assert.deepEqual(currentPluginNoticeForTests(), {
    key: 'vendor.editing.unsupported',
    params: { command: 'MOVE' },
  });
  showPluginNotice('properties.vendor.unknown_key', { key: 'plugin.missing' });
  assert.equal(currentPluginNoticeForTests()?.key, 'properties.vendor.unknown_key');
  dismissPluginNotice();
  assert.equal(currentPluginNoticeForTests(), null);
});
