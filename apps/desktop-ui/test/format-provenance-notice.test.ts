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
//   * dismissal is per open (document label + connector, in memory) and writes
//     nothing, while "don't show again" writes `formatNoticeEnabled` and silences
//     every reverse-engineered format only after the host's own re-read agrees;
//   * the card stays the non-modal `UpdateNotice` pattern, mounted from
//     `Workspace` (never `App.tsx`, which is at its line ceiling) and suppressed
//     by a dialog or another shell notice - but never shown at all when the
//     preference is off.
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
import {
  disableFormatNotice,
  formatNoticeEnabled,
  formatNoticeVisible,
  readFormatNoticePreference,
} from '../src/cad/format-notice-setting.ts';
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

test('"Got it" closes this open only, in memory, and writes no setting', async () => {
  const source = await readModule('components/FormatProvenanceNoticeHost.tsx');
  assert.match(source, /useState<string \| null>\(null\)/);
  assert.match(source, /if \(key === null\) setDismissedKey\(null\)/,
    'a new open must not inherit a previous dismissal');
  assert.match(source, /onDismiss=\{\(\) => setDismissedKey\(key\)\}/);
  assert.doesNotMatch(source, /localStorage|sessionStorage/,
    'the per-open dismissal is never persisted');

  // The card's primary answer is the prop alone: the one `cad_set_setting`
  // path is the *other* button, so "Got it" can never write a preference.
  const card = await readModule('components/FormatProvenanceNotice.tsx');
  assert.match(card, /onClick=\{onDismiss\}/, '"Got it" only calls the dismiss prop');
  assert.equal((card.match(/disableFormatNotice\(/g) ?? []).length, 1,
    'exactly one write helper call site in the card');
  assert.match(card, /const turnOff = async \(\) => \{[\s\S]*?disableFormatNotice\(invoke\)/,
    'the write lives in the second button\'s handler');
});

// --- the durable switch ---------------------------------------------------

test('an absent or malformed preference reads as enabled, never as off', () => {
  assert.equal(formatNoticeEnabled(null), true);
  assert.equal(formatNoticeEnabled(undefined), true);
  assert.equal(formatNoticeEnabled({}), true, 'an older host must not silence the notice');
  assert.equal(formatNoticeEnabled({ formatNoticeEnabled: true }), true);
  assert.equal(formatNoticeEnabled({ formatNoticeEnabled: false }), false);
  for (const bad of ['false', 0, 1, null, {}, []]) {
    assert.equal(formatNoticeEnabled({ formatNoticeEnabled: bad }), true,
      `${JSON.stringify(bad)} is not the host's own false`);
  }
});

/** A fake host: one live setting, `cad_settings` reads it, writes may change it. */
function fakeHost({ acceptWrite = true, apply = true, writeError = null, readError = null } = {}) {
  const state = { formatNoticeEnabled: true };
  const calls = [];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    if (command === 'cad_set_setting') {
      if (writeError) throw new Error(writeError);
      if (acceptWrite && apply) state[args.key] = args.value;
      return { accepted: acceptWrite };
    }
    if (command === 'cad_settings') {
      if (readError) throw new Error(readError);
      return { ...state };
    }
    throw new Error(`unexpected command ${command}`);
  };
  return { invoke, calls, state };
}

test('the preference is read from the host, and a waiting/failed read is not "off"', async () => {
  const enabled = fakeHost();
  assert.equal(await readFormatNoticePreference(enabled.invoke), 'enabled');
  assert.deepEqual(enabled.calls, [{ command: 'cad_settings', args: undefined }]);

  const disabled = fakeHost();
  disabled.state.formatNoticeEnabled = false;
  assert.equal(await readFormatNoticePreference(disabled.invoke), 'disabled');

  // No snapshot is not the user's decision: the card is shown, never suppressed.
  assert.equal(await readFormatNoticePreference(fakeHost({ readError: 'cad_waiting' }).invoke), 'unknown');
  assert.equal(await readFormatNoticePreference(fakeHost({ readError: 'boom' }).invoke), 'unknown');
});

test('"don\'t show again" writes the setting key and needs the host re-read to agree', async () => {
  const host = fakeHost();
  const result = await disableFormatNotice(host.invoke, { sleep: async () => {} });
  assert.deepEqual(result, { kind: 'ok' });
  assert.deepEqual(host.calls[0], {
    command: 'cad_set_setting',
    args: { key: 'formatNoticeEnabled', value: false },
  });
  assert.equal(host.calls[1].command, 'cad_settings', 'the write is confirmed by a read-back');
  assert.equal(host.state.formatNoticeEnabled, false);
});

test('a refused write reports failure and the card is not silenced', async () => {
  // The host answers `{accepted:false}`: nothing is queued, so nothing is read.
  const refused = fakeHost({ acceptWrite: false });
  const result = await disableFormatNotice(refused.invoke, { sleep: async () => {} });
  assert.equal(result.kind, 'failed');
  assert.equal(result.error, 'settings.invalid_value');
  assert.deepEqual(refused.calls.map(call => call.command), ['cad_set_setting']);

  // A thrown refusal keeps the host's own code.
  const thrown = fakeHost({ writeError: 'settings.unknown_key' });
  const thrownResult = await disableFormatNotice(thrown.invoke, { sleep: async () => {} });
  assert.equal(thrownResult.kind, 'failed');
  assert.equal(thrownResult.error, 'settings.unknown_key');
});

test('a write the re-read never shows (host rollback) is a failure, not a success', async () => {
  const rolledBack = fakeHost({ acceptWrite: true, apply: false });
  const result = await disableFormatNotice(rolledBack.invoke, { attempts: 3, sleep: async () => {} });
  assert.equal(result.kind, 'failed');
  assert.equal(result.error, 'settings.not_visible');
  // It really did retry rather than give up after one read: one write, then
  // every read-back round.
  assert.deepEqual(rolledBack.calls.map(call => call.command), [
    'cad_set_setting', 'cad_settings', 'cad_settings', 'cad_settings',
  ]);
});

test('a failed re-read is reported with the read error, never as success', async () => {
  const host = fakeHost({ readError: 'settings.unavailable' });
  const result = await disableFormatNotice(host.invoke, { sleep: async () => {} });
  assert.equal(result.kind, 'failed');
  // The read's own text is preserved verbatim (the transport wraps it once), so
  // the reason survives instead of becoming a generic "not saved".
  assert.ok(result.error.includes('settings.unavailable'), result.error);
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
  assert.match(source, /if \(shell\.status\?\.popup\) return null;/, 'a host dialog must suppress the card');
  assert.match(source, /usePluginNotice\(\)/);
  assert.match(source, /isHostNoticeVisible/, 'the GPU-fallback notice must not stack with it');
  assert.match(source, /useUpdateStatus\(\)/);
  assert.match(source, /update\.kind === 'available'/, 'the update banner shares this slot');
  assert.match(source, /openedSourceOf\(shell\.status\)/);
});

test('a disabled preference renders nothing, before every other suppression rule', async () => {
  const source = await readModule('components/FormatProvenanceNoticeHost.tsx');
  assert.match(source, /readFormatNoticePreference\(invoke\)/, 'the preference comes from the host');
  // The decision is one pure predicate, so "off means no card" is testable:
  // only the host's own `false` hides it.
  assert.equal(formatNoticeVisible('disabled'), false);
  assert.equal(formatNoticeVisible('enabled'), true);
  assert.equal(formatNoticeVisible('unknown'), true, 'a failed read must not hide the notice');
  assert.equal(formatNoticeVisible(null), false, 'no answer yet renders nothing (no flash)');

  const gate = 'if (!formatNoticeVisible(preference)) return null;';
  const gateAt = source.indexOf(gate);
  assert.ok(gateAt > -1, 'the host gates the card on the setting');
  // A dialog, a banner or another notice must not bring the card back for a
  // user who turned it off: the setting is the first reason to render nothing.
  for (const later of [
    'if (shell.status?.popup) return null;',
    'if (hostNotice || pluginNotice !== null) return null;',
    "if (update.kind === 'available') return null;",
    'if (isOpenNoticeDismissed(dismissedKey, key)) return null;',
  ]) {
    const at = source.indexOf(later);
    assert.ok(at > gateAt, `${later} must come after the setting check`);
  }
  // The preference is re-read per open, and after a dialog closes: Options edits
  // the same setting, so a card must not return from a dialog that turned it off.
  assert.match(source, /\}, \[key, popupOpen\]\);/, 're-read on a new open and when a dialog opens/closes');
  // The confirmed write silences this open too.
  assert.match(source, /onDisabled=\{\(\) => setPreference\('disabled'\)\}/);
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

test('the second answer is a quieter button with a busy state and an alert on failure', async () => {
  const card = await readModule('components/FormatProvenanceNotice.tsx');
  // Two real answers, and the primary keeps its chip surface so the card still
  // has one obvious default.
  assert.match(card, /t\('open\.notice\.dont_show_again'\)/);
  assert.match(card, /variant="chip"[\s\S]*?onClick=\{onDismiss\}/, '"Got it" stays the primary chip');
  assert.match(card, /variant="ghost"[\s\S]*?onClick=\{\(\) => \{ void turnOff\(\); \}\}/,
    'the preference button is the quiet one');
  // Busy state: both buttons are disabled while the write is in flight, and the
  // pressed one announces it, so a second press is refused rather than queued.
  assert.match(card, /disabled=\{pending\}/);
  assert.match(card, /aria-busy=\{pending \? true : undefined\}/);
  assert.match(card, /if \(pending\) return;/, 'a double press cannot queue a second write');
  // The failure copy is an alert beside the buttons, never a toast and never a
  // claim that the card is going away.
  assert.match(card, /role="alert"/);
  assert.match(card, /t\('open\.notice\.dont_show_again_failed'\)/);
  assert.match(card, /if \(result\.kind === 'ok'\) \{\s*onDisabled\(\);/,
    'only a confirmed write silences the card');
  assert.match(card, /setFailed\(true\)/, 'an unconfirmed write keeps the card and says so');
});


test('the new sources use erasable TypeScript syntax only', async () => {
  const files = [
    'cad/opened-source.ts',
    'cad/format-notice-setting.ts',
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
    'open.notice.dont_show_again',
    'open.notice.dont_show_again_failed',
    'options.field.formatNotice',
    'options.help.formatNotice',
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
  // Both new sentences are literal copy: no placeholder may drift between the
  // catalogs (the button says the same thing in both languages).
  assert.deepEqual(placeholders(en['open.notice.dont_show_again']), []);
  assert.deepEqual(placeholders(en['open.notice.dont_show_again_failed']), []);
  assert.deepEqual(placeholders(en['options.help.formatNotice']), []);
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
  // Both answers are touch targets in one row, exactly like the update banner's
  // Later/Skip row, so the second button does not grow the card on a phone.
  assert.match(css, /:root\[data-layout="mobile"\] \.format-notice__actions \{[^}]*grid-template-columns: minmax\(0, 1fr\) minmax\(0, 1fr\);/,
    'the two answers share one row');
  assert.match(css, /:root\[data-layout="mobile"\] \.format-notice__actions \.ui-button \{[^}]*min-height: 44px;/,
    'each compact answer is a 44px touch target');
  assert.match(css, /:root\[data-layout="mobile"\] \.format-notice__actions \.ui-button \{[^}]*white-space: normal;/,
    'a long label wraps inside its half instead of overflowing the card');
  // The notice itself stays free of danger red; only the refused-write line uses
  // it, because only that line reports a failure.
  assert.match(css, /\.format-notice__alert \{[^}]*grid-area: alert;/);
  assert.match(css, /\.format-notice__alert \{[^}]*color: var\(--danger\);/);
  assert.match(css, /\.format-notice__actions \.ui-button\[aria-busy="true"\]/, 'a pending write reads as busy');
});
