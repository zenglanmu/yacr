// Contract for the open-time format provenance notice (ADR-0041 D5).
//
// The Rust side is landed and authoritative: `cad_status.openedSource` is either
// absent or `{ connector, spec, canWrite, experimental }`, emitted only for a
// reverse-engineered spec. This suite pins what the chrome does with it:
//
//   * the wire reader (absent, public, malformed, well-formed);
//   * the copy and its order (title, body, per-format detail, save-back truth,
//     the `.ymodel` recommendation) straight from the shared catalog;
//   * the save-back line is exactly one of "unsupported" / "experimental";
//   * dismissal is per open (document label + connector, in memory);
//   * the card stays the non-modal `UpdateNotice` pattern, mounted from
//     `Workspace` (never `App.tsx`, which is at its line ceiling) and suppressed
//     by a dialog or another shell notice.
//
// `.ts` by filename only: Node's type stripping runs it, `tsc` never sees it.
import test from 'node:test';
import assert from 'node:assert/strict';

import { createTranslator } from '../src/i18n.ts';
import {
  connectorLabel,
  isOpenNoticeDismissed,
  openNoticeKey,
  openNoticeView,
  openedSourceOf,
} from '../src/cad/opened-source.ts';
import { readCatalog, readCss, readModule } from './helpers.ts';

const zh = await readCatalog('zh-CN');
const en = await readCatalog('en');
const t = createTranslator({ 'zh-CN': zh, en }, 'en');

/** The four connectors whose spec is reverse engineered today. */
const REVERSE_CONNECTORS = ['dwg', 'dgn', 'skp', 'rvt'];

function source(connector, overrides = {}) {
  return {
    connector,
    spec: 'reverse_engineered',
    canWrite: false,
    experimental: false,
    ...overrides,
  };
}

function placeholders(message) {
  return [...message.matchAll(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g)].map(match => match[1]).sort();
}

// --- the wire reader -----------------------------------------------------

test('an absent, null or non-reverse-engineered openedSource shows nothing', () => {
  assert.equal(openedSourceOf(null), null);
  assert.equal(openedSourceOf(undefined), null);
  assert.equal(openedSourceOf({}), null);
  assert.equal(openedSourceOf({ openedSource: null }), null);
  // A public spec has nothing to warn about and must never produce the notice.
  assert.equal(openedSourceOf({ openedSource: source('dwg', { spec: 'public' }) }), null);
});

test('a malformed block is not rendered as a guessed format or capability', () => {
  assert.equal(openedSourceOf({ openedSource: 'dwg' }), null);
  assert.equal(openedSourceOf({ openedSource: [] }), null);
  assert.equal(openedSourceOf({ openedSource: source('') }), null);
  assert.equal(openedSourceOf({ openedSource: { spec: 'reverse_engineered', canWrite: false, experimental: false } }), null);
  // A truthy string must never become "writes back" or "cannot write".
  assert.equal(openedSourceOf({ openedSource: source('dwg', { canWrite: 'yes' }) }), null);
  assert.equal(openedSourceOf({ openedSource: source('dwg', { experimental: 1 }) }), null);
});

test('a well-formed reverse-engineered block is read verbatim', () => {
  const block = { connector: 'dwg', spec: 'reverse_engineered', canWrite: true, experimental: true };
  assert.deepEqual(openedSourceOf({ openedSource: block }), block);
});

// --- the copy ------------------------------------------------------------

test('dwg reads title, body, per-format detail, save-back and .ymodel in that order', () => {
  const view = openNoticeView(source('dwg', { canWrite: true, experimental: true }), t);
  assert.equal(view.format, en['connector.dwg.name']);
  assert.equal(view.title, en['open.notice.reverse.title'].replace('{format}', en['connector.dwg.name']));
  assert.deepEqual(view.lines.map(line => line.key), ['body', 'detail', 'saveBack', 'ymodel']);
  assert.equal(view.lines[0].text, en['open.notice.reverse.body'].replace('{format}', en['connector.dwg.name']));
  assert.equal(view.lines[1].text, en['open.notice.detail.dwg']);
  assert.equal(view.lines[2].text, en['open.notice.save_back.experimental'].replace('{format}', en['connector.dwg.name']));
  assert.equal(view.lines[3].text, en['open.notice.save_back.ymodel']);
});

test('a connector with no writer says "unsupported" and never "experimental"', () => {
  const view = openNoticeView(source('dgn'), t);
  const saveBack = view.lines.filter(line => line.key === 'saveBack');
  assert.equal(saveBack.length, 1);
  assert.equal(saveBack[0].text, en['open.notice.save_back.unsupported'].replace('{format}', en['connector.dgn.name']));
  const experimentalPrefix = en['open.notice.save_back.experimental'].split('{format}')[0];
  assert.ok(!view.lines.some(line => line.text.startsWith(experimentalPrefix)),
    'the experimental sentence must not also appear');
  assert.deepEqual(view.lines.map(line => line.key), ['body', 'detail', 'saveBack', 'ymodel']);
});

