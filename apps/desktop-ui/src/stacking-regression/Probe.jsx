// Opt-in DOM-on-top stacking regression fixture. DOM-on-top is the production
// arrangement; this renders an opaque button, a translucent panel and a popup
// across the chrome/CAD edge so a real compositor capture can regress it.
// Nothing here opens production native popups.
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { CATALOGS } from '../catalogs.mjs';
import { createTranslator, initialLocale, localeTag } from '../i18n.mjs';
import { bindProbeInput, createInputLane } from '../input.mjs';
import { createStackingDiagnostics, observeCadInput, recordProbeAck, stopControlEvent } from './state.mjs';
import './probe.css';

const controlEvents = {
  onPointerDown: stopControlEvent,
  onPointerMove: stopControlEvent,
  onPointerUp: stopControlEvent,
  onPointerCancel: stopControlEvent,
  onWheel: stopControlEvent,
};

// Fixture-only capture automation. This is not a production behavior: the whole
// fixture is opt-in via the host init-script flag. One scheduled open flips the
// same `popup` state as the toggle button, so a compositor capture can judge the
// popup crossing the chrome/CAD edge without physical input or AT-SPI. The
// evidence collector samples `initial` before this delay and `popup-open` after
// it; the existing YACR_PROBE_EVIDENCE driver supplies the synthetic pointer /
// wheel input used by the `after-synthetic-input` frames.
const AUTO_POPUP_DELAY_MS = 9000;

// Fixture-only marker-motion hook. These are click-free synthetic pointer moves
// on the real `.native-input` binding: they reach the host's MoveCursor path,
// so successive compositor frames show the CAD marker at different x positions.
// They are not physical input and never touch the popup state. Delays are
// chosen to fall before the collector's popup-open (+12s) and after-input
// (+15s) samples.
const MARKER_MOTION_DELAYS_MS = [8000, 11000, 14000, 17000];
const MARKER_MOTION_FRACTIONS = [0.25, 0.5, 0.75, 0.9];

export function StackingRegression() {
  const [locale] = useState(initialLocale);
  const t = useMemo(() => createTranslator(CATALOGS, locale), [locale]);
  const diagnostics = useRef(null);
  if (!diagnostics.current) diagnostics.current = createStackingDiagnostics();
  const [revision, setRevision] = useState(0);
  const [surface, setSurface] = useState(null);
  const d = diagnostics.current;

  useEffect(() => {
    document.documentElement.classList.add('yacr-stacking-regression');
    document.documentElement.lang = localeTag(locale);
    document.title = t('probe.dom_on_top.title');
    d.mounted = true;
    window.__yacr_stacking_regression = d;
    return () => {
      d.mounted = false;
      document.documentElement.classList.remove('yacr-stacking-regression');
      if (window.__yacr_stacking_regression === d) delete window.__yacr_stacking_regression;
    };
  }, [d, locale, t]);

  useEffect(() => {
    const timer = setTimeout(() => {
      if (!d.popup) {
        d.popup = true;
        setRevision(value => value + 1);
      }
    }, AUTO_POPUP_DELAY_MS);
    return () => clearTimeout(timer);
  }, [d]);

  useEffect(() => {
    if (!surface) return undefined;
    const timers = MARKER_MOTION_DELAYS_MS.map((delay, index) => setTimeout(() => {
      const rect = surface.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) return;
      surface.dispatchEvent(new PointerEvent('pointermove', {
        pointerId: 91,
        pointerType: 'mouse',
        isPrimary: true,
        buttons: 0,
        clientX: rect.left + rect.width * MARKER_MOTION_FRACTIONS[index],
        clientY: rect.top + rect.height * 0.5,
        bubbles: true,
        cancelable: true,
      }));
    }, delay));
    return () => timers.forEach(clearTimeout);
  }, [surface]);

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
    <main className="shell stacking-regression" data-revision={revision}>
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
