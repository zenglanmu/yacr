// Popup pipeline contract (ADR 0021 D1), UI half.
//
// The host opens a dialog as a second native window loading the *same* bundle
// and injects `window.__YACR_POPUP__` before page load. These tests lock:
//   * the boot predicate's rejection rules (absent/malformed/unknown kind =>
//     normal shell, never a partial popup and never a thrown error);
//   * the about view (payload version + build, never invented metadata);
//   * the dialog's layout data: one content edge, a centred body, and every
//     text token at or above 4.5:1 on the card surface;
//   * the explicit unknown-kind state;
//   * Escape / Close both go through `cad_close_popup`, and a failed close is
//     reported rather than silently swallowed;
//   * the popup document starts none of the shell's global effects;
//   * the File-tab entry is a shell action that asks for `cad_open_popup`;
//   * every popup catalog key is declared once and resolves in both locales.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readCatalog, readCss, readModule } from './helpers.mjs';
import {
  aboutVersionText,
  closePopup,
  openPopup,
  POPUP_BLANK,
  POPUP_CLOSE_COMMAND,
  POPUP_GLOBAL,
  POPUP_KEYS,
  POPUP_KINDS,
  POPUP_OPEN_COMMAND,
  popupBoot,
  popupView,
  readPopupBoot,
} from '../src/popup.mjs';
import { createTranslator } from '../src/i18n.mjs';
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
} from '../src/popup-layout.mjs';
import { RIBBON_TABS } from '../src/ribbon-model.mjs';

const RIBBON_LITERAL = POPUP_KEYS.ribbonEntry;

const ZH = await readCatalog('zh-CN');
const EN = await readCatalog('en');
const T_ZH = createTranslator({ 'zh-CN': ZH, en: EN }, 'zh-CN');
const T_EN = createTranslator({ 'zh-CN': ZH, en: EN }, 'en');

/** A valid literal exactly as the host injects it. */
function bootLiteral(overrides = {}) {
  return {
    [POPUP_GLOBAL]: {
      kind: 'about',
      modal: true,
      label: 'popup-1',
      payload: { version: '0.1.2', build: 'release' },
      ...overrides,
    },
  };
}

/** An `invoke` stand-in that records calls and answers `response`. */
function recordingInvoke(response = { accepted: true }) {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    return typeof response === 'function' ? response(command, args) : response;
  };
  return { invoke, calls };
}

test('the boot predicate falls back to the normal shell when the literal is absent', () => {
  for (const scope of [undefined, null, {}, { [POPUP_GLOBAL]: undefined }, { [POPUP_GLOBAL]: null }]) {
    assert.equal(popupBoot(scope), null);
    assert.equal(readPopupBoot(scope), null);
  }
});

test('a malformed boot literal is rejected instead of half-rendering a popup', () => {
  const malformed = [
    { [POPUP_GLOBAL]: [] },
    { [POPUP_GLOBAL]: 'about' },
    { [POPUP_GLOBAL]: 3 },
    { [POPUP_GLOBAL]: true },
    // kind
    { [POPUP_GLOBAL]: { modal: true, label: 'popup-1', payload: {} } },
    { [POPUP_GLOBAL]: { kind: 7, modal: true, label: 'popup-1', payload: {} } },
    { [POPUP_GLOBAL]: { kind: '   ', modal: true, label: 'popup-1', payload: {} } },
    // modal
    { [POPUP_GLOBAL]: { kind: 'about', label: 'popup-1', payload: {} } },
    { [POPUP_GLOBAL]: { kind: 'about', modal: 'true', label: 'popup-1', payload: {} } },
    // label
    { [POPUP_GLOBAL]: { kind: 'about', modal: true, payload: {} } },
    { [POPUP_GLOBAL]: { kind: 'about', modal: true, label: 5, payload: {} } },
    { [POPUP_GLOBAL]: { kind: 'about', modal: true, label: '', payload: {} } },
    // payload
    { [POPUP_GLOBAL]: { kind: 'about', modal: true, label: 'popup-1' } },
    { [POPUP_GLOBAL]: { kind: 'about', modal: true, label: 'popup-1', payload: [] } },
    { [POPUP_GLOBAL]: { kind: 'about', modal: true, label: 'popup-1', payload: null } },
  ];
  for (const scope of malformed) {
    assert.equal(popupBoot(scope), null, JSON.stringify(scope));
  }
});

