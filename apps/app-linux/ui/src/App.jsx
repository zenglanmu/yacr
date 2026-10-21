// App: region composition plus the shell state that every region reads.
//
// This component owns the behaviour contracts: the ordered input lane, CAD
// status polling, the command funnel (`cad_command`) and the keyboard shortcuts.
// Visual work lives in the region components.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { CATALOGS } from './catalogs.mjs';
import { createTranslator, initialLocale, LOCALE_STORAGE_KEY, LOCALES, localeTag, storedLocale, storeLocale } from './i18n.mjs';
import { bindProbeInput, createInputLane, createSuspendGate } from './input.mjs';
import { pointerPosition } from './metrics.mjs';
import { queueCadCommand, reconcileToolParameterPending, requestImportCancel, setToolParameterFlag, setToolParameterNumber, startCadPolling, stepToolParameterPending } from './cad.mjs';
import { setToolText } from './text.mjs';
import { setSelectedText } from './textedit.mjs';
import { submitStandardView } from './view3d.mjs';
import { createFpsSampler } from './fps.mjs';
import { bindBrowserDefaults } from './defaults.mjs';
import { admitCommandText, commandCatalog, pushHistory, setCommandCatalog, toolCommandForKey, unavailableReasonKey } from './commands.mjs';
import { I18nContext, ShellContext } from './context.mjs';
import { TitleBar } from './components/TitleBar.jsx';
import { Ribbon } from './components/Ribbon.jsx';
import { DocTabs } from './components/DocTabs.jsx';
import { Workspace } from './components/Workspace.jsx';
import { LayoutTabs } from './components/LayoutTabs.jsx';
import { CommandLine } from './components/CommandLine.jsx';
import { StatusBar } from './components/StatusBar.jsx';
import { PopupRoot } from './components/PopupRoot.jsx';
import { openPopup, popupBoot } from './popup.mjs';
import { isOptionsCommand } from './options.mjs';
import { isPropertiesCommand, setSelectedProperty } from './properties.mjs';
import { textEditOpenError } from './popup-textedit.mjs';

/**
 * Boot switch between the two documents this bundle can be.
 *
 * The host opens dialogs as separate native windows loading the same bundle and
 * injects `window.__YACR_POPUP__` before page load (ADR 0021 D1). The main
 * window never sets it, so `popupBoot` returning `null` is the normal shell.
 *
 * The literal is fixed for the document's lifetime, so it is read once. The
 * branch matters: `Shell` owns every shell effect (status polling, shortcuts,
 * the webview-default guards, the input lane). Returning `PopupRoot` instead of
 * mounting `Shell` is what guarantees none of them start in a dialog window.
 */
export function App() {
  const boot = useMemo(() => popupBoot(typeof window === 'undefined' ? null : window), []);
  return boot ? <PopupRoot boot={boot} /> : <Shell />;
}

