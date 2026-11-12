// The `XCLIP` desktop-UI surface (2026-11-12): the typed-argument grammar the
// chrome admits and the user-facing catalog row.
//
// The Rust lane is the behaviour authority: `XCLIP` is a Work-only extended
// command that stores a **display-only** block-local clip boundary on one
// selected INSERT. This suite therefore pins only what the chrome owns:
//
//   * the local **syntax** predicate (RECT arity, POLY pair count, the four
//     switch words and `?`), never a semantic rule (ordering, polygon validity,
//     world mapping and the declared render limits stay the host's);
//   * admission through the loaded host vocabulary, including the one honest
//     split: `INVERT` is syntactically admitted here and refused by the host
//     (`xclip.invert_not_renderable`), because an inverted region has no exact
//     render path;
//   * the catalog row and its real label/hint in both shared catalogs, with the
//     hint stating every declared boundary (one block reference, block-local
//     coordinates, rectangles render / polygons are stored only, no INVERT, only
//     0/90/180/270 rotations, no DXF/DWG persistence, picking/bounds ignore the
//     clip).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  isSupportedCommand,
  setCommandCatalog,
} from '../src/commands/commands.ts';
import { ARGUMENT_COMMANDS } from '../src/commands/gating.ts';
import { COMMAND_CATALOG } from '../src/commands/catalog.ts';
import { HOST_NAMES, readCatalog, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

function iconNames(source) {
  const map = source.match(/const P = \{([\s\S]*?)\n\};/);
  assert.ok(map, 'expected the icon name map in icons.tsx');
  return new Set([...map[1].matchAll(/\n\s*([A-Za-z][A-Za-z0-9]*):/g)].map(match => match[1]));
}

const DOC = {
  documentLabel: 'fixture.dwg',
  entityCount: 3,
  loading: false,
  canOpen: true,
  canNew: true,
  canSave: true,
  canPlot: true,
  canFit: true,
  selection: { count: 1 },
};

test('XCLIP admits the documented token shapes and nothing else', () => {
  const parse = ARGUMENT_COMMANDS.XCLIP.parse;
  // RECT: exactly four signed block-local decimals, in either case.
  assert.equal(parse('RECT 0 0 10 5'), true);
  assert.equal(parse('rect -1.5 -2 3 4'), true);
  // The host owns `min < max` (`xclip.degenerate_rect`), so an inverted
  // rectangle is still within the local syntax.
  assert.equal(parse('RECT 10 5 0 0'), true);
  // POLY: an even count of at least six coordinates (three point pairs).
  assert.equal(parse('POLY 0 0 10 0 10 5'), true);
  assert.equal(parse('poly 0 0 1 0 1 1 0 1'), true);
  // The four switch words and the bare report.
  for (const word of ['ON', 'OFF', 'DELETE', 'INVERT', 'on', 'off', 'delete', 'invert']) {
    assert.equal(parse(word), true, word);
  }
  assert.equal(parse('?'), true);
  for (const refused of [
    '',
    // RECT arity/type.
    'RECT',
    'RECT 0 0 1',
    'RECT 0 0 1 1 1',
    'RECT a 0 1 1',
    // POLY needs three pairs: two pairs and an odd coordinate count are refused.
    'POLY 0 0 10 0',
    'POLY 0 0 10 0 10',
    'POLY 0 0 10 0 x 5',
    // Switch words take no operand; `?` takes none either.
    'ON EXTRA',
    'OFF 0',
    '? EXTRA',
    // Unknown / truncated words.
    'NONE',
    'REC 0 0 1 1',
    'POL 0 0 1 0 1 1',
  ]) {
    assert.equal(parse(refused), false, refused);
  }
});

test('XCLIP is admitted as a typed argument of the real host command', () => {
  // The host now publishes `XCLIP` as a real command; the mirror carries it.
  setCommandCatalog(HOST_NAMES);
  assert.equal(isSupportedCommand('XCLIP'), true);
  for (const line of [
    'XCLIP RECT 0 0 10 5',
    'xclip rect 0 0 10 5',
    'XCLIP POLY 0 0 10 0 10 5',
    'XCLIP ON',
    'XCLIP OFF',
    'XCLIP DELETE',
    'XCLIP ?',
  ]) {
    assert.equal(admitCommandText(line, { status: DOC }).kind, 'admitted', line);
  }
  // INVERT is locally admitted — the host is the authority that refuses the
  // rendered result (`xclip.invert_not_renderable`), so the chrome must not
  // invent a second, earlier refusal here.
  assert.equal(admitCommandText('XCLIP INVERT', { status: DOC }).kind, 'admitted');
  // A malformed line is refused before IPC, and a bare selector stays
  // name-only: the host answers its payload refusal, the chrome invents nothing.
  for (const line of ['XCLIP RECT 0 0 1', 'XCLIP POLY 0 0 10 0', 'XCLIP NONE']) {
    assert.deepEqual(admitCommandText(line, { status: DOC }), {
      kind: 'unsupported',
      command: line,
      reasonKey: 'command.unsupported',
    });
  }
  // With no advertised host vocabulary even the well-formed line is not
  // admitted on a guess.
  setCommandCatalog([]);
  assert.equal(admitCommandText('XCLIP ON', { status: DOC }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
});

test('the XCLIP catalog row carries a real label and hint in both catalogs', async () => {
  const matches = COMMAND_CATALOG.filter(command => command.id === 'XCLIP');
  assert.equal(matches.length, 1, 'XCLIP must appear exactly once');
  assert.equal(matches[0].labelKey, 'draw.kind.xclip');
  assert.equal(matches[0].group, 'modify');
  // The icon must exist, or the row silently falls back to the dot glyph.
  assert.ok(iconNames(await readModule('icons.tsx')).has(matches[0].icon), matches[0].icon);

  for (const [locale, catalog] of [['zh-CN', zh], ['en', en]]) {
    const label = catalog['draw.kind.xclip'];
    const hint = catalog['draw.kind.xclip.hint'];
    assert.equal(typeof label, 'string', `missing ${locale} label`);
    assert.equal(typeof hint, 'string', `missing ${locale} hint`);
    assert.equal(/refusal/i.test(label), false, `${locale}: ${label}`);
  }
  const hint = en['draw.kind.xclip.hint'];
  // Every declared boundary is part of the user-visible contract.
  assert.match(hint, /exactly one selected block reference/i);
  assert.match(hint, /block-local/i);
  assert.match(hint, /polygon boundaries are stored but not rendered/i);
  assert.match(hint, /INVERT is not implemented/i);
  assert.match(hint, /0\/90\/180\/270/);
  assert.match(hint, /DXF\/DWG/i);
  assert.match(hint, /picking\/bounds ignore/i);
  assert.match(hint, /RECT/);
  assert.match(hint, /POLY/);
  const zhHint = zh['draw.kind.xclip.hint'];
  assert.match(zhHint, /块参照/);
  assert.match(zhHint, /块内坐标/);
  assert.match(zhHint, /多边形边界会保存但不渲染/);
  assert.match(zhHint, /INVERT/);
  assert.match(zhHint, /0\/90\/180\/270/);
  assert.match(zhHint, /DXF\/DWG/);
  assert.match(zhHint, /拾取\/边界忽略/);
});