test('a throwing global getter is contained, not propagated', () => {
  const hostile = {
    get [POPUP_GLOBAL]() { throw new Error('boom'); },
  };
  assert.equal(readPopupBoot(hostile), null);
  assert.equal(popupBoot(hostile), null);
});

test('an unknown-but-well-formed kind is a shell boot, while the envelope stays readable', () => {
  const unknown = bootLiteral({ kind: 'future' });
  // The strict switch never replaces the main window with an unimplemented kind.
  assert.equal(popupBoot(unknown), null);
  // The envelope itself is well formed, so the popup root can name it.
  assert.deepEqual(readPopupBoot(unknown), {
    kind: 'future', modal: true, label: 'popup-1', payload: { version: '0.1.2', build: 'release' },
  });
  // The implemented set grows only by contract: `about` and the text editor.
  assert.deepEqual(POPUP_KINDS, ['about', 'textEdit']);
});

test('a well-formed about literal is accepted and frozen, extra fields ignored', () => {
  const scope = bootLiteral({ futureField: 'ignored' });
  const boot = popupBoot(scope);
  assert.ok(boot, 'the about literal must boot the popup');
  assert.equal(boot.kind, 'about');
  assert.equal(boot.modal, true);
  assert.equal(boot.label, 'popup-1');
  assert.deepEqual(boot.payload, { version: '0.1.2', build: 'release' });
  assert.ok(Object.isFrozen(boot));
  assert.equal(boot.futureField, undefined);
});

test('the about view carries both payload strings the catalog template consumes', () => {
  const view = popupView(popupBoot(bootLiteral()));
  assert.deepEqual(view, {
    kind: 'about', requestedKind: 'about', modal: true, version: '0.1.2', build: 'release',
  });
  // The raw payload object itself never reaches the view (only copied fields).
  assert.equal('payload' in view, false);
});

test('a missing or malformed payload version leaves the view value absent', () => {
  for (const payload of [{}, { version: '' }, { version: '   ' }, { version: 7 }, { version: null }]) {
    const view = popupView(popupBoot(bootLiteral({ payload })));
    assert.equal(view.kind, 'about');
    assert.equal(view.version, null, JSON.stringify(payload));
  }
});

test('a missing or malformed payload build is absent, never a fabricated string', () => {
  for (const payload of [{ build: '' }, { build: '   ' }, { build: 7 }, { build: null }]) {
    const view = popupView(popupBoot(bootLiteral({ payload })));
    assert.equal(view.kind, 'about');
    assert.equal(view.build, null, JSON.stringify(payload));
  }
});

test('the version line is the catalog template filled with version and build', () => {
  const view = popupView(popupBoot(bootLiteral()));
  for (const [t, catalog] of [[T_ZH, ZH], [T_EN, EN]]) {
    const line = aboutVersionText(view, t);
    assert.equal(
      line,
      catalog[POPUP_KEYS.version].replace('{version}', '0.1.2').replace('{build}', 'release'),
    );
    assert.equal(/[{}]/.test(line), false, line);
  }
  assert.ok(aboutVersionText(view, T_EN).includes('0.1.2'));
  assert.ok(aboutVersionText(view, T_EN).includes('release'));
});

test('a valid version without a build gets the blank marker, never a placeholder', () => {
  const view = popupView(popupBoot(bootLiteral({ payload: { version: '0.1.2' } })));
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
  for (const payload of [{}, { version: '' }, { version: '   ' }, { version: 7 }, { version: null }, { build: 'release' }]) {
    const view = popupView(popupBoot(bootLiteral({ payload })));
    assert.equal(aboutVersionText(view, T_EN), POPUP_BLANK, JSON.stringify(payload));
    assert.equal(aboutVersionText(view, T_ZH), POPUP_BLANK, JSON.stringify(payload));
  }
});

test('an unknown kind yields the explicit state, not an about dialog and not a crash', () => {
  const view = popupView({ kind: 'future', modal: true, label: 'popup-2', payload: {} });
  assert.deepEqual(view, { kind: 'unknown', requestedKind: 'future' });
  for (const boot of [null, undefined, {}, { kind: '' }, { kind: 9 }]) {
    assert.deepEqual(popupView(boot), { kind: 'unknown', requestedKind: null });
  }
});

