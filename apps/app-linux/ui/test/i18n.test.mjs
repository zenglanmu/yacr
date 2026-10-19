import test from 'node:test';
import assert from 'node:assert/strict';
import { catalogFor, createTranslator, formatMessage, LOCALES, localeTag } from '../src/i18n.mjs';
import { readAppSource, readCatalog } from './helpers.mjs';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');

test('translator reads the locale catalog and formats {placeholder} values', () => {
  const t = createTranslator({ 'zh-CN': zh, en }, 'en');
  assert.equal(t('file.open'), en['file.open']);
  assert.equal(t('command.unsupported', { command: 'LINE' }), 'Command LINE is not supported yet.');
  const tzh = createTranslator({ 'zh-CN': zh, en }, 'zh-CN');
  assert.equal(tzh('command.unsupported', { command: 'LINE' }), zh['command.unsupported'].replace('{command}', 'LINE'));
});

test('a partially translated locale falls back per key, then to the key itself', () => {
  const partial = createTranslator({ 'zh-CN': zh, en: { 'file.open': 'Open it' } }, 'en', 'zh-CN');
  assert.equal(partial('file.open'), 'Open it');           // primary wins
  assert.equal(partial('qat.new'), zh['qat.new']);          // fallback locale fills the gap
  assert.equal(partial('does.not.exist'), 'does.not.exist'); // explicit marker for a real gap
});

test('unknown placeholders are left untouched and null params are not substituted', () => {
  assert.equal(formatMessage('a {x} b', { x: 1 }), 'a 1 b');
  assert.equal(formatMessage('a {x} b', {}), 'a {x} b');
  assert.equal(formatMessage('a {x} b', { x: null }), 'a {x} b');
  assert.equal(formatMessage('no placeholders'), 'no placeholders');
});

test('catalogFor and localeTag resolve unknown locales to the default', () => {
  assert.equal(catalogFor('de', { 'zh-CN': zh, en }), zh);
  assert.equal(catalogFor('en', { 'zh-CN': zh, en }), en);
  assert.deepEqual(LOCALES, ['zh-CN', 'en']);
  assert.equal(localeTag('de'), 'zh-CN');
  assert.equal(localeTag('en'), 'en');
});

test('both catalogs are complete, flat and placeholder-balanced', () => {
  assert.deepEqual(Object.keys(zh).sort(), Object.keys(en).sort());
  const placeholder = /(?<!\\)\{([A-Za-z_][A-Za-z0-9_]*)\}/g;
  for (const key of Object.keys(zh)) {
    assert.equal(typeof zh[key], 'string', key);
    assert.ok(zh[key].trim().length > 0, key);
    assert.equal(typeof en[key], 'string', key);
    const a = [...zh[key].matchAll(placeholder)].map(match => match[1]).sort();
    const b = [...en[key].matchAll(placeholder)].map(match => match[1]).sort();
    assert.deepEqual(a, b, `placeholders differ for ${key}`);
  }
});

test('the active-tool chip keys exist in both catalogs with matching placeholders', () => {
  const toolKeys = [
    'tool.draw.line', 'tool.draw.circle', 'tool.draw.arc', 'tool.draw.move', 'tool.draw.trim',
    'tool.draw.copy', 'tool.draw.extend', 'tool.draw.rotate', 'tool.draw.scale', 'tool.draw.mirror',
    'tool.draw.offset', 'tool.draw.rectang', 'tool.draw.ellipse',
    'tool.draw.chamfer', 'tool.draw.fillet',
    'tool.draw.arrayrect', 'tool.draw.arraypolar',
    'tool.draw.point', 'tool.draw.spline',
    'tool.measure.distance', 'tool.measure.polyline', 'tool.measure.angle',
    'tool.measure.area', 'tool.points', 'tool.prompt.confirm', 'tool.prompt.cancel',
  ];
  for (const key of toolKeys) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
  }
  assert.equal(formatMessage(zh['tool.points'], { count: 3 }), '3 个点');
  assert.equal(formatMessage(en['tool.points'], { count: 3 }), '3 points');
});

test('every translation key used in the app exists in both catalogs', async () => {
  const source = await readAppSource();
  const keys = new Set();
  for (const match of source.matchAll(/\bt\(\s*'([^']+)'/g)) keys.add(match[1]);
  assert.ok(keys.size > 20, `expected the app to use many keys, saw ${keys.size}`);
  for (const key of keys) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
  }
});