/** The main window: the full chrome plus the one CAD session it drives. */
function Shell() {
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
  const [propertiesOpen, setPropertiesOpen] = useState(false);
  const [commandLineOpen, setCommandLineOpen] = useState(true);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [cursor, setCursor] = useState(null);
  const [surfaceNode, setSurfaceNode] = useState(null);
  // The active-tool numeric stepper's bounded pending delta (`{key, base, delta}
  // | null`): the steps the host has queued but not yet echoed. It composes rapid
  // clicks without ever touching the displayed number.
  const [toolParameterPending, setToolParameterPending] = useState(null);
  // The host command vocabulary (names the Rust router accepts). Empty until
  // `cad_commands` answers; the state exists so the chrome re-renders once it does.
  const [catalogNames, setCatalogNames] = useState(commandCatalog);

  // The host's live dialog (`cad_status.popup`, ADR 0021 D1/D2): non-null while
  // one exists. The host applies the modal input gate; the chrome mirrors it as
  // one of the pointer lane's suspension reasons plus the pressed About button.
  const popupOpen = status?.popup != null;

  const submitting = useRef(false);
  // The text-step submit has its own in-flight guard: it must not block a
  // command (or be blocked by one), and a second Enter while the first is still
  // queued must not send the same text twice.
  const submittingText = useRef(false);
  // The inspector's text edit has its own guard for the same reason: the field
  // must stay focused and editable while one edit is queued.
  const submittingSelectedText = useRef(false);
  const laneRef = useRef(null);
  const bindingRef = useRef(null);
  // The CAD pointer lane has two *independent* suspension reasons (N5): the
  // command line owns the keyboard, or a host dialog is open. The lane is
  // suspended while either holds, and each owner may only clear its own reason
  // — the command line's unconditional `onBlur` (any outside press triggers it)
  // must never resume the lane underneath a modal popup.
  const suspendGate = useRef(null);
  if (!suspendGate.current) {
    suspendGate.current = createSuspendGate(value => bindingRef.current?.suspend(value));
  }
  // Latest host status for the CAD shortcut-menu hook, so the webview-default
  // guards are installed once and never re-bound by a status poll.
  const statusRef = useRef(null);
  const surfaceRef = useCallback(node => setSurfaceNode(node), []);

  const t = useMemo(() => createTranslator(CATALOGS, locale), [locale]);
  const displayError = status?.error || error || pollError || '';
  const commandMessage = useMemo(() => {
    if (!commandResult) return null;
    if (commandResult.kind === 'unsupported') return { kind: 'error', text: t(unavailableReasonKey(commandResult.command), { command: commandResult.command }) };
    if (commandResult.kind === 'failed') return { kind: 'error', text: t('command.failed', { error: commandResult.error }) };
    return null;
  }, [commandResult, t]);

  useEffect(() => {
    document.documentElement.lang = localeTag(locale);
    document.title = t('app.title');
    // Behaviour change: the chosen language is persisted (same origin as a
    // popup window) and now survives a restart. A blocked storage is a no-op.
    storeLocale(locale);
  }, [locale, t]);

  // Live language propagation from the Options dialog (a separate window on the
  // same origin). Two independent carriers, because neither alone is reliable in
  // a WebView: a `storage` event from the other document, and a focus-time
  // re-read for the case where that event is dropped. Only a supported stored
  // value is adopted — an absent entry never overrides the host-resolved locale.
  //
  // NOT VERIFIED in a running GUI: this path was exercised by unit tests and a
  // source scan only, never with two live windows on screen.
  useEffect(() => {
    const readStored = () => {
      const next = storedLocale();
      if (next) setLocale(current => (next === current ? current : next));
    };
    const onStorage = event => {
      if (event.key === LOCALE_STORAGE_KEY || event.key === null) readStored();
    };
    window.addEventListener('storage', onStorage);
    window.addEventListener('focus', readStored);
    return () => {
      window.removeEventListener('storage', onStorage);
      window.removeEventListener('focus', readStored);
    };
  }, []);

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
    // A fresh binding starts accepting: re-assert the reasons that still hold
    // (a remount must not silently resume while text focus or a popup does).
    suspendGate.current.sync();
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

  // Load the host's command vocabulary exactly once. It is the admission source
  // of truth (`cad_commands` -> `cad_ui_core::session::admissible_names`), so a
  // failed load keeps the catalog empty — every command is then explicitly
  // unsupported — and surfaces the failure instead of pretending success.
  useEffect(() => {
    let cancelled = false;
    invoke('cad_commands').then(names => {
      if (cancelled) return;
      setCatalogNames(setCommandCatalog(names));
    }).catch(reason => {
      if (cancelled) return;
      setCatalogNames(setCommandCatalog([]));
      setError(`cad_commands: ${String(reason)}`);
    });
    return () => { cancelled = true; };
  }, []);

  // Reconcile the numeric stepper's pending delta against every fresh status: it
  // resets the moment the host has caught up (the value changed), the tool
  // switches/cancels, the parameter leaves the active tool's map, or the value
  // becomes absent/`null`/invalid. An unchanged poll returns the same reference,
  // so `setState` bails out without a re-render.
  useEffect(() => {
    setToolParameterPending(prev => reconcileToolParameterPending(prev, status));
  }, [status]);

  // Mirror an open host dialog as the lane's second suspension reason (N5). The
  // gate dedupes, so an unchanged poll re-applies nothing; closing the dialog
  // clears only this reason (text focus, if held, keeps the lane suspended).
  useEffect(() => {
    suspendGate.current.set('popupOpen', popupOpen);
  }, [popupOpen]);

  const documentLabel = status?.documentLabel ?? null;
  const documentOpen = documentLabel !== null;
  const loading = Boolean(status?.loading);
  const importProgress = status?.importProgress ?? null;
  const cadReady = Boolean(status) && !loading;
  const toolActive = Boolean(status?.tool?.key);

  // The one open path for the Options dialog: a host-owned window like About.
  // `existing: true` means it is already open and the host focused it — the
  // success it is, never a duplicate window.
  const openOptions = useCallback(() => {
    void openPopup(invoke, 'options').then(result => {
      if (result.kind !== 'accepted') setError(result.error);
    });
  }, []);

  const submitCommand = useCallback(async (text, options = {}) => {
    // `OPTIONS` is a local chrome selector, not a router command: open the
    // dialog and never forward the text. Intercepted before the in-flight guard
    // so a second Enter still reaches the host (which focuses the open window).
    if (isOptionsCommand(text)) {
      setCommandResult(null);
      if (options.fromInput) setCommand('');
      openOptions();
      return;
    }
    // `PROPERTIES` is the docked palette's local selector, exactly like
    // `OPTIONS`: opening it never reaches the router, so it is intercepted
    // before the in-flight guard (a second Enter just re-opens/focuses it).
    if (isPropertiesCommand(text)) {
      setCommandResult(null);
      if (options.fromInput) setCommand('');
      setPropertiesOpen(true);
      return;
    }
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
  }, [openOptions, status]);

  // The terminal text step (ADR 0019 A.1): one `cad_set_tool_text`, never a
  // `cad_command`. The result is returned to the command line so the draft can
  // stay visible with its refusal message. `pending` is deliberately not set:
  // it disables the field, and losing focus mid-edit would break the two-step
  // Escape and the user's text. A re-entrant submit reports `busy` — an explicit
  // non-answer, never a fabricated acceptance.
  const submitToolText = useCallback(async text => {
    if (submittingText.current) return { kind: 'busy' };
    submittingText.current = true;
    try {
      return await setToolText(invoke, text);
    } finally {
      submittingText.current = false;
    }
  }, []);

  // The inspector's text edit (ADR 0020 addendum B): one
  // `cad_set_selected_text`, which targets whatever single text entity the host
  // has selected — the payload carries no id, and this app never changes the
  // selection to make an edit fit. Same admission rules as the text step, and no
  // `pending`: a disabled field would drop focus mid-edit.
  const applySelectedText = useCallback(async text => {
    if (submittingSelectedText.current) return { kind: 'busy' };
    submittingSelectedText.current = true;
    try {
      return await setSelectedText(invoke, text);
    } finally {
      submittingSelectedText.current = false;
    }
  }, []);

  // Global shortcuts: Ctrl+0 clean screen, Ctrl+1 properties palette, Ctrl+9
  // command line, F2 history, Esc.
  // While a CAD tool is active (and the keyboard is not in a field) Enter/Escape
  // are the tool's confirm/cancel instead.
  useEffect(() => {
    const isTyping = target => target instanceof HTMLElement
      && ['INPUT', 'SELECT', 'TEXTAREA'].includes(target.tagName);
    const onKeyDown = event => {
      // Ctrl+0 is both the app's clean-screen toggle and WebKit's zoom reset.
      // Handle it before the `defaultPrevented` check: the webview-default guard
      // also prevents the browser action, and listener order between the two
      // must not decide whether clean screen works. The guard never swallows an
      // ordinary key, so this does not widen what the app intercepts.
      if (event.ctrlKey && !event.metaKey && !event.altKey && event.key === '0') {
        event.preventDefault();
        setCleanScreen(value => !value);
        return;
      }
      if (event.defaultPrevented || event.metaKey || event.altKey) return;
      // An active tool owns Enter/Escape: Enter confirms, Escape cancels, and
      // that Escape does not also close a panel. With no tool active the
      // branches below are exactly as before.
      const toolCommand = toolCommandForKey(event.key, {
        toolActive,
        typing: isTyping(event.target),
        composing: event.isComposing,
        repeat: event.repeat,
      });
      if (toolCommand) {
        event.preventDefault();
        void submitCommand(toolCommand);
        return;
      }
      if (event.ctrlKey && event.key === '9') { event.preventDefault(); setCommandLineOpen(value => !value); return; }
      // Ctrl+1 is AutoCAD's properties-palette toggle. It sits with Ctrl+9,
      // after the `defaultPrevented`/modifier guard, so a webview default that
      // already claimed the chord wins and no plain key is hijacked.
      if (event.ctrlKey && event.key === '1') { event.preventDefault(); setPropertiesOpen(value => !value); return; }
      if (event.key === 'F2') { event.preventDefault(); setHistoryOpen(value => !value); return; }
      // F3 flips the object-snap master switch (ADR 0014 A.4) through the same
      // command funnel every other affordance uses; `preventDefault` keeps the
      // webview from consuming F3 itself. Like F2 it is a global accelerator that
      // a focused text field does not swallow, and it never collides with a tool's
      // Enter/Escape (handled above only for those two keys).
      if (event.key === 'F3') { event.preventDefault(); void submitCommand('OSNAP'); return; }
      if (event.key === 'Escape') {
        setHistoryOpen(false);
        if (isTyping(event.target)) event.target.blur();
        else setDiagnosticsOpen(false);
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [submitCommand, toolActive]);

  // Feed the reserved CAD shortcut-menu hook the selection/tool slices the
  // chrome already has. It is a ref, not a dependency, so polling never
  // re-installs the guards below.
  useEffect(() => { statusRef.current = status; }, [status]);

  // Keep WebKitGTK's own commands out of the session. The native CAD surface is
  // input-transparent, so right-clicks and accelerators reach this DOM; the
  // webview menu's Reload would remount the UI while Rust and the native surface
  // keep running, and page zoom would desync the rect the host composites at.
  // Declared after the shortcut handler above (both listen on `window` in the
  // bubble phase). Ctrl+0 is the only overlap, and the shortcut handler owns it
  // before its `defaultPrevented` check, so clean screen keeps working whichever
  // listener runs first while the browser zoom reset is still suppressed. The
  // guard suppresses every browser command even in a focused text field; the
  // native menu stays in text fields, and a CAD right-click calls the one inert
  // hook.
  useEffect(() => {
    const guard = bindBrowserDefaults(window, {
      context: () => {
        const current = statusRef.current;
        return { selection: current?.selection ?? null, tool: current?.tool ?? null };
      },
    });
    return () => guard.dispose();
  }, []);

  // Admission recomputes when the loaded host vocabulary changes, so the chrome
  // enables exactly the tools the router really accepts.
  const canCommand = useCallback(
    id => admitCommandText(id, { status, pending }).kind === 'admitted',
    [status, pending, catalogNames],
  );

  const shell = {
    status,
    pending,
    loading,
    importProgress,
    cadReady,
    documentLabel,
    documentOpen,
    // True while the host reports a live dialog (`cad_status.popup`). The main
    // window keeps working — the host refuses CAD input itself — but the About
    // affordance reads as pressed, and clicking it again asks the host to focus
    // the open window rather than create a duplicate.
    popupOpen,
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
    // The text step of a text-carrying tool; resolves with the admission result
    // (`accepted` / `refused` / `failed` / `busy`) so the command line can keep
    // the draft and show the honest outcome.
    submitToolText,
    // The inspector's text edit; same result shape as `submitToolText`.
    applySelectedText,
    // One property write for the current selection. `{kind:'accepted'}` is
    // queue admission; the host's next `selection.rows` (or a `property.*`
    // error) is the real outcome, which the palette reconciles itself. The
    // payload is always two strings and never carries an entity id.
    applySelectedProperty: (key, value) => setSelectedProperty(invoke, key, value),
    // The slow path of the same edit: a host-opened dialog that shows the raw
    // string. The host derives the payload from the current single selection and
    // refuses a non-text selection with `popup.no_text_selection` **without
    // opening a window**, so that refusal is translated into the error banner
    // here rather than left as a machine name. `existing: true` (the dialog is
    // already open) is a success: the host focuses that window.
    openTextEditPopup: () => {
      void openPopup(invoke, 'textEdit').then(result => {
        const failure = textEditOpenError(result, t);
        if (failure) setError(failure);
      });
    },
    // The Options dialog (its own host window). One instance: a second request
    // focuses the open window, which the host reports as `existing: true`.
    openOptionsPopup: openOptions,
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
    propertiesOpen,
    setPropertiesOpen,
    commandLineOpen,
    setCommandLineOpen,
    surfaceRef,
    canCommand,
    runCommand: id => { void submitCommand(id); },
    // A standard view carries a payload, so it is one `cad_standard_view {view}`
    // — never a `cad_command`. The host answers admission (queued), not
    // completion: the real camera transition, and any refusal of it, arrives in
    // `cad_status`. A request that was not admitted is translated into the error
    // banner here, so a refused view is never silent.
    runStandardView: view => {
      void submitStandardView({ invoke, view, t, report: setError });
    },
    cancelImport: () => { void requestImportCancel(invoke, status); },
    // The active-tool numeric stepper's bounded pending delta (see the reconcile
    // effect above). The view reads it; the stepper's action updates it.
    toolParameterPending,
    // One numeric parameter step. The pending delta is folded in first so the
    // *next* step composes, then the one IPC is queued. The displayed number is
    // never touched here: it still comes from the next real `cad_status`. A
    // refusal drops the pending delta and reaches the error banner — no fake
    // success and no invented value.
    stepToolParameter: (key, target) => {
      setToolParameterPending(prev => stepToolParameterPending(prev, status, key, target));
      void setToolParameterNumber(invoke, key, target).then(result => {
        if (result.kind !== 'accepted') {
          setToolParameterPending(null);
          setError(t('status.tool_parameter.failed', { error: result.error }));
        }
      });
    },
    // One flag parameter toggle. There is no pending delta for a flag (toggling
    // twice is a no-op, not a composed target), so only the failure reaches the
    // banner — the same honest surface as a refused numeric step.
    toggleToolParameter: (key, next) => {
      void setToolParameterFlag(invoke, key, next).then(result => {
        if (result.kind !== 'accepted') {
          setError(t('status.tool_parameter.failed', { error: result.error }));
        }
      });
    },
    runAction: action => {
      if (action === 'diagnostics') setDiagnosticsOpen(value => !value);
      else if (action === 'cleanScreen') setCleanScreen(value => !value);
      else if (action === 'commandLine') setCommandLineOpen(value => !value);
      else if (action === 'properties') setPropertiesOpen(true);
      else if (action === 'language') setLocale(value => (value === 'zh-CN' ? 'en' : 'zh-CN'));
      // Dialogs are host windows (ADR 0021 D1), never DOM overlays: the host
      // places them beside the native CAD surface. `existing: true` means the
      // dialog is already open and the host focuses that window, so there is
      // nothing to do here — it is not a failure and never a second window.
      else if (action === 'about') {
        void openPopup(invoke, 'about').then(result => {
          if (result.kind !== 'accepted') setError(result.error);
        });
      }
      // The Options dialog is the other host-owned window. Opening it twice is a
      // success: the host focuses the single instance instead of building one.
      else if (action === 'options') {
        openOptions();
      }
    },
    // Suspend the CAD pointer lane while chrome (command line) owns the keyboard.
    // This is the **textFocus** reason only: a blur clears it and leaves a
    // popup's suspension intact (N5).
    suspendPointer: value => suspendGate.current.set('textFocus', value),
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