test('closePopup sends cad_close_popup, reports failure and never throws', async () => {
  const ok = recordingInvoke({ accepted: true });
  assert.deepEqual(await closePopup(ok.invoke), { kind: 'accepted' });
  assert.deepEqual(ok.calls, [{ command: POPUP_CLOSE_COMMAND, args: undefined }]);

  const notAccepted = recordingInvoke({ accepted: false });
  const refused = await closePopup(notAccepted.invoke);
  assert.equal(refused.kind, 'failed');

  const rejected = await closePopup(async () => { throw new Error('ipc down'); });
  assert.equal(rejected.kind, 'failed');
  assert.match(rejected.error, /ipc down/);

  // The IPC is idempotent: a second Escape/click is safe, never an error state.
  const twice = recordingInvoke({ accepted: true });
  await closePopup(twice.invoke);
  assert.deepEqual(await closePopup(twice.invoke), { kind: 'accepted' });
  assert.equal(twice.calls.length, 2);
});

test('openPopup asks for cad_open_popup with the contract payload and maps existing', async () => {
  const fresh = recordingInvoke({ accepted: true, kind: 'about', popup_label: 'popup-1', existing: false });
  assert.deepEqual(await openPopup(fresh.invoke, 'about'), {
    kind: 'accepted', requestedKind: 'about', popupLabel: 'popup-1', existing: false,
  });
  assert.deepEqual(fresh.calls, [{ command: POPUP_OPEN_COMMAND, args: { kind: 'about' } }]);

  // `existing: true` is a success: the host focuses the open window.
  const existing = recordingInvoke({ accepted: true, kind: 'about', popup_label: 'popup-1', existing: true });
  const result = await openPopup(existing.invoke, 'about');
  assert.equal(result.kind, 'accepted');
  assert.equal(result.existing, true);

  // An unknown kind is refused before IPC, never sent to the host.
  const untouched = recordingInvoke();
  assert.equal((await openPopup(untouched.invoke, 'future')).kind, 'refused');
  assert.equal(untouched.calls.length, 0);

  const failed = await openPopup(async () => { throw new Error('no window'); }, 'about');
  assert.equal(failed.kind, 'failed');
});

test('the popup document starts none of the shell global effects', async () => {
  const source = await readModule('components/PopupRoot.jsx');
  for (const forbidden of [
    'startCadPolling(',
    "'cad_status'",
    'bindBrowserDefaults(',
    'bindProbeInput(',
    'createInputLane(',
    'setCommandCatalog(',
    'probe_input',
    './cad.mjs',
    './defaults.mjs',
    './input.mjs',
    './commands.mjs',
  ]) {
    assert.equal(source.includes(forbidden), false, `PopupRoot must not start ${forbidden}`);
  }
  // Only Escape is intercepted: every other accelerator belongs to the main
  // window and would fight it here.
  assert.ok(source.includes("event.key !== 'Escape'"));
  assert.ok(source.includes("window.addEventListener('keydown', onKeyDown)"));
  assert.ok(source.includes('closePopup(invoke)'));
  assert.ok(source.includes('<AboutDialog view={view} onClose={requestClose} failure={failure} />'));
  assert.ok(source.includes('<UnknownDialog kind={view.requestedKind}'));
  assert.ok(source.includes("role=\"dialog\""));
});

test('App renders PopupRoot instead of the shell, so Shell effects never mount', async () => {
  const app = await readModule('App.jsx');
  assert.ok(app.includes('popupBoot(typeof window'));
  assert.match(app, /return boot \? <PopupRoot boot=\{boot\} \/> : <Shell \/>;/);
  const bootAt = app.indexOf('function App()');
  const shellAt = app.indexOf('function Shell()');
  assert.ok(bootAt !== -1 && shellAt !== -1 && bootAt < shellAt, 'App must branch before Shell is defined/mounted');
  // The shell's polling, shortcuts and guards stay in Shell, untouched.
  assert.ok(app.indexOf('startCadPolling(invoke') > shellAt);
  assert.ok(app.indexOf('bindBrowserDefaults(window, {') > shellAt);
});

