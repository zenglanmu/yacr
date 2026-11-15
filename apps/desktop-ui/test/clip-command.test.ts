// The `CLIP` / `CLIPIT` desktop-UI surface (2026-11-15): the typed-argument
// grammar the chrome admits, the selection gate, and the user-facing catalog rows.
//
// Both names used to be named refusals ("no image/xref clipping model"). They are
// now **real** Work-only commands in the Rust lane, and both write the clip models
// `IMAGECLIP`/`XCLIP` already had. This suite therefore pins only what the chrome
// owns:
//
//   * the local **syntax** predicate for `CLIP` (RECT arity, POLY pair count, the
//     three state words), never a semantic rule (target kind, rectangle ordering,
//     polygon validity, the world→block-local mapping and the "images have no
//     ON/OFF" rule stay the host's);
//   * the selection gate: `CLIP` needs at least one selected object, `CLIPIT`
//     needs the target plus a boundary (two), and both are *not* document-scoped
//     entries (a selection requirement already answers the no-document case);
//   * the catalog rows and their real label/hint keys in both shared catalogs,
//     with the hints stating every declared boundary (one target, which clip space
//     each form reads, the block-polygon refusal, the axis-aligned-rectangle rule
//     for a reference, block-only ON/OFF, DELETE, the boundary entity kinds, the
//     chord-approximated arcs and the unmodelled underlay clip).
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  admitCommandText,
  isSupportedCommand,
  setCommandCatalog,
} from '../src/commands/commands.ts';
import {
  ARGUMENT_COMMANDS,
  CAPABILITY_GATES,
  DOCUMENT_SCOPED_COMMANDS,
  SELECTION_REQUIREMENTS,
} from '../src/commands/gating.ts';
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
  selection: { count: 2 },
};
const NO_DOC = { documentLabel: null, loading: false, selection: { count: 0 } };
const EMPTY_SELECTION = { ...DOC, selection: { count: 0 } };
const ONE_SELECTED = { ...DOC, selection: { count: 1 } };

test('CLIP admits the documented token shapes and nothing else', () => {
  const parse = ARGUMENT_COMMANDS.CLIP.parse;
  // RECT: exactly four signed decimals, in either case.
  assert.equal(parse('RECT 0 0 10 5'), true);
  assert.equal(parse('rect -1.5 -2 3 4'), true);
  // The host owns `min < max` and the target's clip space, so an inverted
  // rectangle is still within the local syntax.
  assert.equal(parse('RECT 10 5 0 0'), true);
  // POLY: an even count of at least six coordinates (three world-space pairs).
  assert.equal(parse('POLY 0 0 10 0 10 5'), true);
  assert.equal(parse('poly 0 0 1 0 1 1 0 1'), true);
  // The three state words.
  for (const word of ['ON', 'OFF', 'DELETE', 'on', 'off', 'delete']) {
    assert.equal(parse(word), true, word);
  }
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
    // State words take no operand.
    'ON EXTRA',
    'OFF 0',
    'DELETE 0',
    // The bare IMAGECLIP form (four numbers, no keyword) is NOT the CLIP grammar.
    '0 0 10 5',
    // Unknown / truncated words, and XCLIP's INVERT/`?` which CLIP does not take.
    'NONE',
    'REC 0 0 1 1',
    'POL 0 0 1 0 1 1',
    'INVERT',
    '?',
  ]) {
    assert.equal(parse(refused), false, refused);
  }
});

test('CLIP is admitted as a typed argument; CLIPIT is admitted bare', () => {
  setCommandCatalog(HOST_NAMES);
  assert.equal(isSupportedCommand('CLIP'), true);
  assert.equal(isSupportedCommand('CLIPIT'), true);
  // CLIPIT has no grammar: the selection is its whole input, so it must not be an
  // argument selector (a bare line is the command).
  assert.equal(ARGUMENT_COMMANDS.CLIPIT, undefined);
  assert.equal(admitCommandText('CLIPIT', { status: DOC }).kind, 'admitted');
  for (const line of [
    'CLIP RECT 0 0 10 5',
    'clip rect 0 0 10 5',
    'CLIP POLY 0 0 10 0 10 5',
    'CLIP ON',
    'CLIP OFF',
    'CLIP DELETE',
  ]) {
    assert.equal(admitCommandText(line, { status: DOC }).kind, 'admitted', line);
  }
  // A malformed line is refused before IPC, and a bare selector stays name-only:
  // the host answers its payload refusal, the chrome invents nothing.
  for (const line of ['CLIP RECT 0 0 1', 'CLIP POLY 0 0 10 0', 'CLIP NONE', 'CLIP 0 0 10 5']) {
    assert.deepEqual(admitCommandText(line, { status: DOC }), {
      kind: 'unsupported',
      command: line,
      reasonKey: 'command.unsupported',
    });
  }
  for (const line of ['CLIP', 'CLIPIT']) {
    assert.deepEqual(
      admitCommandText(line, { status: DOC }),
      { kind: 'admitted', command: line, reasonKey: null },
      line,
    );
  }
  // With no advertised host vocabulary even the well-formed line is not admitted
  // on a guess.
  setCommandCatalog([]);
  assert.equal(admitCommandText('CLIP ON', { status: DOC }).kind, 'unsupported');
  assert.equal(admitCommandText('CLIPIT', { status: DOC }).kind, 'unsupported');
  setCommandCatalog(HOST_NAMES);
});