test('a connector with an unverified writer says "experimental" and never "unsupported"', () => {
  const view = openNoticeView(source('skp', { canWrite: true, experimental: true }), t);
  const saveBack = view.lines.filter(line => line.key === 'saveBack');
  assert.equal(saveBack.length, 1);
  assert.equal(saveBack[0].text, en['open.notice.save_back.experimental'].replace('{format}', en['connector.skp.name']));
  const unsupportedPrefix = en['open.notice.save_back.unsupported'].split('{format}')[0];
  assert.ok(!view.lines.some(line => line.text.startsWith(unsupportedPrefix)),
    'the unsupported sentence must not also appear');
});

test('a verified writer omits the capability sentence instead of borrowing a false one', () => {
  // No catalog sentence describes "writes back, verified": inventing one here
  // would be copy the catalogs do not carry, so only .ymodel is stated.
  const view = openNoticeView(source('dwg', { canWrite: true, experimental: false }), t);
  assert.deepEqual(view.lines.map(line => line.key), ['body', 'detail', 'ymodel']);
  assert.equal(view.lines[2].text, en['open.notice.save_back.ymodel']);
});

test('every line resolves through the catalog, and an unknown connector omits its detail', () => {
  const view = openNoticeView(source('nostr'), t);
  assert.equal(view.format, 'nostr', 'the raw id is the honest fallback for a missing name key');
  assert.ok(view.title.includes('nostr'));
  // No `open.notice.detail.nostr` key exists: the line is omitted, never shown
  // as the catalog key itself.
  assert.deepEqual(view.lines.map(line => line.key), ['body', 'saveBack', 'ymodel']);
  for (const line of view.lines) {
    assert.ok(!line.text.startsWith('open.notice.'), `unresolved key rendered: ${line.text}`);
    assert.ok(!line.text.startsWith('connector.'), `unresolved key rendered: ${line.text}`);
  }
  assert.equal(connectorLabel('dwg', t), en['connector.dwg.name']);
  assert.equal(connectorLabel('nostr', t), 'nostr');
});

test('the notice switches language with the translator, not with hardcoded copy', () => {
  const tzh = createTranslator({ 'zh-CN': zh, en }, 'zh-CN');
  const view = openNoticeView(source('dwg', { canWrite: true, experimental: true }), tzh);
  assert.equal(view.title, zh['open.notice.reverse.title'].replace('{format}', zh['connector.dwg.name']));
  assert.equal(view.lines[1].text, zh['open.notice.detail.dwg']);
  assert.equal(view.lines[3].text, zh['open.notice.save_back.ymodel']);
});

// --- per-open dismissal --------------------------------------------------

test('dismissal is keyed on the document label plus the connector', () => {
  const first = openNoticeKey('site-plan.dwg', 'dwg');
  assert.equal(isOpenNoticeDismissed(first, first), true);
  assert.equal(isOpenNoticeDismissed(first, openNoticeKey('other-plan.dwg', 'dwg')), false,
    'another file from the same connector asks again');
  assert.equal(isOpenNoticeDismissed(first, openNoticeKey('site-plan.dwg', 'skp')), false,
    'the same label from another connector asks again');
  assert.equal(isOpenNoticeDismissed(null, first), false, 'nothing dismissed yet');
  assert.equal(isOpenNoticeDismissed(first, null), false, 'New/Close has no notice to suppress');
  assert.equal(openNoticeKey(null, 'dwg'), openNoticeKey('', 'dwg'));
});

