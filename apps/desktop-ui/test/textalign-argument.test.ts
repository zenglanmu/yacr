// `TEXTALIGN` typed-argument admission (2026-11-12): the 15 alignment words,
// `DISTRIBUTE [X|Y]`, and the three spacing words the host accepts and then
// refuses by code.
//
// This pins the local chrome grammar only. The Rust parser and write paths are
// the authority (`crates/cad-ui-core/src/session/text_batch.rs`,
// `crates/cad-app/src/app_drawing/text_batch/align_count.rs`); admission here
// never proves the host performed a write.
import test from 'node:test';
import assert from 'node:assert/strict';
import { ARGUMENT_COMMANDS } from '../src/commands/gating.ts';
import { readCatalog } from './helpers.ts';

const parse = ARGUMENT_COMMANDS.TEXTALIGN.parse;

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

test('TEXTALIGN admits the six single-axis and nine combined alignment words', () => {
  for (const keyword of [
    'LEFT', 'RIGHT', 'CENTER', 'TOP', 'MIDDLE', 'BOTTOM',
    'TOPLEFT', 'TOPCENTER', 'TOPRIGHT',
    'MIDDLELEFT', 'MIDDLECENTER', 'MIDDLERIGHT',
    'BOTTOMLEFT', 'BOTTOMCENTER', 'BOTTOMRIGHT',
  ]) {
    assert.equal(parse(keyword), true, keyword);
    assert.equal(parse(keyword.toLowerCase()), true, `${keyword} (lower case)`);
    assert.equal(parse(`  ${keyword}  `), true, `${keyword} (surrounded)`);
  }
});

test('TEXTALIGN admits DISTRIBUTE with the default Y or an explicit axis', () => {
  for (const argument of ['DISTRIBUTE', 'distribute', 'DISTRIBUTE X', 'distribute y', ' DISTRIBUTE X ']) {
    assert.equal(parse(argument), true, argument);
  }
  for (const argument of ['DISTRIBUTE Z', 'DISTRIBUTE X Y', 'DISTRIBUT', 'DISTRIBUTION']) {
    assert.equal(parse(argument), false, argument);
  }
});

test('TEXTALIGN admits the three named spacing refusals (the host refuses them)', () => {
  for (const argument of ['SETSPACING', 'currentvertical', 'CurrentHorizontal']) {
    assert.equal(parse(argument), true, argument);
  }
});

test('TEXTALIGN refuses abbreviations, hyphenation and joined words', () => {
  for (const argument of [
    '',
    'L',
    'C',
    'T',
    'TOP LEFT',
    'TOP-LEFT',
    'MIDDLE CENTER',
    'BOTTOM_RIGHT',
    'LEFT DISTRIBUTE',
  ]) {
    assert.equal(parse(argument), false, argument);
  }
});

test('the hint lists the supported modes and DISTRIBUTE in both locales', () => {
  for (const locale of [zh, en]) {
    const hint = locale['draw.kind.textalign.hint'];
    assert.equal(typeof hint, 'string');
    assert.match(hint, /TEXTALIGN DISTRIBUTE/);
    assert.match(hint, /BOTTOMRIGHT/);
    assert.match(hint, /SETSPACING/);
  }
});
