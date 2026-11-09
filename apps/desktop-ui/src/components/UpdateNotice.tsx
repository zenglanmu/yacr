// The in-app update banner.
//
// A new release is news, not a blocker: this is a non-modal card docked above
// the status bar, not a dialog and not a host popup. It never takes the
// keyboard, never traps focus and never installs a global key handler, so
// Escape still belongs to the CAD session while the banner is on screen.
//
// Three honest actions:
//
//   * Open download page - asks the host to hand the release URL to the
//     browser. A refusal is shown, never swallowed: the same card then reveals
//     the URL as selectable text plus a Copy link action.
//   * Skip this version - persists the skip through the store.
//   * Later - hides the banner for this session only.
//
// The copy outcome is a real result (clipboard.ts reports `false` when no path
// worked); the card never claims "Copied" it did not get.
import React, { useId, useState } from 'react';
import { Icon } from '../icons.tsx';
import { Button } from './primitives.tsx';
import { useI18n } from '../context.ts';
import { writeClipboardText } from '../clipboard.ts';
import { RELEASE_PAGE_URL } from '../update/config.ts';

/** What opening the release page answered; mirrors `update/store.ts`. */
export type OpenReleaseResult = { readonly ok: true } | { readonly ok: false; readonly reason: string };

/** The transient copy result. `idle` is the quiet state. */
type CopyState = 'idle' | 'copied' | 'failed';

export interface UpdateNoticeProps {
  /** The running version, already normalized (no leading `v`). */
  readonly current: string;
  /** The published version, already normalized (no leading `v`). */
  readonly latest: string;
  /** Ask the host to open the published download page. */
  readonly onOpen: () => Promise<OpenReleaseResult>;
  /** Persist "skip this version" and close the banner. */
  readonly onSkip: () => void;
  /** Hide the banner for this session. */
  readonly onLater: () => void;
}

export function UpdateNotice({ current, latest, onOpen, onSkip, onLater }: UpdateNoticeProps) {
  const { t } = useI18n();
  const titleId = useId();
  const bodyId = useId();
  const [opening, setOpening] = useState(false);
  const [openFailed, setOpenFailed] = useState(false);
  const [copyState, setCopyState] = useState<CopyState>('idle');

  async function handleOpen(): Promise<void> {
    if (opening) return;
    setOpening(true);
    try {
      const result = await onOpen();
      if (result.ok) {
        // The page is in the browser now; the banner stays so Skip/Later are
        // still available, and the stale fallback (if any) is no longer true.
        setOpenFailed(false);
      } else {
        setOpenFailed(true);
        setCopyState('idle');
      }
    } catch {
      // The store's action returns a refusal instead of throwing, but a caller
      // that does reject is still a failed open - not a success.
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
    <aside
      className="update-notice"
      role="status"
      aria-live="polite"
      aria-labelledby={titleId}
      aria-describedby={bodyId}
    >
      <span className="update-notice__mark" aria-hidden="true">
        <Icon name="app" size={18} />
      </span>
      <div className="update-notice__content">
        <p id={titleId} className="update-notice__title">
          {t('update.available.title', { latest: `v${latest}` })}
        </p>
        <p id={bodyId} className="update-notice__body">
          {t('update.available.body', { current: `v${current}` })}
        </p>
        {openFailed ? (
          <div className="update-notice__fallback">
            <p className="update-notice__hint">{t('update.open.failed')}</p>
            <code className="update-notice__url">{RELEASE_PAGE_URL}</code>
            <div className="update-notice__copy">
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
                <span className="update-notice__copy-state is-copied">{t('update.copy.copied')}</span>
              ) : null}
              {copyState === 'failed' ? (
                <span className="update-notice__copy-state is-failed">{t('update.copy.failed')}</span>
              ) : null}
            </div>
          </div>
        ) : null}
      </div>
      <div className="update-notice__actions">
        <Button
          variant="solid"
          size="sm"
          icon="launcher"
          iconSize={14}
          disabled={opening}
          onClick={() => { void handleOpen(); }}
        >
          {t('update.available.open')}
        </Button>
        <Button variant="chip" size="sm" onClick={onSkip}>
          {t('update.available.skip')}
        </Button>
        <Button size="sm" onClick={onLater}>
          {t('update.available.later')}
        </Button>
      </div>
    </aside>
  );
}
