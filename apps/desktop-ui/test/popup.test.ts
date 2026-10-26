// Popup layer contract (ADR 0021 D1/D2, WP3), UI half.
//
// Since WP3 a popup is an in-DOM overlay in the single UI tree. The host owns
// the state (`cad_status.popup`) and the additive `cad_open_popup` payload; this
// suite locks:
//   * the payload parser's rejection rules (tag mismatch/malformed content =>
//     explicit unknown state, never an editor with invented content);
//   * the about/textEdit/options views;
//   * the overlay's persistent chrome (title + close) outside the body
//     boundary, so Close and Escape stay reachable when the body fails;
//   * Escape / Close both go through `cad_close_popup`, and a failed close is
//     reported rather than silently swallowed;
//   * `App` always renders the shell and mounts the overlay from
//     `cad_status.popup`; the shell's global Escape stands down while open;
//   * the File-tab entry is a shell action that asks for `cad_open_popup`;
//   * every popup catalog key is declared once and resolves in both locales.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCatalog, readCss, readModule } from './helpers.ts';
import {
  aboutVersionText,
  closePopup,
  openPopup,
  POPUP_BLANK,
  POPUP_CLOSE_COMMAND,
  POPUP_KEYS,
  POPUP_KINDS,
  POPUP_OPEN_COMMAND,
  popupPayload,
  popupView,
  textEditPayload,
} from '../src/popup/popup.ts';
import { createTranslator } from '../src/i18n.ts';
import {
  CLOSE_GLYPH_INSET,
  CLOSE_INSET,
  DIALOG_BODY,
  DIALOG_CARD,
  DIALOG_CONTENT,
  DIALOG_FOOT,
  DIALOG_HEAD,
  DIALOG_INSET,
  DIALOG_TITLE,
  HEADER_GLYPH_INSET,
  HEADER_INSET,
  POPUP_CARD_SIZE,
} from '../src/popup/popup-layout.ts';
import { RIBBON_TABS } from '../src/ribbon/ribbon-model.ts';

const RIBBON_LITERAL = POPUP_KEYS.ribbonEntry;

const ZH = await readCatalog('zh-CN');
const EN = await readCatalog('en');
const T_ZH = createTranslator({ 'zh-CN': ZH, en: EN }, 'zh-CN');
const T_EN = createTranslator({ 'zh-CN': ZH, en: EN }, 'en');

const ABOUT_PAYLOAD = Object.freeze({ kind: 'about', version: '0.1.2', build: 'release' });
const TEXT_PAYLOAD = Object.freeze({ kind: 'textEdit', text: 'line 1\nline 2', multiline: true });

/** An `invoke` stand-in that records calls and answers `response`. */
function recordingInvoke(response = { accepted: true }) {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    return typeof response === 'function' ? response(command, args) : response;
  };
  return { invoke, calls };
}

test('the implemented kinds grow only by contract', () => {
  assert.deepEqual(POPUP_KINDS, ['about', 'textEdit', 'options']);
});

test('an about payload is tagged, copied into the view and frozen', () => {
  assert.deepEqual(popupPayload('about', ABOUT_PAYLOAD), {
    kind: 'about', version: '0.1.2', build: 'release',
  });
  const view = popupView('about', ABOUT_PAYLOAD);
  assert.deepEqual(view, {
    kind: 'about', requestedKind: 'about', version: '0.1.2', build: 'release',
  });
  // The raw payload object itself never reaches the view (only copied fields).
  assert.equal('payload' in view, false);
  assert.ok(Object.isFrozen(popupPayload('about', ABOUT_PAYLOAD)));
});

test('a payload tagged for another kind is rejected, never read across kinds', () => {
  assert.equal(popupPayload('about', TEXT_PAYLOAD), null);
  assert.equal(popupPayload('textEdit', ABOUT_PAYLOAD), null);
  assert.deepEqual(popupView('textEdit', ABOUT_PAYLOAD), {
    kind: 'unknown', requestedKind: 'textEdit',
  });
});

