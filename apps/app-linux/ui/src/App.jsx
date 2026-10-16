// App: region composition plus the shell state that every region reads.
//
// This component owns the behaviour contracts: the ordered input lane, CAD
// status polling, the command funnel (`cad_command`) and the keyboard shortcuts.
// Visual work lives in the region components.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import zh from '@i18n/zh-CN.json';
import en from '@i18n/en.json';
import { createTranslator, DEFAULT_LOCALE, LOCALES, localeTag } from './i18n.mjs';
import { bindProbeInput, createInputLane } from './input.mjs';
import { pointerPosition } from './metrics.mjs';
import { queueCadCommand, requestImportCancel, startCadPolling } from './cad.mjs';
import { createFpsSampler } from './fps.mjs';
import { admitCommandText, pushHistory } from './commands.mjs';
import { I18nContext, ShellContext } from './context.mjs';
import { TitleBar } from './components/TitleBar.jsx';
import { Ribbon } from './components/Ribbon.jsx';
import { DocTabs } from './components/DocTabs.jsx';
import { Workspace } from './components/Workspace.jsx';
import { LayoutTabs } from './components/LayoutTabs.jsx';
import { CommandLine } from './components/CommandLine.jsx';
import { StatusBar } from './components/StatusBar.jsx';

const CATALOGS = { 'zh-CN': zh, en };

function initialLocale() {
  const stored = typeof document !== 'undefined' ? document.documentElement.lang : '';
  return LOCALES.includes(stored) ? stored : DEFAULT_LOCALE;
}

