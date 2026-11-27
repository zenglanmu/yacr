// The GUIDE argument grammar the Styles tray and command line share.
//
// The host (cad-ui-core `parse_guide_argument`) is the authority; this local
// predicate must admit exactly the shipped shapes and nothing wider, otherwise
// the panel's dispatch is refused before IPC.

import assert from 'node:assert/strict';
import test from 'node:test';

import { isGuideArgument } from '../src/commands/argument-guide-grammar.ts';
import { admitCommandText, setCommandCatalog } from '../src/commands/commands.ts';

test('the guide grammar admits the shipped argument shapes', () => {
  for (const accepted of [
    'LIST',
    'clear',
    'DELETE 1',
    'POINT 1 2 3',
    'POINT -1.5 2e3 0',
    'STYLE COLOR 0 0 0',
    'STYLE COLOR 255 128 51',
    'STYLE DASH ON',
    'STYLE dash off',
    'STYLE DASH 1',
    'STYLE DASH 0',
  ]) {
    assert.equal(isGuideArgument(accepted), true, accepted);
  }
});

test('the guide grammar refuses everything the host does not admit', () => {
  for (const refused of [
    '',
    'LIST NOW',
    'DELETE',
    'DELETE 0',
    'DELETE -1',
    'DELETE 1.5',
    'POINT 1 2',
    'POINT 1 NaN 3',
    'POINT 1 Infinity 3',
    'STYLE COLOR 1 2',
    'STYLE COLOR 1 2 256',
    'STYLE COLOR 1 2 -3',
    'STYLE COLOR 1 2 3 4',
    'STYLE DASH MAYBE',
    'STYLE DASH',
    'OTHER 1 2 3',
  ]) {
    assert.equal(isGuideArgument(refused), false, refused);
  }
});

test('the command line admits the guide style lines once the host catalog has GUIDE', () => {
  setCommandCatalog(['GUIDE']);
  const status = { documentLabel: 'demo', loading: false };
  for (const line of [
    'GUIDE',
    'GUIDE LIST',
    'GUIDE STYLE COLOR 10 20 30',
    'GUIDE STYLE DASH OFF',
    'GUIDE POINT 1 2 3',
    'GUIDE DELETE 2',
  ]) {
    assert.equal(admitCommandText(line, { status }).kind, 'admitted', line);
  }
  assert.equal(
    admitCommandText('GUIDE STYLE COLOR 10 20 300', { status }).kind,
    'unsupported',
  );
});