test('a malformed or absent about payload leaves both view values absent', () => {
  for (const payload of [
    null, undefined, {}, [], 'about', { kind: 'about' },
    { kind: 'about', version: '' },
    { kind: 'about', version: '   ' },
    { kind: 'about', version: 7 },
    { kind: 'about', version: null },
  ]) {
    assert.equal(popupPayload('about', payload), null, JSON.stringify(payload));
    const view = popupView('about', payload);
    assert.equal(view.kind, 'about');
    assert.equal(view.version, null, JSON.stringify(payload));
    assert.equal(view.build, null, JSON.stringify(payload));
  }
  // A real version survives a bad/absent build; only that field degrades.
  for (const payload of [
    { kind: 'about', version: '1' },
    { kind: 'about', version: '1', build: '' },
    { kind: 'about', version: '1', build: 7 },
  ]) {
    assert.deepEqual(popupPayload('about', payload), { kind: 'about', version: '1', build: null });
    assert.equal(popupView('about', payload).version, '1');
    assert.equal(popupView('about', payload).build, null);
  }
});

test('the textEdit payload is verbatim and requires both fields', () => {
  assert.deepEqual(popupPayload('textEdit', TEXT_PAYLOAD), {
    kind: 'textEdit', text: 'line 1\nline 2', multiline: true,
  });
  // Empty text is legal data: it is not trimmed and not invented.
  assert.deepEqual(textEditPayload({ kind: 'textEdit', text: '', multiline: false }), {
    text: '', multiline: false,
  });
  for (const payload of [
    null, {}, { kind: 'textEdit' },
    { kind: 'textEdit', text: 7, multiline: true },
    { kind: 'textEdit', text: null, multiline: true },
    { kind: 'textEdit', text: 'x', multiline: 'true' },
    { kind: 'textEdit', text: 'x' },
  ]) {
    assert.equal(textEditPayload(payload), null, JSON.stringify(payload));
  }
  const view = popupView('textEdit', TEXT_PAYLOAD);
  assert.deepEqual(view, {
    kind: 'textEdit', requestedKind: 'textEdit', text: 'line 1\nline 2', multiline: true,
  });
  assert.deepEqual(popupView('textEdit', null), { kind: 'unknown', requestedKind: 'textEdit' });
});

test('options carries no payload and the view is only the envelope', () => {
  assert.equal(popupPayload('options', undefined), null);
  assert.equal(popupPayload('options', { kind: 'options' }), null);
  assert.deepEqual(popupView('options', undefined), {
    kind: 'options', requestedKind: 'options',
  });
});

test('an unknown kind yields the explicit state, not an about dialog and not a crash', () => {
  assert.deepEqual(popupView('future', ABOUT_PAYLOAD), {
    kind: 'unknown', requestedKind: 'future',
  });
  for (const kind of [null, undefined, '', '   ', 9]) {
    assert.deepEqual(popupView(kind, null), { kind: 'unknown', requestedKind: null });
  }
});

test('the version line is the catalog template filled with version and build', () => {
  const view = popupView('about', ABOUT_PAYLOAD);
  for (const [t, catalog] of [[T_ZH, ZH], [T_EN, EN]]) {
    const line = aboutVersionText(view, t);
    assert.equal(
      line,
      catalog[POPUP_KEYS.version].replace('{version}', '0.1.2').replace('{build}', 'release'),
    );
    assert.equal(/[{}]/.test(line), false, line);
  }
});

test('a valid version without a build gets the blank marker, never a placeholder', () => {
  const view = popupView('about', { kind: 'about', version: '0.1.2' });
  for (const [t, catalog] of [[T_ZH, ZH], [T_EN, EN]]) {
    const line = aboutVersionText(view, t);
    assert.equal(
      line,
      catalog[POPUP_KEYS.version].replace('{version}', '0.1.2').replace('{build}', POPUP_BLANK),
    );
    assert.equal(/[{}]/.test(line), false, line);
    assert.ok(line.includes(POPUP_BLANK), line);
  }
});

