// PopupRoot: the entire document of a host-opened dialog window (ADR 0021 D1).
//
// The host loads this same bundle in a second native window and injects
// `window.__YACR_POPUP__`; `App` then renders this component *instead of* the
// shell. None of the shell's session starts here, deliberately:
//
//   * no `cad_status` polling — the host renders and owns this window, and a
//     second poller would only duplicate the main window's reads;
//   * no global CAD shortcuts — the main window owns clean screen, F2/F3,
//     Ctrl+9 and the tool's Enter/Escape. Here Escape means "close this
//     dialog" and nothing else is intercepted;
//   * no `bindBrowserDefaults` (including the CAD right-click hook) — that
//     binding belongs to the main window; two owners would fight over the same
//     webview commands.
//
// The dialog's only inputs are Escape, its Close buttons and the window manager.
// The text editor adds its own field and Apply, which reach exactly one IPC
// (`cad_set_selected_text`, through `TextEditDialog`) — still no polling and no
// global shortcut.
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { CATALOGS } from '../catalogs.mjs';
import { createTranslator, initialLocale, localeTag, LOCALES } from '../i18n.mjs';
import { I18nContext, useI18n } from '../context.mjs';
import { Icon } from '../icons.jsx';
import { Button, IconButton } from './primitives.jsx';
import { closePopup, POPUP_KEYS, popupView } from '../popup.mjs';
import { POPUP_TEXT_KEYS } from '../popup-textedit.mjs';
import { DIALOG_BODY, DIALOG_CARD, DIALOG_CONTENT, DIALOG_FOOT, DIALOG_HEAD, DIALOG_TITLE } from '../popup-layout.mjs';
import { AboutDialog } from './AboutDialog.jsx';
import { TextEditDialog } from './TextEditDialog.jsx';

function Failure({ text }) {
  if (!text) return null;
  return (
    <p
      className="palette__error"
      role="alert"
      style={{ margin: '0 auto 0 0', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
    >
      {text}
    </p>
  );
}

/**
 * The explicit state for a boot literal whose kind this bundle does not
 * implement: the requested kind and the honest "not available in this build"
 * line, plus Close. It is a labelled window, never a blank one.
 *
 * `status.toggle_unsupported` is the one catalog line that states this fact
 * without inventing copy; the popup layer has no dedicated `dialog.unknown.*`
 * key yet.
 */
function UnknownDialog({ kind, onClose, failure }) {
  const { t } = useI18n();
  return (
    <>
      <header className="palette__head" style={DIALOG_HEAD}>
        <h2 className="mono" style={{ ...DIALOG_TITLE, fontFamily: 'var(--font-mono)' }}>
          <Icon name="alert" size={15} />
          {kind ?? '\u2014'}
        </h2>
        <IconButton icon="close" size="xs" label={t(POPUP_KEYS.close)} onClick={onClose} />
      </header>
      <div style={DIALOG_BODY}>
        <div style={DIALOG_CONTENT}>
          <p style={{ margin: 0, color: 'var(--text-dim)', lineHeight: 1.5, maxWidth: '38ch' }}>
            {t('status.toggle_unsupported', { name: kind ?? '\u2014' })}
          </p>
          <p className="mono" style={{ margin: 0, color: 'var(--text-muted)', fontSize: 11 }}>
            window.__YACR_POPUP__.kind
          </p>
        </div>
      </div>
      <footer style={DIALOG_FOOT}>
        <Failure text={failure} />
        <Button variant="solid" onClick={onClose}>{t(POPUP_KEYS.close)}</Button>
      </footer>
    </>
  );
}

export function PopupRoot({ boot }) {
  // `initialLocale` reads the host-injected `window.__YACR_HOST_LOCALE__` (this
  // popup window is loaded with the same value as the main window), so the
  // dialog's chrome language cannot diverge from the native labels and title.
  const [locale] = useState(initialLocale);
  const [failure, setFailure] = useState('');
  const t = useMemo(() => createTranslator(CATALOGS, locale), [locale]);
  const view = useMemo(() => popupView(boot), [boot]);
  // The window title and the dialog's accessible name are the same line: the
  // kind's catalog title, or the requested kind itself when this bundle does not
  // implement it.
  const dialogTitle = view.kind === 'about'
    ? t(POPUP_KEYS.title)
    : view.kind === 'textEdit'
      ? t(POPUP_TEXT_KEYS.title)
      : String(view.requestedKind ?? '');

  // Every close path funnels here. The host owns the window: an accepted close
  // tears it down, so nothing local is hidden. A failure is reported in place
  // and the buttons stay live, so a retry is always possible — and when the
  // host keeps native window controls, the window-manager close still works.
  // The IPC is idempotent, so Escape then click is safe.
  const requestClose = useCallback(() => {
    void closePopup(invoke).then(result => {
      if (result.kind !== 'accepted') setFailure(result.error);
    });
  }, []);

  useEffect(() => {
    document.documentElement.lang = localeTag(locale);
    document.title = dialogTitle;
  }, [locale, t, view, dialogTitle]);

  // The one key this document owns. Every other accelerator belongs to the main
  // window and is left alone here. An IME composition owns its own keys, so the
  // Escape that cancels a composition never closes the window (and never costs a
  // draft half-way through a CJK sentence).
  useEffect(() => {
    const onKeyDown = event => {
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      if (event.isComposing || event.keyCode === 229) return;
      event.preventDefault();
      requestClose();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [requestClose]);

  const i18n = useMemo(() => ({ t, locale, locales: LOCALES }), [t, locale]);
  const label = dialogTitle || undefined;

  return (
    <I18nContext.Provider value={i18n}>
      <main
        className="shell"
        role="dialog"
        aria-modal={view.modal ? 'true' : undefined}
        aria-label={label}
        style={DIALOG_CARD}
      >
        {view.kind === 'about'
          ? <AboutDialog view={view} onClose={requestClose} failure={failure} />
          : view.kind === 'textEdit'
            ? <TextEditDialog view={view} onClose={requestClose} closeFailure={failure} />
            : <UnknownDialog kind={view.requestedKind} onClose={requestClose} failure={failure} />}
      </main>
    </I18nContext.Provider>
  );
}
