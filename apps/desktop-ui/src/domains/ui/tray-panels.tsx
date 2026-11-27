// Match Photo and Overlays tray bodies, split out of `DomainTray.tsx` for the
// file-size gate. Both are host-read-backed; neither read writes anything.

import { useEffect, useState } from 'react';
import { useI18n, useShell } from '../../context.ts';
import { invoke } from '../../host/invoke.ts';
import { readMatchPhoto, readOverlays } from '../concept/panels.ts';
import type { MatchPhotoAnswer, OverlaysAnswer } from '../concept/panels.ts';

/**
 * The Match Photo panel.
 *
 * Photo matching is not modeled: the host read reports `modeled: false` with a
 * stable reason, and the action dispatches `MATCHPHOTO`, whose refusal names the
 * same missing model. The panel never shows a fake photo list.
 */
export function MatchPhotoBody() {
  const { t } = useI18n();
  const shell = useShell();
  const documentLabel = shell.status?.documentLabel ?? null;
  const [state, setState] = useState<{
    phase: 'ready' | 'loading' | 'error';
    answer: MatchPhotoAnswer | null;
    reason: string;
  }>({ phase: 'ready', answer: null, reason: '' });
  useEffect(() => {
    if (documentLabel === null) {
      setState({ phase: 'ready', answer: null, reason: '' });
      return undefined;
    }
    let live = true;
    setState(previous => ({ phase: 'loading', answer: previous.answer, reason: '' }));
    readMatchPhoto(invoke)
      .then(answer => {
        if (live) setState({ phase: 'ready', answer, reason: '' });
      })
      .catch((error: unknown) => {
        if (!live) return;
        const reason = error instanceof Error ? error.message : String(error);
        setState(previous => ({ phase: 'error', answer: previous.answer, reason }));
      });
    return () => { live = false; };
  }, [documentLabel]);
  if (documentLabel === null) {
    return <p className="concept-panel__hint">{t('concept.panels.no_document')}</p>;
  }
  return (
    <>
      <p className="concept-panel__hint">{t('concept.match_photo.hint')}</p>
      {state.phase === 'error' ? (
        <p className="concept-panel__hint" role="alert">
          {t('concept.match_photo.error', { reason: state.reason })}
        </p>
      ) : null}
      {state.answer !== null && !state.answer.modeled ? (
        <p className="concept-panel__hint" role="status">{state.answer.reason}</p>
      ) : null}
      <button
        type="button"
        className="concept-panel__action"
        onClick={() => shell.runCommand('MATCHPHOTO')}
      >
        {t('concept.match_photo.action')}
      </button>
    </>
  );
}

/**
 * The Overlays panel.
 *
 * The rows are the renderer-owned overlay channels this build really draws, with
 * their current on/off state. There is no user-managed overlay definition model,
 * so the management action dispatches `OVERLAYS`, whose refusal says so.
 */
export function OverlaysBody() {
  const { t } = useI18n();
  const shell = useShell();
  const documentLabel = shell.status?.documentLabel ?? null;
  const [state, setState] = useState<{
    phase: 'ready' | 'loading' | 'error';
    answer: OverlaysAnswer | null;
    reason: string;
  }>({ phase: 'ready', answer: null, reason: '' });
  useEffect(() => {
    if (documentLabel === null) {
      setState({ phase: 'ready', answer: null, reason: '' });
      return undefined;
    }
    let live = true;
    setState(previous => ({ phase: 'loading', answer: previous.answer, reason: '' }));
    readOverlays(invoke)
      .then(answer => {
        if (live) setState({ phase: 'ready', answer, reason: '' });
      })
      .catch((error: unknown) => {
        if (!live) return;
        const reason = error instanceof Error ? error.message : String(error);
        setState(previous => ({ phase: 'error', answer: previous.answer, reason }));
      });
    return () => { live = false; };
  }, [documentLabel]);
  if (documentLabel === null) {
    return <p className="concept-panel__hint">{t('concept.panels.no_document')}</p>;
  }
  const rows = state.answer?.rows ?? [];
  return (
    <>
      <p className="concept-panel__hint">{t('concept.overlays.hint')}</p>
      {state.phase === 'error' ? (
        <p className="concept-panel__hint" role="alert">
          {t('concept.overlays.error', { reason: state.reason })}
        </p>
      ) : null}
      {rows.length === 0 ? (
        <p className="concept-panel__hint">
          {state.phase === 'loading' ? t('concept.panels.loading') : t('concept.overlays.hint')}
        </p>
      ) : (
        <ul className="concept-rows">
          {rows.map(row => (
            <li key={row.key}>
              <span className="concept-rows__label">{t(`concept.overlays.row.${row.key}`)}</span>
              <span className="concept-rows__value">
                {t(row.active ? 'concept.overlays.on' : 'concept.overlays.off')}
              </span>
            </li>
          ))}
        </ul>
      )}
      <button
        type="button"
        className="concept-panel__action"
        onClick={() => shell.runCommand('OVERLAYS')}
      >
        {t('concept.overlays.manage')}
      </button>
    </>
  );
}

