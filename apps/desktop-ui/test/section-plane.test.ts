// The View > Section Planes row and the SECTIONPLANE command boundary.

import assert from 'node:assert/strict';
import test from 'node:test';

import { isSectionPlaneArgument } from '../src/commands/argument-section-grammar.ts';
import { admitCommandText, setCommandCatalog } from '../src/commands/commands.ts';
import { CONCEPT_MENUS } from '../src/domains/concept/menus.ts';

test('the section-plane grammar admits the shipped options', () => {
  for (const accepted of [
    'ON',
    'off',
    'TOGGLE',
    'clear',
    'LIST',
    'SET 0 0 0 0 0 1',
    'SET -1.5 2 3 0 0 -2',
  ]) {
    assert.equal(isSectionPlaneArgument(accepted), true, accepted);
  }
  for (const refused of ['', 'SET 1 2 3', 'SET 1 2 3 0 0 NaN', 'ON NOW', 'MAYBE']) {
    assert.equal(isSectionPlaneArgument(refused), false, refused);
  }
});

test('the command line admits SECTIONPLANE once the host catalog has it', () => {
  setCommandCatalog(['SECTIONPLANE']);
  const status = { documentLabel: 'demo', loading: false };
  for (const line of [
    'SECTIONPLANE',
    'SECTIONPLANE LIST',
    'SECTIONPLANE SET 0 0 0 0 0 1',
    'SECTIONPLANE OFF',
  ]) {
    assert.equal(admitCommandText(line, { status }).kind, 'admitted', line);
  }
  assert.equal(
    admitCommandText('SECTIONPLANE BOGUS', { status }).kind,
    'unsupported',
  );
});

test('the View menu row dispatches the real command, not a pending marker', () => {
  const items = CONCEPT_MENUS.flatMap(tab => tab.panels.flatMap(panel => panel.items));
  const row = items.find(item => item.id === 'SECTIONPLANES');
  assert.ok(row, 'the Section Planes row exists');
  assert.equal((row as { command?: string }).command, 'SECTIONPLANE');
  assert.equal((row as { pendingKey?: string }).pendingKey, undefined);
});
