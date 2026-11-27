// The Components panel's two actions: both are the ordinary typed selectors,
// with the block name supplied up front.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  componentDefineCommand,
  componentInsertCommand,
} from '../src/domains/concept/components.ts';

test('a listed block inserts through the typed INSERT selector', () => {
  assert.equal(componentInsertCommand('Widget'), 'INSERT Widget');
  assert.equal(componentInsertCommand('  Cabin  '), 'INSERT Cabin');
  assert.equal(componentInsertCommand(''), null);
  assert.equal(componentInsertCommand('A\nB'), null);
  assert.equal(componentInsertCommand(null), null);
});

test('creating a component uses the typed BLOCK selector', () => {
  assert.equal(componentDefineCommand('Cabin'), 'BLOCK Cabin');
  assert.equal(componentDefineCommand('  A  '), 'BLOCK A');
  assert.equal(componentDefineCommand('   '), null);
  assert.equal(componentDefineCommand(undefined), null);
});
