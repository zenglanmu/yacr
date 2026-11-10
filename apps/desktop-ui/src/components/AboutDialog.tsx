// About dialog: the application name, the version the host reported in the
// popup payload, the product line, one honest intro paragraph, the author
// contact, a manual update check and Close. The shared overlay chrome owns the
// title, the close button and Escape; this file renders only the body and footer
// of the About dialog (ADR 0021 D1, WP3).
//
// The update section is a *view* of `update/store.ts`: the store owns the
// session check, the forced check and every failure kind. This dialog only
// translates the current status, never upgrades it (only `up-to-date` says the
// app is current) and never hides a reason the store reported.
//
// Styling: `style.css` is not part of this change, so the dialog reuses the
// existing chrome classes (`.palette__head`, `.mono`, the shared Button
// primitives) and reads its layout metrics from `popup-layout.ts` — one
// content edge, one padding rhythm, and a vertically centred body, shared with
// the unknown-kind dialog so the two cannot drift. The update section is inline
// style data for the same reason.
import React, { useEffect, useState } from 'react';
import type { CSSProperties } from 'react';
import { Icon } from '../icons.tsx';
import { Button } from './primitives.tsx';
import { useI18n } from '../context.ts';
import { POPUP_KEYS, aboutVersionText } from '../popup/popup.ts';
import type { PopupView } from '../popup/popup.ts';
import { DIALOG_BODY, DIALOG_CONTENT, DIALOG_FOOT } from '../popup/popup-layout.ts';
import { writeClipboardText } from '../clipboard.ts';
import { RELEASE_PAGE_URL } from '../update/config.ts';
import {
  forceUpdateCheck,
  openReleasePage,
  useUpdateStatus,
} from '../update/store.ts';

// The same square mark the title bar uses, in token colours only.
const MARK = {
  display: 'grid',
  placeItems: 'center',
  width: 40,
  height: 40,
  flex: '0 0 auto',
  border: '1px solid var(--line-soft)',
  borderRadius: 'var(--radius)',
  background: 'var(--field)',
  color: 'var(--accent-text)',
};

const UPDATE_SECTION: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 6,
  borderTop: '1px solid var(--line)',
};

const UPDATE_LINE: CSSProperties = {
  margin: 0,
  color: 'var(--text-dim)',
  fontSize: 11.5,
  lineHeight: 1.45,
};

/** The store's verbatim reason: small, muted, monospace, never hidden. */
const UPDATE_REASON: CSSProperties = {
  margin: 0,
  color: 'var(--text-muted)',
  fontSize: 10.5,
  lineHeight: 1.45,
  fontFamily: 'var(--font-mono)',
  overflowWrap: 'anywhere',
};

const UPDATE_ACTIONS: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  flexWrap: 'wrap',
  gap: 8,
};

const UPDATE_HINT: CSSProperties = {
  margin: 0,
  color: 'var(--text-dim)',
  fontSize: 11,
  lineHeight: 1.45,
  overflowWrap: 'anywhere',
};

const UPDATE_URL: CSSProperties = {
  display: 'block',
  padding: '4px 6px',
  background: 'var(--field)',
  border: '1px solid var(--line-soft)',
  borderRadius: 'var(--radius)',
  color: 'var(--accent-text)',
  fontFamily: 'var(--font-mono)',
  fontSize: 10.5,
  lineHeight: 1.4,
  overflowWrap: 'anywhere',
  userSelect: 'text',
};

const COPY_STATE: CSSProperties = {
  fontSize: 10.5,
  lineHeight: 1.4,
  color: 'var(--text-muted)',
};

const COPY_STATE_OK: CSSProperties = { ...COPY_STATE, color: 'var(--ok)' };
const COPY_STATE_FAILED: CSSProperties = { ...COPY_STATE, color: 'var(--danger)' };

/** The transient copy result. `idle` is the quiet state. */
type CopyState = 'idle' | 'copied' | 'failed';

/** The About body: the host's payload view plus the overlay's close path. */
interface AboutDialogProps {
  readonly view: Extract<PopupView, { kind: 'about' }>;
  readonly onClose: () => void;
  /** A failed close, shown in the footer; empty when there is none. */
  readonly failure: string;
}