test('the host keeps the dismissal in memory and clears it when the document goes away', async () => {
  const source = await readModule('components/FormatProvenanceNoticeHost.tsx');
  assert.match(source, /useState<string \| null>\(null\)/);
  assert.match(source, /if \(key === null\) setDismissedKey\(null\)/,
    'a new open must not inherit a previous dismissal');
  assert.match(source, /onDismiss=\{\(\) => setDismissedKey\(key\)\}/);
  assert.doesNotMatch(source, /localStorage|sessionStorage|invoke\(/,
    'dismissal is in-memory per open, never persisted or sent to the host');
});

// --- mount point, suppression, pattern -----------------------------------

test('Workspace mounts the notice host beside the update banner, not App', async () => {
  const workspace = await readModule('components/Workspace.tsx');
  assert.match(workspace, /import \{ FormatProvenanceNoticeHost \} from '\.\/FormatProvenanceNoticeHost\.tsx';/);
  assert.match(workspace, /<UpdateNoticeHost \/>\s*<FormatProvenanceNoticeHost \/>/);

  const app = await readModule('App.tsx');
  assert.doesNotMatch(app, /FormatProvenanceNotice/, 'App.tsx is at its line ceiling');
});

test('the host defers to a live dialog and to the other shell notices', async () => {
  const source = await readModule('components/FormatProvenanceNoticeHost.tsx');
  assert.match(source, /shell\.status\?\.popup/, 'a host dialog must suppress the card');
  assert.match(source, /usePluginNotice\(\)/);
  assert.match(source, /isHostNoticeVisible/, 'the GPU-fallback notice must not stack with it');
  assert.match(source, /useUpdateStatus\(\)/);
  assert.match(source, /update\.kind === 'available'/, 'the update banner shares this slot');
  assert.match(source, /openedSourceOf\(shell\.status\)/);
});

test('the card is a polite non-modal status region that never takes the keyboard', async () => {
  const card = await readModule('components/FormatProvenanceNotice.tsx');
  assert.match(card, /role="status"/);
  assert.match(card, /aria-live="polite"/);
  assert.match(card, /aria-labelledby=/);
  assert.match(card, /t\('open\.notice\.dismiss'\)/);
  assert.match(card, /aria-label=\{dismissLabel\}/, 'the dismiss button carries the catalog label');
  assert.match(card, /<Button/, 'dismiss is a real focusable button');
  assert.match(card, /openNoticeView\(source, t\)/, 'the copy comes from the one builder');
  assert.doesNotMatch(card, /popup-overlay|HostNotice|aria-modal|role="alertdialog"/,
    'the modal host-notice pattern is not reused');
  assert.doesNotMatch(card, /useNoticeFocus|addEventListener\(['"]keydown['"]|\.focus\(\)/,
    'no focus trap, no key handler, no stolen focus');
  assert.doesNotMatch(card, /[\u3400-\u4dbf\u4e00-\u9fff]/, 'no hardcoded CJK copy in the TSX');

  const host = await readModule('components/FormatProvenanceNoticeHost.tsx');
  assert.doesNotMatch(host, /useNoticeFocus|addEventListener\(['"]keydown['"]|\.focus\(\)/);
  assert.doesNotMatch(host, /[\u3400-\u4dbf\u4e00-\u9fff]/);
});

test('the new sources use erasable TypeScript syntax only', async () => {
  const files = [
    'cad/opened-source.ts',
    'components/FormatProvenanceNotice.tsx',
    'components/FormatProvenanceNoticeHost.tsx',
  ];
  for (const name of files) {
    const source = await readModule(name);
    assert.doesNotMatch(source, /^\s*(export\s+)?enum\s/m, `${name}: enum is not erasable`);
    assert.doesNotMatch(source, /^\s*(export\s+)?namespace\s/m, `${name}: namespace is not erasable`);
    assert.doesNotMatch(source, /constructor\s*\([^)]*\b(private|public|protected|readonly)\s/,
      `${name}: parameter properties are not erasable`);
  }
});

// --- i18n ----------------------------------------------------------------

test('both catalogs carry the notice keys with matching placeholders', () => {
  const keys = [
    'open.notice.reverse.title',
    'open.notice.reverse.body',
    'open.notice.save_back.ymodel',
    'open.notice.save_back.experimental',
    'open.notice.save_back.unsupported',
    'open.notice.dismiss',
  ];
  for (const connector of REVERSE_CONNECTORS) {
    keys.push(`connector.${connector}.name`, `open.notice.detail.${connector}`);
  }
  for (const key of keys) {
    assert.equal(typeof en[key], 'string', `en catalog is missing ${key}`);
    assert.equal(typeof zh[key], 'string', `zh-CN catalog is missing ${key}`);
    assert.deepEqual(placeholders(en[key]), placeholders(zh[key]), `${key} placeholders drift`);
  }
  assert.deepEqual(placeholders(en['open.notice.reverse.title']), ['format']);
  assert.deepEqual(placeholders(en['open.notice.reverse.body']), ['format']);
  assert.deepEqual(placeholders(en['open.notice.save_back.experimental']), ['format']);
  assert.deepEqual(placeholders(en['open.notice.save_back.unsupported']), ['format']);
  assert.deepEqual(placeholders(en['open.notice.save_back.ymodel']), []);
  for (const connector of REVERSE_CONNECTORS) {
    assert.deepEqual(placeholders(en[`open.notice.detail.${connector}`]), [], `${connector} detail takes no placeholder`);
  }
});

test('every reverse-engineered connector renders all of its catalog copy', () => {
  for (const connector of REVERSE_CONNECTORS) {
    const view = openNoticeView(source(connector), t);
    assert.ok(view.title.length > 0, connector);
    assert.deepEqual(view.lines.map(line => line.key), ['body', 'detail', 'saveBack', 'ymodel'], connector);
    assert.equal(view.lines[1].text, en[`open.notice.detail.${connector}`], connector);
  }
});

// --- styling -------------------------------------------------------------

test('the card takes the update banner slot: above the carrier, below palettes', async () => {
  const css = await readCss();
  assert.match(css, /\.format-notice-layer\s*\{[^}]*z-index:\s*9;/);
  assert.match(css, /\.palette-layer\s*\{[^}]*z-index:\s*10;/);
  assert.match(css, /\.format-notice-layer\s*\{[^}]*pointer-events:\s*none;/,
    'the layer must not eat presses meant for the CAD surface');
  assert.match(css, /\.format-notice\s*\{[^}]*pointer-events:\s*auto;/,
    'the card itself stays interactive');
  assert.match(css, /:root\[data-layout="mobile"\] \.format-notice\s*\{/, 'compact layout gets its own inset');
  assert.match(css, /env\(safe-area-inset-left\)/);
});
