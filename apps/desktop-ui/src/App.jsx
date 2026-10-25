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
import { rectFromElement } from './metrics.mjs';
import { CANVAS_STEP_MODULE, bindCanvasNavigation, canvasModuleUrl, createCanvasTransport, loadCanvasModule, watchCanvasRect } from './canvas-host.mjs';
import { canvasSessionFromShell, canvasSessionPayload, createCanvasSessionReporter, SURFACE_MODE_DOM_CANVAS, surfaceModeFromShell } from './surface.mjs';
import { queueCadCommand, reconcileToolParameterPending, requestImportCancel, setToolParameterFlag, setToolParameterNumber, startCadPolling, stepToolParameterPending, draftingAcceleratorCommand } from './cad.mjs';
import { setToolText } from './text.mjs';
import { setSelectedText } from './textedit.mjs';
import { submitStandardView } from './view3d.mjs';
import { createFpsSampler } from './fps.mjs';
import { bindBrowserDefaults, cadContextMenuRequest } from './defaults.mjs';
import { admitCommandText, commandAliases, commandCatalog, pushHistory, setCommandAliases, setCommandCatalog, toolCommandForKey, unavailableReasonKey } from './commands.mjs';
import { I18nContext, ShellContext } from './context.mjs';
import { TitleBar } from './components/TitleBar.jsx';
import { Ribbon } from './components/Ribbon.jsx';
import { DocTabs } from './components/DocTabs.jsx';
import { Workspace } from './components/Workspace.jsx';
import { LayoutTabs } from './components/LayoutTabs.jsx';
import { CommandLine } from './components/CommandLine.jsx';
import { StatusBar } from './components/StatusBar.jsx';
import { PopupOverlay } from './components/PopupOverlay.jsx';
import { ContextMenu } from './components/ContextMenu.jsx';
import { openPopup } from './popup.mjs';
import { isOptionsCommand } from './options.mjs';
import { isLayersCommand } from './layers.mjs';
import { isPropertiesCommand, setSelectedProperty } from './properties.mjs';
import { textEditOpenError } from './popup-textedit.mjs';

/**
 * The single UI document.
 *
 * There is no second document and no boot switch: since WP3 every popup is an
 * in-DOM overlay rendered by [`Shell`] over this same tree. `Shell` owns the
 * shell effects (status polling, shortcuts, the webview-default guards, the
 * input lane) and the overlay; the host owns the modal gate.
 */