test('no usable version renders a plain blank line and skips the template entirely', () => {
  for (const payload of [null, {}, { kind: 'about' }, { kind: 'about', build: 'release' }]) {
    const view = popupView('about', payload);
    assert.equal(aboutVersionText(view, T_EN), POPUP_BLANK, JSON.stringify(payload));
    assert.equal(aboutVersionText(view, T_ZH), POPUP_BLANK, JSON.stringify(payload));
  }
});

test('closePopup sends cad_close_popup, reports failure and never throws', async () => {
  const ok = recordingInvoke({ accepted: true });
  assert.deepEqual(await closePopup(ok.invoke), { kind: 'accepted' });
  assert.deepEqual(ok.calls, [{ command: POPUP_CLOSE_COMMAND, args: undefined }]);

  const notAccepted = recordingInvoke({ accepted: false });
  assert.equal((await closePopup(notAccepted.invoke)).kind, 'failed');

  const rejected = await closePopup(async () => { throw new Error('ipc down'); });
  assert.equal(rejected.kind, 'failed');
  assert.match(rejected.error, /ipc down/);

  // The IPC is idempotent: a second Escape/click is safe, never an error state.
  const twice = recordingInvoke({ accepted: true });
  await closePopup(twice.invoke);
  assert.deepEqual(await closePopup(twice.invoke), { kind: 'accepted' });
  assert.equal(twice.calls.length, 2);
});

test('openPopup asks for cad_open_popup and maps the additive payload', async () => {
  const fresh = recordingInvoke({
    accepted: true, kind: 'about', popup_label: 'popup-1', existing: false, payload: ABOUT_PAYLOAD,
  });
  assert.deepEqual(await openPopup(fresh.invoke, 'about'), {
    kind: 'accepted',
    requestedKind: 'about',
    popupLabel: 'popup-1',
    existing: false,
    payload: { kind: 'about', version: '0.1.2', build: 'release' },
  });
  assert.deepEqual(fresh.calls, [{ command: POPUP_OPEN_COMMAND, args: { kind: 'about' } }]);

  // `existing: true` carries no payload and is a success: the overlay already
  // shows that instance, so nothing may be replaced.
  const existing = recordingInvoke({
    accepted: true, kind: 'about', popup_label: 'popup-1', existing: true, payload: ABOUT_PAYLOAD,
  });
  const result = await openPopup(existing.invoke, 'about');
  assert.equal(result.kind, 'accepted');
  assert.equal(result.existing, true);
  assert.equal(result.payload, null);

  // A payload tagged for a different kind is dropped, not rendered.
  const mismatched = recordingInvoke({
    accepted: true, kind: 'about', popup_label: 'popup-1', existing: false, payload: TEXT_PAYLOAD,
  });
  assert.equal((await openPopup(mismatched.invoke, 'about')).payload, null);

  // An absent payload (the pre-WP3 four-field ack) is accepted: the host gate is
  // open, the body degrades to its honest empty state.
  const legacy = recordingInvoke({
    accepted: true, kind: 'options', popup_label: 'popup-1', existing: false,
  });
  assert.equal((await openPopup(legacy.invoke, 'options')).payload, null);

  // An unknown kind is refused before IPC, never sent to the host.
  const untouched = recordingInvoke();
  assert.equal((await openPopup(untouched.invoke, 'future')).kind, 'refused');
  assert.equal(untouched.calls.length, 0);

  const failed = await openPopup(async () => { throw new Error('no gate'); }, 'about');
  assert.equal(failed.kind, 'failed');
});