export function AboutDialog({ view, onClose, failure }: AboutDialogProps) {
  const { t } = useI18n();
  const status = useUpdateStatus();
  const [opening, setOpening] = useState(false);
  const [openFailed, setOpenFailed] = useState(false);
  const [copyState, setCopyState] = useState<CopyState>('idle');

  // A new store answer starts a new proposition: the previous open refusal (and
  // its "copied" line) no longer describes what is on screen.
  useEffect(() => {
    setOpenFailed(false);
    setCopyState('idle');
  }, [status.kind]);

  // The brand is the part of `app.title` before the tagline separator, exactly
  // as the title bar derives it.
  const appName = t('app.title').split(' \u2014 ')[0];
  // One line from the catalog template: both `{version}` and `{build}` are
  // filled from the payload (or an explicit `—`), never left as placeholders.
  const versionLine = aboutVersionText(view, t);

  // Only the store decides what may be claimed. `idle` and `skipped` are quiet;
  // in particular "up to date" exists only for `up-to-date`.
  let updateLine: string | null = null;
  if (status.kind === 'checking') updateLine = t(POPUP_KEYS.updateChecking);
  else if (status.kind === 'up-to-date') updateLine = t(POPUP_KEYS.updateUpToDate);
  else if (status.kind === 'available') updateLine = t(POPUP_KEYS.updateAvailable, { version: `v${status.latest}` });
  else if (status.kind === 'unavailable') updateLine = t(POPUP_KEYS.updateUnavailable);
  else if (status.kind === 'unsupported') updateLine = t(POPUP_KEYS.updateUnsupported);

  function handleCheck(): void {
    setOpenFailed(false);
    setCopyState('idle');
    forceUpdateCheck();
  }

  async function handleOpenDownload(): Promise<void> {
    if (opening) return;
    setOpening(true);
    try {
      const result = await openReleasePage();
      if (result.ok) {
        setOpenFailed(false);
      } else {
        setOpenFailed(true);
        setCopyState('idle');
      }
    } catch {
      // A rejecting caller is a failed open, never a success.
      setOpenFailed(true);
      setCopyState('idle');
    } finally {
      setOpening(false);
    }
  }

  async function handleCopy(): Promise<void> {
    try {
      const copied = await writeClipboardText(RELEASE_PAGE_URL);
      setCopyState(copied ? 'copied' : 'failed');
    } catch {
      setCopyState('failed');
    }
  }

  return (
    <>
      <div style={DIALOG_BODY}>
        <div style={DIALOG_CONTENT}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <span style={MARK} aria-hidden="true"><Icon name="app" size={22} /></span>
            <div style={{ minWidth: 0 }}>
              <p style={{ margin: 0, fontSize: 15, fontWeight: 600, letterSpacing: '.2px', color: 'var(--text)' }}>
                {appName}
              </p>
              <p style={{ margin: '3px 0 0', fontSize: 11, color: 'var(--text-muted)', fontVariantNumeric: 'tabular-nums' }}>
                {versionLine}
              </p>
            </div>
          </div>
          {/* The update result sits directly under the version, not at the
              bottom of the body: the About card has a fixed height, so an
              available release (with its action and possible copy fallback)
              must stay visible when the product copy scrolls. */}
          {updateLine !== null ? (
            <div role="status" aria-live="polite" style={UPDATE_SECTION}>
              <p style={UPDATE_LINE}>{updateLine}</p>
              {status.kind === 'unavailable' ? (
                <p style={UPDATE_REASON}>{status.reason}</p>
              ) : null}
              {status.kind === 'available' ? (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                  <div style={UPDATE_ACTIONS}>
                    <Button
                      variant="chip"
                      size="sm"
                      icon="launcher"
                      iconSize={14}
                      disabled={opening}
                      onClick={() => { void handleOpenDownload(); }}
                    >
                      {t(POPUP_KEYS.openDownload)}
                    </Button>
                  </div>
                  {openFailed ? (
                    <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
                      <p style={UPDATE_HINT}>{t('update.open.failed')}</p>
                      <code style={UPDATE_URL}>{RELEASE_PAGE_URL}</code>
                      <div style={UPDATE_ACTIONS}>
                        <Button
                          variant="chip"
                          size="sm"
                          icon="copy"
                          iconSize={14}
                          onClick={() => { void handleCopy(); }}
                        >
                          {t('update.copy')}
                        </Button>
                        {copyState === 'copied' ? (
                          <span style={COPY_STATE_OK}>{t('update.copy.copied')}</span>
                        ) : null}
                        {copyState === 'failed' ? (
                          <span style={COPY_STATE_FAILED}>{t('update.copy.failed')}</span>
                        ) : null}
                      </div>
                    </div>
                  ) : null}
                </div>
              ) : null}
            </div>
          ) : null}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8, maxWidth: '52ch' }}>
            <p style={{ margin: 0, color: 'var(--accent-text)', fontSize: 12, fontWeight: 600, letterSpacing: '.2px' }}>
              {t(POPUP_KEYS.subtitle)}
            </p>
            <p style={{ margin: 0, color: 'var(--text-dim)', fontSize: 12, lineHeight: 1.5 }}>
              {t(POPUP_KEYS.intro)}
            </p>
            <p style={{ margin: 0, color: 'var(--text-muted)', fontSize: 11.5, lineHeight: 1.45 }}>
              {t(POPUP_KEYS.contact)}
            </p>
          </div>
        </div>
      </div>
      <footer style={DIALOG_FOOT}>
        {failure ? (
          <p
            className="palette__error"
            role="alert"
            style={{ margin: '0 auto 0 0', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
          >
            {failure}
          </p>
        ) : null}
        <Button
          variant="chip"
          onClick={handleCheck}
          disabled={status.kind === 'checking'}
        >
          {t(POPUP_KEYS.updateCheck)}
        </Button>
        <Button variant="solid" onClick={onClose}>{t(POPUP_KEYS.close)}</Button>
      </footer>
    </>
  );
}
