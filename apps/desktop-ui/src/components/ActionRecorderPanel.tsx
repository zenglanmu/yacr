// Manage ▸ Action Recorder: recording state, start/stop and the session's macros
// (ADR-0056 D4).
//
// The surface is a palette window on the desktop band and a bottom sheet on the
// compact band, like the Parameters Manager. It is deliberately small: every
// action is a real command line (`ACTRECORD`, `ACTSTOP`, `ACTMANAGER PLAY|
// DELETE|CONTINUE`) and every row comes from the host's `cad_action_recorder`
// read. There is **no** file picker: the macro destination is the typed path in
// this panel, so a web/wasm carrier's `unsupported-runtime: action-recorder`
// refusal is visible instead of a dialog that cannot exist.
//
// States:
//  * `loading` — the first read is still in flight;
//  * `empty` — no macro in this session and no recording (a real empty state);
//  * `ready` — a recording and/or session macros are shown;
//  * `error` — the read or the last command failed; the host's raw reason is shown.
import React, { useState } from 'react';
import { useI18n } from '../context.ts';
import { Icon } from '../icons.tsx';
import { Button, IconButton } from './primitives.tsx';
import { closeRecorderPanel } from '../recorder/panel.ts';
import { useRecorderStore } from '../recorder/use-recorder-store.ts';

/** The panel's two carriers: the desktop palette window and the compact sheet. */
export type ActionRecorderPanelVariant = 'window' | 'floating';

export interface ActionRecorderPanelProps {
  readonly variant?: ActionRecorderPanelVariant;
}

export function ActionRecorderPanel({ variant = 'window' }: ActionRecorderPanelProps) {
  const { t } = useI18n();
  const { store, state } = useRecorderStore();
  const [destination, setDestination] = useState('');
  const [notice, setNotice] = useState<string | null>(null);
  const floating = variant === 'floating';
  const firstRead = state.phase === 'idle' || state.phase === 'loading';
  const recording = state.answer.recording;
  const playback = state.answer.playback;

  const report = (ack: { kind: string; reason?: string }) => {
    if (ack.kind === 'error') setNotice(ack.reason ?? t('recorder.unknown_error'));
    else if (ack.kind === 'busy') setNotice(t('recorder.busy'));
    else setNotice(null);
  };

  const start = (): void => {
    void store.start(destination).then(report);
  };
  const stop = (): void => {
    void store.stop(destination).then(report);
  };
  const play = (name: string): void => {
    void store.play(name).then(report);
  };
  const remove = (name: string): void => {
    void store.remove(name).then(report);
  };
  const resume = (): void => {
    void store.resume().then(report);
  };

  return (
    <section
      className={['palette', 'params', 'recorder', floating ? 'params--floating' : ''].filter(Boolean).join(' ')}
      aria-label={t('recorder.title')}
      role={floating ? 'dialog' : undefined}
      aria-modal={floating ? true : undefined}
    >
      <header className="palette__head">
        <h2><Icon name="settings" size={15} />{t('recorder.title')}</h2>
        <IconButton icon="close" size="xs" label={t('recorder.close')} onClick={closeRecorderPanel} />
      </header>
      <div className="palette__body params__body" aria-busy={firstRead || state.busy ? true : undefined}>
        <p className="params__note">{t('recorder.note')}</p>

        <p className="params__active" role="status">
          {recording
            ? t('recorder.recording', { name: recording.name, steps: String(recording.steps) })
            : t('recorder.not_recording')}
        </p>

        {/* The typed path is the only file surface: no dialog, no picker. */}
        <label className="params__field">
          <span className="params__label">{t('recorder.destination')}</span>
          <input
            className="params__input"
            type="text"
            value={destination}
            placeholder={t('recorder.destination_placeholder')}
            onChange={event => setDestination(event.currentTarget.value)}
          />
        </label>
        <div className="params__actions">
          <Button size="sm" disabled={state.busy || recording !== null} onClick={start}>
            {t('recorder.start')}
          </Button>
          <Button size="sm" disabled={state.busy || recording === null} onClick={stop}>
            {t('recorder.stop')}
          </Button>
        </div>

        {playback ? (
          <div className="params__row">
            <p className="params__active">
              {t('recorder.paused', {
                name: playback.name,
                step: String(playback.nextStep),
                total: String(playback.total),
              })}
            </p>
            <Button size="sm" disabled={state.busy} onClick={resume}>
              {t('recorder.continue')}
            </Button>
          </div>
        ) : null}

        {firstRead ? <p className="palette__empty">{t('recorder.loading')}</p> : null}

        {state.error ? (
          <p className="palette__error" role="alert">
            {t('recorder.failed', { reason: state.error.reason })}
          </p>
        ) : null}
        {state.answer.lastError === null ? null : (
          <p className="palette__error" role="alert">
            {t('recorder.failed', { reason: state.answer.lastError })}
          </p>
        )}
        {notice === null ? null : (
          <p className="params__notice" role="alert">{notice}</p>
        )}

        <h3 className="palette__section">{t('recorder.macros')}</h3>
        {!firstRead && state.answer.macros.length === 0 ? (
          <p className="palette__empty">{t('recorder.empty')}</p>
        ) : null}
        <ul className="params__list">
          {state.answer.macros.map(macro => (
            <li key={macro.name} className="params__row">
              <div className="params__row-head">
                <span className="params__name">{macro.name}</span>
                <span className="params__seq">{t('recorder.steps', { count: String(macro.steps) })}</span>
              </div>
              <div className="params__meta">
                <span>{macro.path ?? t('recorder.unsaved_path')}</span>
              </div>
              <div className="params__actions">
                <Button size="sm" disabled={state.busy || recording !== null} onClick={() => play(macro.name)}>
                  {t('recorder.play')}
                </Button>
                <Button size="sm" disabled={state.busy} onClick={() => remove(macro.name)}>
                  {t('recorder.delete')}
                </Button>
              </div>
            </li>
          ))}
        </ul>

        {state.answer.lastMacro === null ? null : (
          <p className="params__hint">
            {t('recorder.last_macro', { name: state.answer.lastMacro })}
          </p>
        )}
      </div>
    </section>
  );
}