test('the overlay keeps its chrome outside the body boundary and owns Escape', async () => {
  const source = await readModule('components/PopupOverlay.tsx');
  // The chrome (title + close) is rendered before the boundary, so a throwing
  // body can never take Close or Escape away.
  const chromeAt = source.indexOf('<header className="palette__head"');
  const boundaryAt = source.indexOf('<PopupBodyBoundary');
  assert.ok(chromeAt !== -1 && boundaryAt !== -1 && chromeAt < boundaryAt,
    'the close affordance must precede the body boundary');
  assert.ok(source.includes("event.key !== 'Escape'"));
  assert.ok(source.includes("window.addEventListener('keydown', onKeyDown)"));
  assert.ok(source.includes('closePopup(invoke)'));
  assert.ok(source.includes('getDerivedStateFromError'));
  assert.ok(source.includes('POPUP_KEYS.bodyFailed'));
  assert.ok(source.includes('createPortal(content, document.body)'));
  assert.ok(source.includes('role="dialog"'));
  assert.ok(source.includes("aria-modal={popup?.modal === true ? 'true' : undefined}"));
});

test('App always renders the shell and mounts the overlay from cad_status.popup', async () => {
  const app = await readModule('App.tsx');
  assert.match(app, /export function App\(\) \{\s*return <Shell \/>;/);
  assert.equal(app.includes('PopupRoot'), false);
  assert.equal(app.includes('popupBoot'), false);
  assert.ok(app.includes("import { PopupOverlay } from './components/PopupOverlay.tsx';"));
  assert.ok(app.includes('const popupOpen = status?.popup != null;'));
  // The overlay still takes the popup straight off the status (now optional-chained
  // in TS), never a constant or a local mirror of the state.
  assert.match(app, /\{popupOpen \? <PopupOverlay popup=\{status\??\.popup\} payload=\{popupPayload\} \/> : null\}/);
  // The shell's global accelerator stands down while the overlay owns Escape.
  assert.ok(app.includes('if (popupOpen) return;'));
  // The fresh payload is local state seeded from the ack, never re-derived from
  // status (the wire carries it once).
  assert.ok(app.includes('setPopupPayload(result.payload)'));
  assert.ok(app.includes('if (!popupOpen) setPopupPayload(null);'));
});

test('the File tab opens About through cad_open_popup, reflected by cad_status.popup', async () => {
  const file = RIBBON_TABS.find(tab => tab.id === 'file');
  const utilities = file.panels.find(panel => panel.id === 'utilities');
  const about = utilities.items.find(item => item.id === 'about');
  assert.ok(about, 'the File utilities panel must carry the About entry');
  assert.equal(about.labelKey, RIBBON_LITERAL);
  assert.equal(about.action, 'about');
  assert.equal(about.command, undefined, 'About is a shell action, never a router command');

  const app = await readModule('App.tsx');
  assert.ok(app.includes("else if (action === 'about')"));
  assert.ok(app.includes("requestOpenPopup('about')"));
  assert.ok(app.includes('popupOpen,'));
});

test('the About dialog renders only body and footer; the overlay owns the header', async () => {
  const source = await readModule('components/AboutDialog.tsx');
  // The version line comes from `aboutVersionText`, which fills BOTH template
  // placeholders; the component must never call the template without params.
  assert.ok(source.includes('aboutVersionText(view, t)'));
  assert.equal(source.includes('t(POPUP_KEYS.version)'), false, 'the version template must be filled');
  assert.ok(source.includes('{versionLine}'));
  assert.ok(source.includes('t(POPUP_KEYS.subtitle)'));
  assert.ok(source.includes('t(POPUP_KEYS.close)'));
  assert.ok(source.includes("t('app.title')"));
  // One header owner only.
  assert.equal(source.includes('<header'), false);
  assert.equal(source.includes('IconButton'), false);
  // No hand-built metadata and no feature/marketing list.
  assert.equal(source.includes('view.build'), false);
  assert.equal(source.includes('payload.build'), false);
  assert.equal(/<ul|<ol/.test(source), false);
});

test('the text editor and Options dialogs also leave the header to the overlay', async () => {
  for (const file of ['components/TextEditDialog.tsx', 'components/OptionsDialog.tsx']) {
    const source = await readModule(file);
    assert.equal(source.includes('<header'), false, `${file} must not render its own header`);
    assert.equal(source.includes('IconButton'), false, `${file} must not render its own close button`);
  }
  const options = await readModule('components/OptionsDialog.tsx');
  assert.ok(options.includes("useI18n()"), 'Options still needs the shared i18n context');
});

test('popup copy comes only from the catalog registry, with no hardcoded CJK', async () => {
  assert.deepEqual(Object.values(POPUP_KEYS).sort(), [
    'dialog.about.subtitle',
    'dialog.about.title',
    'dialog.about.version',
    'dialog.popup.body_failed',
    'dialog.popup.close',
    'file.about',
  ]);
  // `popup.mjs` is the single declaration site; the components address these
  // keys through `POPUP_KEYS` and never repeat a literal.
  const registry = await readModule('popup/popup.ts');
  for (const key of Object.values(POPUP_KEYS)) {
    assert.equal(registry.includes(`'${key}'`), true, `popup.mjs must declare ${key}`);
  }
  for (const file of ['components/PopupOverlay.tsx', 'components/AboutDialog.tsx']) {
    const source = await readModule(file);
    assert.equal(/'(dialog\.|file\.about)/.test(source), false, `${file} must not inline catalog keys`);
    assert.equal(/[\u3400-\u9fff\uf900-\ufaff]/.test(source), false, `${file} must not hardcode CJK`);
  }
  // The ribbon entry's label is the registry key, not a literal.
  assert.ok(RIBBON_TABS.some(tab => tab.panels.some(panel =>
    panel.items.some(item => item.id === 'about' && item.labelKey === POPUP_KEYS.ribbonEntry))));
});

test('popup components import every hook they call', async () => {
  // The bundle does not resolve free identifiers, so a missing import only
  // fails at runtime. Keep the overlay and the About body honest here.
  for (const file of ['components/PopupOverlay.tsx', 'components/AboutDialog.tsx']) {
    const source = await readModule(file);
    if (source.includes('useI18n()')) {
      assert.match(source, /import \{[^}]*useI18n[^}]*\} from '\.\.\/context\.ts'/,
        `${file} calls useI18n but does not import it`);
    }
    assert.equal(source.includes('useShell()'), false, `${file} has no shell to read`);
  }
});