test('the File tab opens About through cad_open_popup, reflected by cad_status.popup', async () => {
  const file = RIBBON_TABS.find(tab => tab.id === 'file');
  const utilities = file.panels.find(panel => panel.id === 'utilities');
  const about = utilities.items.find(item => item.id === 'about');
  assert.ok(about, 'the File utilities panel must carry the About entry');
  assert.equal(about.labelKey, RIBBON_LITERAL);
  assert.equal(about.action, 'about');
  assert.equal(about.command, undefined, 'About is a shell action, never a router command');

  const app = await readModule('App.jsx');
  assert.ok(app.includes("else if (action === 'about')"));
  assert.ok(app.includes("openPopup(invoke, 'about')"));
  assert.ok(app.includes('popupOpen,'));
});

test('the about dialog renders the template line, one subtitle and Close — nothing else', async () => {
  const source = await readModule('components/AboutDialog.jsx');
  // The version line comes from `aboutVersionText`, which fills BOTH template
  // placeholders; the component must never call the template without params.
  assert.ok(source.includes('aboutVersionText(view, t)'));
  assert.equal(source.includes('t(POPUP_KEYS.version)'), false, 'the version template must be filled');
  assert.ok(source.includes('{versionLine}'));
  assert.ok(source.includes('t(POPUP_KEYS.title)'));
  assert.ok(source.includes('t(POPUP_KEYS.subtitle)'));
  assert.ok(source.includes('t(POPUP_KEYS.close)'));
  assert.ok(source.includes("t('app.title')"));
  // No hand-built metadata and no feature/marketing list.
  assert.equal(source.includes('view.build'), false);
  assert.equal(source.includes('payload.build'), false);
  assert.equal(/<ul|<ol/.test(source), false);
});

test('popup copy comes only from the catalog registry, with no hardcoded CJK', async () => {
  assert.deepEqual(Object.values(POPUP_KEYS).sort(), [
    'dialog.about.close',
    'dialog.about.subtitle',
    'dialog.about.title',
    'dialog.about.version',
    'file.about',
  ]);
  // `popup.mjs` is the single declaration site; the components address these
  // keys through `POPUP_KEYS` and never repeat a literal.
  const registry = await readModule('popup.mjs');
  for (const key of Object.values(POPUP_KEYS)) {
    assert.equal(registry.includes(`'${key}'`), true, `popup.mjs must declare ${key}`);
  }
  for (const file of ['components/PopupRoot.jsx', 'components/AboutDialog.jsx']) {
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
  // fails at runtime. Keep the two dialog files honest here.
  for (const file of ['components/PopupRoot.jsx', 'components/AboutDialog.jsx']) {
    const source = await readModule(file);
    if (source.includes('useI18n()')) {
      assert.match(source, /import \{[^}]*useI18n[^}]*\} from '\.\.\/context\.mjs'/,
        `${file} calls useI18n but does not import it`);
    }
    assert.equal(source.includes('useShell()'), false, `${file} has no shell to read`);
  }
});

test('the dialog uses one content edge for header, body and footer', async () => {
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

  // Both dialogs read the shared metrics; neither keeps its own copy, so the
  // pixel-review fix cannot drift back out of one of them.
  for (const file of ['components/AboutDialog.jsx', 'components/PopupRoot.jsx']) {
    const source = await readModule(file);
    assert.ok(source.includes("from '../popup-layout.mjs'"), `${file} must use the shared metrics`);
    assert.equal(source.includes('20px 20px 18px'), false, `${file} still carries the old body inset`);
    assert.equal(source.includes("'0 6px 0 12px'"), false, `${file} still carries the old header inset`);
  }
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
  // The card is `--chrome-2`; every text token the dialog renders is measured
  // against it (the pixel review found the 12px header run at 2.65:1).
  const surface = token('chrome-2');
  for (const name of ['text', 'text-dim', 'text-muted']) {
    const ratio = contrast(token(name), surface);
    assert.ok(ratio >= 4.5, `--${name} on --chrome-2 = ${ratio.toFixed(2)}`);
  }
  // The header is a title, never muted: full text token, one step larger than
  // the shared 12px panel-title size so its ink reads at full strength.
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
  // `dialog.about.version` is a template: both placeholders must exist in both
  // locales and both must be consumed by the dialog's one rendering path.
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