export function App() {
  const [locale, setLocale] = useState(initialLocale);
  const [status, setStatus] = useState(null);
  const [pollError, setPollError] = useState('');
  const [error, setError] = useState('');
  const [fps, setFps] = useState(null);
  const [command, setCommand] = useState('');
  const [commandResult, setCommandResult] = useState(null);
  const [history, setHistory] = useState([]);
  const [pending, setPending] = useState(false);
  const [ribbonTab, setRibbonTab] = useState('home');
  const [ribbonCollapsed, setRibbonCollapsed] = useState(false);
  const [cleanScreen, setCleanScreen] = useState(false);
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
  const [commandLineOpen, setCommandLineOpen] = useState(true);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [cursor, setCursor] = useState(null);
  const [surfaceNode, setSurfaceNode] = useState(null);

  const submitting = useRef(false);
  const laneRef = useRef(null);
  const bindingRef = useRef(null);
  const surfaceRef = useCallback(node => setSurfaceNode(node), []);

  const t = useMemo(() => createTranslator(CATALOGS, locale), [locale]);
  const displayError = status?.error || error || pollError || '';
  const commandMessage = useMemo(() => {
    if (!commandResult) return null;
    if (commandResult.kind === 'unsupported') return { kind: 'error', text: t('command.unsupported', { command: commandResult.command }) };
    if (commandResult.kind === 'failed') return { kind: 'error', text: t('command.failed', { error: commandResult.error }) };
    return null;
  }, [commandResult, t]);

  useEffect(() => {
    document.documentElement.lang = localeTag(locale);
    document.title = t('app.title');
  }, [locale, t]);

  // One ordered IPC lane for the whole session: sequence stays monotonic even
  // when the canvas element is remounted.
  useEffect(() => {
    const lane = createInputLane(input => invoke('probe_input', { input }), setError);
    laneRef.current = lane;
    return () => { lane.dispose(); laneRef.current = null; };
  }, []);

  // Bind (and re-bind) the input lane to the CAD partition element. The binding
  // reports the partition rect immediately, and re-reports it on every resize,
  // DPR change and layout shift so the host's surface metrics never go stale.
  useEffect(() => {
    if (!surfaceNode || !laneRef.current) return undefined;
    const binding = bindProbeInput(surfaceNode, laneRef.current);
    bindingRef.current = binding;
    return () => { binding.dispose(); if (bindingRef.current === binding) bindingRef.current = null; };
  }, [surfaceNode]);

  // Local pointer readout for the status-bar coordinate box (chrome only; the
  // CAD surface keeps receiving the real input through the lane).
  useEffect(() => {
    if (!surfaceNode) return undefined;
    const move = event => setCursor(pointerPosition(surfaceNode, event));
    const leave = () => setCursor(null);
    surfaceNode.addEventListener('pointermove', move);
    surfaceNode.addEventListener('pointerleave', leave);
    return () => {
      surfaceNode.removeEventListener('pointermove', move);
      surfaceNode.removeEventListener('pointerleave', leave);
    };
  }, [surfaceNode]);

  useEffect(() => {
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
    };
  }, []);

  // Global shortcuts: Ctrl+0 clean screen, Ctrl+9 command line, F2 history, Esc.
  useEffect(() => {
    const isTyping = target => target instanceof HTMLElement
      && ['INPUT', 'SELECT', 'TEXTAREA'].includes(target.tagName);
    const onKeyDown = event => {
      if (event.defaultPrevented || event.metaKey || event.altKey) return;
      if (event.ctrlKey && event.key === '0') { event.preventDefault(); setCleanScreen(value => !value); return; }
      if (event.ctrlKey && event.key === '9') { event.preventDefault(); setCommandLineOpen(value => !value); return; }
      if (event.key === 'F2') { event.preventDefault(); setHistoryOpen(value => !value); return; }
      if (event.key === 'Escape') {
        setHistoryOpen(false);
        if (isTyping(event.target)) event.target.blur();
        else setDiagnosticsOpen(false);
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  const documentLabel = status?.documentLabel ?? null;
  const documentOpen = documentLabel !== null;
  const loading = Boolean(status?.loading);
  const importProgress = status?.importProgress ?? null;
  const cadReady = Boolean(status) && !loading;

  const submitCommand = useCallback(async (text, options = {}) => {
    if (submitting.current) return;
    submitting.current = true;
    setPending(true);
    try {
      const result = await queueCadCommand(invoke, text, status, false);
      setCommandResult(result);
      if (result.kind === 'accepted') {
        setHistory(entries => pushHistory(entries, result.command));
        if (result.kind === 'accepted' && options.fromInput) setCommand('');
      }
    } finally {
      submitting.current = false;
      setPending(false);
    }
  }, [status]);

  const shell = {
    status,
    pending,
    loading,
    importProgress,
    cadReady,
    documentLabel,
    documentOpen,
    fpsText: fps === null || displayError ? t('render.fps.unavailable') : fps < 10 ? fps.toFixed(1) : String(Math.round(fps)),
    displayedError: displayError,
    errorText: displayError,
    error,
    pollError,
    cursor,
    history,
    command,
    setCommand,
    commandResult,
    commandMessage,
    submitCommand: (text, options) => submitCommand(text, { fromInput: true, ...options }),
    historyOpen,
    setHistoryOpen,
    clearHistory: () => setHistory([]),
    ribbonTab,
    setRibbonTab,
    ribbonCollapsed,
    setRibbonCollapsed,
    cleanScreen,
    setCleanScreen,
    diagnosticsOpen,
    setDiagnosticsOpen,
    commandLineOpen,
    setCommandLineOpen,
    surfaceRef,
    canCommand: id => admitCommandText(id, { status, pending }).kind === 'admitted',
    runCommand: id => { void submitCommand(id); },
    cancelImport: () => { void requestImportCancel(invoke, status); },
    runAction: action => {
      if (action === 'diagnostics') setDiagnosticsOpen(value => !value);
      else if (action === 'cleanScreen') setCleanScreen(value => !value);
      else if (action === 'commandLine') setCommandLineOpen(value => !value);
      else if (action === 'language') setLocale(value => (value === 'zh-CN' ? 'en' : 'zh-CN'));
    },
    // Suspend the CAD pointer lane while chrome (command line) owns the keyboard.
    suspendPointer: value => bindingRef.current?.suspend(value),
  };
  const i18n = useMemo(() => ({ t, locale, setLocale, locales: LOCALES }), [t, locale]);

  return (
    <I18nContext.Provider value={i18n}>
      <ShellContext.Provider value={shell}>
        <main className={['shell', cleanScreen ? 'is-clean' : ''].filter(Boolean).join(' ')}>
          {cleanScreen ? null : <TitleBar />}
          {cleanScreen ? null : <Ribbon />}
          {cleanScreen ? null : <DocTabs />}
          <Workspace />
          {cleanScreen ? null : <LayoutTabs />}
          {commandLineOpen ? <CommandLine /> : null}
          <StatusBar />
        </main>
      </ShellContext.Provider>
    </I18nContext.Provider>
  );
}