test('the overlay uses one content edge for header, body and footer', async () => {
  assert.equal(DIALOG_INSET, 16);
  // Box edges: body content and the footer action sit on the same inset.
  assert.equal(DIALOG_BODY.padding, `20px ${DIALOG_INSET}px 18px`);
  assert.equal(DIALOG_FOOT.padding, `10px ${DIALOG_INSET}px`);
  // The header is icon-led and the close affordance ends in a glyph, so each
  // travels its glyph's own ink inset to land on that same optical edge.
  const headerInk = HEADER_INSET + HEADER_GLYPH_INSET;
  const closeInk = CLOSE_INSET + CLOSE_GLYPH_INSET;
  assert.equal(DIALOG_HEAD.padding, `0 ${CLOSE_INSET}px 0 ${HEADER_INSET}px`);
  assert.ok(Math.abs(headerInk - DIALOG_INSET) <= 0.5, `header ink ${headerInk}`);
  assert.ok(Math.abs(closeInk - DIALOG_INSET) <= 0.5, `close ink ${closeInk}`);
  // Symmetric close affordance: the same optical gap on both header ends.
  assert.equal(closeInk, headerInk);

  // Every dialog body reads the shared metrics; none keeps its own copy, so the
  // pixel-review fix cannot drift back out of one of them.
  for (const file of [
    'components/AboutDialog.tsx',
    'components/TextEditDialog.tsx',
    'components/OptionsDialog.tsx',
  ]) {
    const source = await readModule(file);
    assert.ok(source.includes("from '../popup/popup-layout.ts'"), `${file} must use the shared metrics`);
    assert.equal(source.includes('20px 20px 18px'), false, `${file} still carries the old body inset`);
    assert.equal(source.includes("'0 6px 0 12px'"), false, `${file} still carries the old header inset`);
  }
});

