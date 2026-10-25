import test from 'node:test';
import assert from 'node:assert/strict';
import { createFpsSampler } from '../src/fps.ts';
import { readAppSource, readCatalog, readComponentsSource, readCss } from './helpers.ts';

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
  const source = await readAppSource();
  const components = await readComponentsSource();
  const css = await readCss();
  // Raw counters belong to the protocol validator, never to a rendered region.
  assert.ok(!components.includes('frameCount'));
  assert.ok(!source.includes('frameCount /'));
  assert.ok(source.includes('performance.now()'));
  assert.ok(source.includes("document.addEventListener('visibilitychange', resetFps)"));
  assert.ok(source.includes('staleTimer = setTimeout(resetFps, 5000)'));
  assert.ok(source.includes("t('render.fps')"));
  assert.match(css, /\.native-input\s*\{[^}]*background:\s*transparent/);
  assert.ok(css.includes('--chrome: #3b4453'));
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  assert.deepEqual(Object.keys(zh).sort(), Object.keys(en).sort());
  assert.equal(zh['render.fps'], '帧率（FPS）');
  assert.equal(en['render.fps'], 'Frame rate (FPS)');
});

test('dark chrome keeps readable text, active and disabled control contrasts', async () => {
  const luminance = hex => {
    const rgb = hex.match(/\w\w/g).map(pair => parseInt(pair, 16) / 255)
      .map(channel => channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4);
    return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
  };
  const contrast = (a, b) => {
    const values = [luminance(a), luminance(b)].sort((x, y) => y - x);
    return (values[0] + 0.05) / (values[1] + 0.05);
  };
  const css = await readCss();
  // Text tokens from the stylesheet, checked against each chrome surface they sit on.
  const pairs = [
    ['e9edf3', '3b4453'], // body text on ribbon chrome
    ['c3ccda', '3b4453'], // secondary text
    ['aab5c6', '3b4453'], // muted panel titles
    ['8fc4ff', '3b4453'], // accent text
    ['ff9a9a', '3b4453'], // error text
    ['e9edf3', '2b333f'], // text on status bar
    ['aab5c6', '2b333f'], // muted on status bar
    ['ff9a9a', '2b333f'], // error on status bar
    ['ffffff', '1f6fd0'], // ON toggle label
    ['ffffff', '2a6fc0'], // primary button label
    ['98a3b4', '2c3540'], // disabled label
  ];
  for (const [text, background] of pairs) {
    assert.ok(contrast(text, background) >= 4.5, `${text}/${background} = ${contrast(text, background).toFixed(2)}`);
  }
  for (const token of ['--chrome: #3b4453', '--text: #e9edf3', '--on: #1f6fd0']) {
    assert.ok(css.includes(token), `stylesheet must define ${token}`);
  }
});