test('both names are selection-gated, not document-scoped, and carry no capability gate', () => {
  for (const [id, count, reasonKey] of [
    ['CLIP', 1, 'draw.kind.clip.hint'],
    ['CLIPIT', 2, 'draw.kind.clipit.hint'],
  ]) {
    assert.equal(DOCUMENT_SCOPED_COMMANDS.includes(id), false, `${id} is selection-gated now`);
    assert.equal(SELECTION_REQUIREMENTS[id].count, count, `${id} minimum`);
    assert.equal(SELECTION_REQUIREMENTS[id].reasonKey, reasonKey, `${id} sentence`);
    assert.equal(CAPABILITY_GATES[id], undefined, `${id} involves no file I/O`);
  }
  // An empty selection answers with the command's own sentence, not "unsupported".
  for (const id of ['CLIP', 'CLIPIT']) {
    assert.deepEqual(admitCommandText(id, { status: EMPTY_SELECTION }), {
      kind: 'unavailable',
      command: id,
      reasonKey: SELECTION_REQUIREMENTS[id].reasonKey,
    });
  }
  // CLIP needs only the target; CLIPIT needs the target plus a boundary.
  assert.equal(admitCommandText('CLIP', { status: ONE_SELECTED }).kind, 'admitted');
  assert.deepEqual(admitCommandText('CLIPIT', { status: ONE_SELECTED }), {
    kind: 'unavailable',
    command: 'CLIPIT',
    reasonKey: 'draw.kind.clipit.hint',
  });
  // With no drawing open the honest reason is the shared no-document sentence.
  for (const id of ['CLIP', 'CLIPIT']) {
    assert.deepEqual(admitCommandText(id, { status: NO_DOC }), {
      kind: 'unavailable',
      command: id,
      reasonKey: 'plot.no_document',
    });
  }
});

test('the CLIP/CLIPIT catalog rows carry real labels and hints in both catalogs', async () => {
  const glyphs = await iconNames(await readModule('icons.tsx'));
  for (const id of ['CLIP', 'CLIPIT']) {
    const matches = COMMAND_CATALOG.filter(command => command.id === id);
    assert.equal(matches.length, 1, `${id} must appear exactly once`);
    const [row] = matches;
    assert.equal(row.group, 'modify', `${id} lives in the modify group`);
    assert.ok(glyphs.has(row.icon), `${id}: icons.tsx must define '${row.icon}'`);
    for (const [locale, catalog] of [['zh-CN', zh], ['en', en]]) {
      assert.equal(typeof catalog[row.labelKey], 'string', `missing ${locale} label`);
      assert.equal(typeof catalog[`${row.labelKey}.hint`], 'string', `missing ${locale} hint`);
      assert.equal(/refusal/i.test(catalog[row.labelKey]), false, `${locale}: ${catalog[row.labelKey]}`);
    }
  }

  const clip = en['draw.kind.clip.hint'];
  // Every declared boundary of the CLIP form is part of the user-visible contract,
  // including the two that keep "accepted" and "drawn" the same statement: a block
  // polygon and a reference whose rectangle does not map to an axis-aligned world
  // rectangle are refused, not stored.
  for (const fact of [
    'exactly one selected raster image or block reference',
    'one transaction / one undo step',
    'RECT',
    'POLY',
    'axis-aligned world XY rectangle is refused',
    'a block reference has no renderable polygon clip, so the word is refused by name',
    'ON/OFF apply to a block reference only',
    'refused by name',
    'DELETE',
    'Only a convex image polygon is drawn',
    'non-convex or self-intersecting ring is refused by name',
    'PDF/DGN/DWF underlay clipping is not modelled',
  ]) {
    assert.ok(clip.includes(fact), `en CLIP hint must say ${JSON.stringify(fact)}: ${clip}`);
  }
  const clipit = en['draw.kind.clipit.hint'];
  for (const fact of [
    'clip exactly one selected raster image with the boundary',
    'LINE, ARC and LWPOLYLINE',
    'JOIN rule',
    'must close',
    'self-intersecting',
    'refused by name',
    '32 chords',
    'one transaction / one undo step',
    "must be convex in the image's pixel space",
    'reflex boundary is refused by name',
    'A block reference or xref target is refused by name',
    'use CLIP RECT on the reference instead',
    'PDF/DGN/DWF underlay clipping is not modelled',
  ]) {
    assert.ok(clipit.includes(fact), `en CLIPIT hint must say ${JSON.stringify(fact)}: ${clipit}`);
  }
  const zhClip = zh['draw.kind.clip.hint'];
  for (const fact of [
    '栅格图像或块参照',
    '像素坐标',
    '块内坐标',
    '轴对齐矩形',
    '没有可渲染的多边形裁剪',
    'ON/OFF',
    'DELETE',
    '非凸或自交的环会按名拒绝',
    '未建模',
  ]) {
    assert.ok(zhClip.includes(fact), `zh CLIP hint must say ${fact}: ${zhClip}`);
  }
  const zhClipit = zh['draw.kind.clipit.hint'];
  for (const fact of [
    'LINE、ARC、LWPOLYLINE',
    'JOIN',
    '闭合',
    '自交',
    '32 段弦',
    '撤销',
    '图像像素空间中为凸',
    '按名拒绝',
    '块参照/外部参照目标按名拒绝',
    'CLIP RECT',
    '未建模',
  ]) {
    assert.ok(zhClipit.includes(fact), `zh CLIPIT hint must say ${fact}: ${zhClipit}`);
  }
  // The hints must no longer claim the command is a refusal.
  for (const [locale, catalog] of [['zh-CN', zh], ['en', en]]) {
    for (const key of ['draw.kind.clip.hint', 'draw.kind.clipit.hint']) {
      assert.equal(/refusal/i.test(catalog[key]), false, `${locale} ${key} still says refusal`);
      assert.equal(/具名拒绝/.test(catalog[key]), false, `${locale} ${key} still says 具名拒绝`);
    }
  }
});
