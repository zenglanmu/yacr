// The raster-image command surface: `IMAGE` / `-IMAGE` (read-only report),
// `IMAGEATTACH` and `IMAGECLIP` (real writes). The Rust lane is the behaviour
// authority; this suite pins the typed grammars the chrome admits and the
// user-facing catalog rows (real labels/hints, not "not implemented").
import test from 'node:test';
import assert from 'node:assert/strict';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { ARGUMENT_COMMANDS } from '../src/commands/gating.ts';
import { readCatalog } from './helpers.ts';

test('IMAGEATTACH admits a single-token file plus five numbers', () => {
  const parse = ARGUMENT_COMMANDS.IMAGEATTACH.parse;
  assert.equal(parse('textures/logo.png 10 20 640 480 0.05'), true);
  assert.equal(parse('logo.png -1 -2.5 1 1 2'), true);
  for (const refused of [
    '',
    'logo.png',
    'logo.png 0 0 1 1',
    'my logo.png 0 0 1 1 1',
    'logo.png 0 0 1 1 1 2',
    'logo.png a 0 1 1 1',
    'logo.png 0 0 0 1 1',
    'logo.png 0 0 -1 1 1',
    'logo.png 0 0 1 1 0',
    'logo.png 0 0 1 1 1e-2',
  ]) {
    assert.equal(parse(refused), false, refused);
  }
});

test('IMAGECLIP admits a four-number rectangle or OFF', () => {
  const parse = ARGUMENT_COMMANDS.IMAGECLIP.parse;
  assert.equal(parse('0 0 320 240'), true);
  assert.equal(parse('-5 -5 10.5 10'), true);
  assert.equal(parse('OFF'), true);
  assert.equal(parse('off'), true);
  // Ordering is the host's `imageclip.invalid_rect`, not local grammar.
  assert.equal(parse('10 0 0 10'), true);
  for (const refused of ['', '0 0 1', '0 0 1 1 1', '0 0 x 1', 'OFF EXTRA', 'NONE']) {
    assert.equal(parse(refused), false, refused);
  }
});

test('the four raster-image rows carry real labels and hints in both catalogs', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const rows: readonly (readonly [string, string])[] = [
    ['IMAGE', 'draw.kind.image'],
    ['-IMAGE', 'draw.kind.dashimage'],
    ['IMAGEATTACH', 'draw.kind.imageattach'],
    ['IMAGECLIP', 'draw.kind.imageclip'],
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
      // These four are implemented now: the old "not implemented"/refusal text
      // must not survive on a row that really runs.
      assert.equal(/not implemented/i.test(label), false, `${locale} ${id}: ${label}`);
      assert.equal(/not implemented/i.test(hint), false, `${locale} ${id}: ${hint}`);
      assert.equal(/refusal/i.test(hint), false, `${locale} ${id}: ${hint}`);
    }
  }
});

test('the still-missing image names stay named refusals', async () => {
  const en = await readCatalog('en');
  for (const key of [
    'draw.kind.imageadjust.hint',
    'draw.kind.imagequality.hint',
    'draw.kind.imageapp.hint',
    'draw.kind.imageasyncwait.hint',
    'draw.kind.imageedit.hint',
    'draw.kind.classicimage.hint',
  ]) {
    assert.equal(typeof en[key], 'string', key);
    assert.equal(/refusal/i.test(en[key]), true, `${key}: ${en[key]}`);
  }
});
