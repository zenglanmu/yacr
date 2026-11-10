// The table command surface: `TABLE` / `-TABLE` (real table creation) and
// `TABLEDIT` (real cell-text editing). The Rust lane is the behaviour
// authority; this suite pins the typed grammars the chrome admits and the
// user-facing catalog rows (real labels/hints for the implemented commands,
// named refusals for the still-missing ones).
import test from 'node:test';
import assert from 'node:assert/strict';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { ARGUMENT_COMMANDS } from '../src/commands/gating.ts';
import { readCatalog } from './helpers.ts';

test('TABLE and -TABLE admit six tokens: two counts, two sizes, an origin', () => {
  for (const id of ['TABLE', '-TABLE'] as const) {
    const parse = ARGUMENT_COMMANDS[id].parse;
    assert.equal(parse('3 2 5 2 10 20'), true, id);
    assert.equal(parse('1 1 0.5 0.5 -10.5 20'), true, id);
    for (const refused of [
      '',
      '3 2 5 2 10',
      '3 2 5 2 10 20 0',
      '0 2 5 2 0 0',
      '3 0 5 2 0 0',
      '-3 2 5 2 0 0',
      '3.5 2 5 2 0 0',
      '3 2 0 2 0 0',
      '3 2 5 0 0 0',
      '3 2 -5 2 0 0',
      '3 2 5 2 NaN 0',
      '3 2 5 2 0 inf',
      '3 2 5 2 1e3 0',
    ]) {
      assert.equal(parse(refused), false, `${id} ${refused}`);
    }
  }
});

test('TABLEDIT admits a 1-based address and preserves the text remainder', () => {
  const parse = ARGUMENT_COMMANDS.TABLEDIT.parse;
  assert.equal(parse('1 1 hello'), true);
  assert.equal(parse('2 1 hello world'), true);
  assert.equal(parse('2 1'), true, 'an omitted text clears the cell');
  assert.equal(parse('2 1 hello   world  '), true);
  for (const refused of ['', '2', '0 1 hello', '1 0 hello', '-1 1 hello', '1.5 1 hello', 'a 1']) {
    assert.equal(parse(refused), false, refused);
  }
});

test('the three table rows carry real labels and hints in both catalogs', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const rows: readonly (readonly [string, string])[] = [
    ['TABLE', 'draw.kind.table'],
    ['-TABLE', 'draw.kind.dashtable'],
    ['TABLEDIT', 'draw.kind.tabledit'],
  ];
  for (const [id, labelKey] of rows) {
    const matches = COMMAND_CATALOG.filter(command => command.id === id);
    assert.equal(matches.length, 1, `${id} must appear exactly once`);
    assert.equal(matches[0].labelKey, labelKey, `${id} names its own label key`);
    for (const [locale, catalog] of [['zh-CN', zh], ['en', en]] as const) {
      const label = catalog[labelKey];
      const hint = catalog[`${labelKey}.hint`];
      assert.equal(typeof label, 'string', `${locale} missing ${labelKey}`);
      assert.equal(typeof hint, 'string', `${locale} missing ${labelKey}.hint`);
      // These three are implemented now: the old refusal text must not survive
      // on a row that really writes.
      assert.equal(/not implemented/i.test(label), false, `${locale} ${id}: ${label}`);
      assert.equal(/not implemented/i.test(hint), false, `${locale} ${id}: ${hint}`);
      assert.equal(/refusal/i.test(hint), false, `${locale} ${id}: ${hint}`);
    }
  }
});

test('the still-missing table names stay named refusals', async () => {
  const en = await readCatalog('en');
  for (const key of [
    'draw.kind.tableexport.hint',
    'draw.kind.tablestyle.hint',
    'draw.kind.tinsert.hint',
    'draw.kind.matchcell.hint',
  ]) {
    assert.equal(typeof en[key], 'string', key);
    assert.equal(/refusal/i.test(en[key]), true, `${key}: ${en[key]}`);
  }
});
