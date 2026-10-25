// PopupOverlay: the one in-DOM modal overlay for every popup kind (ADR 0021
// D1/D2, WP3).
//
// Since WP3 the popup is not a second window: it is an overlay in this same
// document, rendered through a portal into `document.body` so it sits above the
// whole shell. The host keeps the state machine and the modal input gate;
// `cad_status.popup` decides whether this component is mounted at all.
//
// Structure:
//
//   * **persistent chrome** — title and close button, rendered *outside* the
//     body boundary. It exists even when the body cannot render (a malformed
//     payload, a body component that throws), so the close affordance and Esc
//     are always reachable;
//   * **body boundary** — a React error boundary around the kind's body. A
//     throwing body is replaced by an explicit `dialog.popup.body_failed` line
//     instead of blanking the overlay (or the shell behind it).
//
// Escape is owned here and nowhere else while a popup is open: the shell's
// global handler returns early, so a tool's Escape never competes with the
// dialog's close. An IME composition owns its own keys.
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import type { CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { Icon } from '../icons.tsx';
import type { IconName } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { useI18n } from '../context.ts';
import { closePopup, POPUP_KEYS, popupView } from '../popup.ts';
import type { PopupPayload, PopupView } from '../popup.ts';
import type { Translator } from '../i18n.ts';
import { POPUP_TEXT_KEYS } from '../popup-textedit.ts';
import { OPTIONS_KEYS } from '../options.ts';
import {
  DIALOG_BODY,
  DIALOG_CARD,
  DIALOG_CONTENT,
  DIALOG_HEAD,
  DIALOG_TITLE,
  POPUP_CARD_SIZE,
} from '../popup-layout.ts';
import { AboutDialog } from './AboutDialog.tsx';
import { OptionsDialog } from './OptionsDialog.tsx';
import { TextEditDialog } from './TextEditDialog.tsx';

const TYPE_STYLE = Object.freeze({
  margin: 0,
  color: 'var(--text-dim)',
  lineHeight: 1.5,
  maxWidth: '46ch',
});

/** A body that throws is replaced by this explicit line, never by a blank card. */
function BodyFailure({ text }: { readonly text: string }) {
  return (
    <div style={DIALOG_BODY}>
      <div style={DIALOG_CONTENT}>
        <p className="palette__error" role="alert" style={TYPE_STYLE}>
          {text}
        </p>
      </div>
    </div>
  );
}

/** The error boundary around one dialog body (the chrome above it survives). */
interface BodyBoundaryProps {
  readonly fallback: React.ReactNode;
  readonly children?: React.ReactNode;
}

class PopupBodyBoundary extends React.Component<BodyBoundaryProps, { failed: boolean }> {
  constructor(props: BodyBoundaryProps) {
    super(props);
    this.state = { failed: false };
  }

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch(error: unknown) {
    // Technical console line only; the user-visible line is catalog copy.
    console.error('popup body failed', error);
  }

  render() {
    return this.state.failed ? this.props.fallback : this.props.children;
  }
}

/**
 * The explicit state for a kind this bundle does not implement (or a payload
 * that did not match its kind): the requested kind and the honest "not
 * available in this build" line. The persistent chrome still provides Close.
 */
function UnknownDialog({ kind, failure }: { readonly kind: string | null; readonly failure: string }) {
  const { t } = useI18n();
  return (
    <div style={DIALOG_BODY}>
      <div style={DIALOG_CONTENT}>
        <p style={TYPE_STYLE}>
          {t('status.toggle_unsupported', { name: kind ?? '\u2014' })}
        </p>
        {failure ? (
          <p className="palette__error" role="alert" style={TYPE_STYLE}>{failure}</p>
        ) : null}
      </div>
    </div>
  );
}

function titleFor(view: PopupView, t: Translator): string {
  if (view.kind === 'about') return t(POPUP_KEYS.title);
  if (view.kind === 'textEdit') return t(POPUP_TEXT_KEYS.title);
  if (view.kind === 'options') return t(OPTIONS_KEYS.title);
  return String(view.requestedKind ?? '');
}

function iconFor(view: PopupView): IconName {
  if (view.kind === 'about') return 'app';
  if (view.kind === 'textEdit') return 'text';
  if (view.kind === 'options') return 'settings';
  return 'alert';
}

function cardStyle(view: PopupView): CSSProperties {
  const size = POPUP_CARD_SIZE[view.kind] ?? POPUP_CARD_SIZE.about;
  return {
    ...DIALOG_CARD,
    display: 'flex',
    flexDirection: 'column',
    width: `min(${size.width}px, calc(100vw - 32px))`,
    height: `min(${size.height}px, calc(100vh - 32px))`,
    boxShadow: '0 18px 48px rgba(0, 0, 0, .45)',
  };
}

/** The host's dialog block (`cad_status.popup`) as this overlay reads it. */
interface OpenPopupState {
  /** The locked dialog kind (`about`, `textEdit`, `options`). */
  readonly kind: string;
  /** Whether the host holds the modal input gate for it. */
  readonly modal: boolean;
}

/** What the app hands the overlay: the host's dialog block and the fresh payload. */
interface PopupOverlayProps {
  /** `cad_status.popup` (`{kind, label, modal}`), or `null`/absent when closed. */
  readonly popup: OpenPopupState | null | undefined;
  /** The validated payload of the fresh open, or `null` (never fetched here). */
  readonly payload: PopupPayload | null | undefined;
}

/**
 * @param popup  the host's `cad_status.popup` block (`{ kind, label, modal }`)
 * @param payload the validated payload of the fresh open, or `null` (existing
 *   instance / no payload kind). Never fetched from the host here.
 */
export function PopupOverlay({ popup, payload }: PopupOverlayProps) {
  const { t } = useI18n();
  const [failure, setFailure] = useState('');
  const view = useMemo(
    () => popupView(popup?.kind, payload),
    [popup?.kind, payload],
  );
  const dialogTitle = titleFor(view, t);

  // Every close path funnels here. The host owns the gate: an accepted close
  // clears it, and the next `cad_status` unmounts the overlay. A failure is
  // reported in place and the persistent chrome stays live, so a retry is
  // always possible.
  const requestClose = useCallback(() => {
    void closePopup(invoke).then(result => {
      if (result.kind !== 'accepted') setFailure(result.error);
      else setFailure('');
    });
  }, []);

  // The overlay's one accelerator. The shell's global handler returns early
  // while a popup is open, so there is exactly one owner.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      if (event.isComposing || event.keyCode === 229) return;
      event.preventDefault();
      requestClose();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [requestClose]);

  let body: React.ReactNode;
  if (view.kind === 'about') {
    body = <AboutDialog view={view} onClose={requestClose} failure={failure} />;
  } else if (view.kind === 'textEdit') {
    body = <TextEditDialog view={view} onClose={requestClose} closeFailure={failure} />;
  } else if (view.kind === 'options') {
    body = <OptionsDialog onClose={requestClose} closeFailure={failure} />;
  } else {
    body = <UnknownDialog kind={view.requestedKind} failure={failure} />;
  }

  const content = (
    <div className="popup-overlay" data-yacr-popup={view.requestedKind ?? 'unknown'}>
      <main
        className="popup-dialog"
        role="dialog"
        aria-modal={popup?.modal === true ? 'true' : undefined}
        aria-label={dialogTitle || undefined}
        style={cardStyle(view)}
      >
        <header className="palette__head" style={DIALOG_HEAD}>
          <h2 style={DIALOG_TITLE}>
            <Icon name={iconFor(view)} size={15} />
            {dialogTitle}
          </h2>
          <IconButton icon="close" size="xs" label={t(POPUP_KEYS.close)} onClick={requestClose} />
        </header>
        <PopupBodyBoundary
          key={view.requestedKind ?? 'unknown'}
          fallback={<BodyFailure text={t(POPUP_KEYS.bodyFailed)} />}
        >
          {body}
        </PopupBodyBoundary>
      </main>
    </div>
  );

  return typeof document === 'undefined' ? content : createPortal(content, document.body);
}