export function App() {
  return <Shell />;
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
  // The layers palette. Closed on the start page (the dock keeps the start panel),
  // opened by `LAYER` / the ribbon / the toggle. `layersFocus` is bumped by every
  // `LAYER` submission so an already-open palette still takes the keyboard and
  // re-reads the host — a no-op otherwise.
  const [layersOpen, setLayersOpen] = useState(false);
  const [layersFocus, setLayersFocus] = useState(0);
  const [commandLineOpen, setCommandLineOpen] = useState(true);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [surfaceNode, setSurfaceNode] = useState(null);
  // The CAD shortcut menu: `{ point, rect, menu }` while it is open, `null`
  // otherwise. `point` is the press in viewport CSS px (the hook reports it
  // relative to the partition, so the app converts it with `rect`), `menu` is the
  // pure model `cadContextMenuRequest` builds. One menu, one overlay — never a
  // second window.
  const [contextMenu, setContextMenu] = useState(null);
  // The active-tool numeric stepper's bounded pending delta (`{key, base, delta}
  // | null`): the steps the host has queued but not yet echoed. It composes rapid
  // clicks without ever touching the displayed number.
  const [toolParameterPending, setToolParameterPending] = useState(null);
  // The host command vocabulary (names the Rust router accepts). Empty until
  // `cad_commands` answers; the state exists so the chrome re-renders once it does.
  const [catalogNames, setCatalogNames] = useState(commandCatalog);
  // The host's alias table (`cad_command_aliases`), held the same way: it is
  // suggestion metadata only, so a failed load leaves it empty ("no aliases
  // known") and never blocks the command line.
  const [aliasPairs, setAliasPairs] = useState(commandAliases);

  // The host's live dialog (`cad_status.popup`, ADR 0021 D1/D2): non-null while
  // one exists. The host applies the modal input gate; the chrome mirrors it as
  // one of the pointer lane's suspension reasons plus the pressed About button.
  const popupOpen = status?.popup != null;
  // The host-reported pixel carrier (ADR 0024 D1). One data flag, read by the one
  // helper that owns the decision; everything else in this component branches on
  // the result, never on a platform hint. An older host that reports nothing is
  // `native-below`: the mode every shipped host has, with today's exact behaviour.
  const surfaceMode = surfaceModeFromShell({ status });
  // The `dom-canvas` session's own report of the drawing it carries (ADR 0024
  // D1/D2). The native shell deliberately holds no document state (iron rule 8),
  // so the relayed `status.canvasSession` is the only honest identity: it fills the
  // document slot the host fills on a `native-below` carrier, and when it is
  // absent, unreadable or label-less the chrome keeps the start state it has today.
  const canvasSession = useMemo(() => canvasSessionFromShell({ status }), [status]);
  // The docked palettes project host-side document state (selection rows, import
  // identity, the host's own diagnostics). A `dom-canvas` shell holds none of that
  // by design, so the palettes are not offered at all instead of shown empty. One
  // carrier flag — never a platform check — and an absent flag means "as today".
  const palettesAvailable = surfaceMode !== SURFACE_MODE_DOM_CANVAS;
  // The validated payload of the **fresh** open (`cad_open_popup`'s additive
  // field). It is local UI state on purpose: the wire only carries the content
  // once, so an `existing: true` answer can never overwrite a rendered dialog.
  const [popupPayload, setPopupPayload] = useState(null);

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
  // The `dom-canvas` transport (`null` on every other carrier) and the wasm
  // session's most recent status. The raw status stays a ref: the chrome reads the
  // **host's** `cad_status`, which is where the relayed identity arrives, and the
  // session's own numbers are never painted directly.
  const transportRef = useRef(null);
  const canvasStatusRef = useRef(null);
  // The CAD pointer lane has two *independent* suspension reasons (N5): the
  // command line owns the keyboard, or a host dialog is open. The lane is
  // suspended while either holds, and each owner may only clear its own reason
  // — the command line's unconditional `onBlur` (any outside press triggers it)
  // must never resume the lane underneath a modal popup.
  const suspendGate = useRef(null);
  if (!suspendGate.current) {
    suspendGate.current = createSuspendGate(value => bindingRef.current?.suspend(value));
  }
  // Latest host status and command-line history for the CAD shortcut-menu hook,
  // so the webview-default guards are installed once and never re-bound by a
  // status poll or a new history entry. The menu's "repeat" item reads the same
  // history the command line keeps — there is no second command log.
  const statusRef = useRef(null);
  const historyRef = useRef([]);
  // The partition element itself, for two things the DOM event cannot carry: the
  // rect that turns the hook's partition-relative point into viewport px, and the
  // node that takes the keyboard back when the menu closes on Escape/Tab.
  const surfaceNodeRef = useRef(null);
  const surfaceRef = useCallback(node => {
    surfaceNodeRef.current = node;
    setSurfaceNode(node);
  }, []);

  const t = useMemo(() => createTranslator(CATALOGS, locale), [locale]);
  const displayError = status?.error || error || pollError || '';
  const commandMessage = useMemo(() => {
    if (!commandResult) return null;
    // The refusal sentence is the catalog key the admission itself reported
    // (`reasonKey`), so a blocked command reads the same in the command line as
    // in the disabled button that offers it — the busy sentence, "no drawing is
    // open", the gesture hint, or "not supported yet".
    if (commandResult.kind === 'unsupported') {
      return { kind: 'error', text: t(commandResult.reasonKey ?? unavailableReasonKey(commandResult.command), { command: commandResult.command }) };
    }
    if (commandResult.kind === 'failed') {
      if (commandResult.reasonKey) return { kind: 'error', text: t(commandResult.reasonKey, { command: commandResult.command }) };
      return { kind: 'error', text: t('command.failed', { error: commandResult.error }) };
    }
    return null;
  }, [commandResult, t]);

  useEffect(() => {
    document.documentElement.lang = localeTag(locale);
    document.title = t('app.title');
    // Behaviour change: the chosen language is persisted (same origin as a
    // popup window) and now survives a restart. A blocked storage is a no-op.
    storeLocale(locale);
  }, [locale, t]);

  // Live language propagation. The Options dialog is rendered in this same tree
  // (an in-DOM overlay), so its `setLocale` is this provider's setter and the
  // change is immediate. The stored-value subscription is kept as a defensive
  // same-origin fallback (a reload, or a second document in a test harness):
  // only a supported stored value is adopted — an absent entry never overrides
  // the host-resolved locale.
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
  //
  // `dom-canvas` hosts have no native surface behind this window, so their input
  // must not be forwarded to a host input gate: the wasm session receives it
  // directly (the transport effect below) and **no** `probe_input` lane exists.
  // `native-below` - including the mount before the host reports any mode - runs
  // exactly the code it always did.
  useEffect(() => {
    if (surfaceMode === SURFACE_MODE_DOM_CANVAS) return undefined;
    const lane = createInputLane(input => invoke('probe_input', { input }), setError);
    laneRef.current = lane;
    return () => { lane.dispose(); laneRef.current = null; };
  }, [surfaceMode]);

  // The `dom-canvas` transport (ADR 0024 D1/D2/D3). The wasm session is the single
  // pixel carrier and the single camera owner; this component only measures the
  // partition, hands the module a size, and forwards drag/zoom. `.native-input`
  // stays the single input surface: navigation is bound to it and never to the
  // canvas node, which receives no listener and no state.
  //
  // A module that cannot load (missing artifact, CSP without
  // `'wasm-unsafe-eval'`, broken instantiation) never reaches the session, so it
  // has no status of its own: the module-load step is classified here and reported
  // to the host. One failed report, one refusal - never a blank canvas and never a
  // retry loop.
  useEffect(() => {
    if (surfaceMode !== SURFACE_MODE_DOM_CANVAS || !surfaceNode) return undefined;
    let cancelled = false;
    let transport = null;
    let unbindNavigation = null;
    let stopWatching = null;
    // The one channel that publishes *this* session's drawing identity to the
    // host. The native shell must not hold document state of its own (iron rule
    // 8), so the identity travels one way only: the session reports it, the host
    // stores the relay, the chrome reads the relay back (`status.canvasSession`).
    // Every send is deduplicated against the previous one, so a gesture that does
    // not change the reported payload never reaches the host, and this effect —
    // which only exists on `dom-canvas` — is the only caller: a `native-below`
    // host never sends a session report at all.
    const reporter = createCanvasSessionReporter(report => {
      void invoke('canvas_session_status', canvasSessionPayload(report))
        .catch(reason => setError(String(reason)));
    });
    const fail = failure => {
      // The host owns the refusal: it maps the step to its stable marker line and
      // exit code (`webgl2` -> `yacr: unsupported-runtime: webgl2`, exit code 6).
      // This side forwards the machine-readable step and the session's own detail
      // only; it never invents user-facing copy (the i18n catalog owns every word).
      void invoke('canvas_runtime_failed', { step: failure.step, detail: failure.detail })
        .catch(reason => setError(String(reason)));
    };
    void loadCanvasModule(canvasModuleUrl()).then(module => {
      if (cancelled) return;
      transport = createCanvasTransport({
        module,
        element: surfaceNode,
        rect: rectFromElement(surfaceNode, window.devicePixelRatio || 1),
        onFailure: fail,
        onStatus: next => {
          canvasStatusRef.current = next;
          reporter.report(next);
        },
      });
      transportRef.current = transport;
      unbindNavigation = bindCanvasNavigation(surfaceNode, {
        pan: (dx, dy) => transport.pan(dx, dy),
        zoom: (factor, x, y) => transport.zoom(factor, x, y),
      });
      stopWatching = watchCanvasRect(surfaceNode, rect => transport.update(rect));
    }).catch(error => {
      if (cancelled) return;
      fail({ step: CANVAS_STEP_MODULE, detail: String(error) });
    });
    return () => {
      cancelled = true;
      unbindNavigation?.();
      stopWatching?.();
      transport?.dispose();
      if (transportRef.current === transport) transportRef.current = null;
    };
  }, [surfaceMode, surfaceNode]);

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
    // The alias table is suggestion metadata, fetched once from the same Rust
    // source (`cad_command_aliases`). A failure is deliberately **not** an error:
    // the command line simply offers no shorthand rows and admission still works,
    // because the host canonicalises whatever is typed. Nothing is fabricated.
    invoke('cad_command_aliases').then(pairs => {
      if (cancelled) return;
      setAliasPairs(setCommandAliases(pairs));
    }).catch(() => {
      if (cancelled) return;
      setAliasPairs(setCommandAliases([]));
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

  // A closed popup drops its payload: the next open must seed from its own ack,
  // never from stale content.
  useEffect(() => {
    if (!popupOpen) setPopupPayload(null);
  }, [popupOpen]);

  // One document slot, two honest sources, never a guess: the host's own label on
  // `native-below`, or the live session's label on `dom-canvas`. A carrier that
  // reports neither keeps the start state, exactly as before.
  const hostDocumentLabel = status?.documentLabel ?? null;
  const documentLabel = hostDocumentLabel ?? canvasSession?.label ?? null;
  // Two different facts, deliberately not merged into one flag:
  //
  // * `documentOpen` is the **host document** fact (unchanged): the host holds a
  //   drawing it can act on, which is what admits document-scoped commands and the
  //   standard views. A `dom-canvas` shell holds none of that by design.
  // * `documentVisible` is the **layout** fact: something is actually drawn in the
  //   CAD partition. That is what replaces the start panel and what the status bar
  //   reports, and on a `dom-canvas` host the live session is the only evidence.
  //
  // Keeping them apart is what lets the partition look honest without enabling
  // data this carrier does not have.
  const documentOpen = hostDocumentLabel !== null;
  const documentVisible = documentOpen || canvasSession !== null;
  const loading = Boolean(status?.loading);
  const importProgress = status?.importProgress ?? null;
  const cadReady = Boolean(status) && !loading;
  const toolActive = Boolean(status?.tool?.key);

  // The one open path for every dialog. The host owns admission and the gate;
  // this only keeps the fresh payload for the overlay. `existing: true` means
  // the same dialog is already open, so the success is a no-op here and no
  // second instance (or replacement payload) is created.
  const requestOpenPopup = useCallback(kind => {
    void openPopup(invoke, kind).then(result => {
      if (result.kind !== 'accepted') {
        setError(result.error);
        return;
      }
      if (!result.existing && result.payload) setPopupPayload(result.payload);
    });
  }, []);

  const submitCommand = useCallback(async (text, options = {}) => {
    // `OPTIONS` is a local chrome selector, not a router command: open the
    // dialog and never forward the text. Intercepted before the in-flight guard
    // so a second Enter still reaches the host (which focuses the open window).
    if (isOptionsCommand(text)) {
      setCommandResult(null);
      if (options.fromInput) setCommand('');
      requestOpenPopup('options');
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
    // `LAYER` opens the docked layers palette through the same chrome-local path.
    // The router has no payload-free `LAYER` (only the layer *operations*), so the
    // text never reaches `cad_command`; the palette reads the host on mount and on
    // every repeat, which is what makes a second `LAYER` a refresh rather than a
    // no-op.
    if (isLayersCommand(text)) {
      setCommandResult(null);
      if (options.fromInput) setCommand('');
      setLayersOpen(true);
      setLayersFocus(value => value + 1);
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
  }, [requestOpenPopup, status]);

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
      // While a popup is open the overlay owns its one accelerator (Escape). The
      // shell must not also cancel a tool, toggle a panel or blur a field.
      if (popupOpen) return;
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
      // The drafting function keys (F3 object snap, F7 grid, F8 ortho) flip
      // session switches through the same command funnel every other affordance
      // uses; the mapping table lives in `cad.mjs` so the status-bar chips and
      // these accelerators can never disagree. `preventDefault` keeps the webview
      // from consuming the key itself. Like F2 these are global accelerators that
      // a focused text field does not swallow, and they never collide with a
      // tool's Enter/Escape (handled above only for those two keys).
      const draftingCommand = draftingAcceleratorCommand(event.key);
      if (draftingCommand) { event.preventDefault(); void submitCommand(draftingCommand); return; }
      if (event.key === 'Escape') {
        setHistoryOpen(false);
        if (isTyping(event.target)) event.target.blur();
        else setDiagnosticsOpen(false);
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [popupOpen, submitCommand, toolActive]);

  // Feed the CAD shortcut-menu hook the state slices the chrome already has. It
  // is a ref, not a dependency, so polling never re-installs the guards below.
  useEffect(() => { statusRef.current = status; }, [status]);
  useEffect(() => { historyRef.current = history; }, [history]);

  // Keep WebKitGTK's own commands out of the session. The native CAD surface is
  // input-transparent, so right-clicks and accelerators reach this DOM; the
  // webview menu's Reload would remount the UI while Rust and the native surface
  // keep running, and page zoom would desync the rect the host composites at.
  // Declared after the shortcut handler above (both listen on `window` in the
  // bubble phase). Ctrl+0 is the only overlap, and the shortcut handler owns it
  // before its `defaultPrevented` check, so clean screen keeps working whichever
  // listener runs first while the browser zoom reset is still suppressed. The
  // guard suppresses every browser command even in a focused text field; the
  // native menu stays in text fields, and a CAD right-click opens the shortcut
  // menu — the press itself is still forwarded to the host exactly as before.
  useEffect(() => {
    const guard = bindBrowserDefaults(window, {
      context: () => {
        const current = statusRef.current;
        return { selection: current?.selection ?? null, tool: current?.tool ?? null };
      },
      // The hook reports the point relative to the partition; the overlay is
      // viewport-fixed, so the partition rect is what converts it. `repeat` reads
      // the command line's own newest history entry, and `canFit`/`hasDocument`
      // are the host facts the command-line admission rule already uses.
      onContextMenu: detail => {
        const node = surfaceNodeRef.current;
        const bounds = node ? node.getBoundingClientRect() : null;
        const current = statusRef.current;
        setContextMenu({
          point: {
            x: (bounds?.left ?? 0) + detail.point.x,
            y: (bounds?.top ?? 0) + detail.point.y,
          },
          rect: bounds
            ? { x: bounds.left, y: bounds.top, width: bounds.width, height: bounds.height }
            : null,
          menu: cadContextMenuRequest({
            ...detail,
            lastCommand: historyRef.current.at(-1) ?? null,
            canFit: current?.canFit === true,
            hasDocument: (current?.documentLabel ?? null) !== null,
          }),
        });
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
    // Whether a drawing is actually on screen (the layout fact). Equal to
    // `documentOpen` on every `native-below` host; on `dom-canvas` the live session
    // supplies it, and it only drives layout and the status-bar state word.
    documentVisible,
    // Whether the docked properties/diagnostics palettes exist on this carrier.
    // `false` only on a reported `dom-canvas` host, which holds no host-side
    // document state for them to project.
    palettesAvailable,
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
    history,
    command,
    setCommand,
    commandResult,
    commandMessage,
    // The host's alias table (`cad_command_aliases`). It is suggestion metadata
    // only: the command line offers these rows, admission still goes through
    // `cad_command`, and an empty table just means "no aliases known".
    commandAliases: aliasPairs,
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
        if (failure) {
          setError(failure);
          return;
        }
        if (!result.existing && result.payload) setPopupPayload(result.payload);
      });
    },
    // The Options dialog (the same in-DOM overlay layer). One instance: a second
    // request is answered as `existing: true`, which is not a failure.
    openOptionsPopup: () => requestOpenPopup('options'),
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
    // The layers palette: the same open/close plumbing as its two neighbours. The
    // palette itself reads the host; this side only owns whether it is docked.
    layersOpen,
    setLayersOpen,
    toggleLayers: () => setLayersOpen(value => !value),
    // Bumped by every `LAYER` (open + focus + re-read); the palette watches it.
    layersFocus,
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
      // Dialogs are in-DOM overlays (ADR 0021 D1/D2). The host still owns the
      // modal gate and admission; `existing: true` means the same dialog is
      // already open, so there is nothing to do here and it is never a failure.
      else if (action === 'about') {
        requestOpenPopup('about');
      }
      // The Options dialog is the same overlay layer. Opening it twice is a
      // success: the host reports the single instance as existing.
      else if (action === 'options') {
        requestOpenPopup('options');
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
        {/* The CAD shortcut menu: one in-DOM overlay above the workspace. It has
            no backdrop, so the press that dismisses it still reaches whatever is
            underneath — the CAD partition keeps forwarding every press to the
            host. Running an item uses the same funnel as the ribbon and the
            command line (`runCommand`), which is what also resolves the
            chrome-local `PROPERTIES` selector to the docked palette. */}
        {contextMenu ? (
          <ContextMenu
            menu={contextMenu.menu}
            anchor={contextMenu.point}
            onRun={item => {
              setContextMenu(null);
              if (item.command) shell.runCommand(item.command);
            }}
            onDismiss={({ restoreFocus } = {}) => {
              setContextMenu(null);
              // Escape/Tab hand the keyboard back to the drawing area; a press
              // outside must not steal focus from what that press just focused.
              if (restoreFocus) surfaceNodeRef.current?.focus?.({ preventScroll: true });
            }}
          />
        ) : null}
        {popupOpen ? <PopupOverlay popup={status.popup} payload={popupPayload} /> : null}
      </ShellContext.Provider>
    </I18nContext.Provider>
  );
}
