// Isolated compositor experiment. Nothing here opens production native popups.
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { CATALOGS } from '../catalogs.mjs';
import { createTranslator, initialLocale, localeTag } from '../i18n.mjs';
import { bindProbeInput, createInputLane } from '../input.mjs';
import { createProbeDiagnostics, observeCadInput, recordProbeAck, stopControlEvent } from './state.mjs';
import './probe.css';

const controlEvents = {
  onPointerDown: stopControlEvent,
  onPointerMove: stopControlEvent,
  onPointerUp: stopControlEvent,
  onPointerCancel: stopControlEvent,
  onWheel: stopControlEvent,
};

export function DomOnTopProbe() {
  const [locale] = useState(initialLocale);
  const t = useMemo(() => createTranslator(CATALOGS, locale), [locale]);
  const diagnostics = useRef(null);
  if (!diagnostics.current) diagnostics.current = createProbeDiagnostics();
  const [revision, setRevision] = useState(0);
  const [surface, setSurface] = useState(null);
  const d = diagnostics.current;

  useEffect(() => {
    document.documentElement.classList.add('yacr-dom-on-top-probe');
    document.documentElement.lang = localeTag(locale);
    document.title = t('probe.dom_on_top.title');
    d.mounted = true;
    window.__yacr_dom_on_top_probe = d;
    return () => {
      d.mounted = false;
      document.documentElement.classList.remove('yacr-dom-on-top-probe');
      if (window.__yacr_dom_on_top_probe === d) delete window.__yacr_dom_on_top_probe;
    };
  }, [d, locale, t]);

  useEffect(() => {
    if (!surface) return undefined;
    const changed = () => setRevision(value => value + 1);
    const lane = createInputLane(async input => {
      d.ipc.submitted += 1;
      d.ipc.lastKind = input.kind;
      if (input.kind === 'resize') d.ipc.lastResize = { ...input };
      const ack = await invoke('probe_input', { input });
      recordProbeAck(d, ack);
      changed();
    }, error => { d.lastIpcError = String(error); changed(); });
    // The existing binding owns pointer capture, wheel normalization, ordered
    // resize/DPR reporting and real host admission. Never synthesize readiness.
    const binding = bindProbeInput(surface, lane);
    const unobserve = observeCadInput(surface, d, changed);
    return () => { unobserve(); binding.dispose(); lane.dispose(); };
  }, [surface, d]);

  const togglePopup = () => { d.popup = !d.popup; setRevision(value => value + 1); };
  return (
    <main className="shell dom-on-top-probe" data-revision={revision}>
      <header className="app-region probe-chrome" {...controlEvents}>
        <h1>{t('probe.dom_on_top.title')}</h1>
        <p>{t('probe.dom_on_top.instructions')}</p>
        <button type="button" data-probe="popup-toggle" aria-expanded={d.popup} onClick={togglePopup}>
          {t('probe.dom_on_top.popup_toggle')}
        </button>
      </header>
      <section className="probe-workspace">
        <div className="native-input" ref={setSurface} data-probe="cad" aria-label={t('probe.dom_on_top.cad')} />
        <div className="probe-overlay probe-translucent" {...controlEvents}>
          <strong>{t('probe.dom_on_top.translucent')}</strong>
          <p>{t('probe.dom_on_top.translucent_hint')}</p>
        </div>
        <div className="probe-overlay probe-comparison" aria-hidden="true">
          <span>{t('probe.dom_on_top.transparent')}</span>
        </div>
        <button type="button" className="probe-overlay probe-opaque" data-probe="opaque-button" {...controlEvents}
          onClick={event => { stopControlEvent(event); d.counters.button += 1; setRevision(value => value + 1); }}>
          {t('probe.dom_on_top.button', { count: d.counters.button })}
        </button>
      </section>
      {d.popup ? <section className="probe-local-popup" data-probe="popup" role="dialog"
        aria-label={t('probe.dom_on_top.popup_title')} {...controlEvents}>
        <strong>{t('probe.dom_on_top.popup_title')}</strong>
        <p>{t('probe.dom_on_top.popup_hint')}</p>
        <button type="button" onClick={togglePopup}>{t('probe.dom_on_top.close')}</button>
      </section> : null}
      <footer className="app-region probe-readout" {...controlEvents}>
        <div>{t('probe.dom_on_top.counters', d.counters)}</div>
        <div>{t('probe.dom_on_top.ipc', d.ipc)}</div>
        <div data-probe="admission" data-refused={d.ipc.refused > 0}>
          {t('probe.dom_on_top.admission', d.ipc)}
          {d.ipc.refused > 0 ? t('probe.dom_on_top.refusal', {
            reason: d.ipc.lastRefusalReason ?? t('probe.dom_on_top.missing_reason'),
          }) : null}
        </div>
        <div role="status" data-probe="ipc-error" data-error={d.lastIpcError !== null}>
          {d.lastIpcError ? t('probe.dom_on_top.error', { error: d.lastIpcError }) : t('probe.dom_on_top.no_error')}
        </div>
        <small>{t('probe.dom_on_top.limit')}</small>
      </footer>
    </main>
  );
}
