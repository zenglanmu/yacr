import React, { useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import zh from '../../../../crates/cad-ui-slint/i18n/zh-CN.json';
import en from '../../../../crates/cad-ui-slint/i18n/en.json';
import { bindProbeInput, createInputLane } from './input.mjs';
import { commandEnabled, handleCommandKey, queueCadCommand, startCadPolling } from './cad.mjs';
import { createFpsSampler } from './fps.mjs';
import './style.css';

const NativeInput = React.memo(function NativeInput({ surfaceRef, label }) {
  return <div ref={surfaceRef} className="native-input" aria-label={label} />;
});

function ReactMarker() {
  const [tick, setTick] = useState(0);
  useEffect(() => {
    const clock = setInterval(() => setTick(value => value + 1), 100);
    return () => clearInterval(clock);
  }, []);
  return <output className="react-marker"><b style={{ transform: `translateX(${tick % 12 * 3}px)` }} />React / {tick}</output>;
}

function App() {
  const [locale, setLocale] = useState('zh-CN');
  const [overlay, setOverlay] = useState(false);
  const [status, setStatus] = useState(null);
  const [error, setError] = useState('');
  const [pollError, setPollError] = useState('');
  const [fps, setFps] = useState(null);
  const [command, setCommand] = useState('');
  const [pending, setPending] = useState(false);
  const [commandResult, setCommandResult] = useState(null);
  const submitting = useRef(false);
  const surface = useRef(null);
  const bridge = useRef(null);
  const t = key => (locale === 'zh-CN' ? zh : en)[key];
  const displayedError = status?.error || error || pollError;
  const fpsText = fps === null || displayedError ? t('render.fps.unavailable') : fps < 10 ? fps.toFixed(1) : String(Math.round(fps));
  const commandError = commandResult?.kind === 'unsupported'
    ? t('command.unsupported').replace('{command}', commandResult.command)
    : commandResult?.kind === 'failed' ? t('command.failed').replace('{error}', commandResult.error) : null;
  const statusText = !status ? t('diagnostics.empty') : status.loading ? t('import.panel')
    : status.error ? t('diagnostics.severity.error') : status.documentLabel ?? t('plot.no_document');
  useEffect(() => {
    document.documentElement.lang = locale;
    document.title = t('app.title');
  }, [locale]);
  useEffect(() => {
    const lane = createInputLane(input => invoke('probe_input', { input }), setError);
    bridge.current = bindProbeInput(surface.current, lane);
    const sampler = createFpsSampler();
    let staleTimer;
    const resetFps = () => { clearTimeout(staleTimer); sampler.reset(); setFps(null); };
    document.addEventListener('visibilitychange', resetFps);
    const stopPolling = startCadPolling(invoke, next => {
      setStatus(next);
      clearTimeout(staleTimer);
      setFps(sampler.sample(next?.frameCount, performance.now(), Boolean(next && !next.error && !document.hidden)));
      if (next && !next.error && !document.hidden) staleTimer = setTimeout(resetFps, 5000);
      if (next) setPollError('');
    }, setPollError);
    return () => {
      stopPolling();
      clearTimeout(staleTimer);
      document.removeEventListener('visibilitychange', resetFps);
      bridge.current.dispose();
      bridge.current = null;
      lane.dispose();
    };
  }, []);
  async function submit(text, fromInput = false) {
    if (submitting.current) return;
    submitting.current = true;
    setPending(true);
    try {
      const result = await queueCadCommand(invoke, text, status, false);
      setCommandResult(result);
      if (result.kind === 'accepted' && fromInput) setCommand('');
    } finally { submitting.current = false; setPending(false); }
  }
  function toggleOverlay(value) {
    bridge.current?.suspend(true);
    bridge.current?.suspend(false);
    setOverlay(value);
  }
  return <main className="shell">
    <NativeInput surfaceRef={surface} label={t('view.panel')} />
    <header className="ribbon">
       <div className="ribbon-top"><strong>{t('app.title').split(' — ')[0]}</strong><span>{t('ribbon.home')}</span><span className="scaffold" title={statusText} aria-live="polite">{statusText}</span>
        <select aria-label={t('host.language_label')} value={locale} onChange={event => setLocale(event.target.value)}><option value="zh-CN">zh-CN</option><option value="en">en</option></select>
      </div>
       <div className="tools"><button disabled={!commandEnabled(status, pending, 'OPEN')} onClick={() => void submit('OPEN')}>{t('file.open')}</button><button disabled={!commandEnabled(status, pending, 'FIT')} onClick={() => void submit('FIT')}>{t('toolbar.fit')}</button><i /><button aria-pressed={overlay} onClick={() => toggleOverlay(!overlay)}>{t('toolbar.diagnostics')}</button>
         <ReactMarker />
      </div>
    {overlay && <section className="diagnostic-panel" aria-label={t('diagnostics.title')}>
      <div className="panel-heading"><h2>{t('diagnostics.title')}</h2><button onClick={() => toggleOverlay(false)}>{t('diagnostics.close')}</button></div>
      {!status && <p>{t('diagnostics.empty')}</p>}
      <dl>{['backend', 'adapter', 'documentLabel', 'entityCount', 'importCount', 'loading', 'canOpen', 'canFit', 'camera'].map(key => <React.Fragment key={key}><dt>{key}</dt><dd>{status?.[key] == null ? '—' : typeof status[key] === 'object' ? JSON.stringify(status[key]) : String(status[key])}</dd></React.Fragment>)}<dt>{t('render.fps')}</dt><dd>{fpsText}</dd><dt>accepted</dt><dd>{String(commandResult?.kind === 'accepted')}</dd></dl>
      {displayedError && <pre role="alert">{displayedError}</pre>}
    </section>}
    </header>
    <footer className="command-bar"><label htmlFor="command">{t('command.title')}</label><input id="command" value={command} onChange={event => setCommand(event.target.value)} disabled={!status || status.loading || pending} onKeyDown={event => handleCommandKey(event, () => submit(command, true))} onFocus={() => bridge.current?.suspend(true)} onBlur={() => bridge.current?.suspend(false)} placeholder="OPEN / FIT" autoComplete="off" spellCheck={false} /><output>{status?.backend ?? '—'} · {status?.adapter ?? '—'} · entityCount / {status?.entityCount ?? '—'} · {t('render.fps')} / {fpsText}</output>{(commandError || displayedError) && <span className="error" role="alert">{commandError || `${t('diagnostics.severity.error')}: ${displayedError}`}</span>}</footer>
  </main>;
}

createRoot(document.getElementById('root')).render(<App />);