test('the card size is UI-owned and clamped to the viewport', async () => {
  assert.deepEqual(POPUP_CARD_SIZE, {
    about: { width: 520, height: 360 },
    textEdit: { width: 640, height: 480 },
    options: { width: 720, height: 560 },
  });
  const overlay = await readModule('components/PopupOverlay.tsx');
  assert.ok(overlay.includes('calc(100vw - 32px)'));
  assert.ok(overlay.includes('calc(100vh - 32px)'));
  // No host-window constants survive in the UI layer.
  assert.equal(/POPUP_WIDTH|POPUP_HEIGHT|TEXT_EDIT_WIDTH|TEXT_EDIT_HEIGHT/.test(overlay), false);
});

test('the body centres its content block and stays scroll-safe above the footer', async () => {
  // `margin: auto 0` (not `justify-content: center`) is what removes the void:
  // it centres when there is slack and collapses to 0 when the content is
  // taller, so the body can still scroll instead of clipping the top.
  assert.equal(DIALOG_CONTENT.margin, 'auto 0');
  assert.equal(DIALOG_BODY.overflow, 'auto');
  assert.equal('justifyContent' in DIALOG_BODY, false);
  assert.equal(DIALOG_BODY.flex, '1 1 auto');
  assert.equal(DIALOG_CARD.background, 'var(--chrome-2)');
});

test('the overlay layer is a fixed, above-shell backdrop', async () => {
  const css = await readCss();
  const overlay = css.match(/\.popup-overlay\s*\{([^}]*)\}/);
  assert.ok(overlay, 'style.css must define .popup-overlay');
  assert.match(overlay[1], /position:\s*fixed/);
  assert.match(overlay[1], /inset:\s*0/);
  assert.match(overlay[1], /z-index:\s*\d+/);
});

test('every dialog text token clears 4.5:1 on the card surface', async () => {
  const css = await readCss();
  const token = name => {
    const match = css.match(new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6})`));
    assert.ok(match, `stylesheet must define --${name}`);
    return match[1].slice(1);
  };
  const luminance = hex => {
    const rgb = hex.match(/\w\w/g).map(pair => parseInt(pair, 16) / 255)
      .map(channel => channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4);
    return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
  };
  const contrast = (a, b) => {
    const values = [luminance(a), luminance(b)].sort((x, y) => y - x);
    return (values[0] + 0.05) / (values[1] + 0.05);
  };
  const surface = token('chrome-2');
  for (const name of ['text', 'text-dim', 'text-muted']) {
    const ratio = contrast(token(name), surface);
    assert.ok(ratio >= 4.5, `--${name} on --chrome-2 = ${ratio.toFixed(2)}`);
  }
  assert.equal(DIALOG_TITLE.color, 'var(--text)');
  assert.equal(DIALOG_TITLE.fontSize, '13px');
  assert.ok(contrast(token('text'), surface) >= 9);
});

test('both locales resolve every popup catalog key, filled with no placeholder left', () => {
  for (const key of Object.values(POPUP_KEYS)) {
    for (const [locale, catalog] of [['zh-CN', ZH], ['en', EN]]) {
      assert.equal(typeof catalog[key], 'string', `${locale} missing ${key}`);
      assert.ok(catalog[key].trim().length > 0, `${locale} has empty ${key}`);
    }
  }
  for (const [locale, catalog, t] of [['zh-CN', ZH, T_ZH], ['en', EN, T_EN]]) {
    const template = catalog[POPUP_KEYS.version];
    for (const placeholder of ['{version}', '{build}']) {
      assert.ok(template.includes(placeholder), `${locale} template missing ${placeholder}`);
    }
    const filled = t(POPUP_KEYS.version, { version: '9.8.7', build: 'debug' });
    assert.ok(filled.includes('9.8.7'), filled);
    assert.ok(filled.includes('debug'), filled);
    assert.equal(/\{[A-Za-z_]/.test(filled), false, `${locale} left a placeholder: ${filled}`);
  }
});
