import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createFpsSampler } from '../src/fps.mjs';

test('FPS uses actual frame deltas and monotonic elapsed time after warmup', () => {
  const sampler = createFpsSampler();
  assert.equal(sampler.sample(900, 0), null);
  assert.equal(sampler.sample(915, 250), null);
  assert.equal(sampler.sample(960, 1000), 60);
  assert.equal(sampler.sample(1000, 2000), 50);
  assert.equal(sampler.sample(1040, 3000), 40);
});

test('rolling low frame rates tolerate unchanged samples, then paused counters have no data', () => {
  const sampler = createFpsSampler();
  assert.equal(sampler.sample(0, 0), null);
  assert.equal(sampler.sample(1, 1000), 1);
  assert.equal(sampler.sample(1, 1250), 0.8);
  assert.equal(sampler.sample(2, 2000), 1);
  assert.equal(sampler.sample(2, 4000), null);
  assert.equal(sampler.sample(2, 5000), null);
});

test('counter reset, reversed/equal time and stale gaps restart instead of negative FPS', () => {
  for (const next of [[1, 1500], [61, 1000], [61, 500], [10000, 7001]]) {
    const sampler = createFpsSampler();
    sampler.sample(0, 0);
    assert.equal(sampler.sample(60, 1000), 60);
    assert.equal(sampler.sample(...next), null);
    assert.equal(sampler.sample(next[0] + 20, next[1] + 1000), 20);
  }
});

test('errors, invalid data, hidden state and explicit visibility reset never retain fake FPS', () => {
  for (const invalid of [[undefined, 2000], [-1, 2000], [1.5, 2000], [120, NaN], [120, 2000, false]]) {
    const sampler = createFpsSampler();
    sampler.sample(0, 0);
    assert.equal(sampler.sample(60, 1000), 60);
    assert.equal(sampler.sample(...invalid), null);
    assert.equal(sampler.sample(999, 3000), null);
  }
  const sampler = createFpsSampler();
  sampler.sample(0, 0); sampler.sample(60, 1000);
  sampler.reset();
  assert.equal(sampler.sample(600, 10000), null);
});

test('visible UI removes raw frameCount, uses bilingual FPS labels and keeps CAD DOM transparent', async () => {
  const main = await readFile(new URL('../src/main.jsx', import.meta.url), 'utf8');
  const css = await readFile(new URL('../src/style.css', import.meta.url), 'utf8');
  assert.ok(!main.includes('frameCount /'));
  assert.ok(!main.includes("'importCount', 'frameCount'"));
  assert.ok(main.includes('performance.now()'));
  assert.ok(main.includes("document.addEventListener('visibilitychange', resetFps)"));
  assert.ok(main.includes('staleTimer = setTimeout(resetFps, 5000)'));
  assert.ok(main.includes("t('render.fps')"));
  assert.match(css, /\.native-input\{[^}]*background:transparent/);
  assert.ok(css.includes('--chrome:#dedede'));
  const zh = JSON.parse(await readFile(new URL('../../../../crates/cad-ui-slint/i18n/zh-CN.json', import.meta.url)));
  const en = JSON.parse(await readFile(new URL('../../../../crates/cad-ui-slint/i18n/en.json', import.meta.url)));
  assert.deepEqual(Object.keys(zh).sort(), Object.keys(en).sort());
  assert.equal(zh['render.fps'], '帧率（FPS）');
  assert.equal(en['render.fps'], 'Frame rate (FPS)');
});

test('neutral gray chrome has readable text, error and disabled control contrasts', () => {
  const luminance = hex => {
    const rgb = hex.match(/\w\w/g).map(pair => parseInt(pair, 16) / 255)
      .map(channel => channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4);
    return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
  };
  const contrast = (a, b) => {
    const values = [luminance(a), luminance(b)].sort((x, y) => y - x);
    return (values[0] + 0.05) / (values[1] + 0.05);
  };
  for (const [text, background] of [['252525', 'dedede'], ['505050', 'dedede'], ['245c50', 'dedede'], ['9b241b', 'dedede'], ['9b241b', 'eeeeee'], ['616161', 'f7f7f7']]) {
    assert.ok(contrast(text, background) >= 4.5, `${text}/${background}`);
  }
  assert.ok(contrast('656565', 'd5d5d5') >= 3);
});
