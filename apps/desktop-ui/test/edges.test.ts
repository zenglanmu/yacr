// The Styles tray's body-edge toggle is the payload-free EDGES selector.

import assert from 'node:assert/strict';
import test from 'node:test';

import { admitCommandText, setCommandCatalog } from '../src/commands/commands.ts';

test('EDGES is a bare session toggle, not a typed argument', () => {
  setCommandCatalog(['EDGES']);
  const status = { documentLabel: 'demo', loading: false };
  assert.equal(admitCommandText('EDGES', { status }).kind, 'admitted');
  assert.equal(
    admitCommandText('EDGES ON', { status }).kind,
    'unsupported',
    'the toggle has no argument grammar; the tray flips it by re-running EDGES',
  );
});
