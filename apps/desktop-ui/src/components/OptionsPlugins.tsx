// The Plugins tab of the Options dialog.
//
// This tab is **not staged**. Every other Options page edits a local draft and
// writes it on Apply/OK; a plugin action is a host request with a host answer, so
// it is sent immediately and the list is re-read from `readPluginStatus`. The one
// thing that is gated is *loading*: `plugins/plugin-manager.ts` saves the choice,
// and the running process keeps the decoder it already loaded. The persistent
// restart banner is therefore the tab's main promise, and no branch here claims a
// change is live: "accepted" only means the host queued it.
//
// The shape, and why:
//
//   * **One radio group, not checkboxes.** The host allows exactly one decoder.
//     A checkbox list would advertise a state the host refuses; the radio group
//     makes "enabling this disables the others" visible before the request. A
//     separate Enable/Disable button accompanies the radio because a radio
//     cannot be unchecked — the enabled row needs a way back to "no decoder".
//   * **Identity on a second line.** Name, source badge and state pill fit on
//     one 499px content row; the mono `id · version · ABI` run wraps instead of
//     squeezing the name. That keeps the state column aligned down the list.
//   * **Inline confirmations, never a second dialog.** Installing and
//     uninstalling are one sentence plus two buttons inside the tab. The install
//     panel names the picked file and repeats the trust warning at the moment the
//     decision is made; the uninstall strip sits in the row it removes, so there
//     is no ambiguity about which plugin it refers to.
//   * **No success message.** An accepted install/uninstall is visible as the row
//     appearing or disappearing (and, when the host says so, as the restart
//     banner). A refusal keeps the host's stable reason text on screen and never
//     turns into "installed".
//   * **Two kinds of diagnostics.** The last vendor pass (`subscribeVendorSettled`)
//     is what actually ran against the drawing; `frame.diagnostics` is what the
//     status scan found. They are labelled apart because merging them would
//     present a scan error as a pass failure.
//
// Focus: opening a confirmation moves focus into it, and closing one puts focus
// back on a row control that still exists (the radio), so a keyboard user is
// never dropped at the top of the dialog. The native file input is visually
// hidden but driven by the visible "Install plugin…" button.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ChangeEvent, CSSProperties, KeyboardEvent } from 'react';
import { invoke } from '../host/invoke.ts';
import { Icon } from '../icons.tsx';
import { Button, IconButton } from './primitives.tsx';
import { useI18n } from '../context.ts';
import {
  canDisable,
  canUninstall,
  pluginFileProblem,
  pluginFileProblemKey,
  pluginRowDomId,
  pluginState,
  pluginStateKey,
  scanDiagnostics,
  vendorSettledSummary,
} from '../options/plugins-view.ts';
import type { PluginState, VendorSettledSummary } from '../options/plugins-view.ts';
import {
  installPluginFile,
  lastVendorSettled,
  readPluginStatus,
  setPluginEnabled,
  subscribeVendorSettled,
  uninstallPlugin,
} from '../plugins/plugin-manager.ts';
import type { PluginMutationResult, PluginRow, PluginStatusFrame } from '../plugins/types.ts';
import {
  OPTIONS_BADGE,
  OPTIONS_BANNER,
  OPTIONS_BANNER_TEXT,
  OPTIONS_CONFIRM,
  OPTIONS_CONFIRM_TEXT,
  OPTIONS_DIAGNOSTICS,
  OPTIONS_DIAGNOSTICS_LIST,
  OPTIONS_HELP,
  OPTIONS_PILL,
  OPTIONS_PILL_ACTIVE,
  OPTIONS_PILL_INVALID,
  OPTIONS_PILL_PENDING,
  OPTIONS_PLUGIN_ACTIONS,
  OPTIONS_PLUGIN_BUTTONS,
  OPTIONS_PLUGIN_LIST,
  OPTIONS_PLUGIN_META,
  OPTIONS_PLUGIN_META_VALUE,
  OPTIONS_PLUGIN_NAME,
  OPTIONS_PLUGIN_NAME_TEXT,
  OPTIONS_PLUGIN_ROW,
  OPTIONS_PLUGIN_ROW_RULE,
  OPTIONS_SECTION_LABEL,
  OPTIONS_STATE,
} from '../popup/popup-layout.ts';

/** The one radio group (one decoder at a time). */
const DECODER_GROUP = 'options-plugin-decoder';

/**
 * Keys that move between rows.
 *
 * A native radio group changes its selection on an arrow key, and here every
 * selection is a **host write**: arrowing across three plugins would send three
 * enables and leave three restart notices. The tab therefore takes the arrows
 * and moves focus only; Space (the radio's own activation) is what commits.
 */
const RADIO_FOCUS_KEYS = new Set(['ArrowDown', 'ArrowRight', 'ArrowUp', 'ArrowLeft', 'Home', 'End']);

/** The file input is driven by the visible install button. */
const FILE_INPUT_ID = 'options-plugin-file';

const INTRO_ID = 'options-plugins-intro';
const INSTALL_HINT_ID = 'options-plugin-install-hint';
const INSTALL_FILE_ID = 'options-plugin-install-file';
const INSTALL_WARNING_ID = 'options-plugin-install-warning';

/** The tab's load states; the frame itself is the fourth (ready). */
type Phase =
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly frame: PluginStatusFrame }
  | { readonly kind: 'failed'; readonly error: string };

/** A host refusal (or a transport failure) kept as its stable text. */
interface Refusal {
  readonly reason: string;
}

/** The four ids one row needs (a plugin id may contain dots, so it is stemmed). */
function rowIds(id: string) {
  const stem = pluginRowDomId(id);
  return {
    radio: `options-plugin-radio-${stem}`,
    meta: `options-plugin-meta-${stem}`,
    state: `options-plugin-state-${stem}`,
    confirm: `options-plugin-uninstall-${stem}`,
  };
}

/** The tint of one state pill; `disabled` keeps the shared neutral surface. */
function pillStyle(state: PluginState): CSSProperties {
  switch (state) {
    case 'active':
      return OPTIONS_PILL_ACTIVE;
    case 'pendingRestart':
      return OPTIONS_PILL_PENDING;
    case 'invalid':
      return OPTIONS_PILL_INVALID;
    default:
      return {};
  }
}

/** The user-plugin badge is the only accented badge; built-in keeps the neutral one. */
const USER_BADGE: CSSProperties = Object.freeze({
  ...OPTIONS_BADGE,
  color: 'var(--accent-text)',
  borderColor: 'var(--accent-text)',
});

const RADIO: CSSProperties = Object.freeze({
  width: 14,
  height: 14,
  margin: '2px 0 0',
  accentColor: 'var(--accent)',
});

/** The install panel: the shared confirmation box, stacked. */
const INSTALL_PANEL: CSSProperties = Object.freeze({
  ...OPTIONS_CONFIRM,
  marginTop: 0,
  flexDirection: 'column',
  alignItems: 'stretch',
  gap: 8,
});

/** The refusal band: same box, danger edge. */
const REFUSAL: CSSProperties = Object.freeze({
  ...INSTALL_PANEL,
  borderColor: 'var(--danger)',
});

const FILE_NAME: CSSProperties = Object.freeze({
  margin: 0,
  fontFamily: 'var(--font-mono)',
  fontSize: 12,
  lineHeight: 1.4,
  color: 'var(--text)',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  whiteSpace: 'nowrap',
});

/** The title/intro rows of the tab. */
const ROOT: CSSProperties = Object.freeze({
  display: 'flex',
  flexDirection: 'column',
  gap: 10,
  minWidth: 0,
});

const INSTALL_ROW: CSSProperties = Object.freeze({
  display: 'flex',
  alignItems: 'center',
  gap: 8,
  flexWrap: 'wrap',
});

const EMPTY: CSSProperties = Object.freeze({
  margin: 0,
  padding: '10px 12px',
  border: '1px dashed var(--line-soft)',
  borderRadius: 'var(--radius)',
  fontSize: 11,
  lineHeight: 1.5,
  color: 'var(--text-muted)',
  textAlign: 'center',
});

/** The mono line under a diagnostics list ("Plugin scan"). */
const DIAGNOSTICS_LABEL: CSSProperties = Object.freeze({
  margin: '7px 0 0',
  fontFamily: 'var(--font-ui)',
  fontSize: 10.5,
  letterSpacing: '.4px',
  textTransform: 'uppercase',
  color: 'var(--text-muted)',
});

export function OptionsPlugins() {
  const { t } = useI18n();
  const [phase, setPhase] = useState<Phase>({ kind: 'loading' });
  const [busy, setBusy] = useState(false);
  /** The file picked from disk, waiting for the trust confirmation. */
  const [pendingFile, setPendingFile] = useState<File | null>(null);
  /** The user plugin whose inline uninstall confirmation is open. */
  const [confirmUninstall, setConfirmUninstall] = useState<string | null>(null);
  const [refusal, setRefusal] = useState<Refusal | null>(null);
  /** Set by a mutation that reported `restartRequired`; re-reads stay authoritative. */
  const [mutationRestart, setMutationRestart] = useState(false);
  const [settled, setSettled] = useState<VendorSettledSummary | null>(() => vendorSettledSummary(lastVendorSettled()));

  const alive = useRef(true);
  const inFlight = useRef(false);
  const fileRef = useRef<HTMLInputElement | null>(null);
  const installRef = useRef<HTMLButtonElement | null>(null);
  const installCancelRef = useRef<HTMLButtonElement | null>(null);
  const installConfirmRef = useRef<HTMLButtonElement | null>(null);
  const uninstallConfirmRef = useRef<HTMLButtonElement | null>(null);

  // Derived before the handlers: `moveRadioFocus` reads `rows`, and a value
  // declared after its `useCallback` would be in its temporal dead zone during
  // the hook call itself.
  const frame = phase.kind === 'ready' ? phase.frame : null;
  const rows = useMemo(
    () => (frame && Array.isArray(frame.plugins) ? frame.plugins : []),
    [frame],
  );
  const scan = useMemo(() => scanDiagnostics(frame), [frame]);

  /**
   * Re-read the host's list. `quiet` keeps the current rows on screen during a
   * refresh after an action; the first read has no rows yet and shows the
   * loading state instead.
   */
  const load = useCallback(async (quiet = false) => {
    if (!quiet) setPhase({ kind: 'loading' });
    try {
      const frame = await readPluginStatus(invoke);
      if (!alive.current) return;
      // A frame without its policy flag is not a usable answer. Showing an empty
      // list would claim "no plugins" about a payload we did not understand.
      if (!frame || typeof frame !== 'object' || typeof frame.supported !== 'boolean') {
        setPhase({ kind: 'failed', error: 'PLUGIN_STATUS_INVALID' });
        return;
      }
      setPhase({ kind: 'ready', frame });
      setMutationRestart(frame.restartRequired === true);
    } catch (error) {
      if (!alive.current) return;
      setPhase({ kind: 'failed', error: String(error ?? '') });
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    void load();
    return () => {
      alive.current = false;
    };
  }, [load]);

  // The last vendor pass is a push event: seed from the last record so opening
  // the dialog after a pass still shows it, then keep listening while open.
  useEffect(() => {
    setSettled(vendorSettledSummary(lastVendorSettled()));
    return subscribeVendorSettled(value => {
      if (alive.current) setSettled(vendorSettledSummary(value));
    });
  }, []);

  // Focus moves into whichever confirmation just opened; a disabled confirm
  // button cannot take it, so a rejected file sends focus to Cancel instead.
  const problem = pendingFile ? pluginFileProblem(pendingFile) : null;

  useEffect(() => {
    if (!pendingFile) return;
    if (problem) installCancelRef.current?.focus();
    else installConfirmRef.current?.focus();
  }, [pendingFile, problem]);

  useEffect(() => {
    if (confirmUninstall) uninstallConfirmRef.current?.focus();
  }, [confirmUninstall]);

  /**
   * Send one host request, report its answer, then re-read the list.
   *
   * `accepted` is the only success: a refusal keeps its stable reason and the
   * rows are still re-read, because a refusal can leave the host in a state the
   * old frame no longer describes. The re-read is quiet so the list does not
   * flash through the loading state after every click.
   */
  const mutate = useCallback(async (action: () => Promise<PluginMutationResult>): Promise<boolean> => {
    if (inFlight.current) return false;
    inFlight.current = true;
    setBusy(true);
    setRefusal(null);
    let accepted = false;
    try {
      const result = await action();
      if (!alive.current) return false;
      accepted = Boolean(result) && result.accepted === true;
      if (accepted) {
        if (result.restartRequired) setMutationRestart(true);
      } else {
        const reason = typeof result?.reason === 'string' ? result.reason.trim() : '';
        setRefusal({ reason });
      }
      await load(true);
    } catch (error) {
      if (alive.current) setRefusal({ reason: String(error ?? '').trim() });
    } finally {
      if (alive.current) {
        inFlight.current = false;
        setBusy(false);
      }
    }
    return accepted && alive.current;
  }, [load]);

  const setEnabled = useCallback((row: PluginRow, enabled: boolean) => {
    if (row.enabled === enabled) return;
    setConfirmUninstall(null);
    const radioId = rowIds(row.id).radio;
    void mutate(() => setPluginEnabled(invoke, row.id, enabled)).then(ok => {
      // Disable replaces its own button with Enable, so focus would otherwise be
      // lost. The radio always exists and is the row's stable control.
      if (ok && alive.current) document.getElementById(radioId)?.focus();
    });
  }, [mutate]);

  const moveRadioFocus = useCallback((event: KeyboardEvent<HTMLInputElement>, id: string) => {
    if (!RADIO_FOCUS_KEYS.has(event.key)) return;
    event.preventDefault();
    const index = rows.findIndex(entry => entry.id === id);
    if (index < 0) return;
    let next = index;
    if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = rows.length - 1;
    else if (event.key === 'ArrowDown' || event.key === 'ArrowRight') next = (index + 1) % rows.length;
    else next = (index - 1 + rows.length) % rows.length;
    document.getElementById(rowIds(rows[next].id).radio)?.focus();
  }, [rows]);

  const cancelUninstall = useCallback((row: PluginRow) => {
    setConfirmUninstall(null);
    document.getElementById(rowIds(row.id).radio)?.focus();
  }, []);

  const remove = useCallback((row: PluginRow) => {
    setConfirmUninstall(null);
    void mutate(() => uninstallPlugin(invoke, row.id)).then(ok => {
      // The row (and the button that had focus) is gone on success; the install
      // button is always present and keeps the keyboard inside the tab.
      if (ok && alive.current) installRef.current?.focus();
    });
  }, [mutate]);

  const pickFile = useCallback(() => {
    setRefusal(null);
    fileRef.current?.click();
  }, []);

  const chooseFile = useCallback((event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0] ?? null;
    // Clear the input so choosing the same file twice still fires `change`.
    event.target.value = '';
    if (!file) return;
    setRefusal(null);
    setPendingFile(file);
  }, []);

  const cancelInstall = useCallback(() => {
    setPendingFile(null);
    installRef.current?.focus();
  }, []);

  const confirmInstall = useCallback(() => {
    if (!pendingFile || pluginFileProblem(pendingFile)) return;
    const file = pendingFile;
    void mutate(() => installPluginFile(invoke, file)).then(ok => {
      if (ok && alive.current) {
        setPendingFile(null);
        installRef.current?.focus();
      }
      // A refusal keeps the panel open: the reason is below it and the user can
      // cancel or pick another file.
    });
  }, [mutate, pendingFile]);

  // A ready frame is authoritative; `mutationRestart` only survives when the
  // re-read after a mutation failed and there is no frame to ask.
  const restartRequired = mutationRestart || frame?.restartRequired === true;

  const banner = restartRequired ? (
    <div style={OPTIONS_BANNER} role="status">
      <Icon name="alert" size={16} style={{ color: 'var(--accent-text)', flex: '0 0 auto', marginTop: 1 }} />
      <p style={OPTIONS_BANNER_TEXT}>{t('plugins.restart')}</p>
    </div>
  ) : null;

  if (phase.kind === 'loading') {
    return (
      <section aria-label={t('plugins.title')} aria-busy="true" style={ROOT}>
        {banner}
        <div style={OPTIONS_STATE} role="status" aria-live="polite">
          <Icon name="spinner" size={20} className="spin" />
          <p style={{ margin: 0, fontSize: 12, color: 'var(--text-dim)' }}>{t('plugins.loading')}</p>
        </div>
      </section>
    );
  }

  if (phase.kind === 'failed') {
    return (
      <section aria-label={t('plugins.title')} style={ROOT}>
        {banner}
        <div style={OPTIONS_STATE}>
          <Icon name="alert" size={22} style={{ color: 'var(--danger)' }} />
          <p style={{ margin: 0, fontSize: 12, lineHeight: 1.5, color: 'var(--text-dim)' }} title={phase.error}>
            {t('plugins.errorLoad')}
          </p>
          <Button icon="undo" onClick={() => { void load(); }}>{t('plugins.refresh')}</Button>
        </div>
      </section>
    );
  }

  // `supported: false` is a carrier answer, not an error: name the carrier and
  // offer no install/uninstall path that could only fail.
  if (!frame || frame.supported !== true) {
    return (
      <section aria-label={t('plugins.title')} style={ROOT}>
        {banner}
        <div style={OPTIONS_STATE}>
          <Icon name="alert" size={22} style={{ color: 'var(--text-muted)' }} />
          <p style={{ margin: 0, fontSize: 12, lineHeight: 1.5, color: 'var(--text-dim)' }}>
            {t('plugins.unsupported')}
          </p>
          {typeof frame?.carrier === 'string' && frame.carrier.trim().length > 0 ? (
            <code style={{ fontFamily: 'var(--font-mono)', fontSize: 11, color: 'var(--text-muted)' }}>
              {frame.carrier}
            </code>
          ) : null}
          {/* The registry's own refusal is the reason this carrier is unsupported;
              naming the carrier alone would hide why. */}
          {scan.length > 0 ? (
            <ul style={{ ...OPTIONS_DIAGNOSTICS_LIST, textAlign: 'left', margin: 0 }}>
              {scan.map((line, index) => (
                <li key={`unsupported-${index}`} style={{ fontFamily: 'var(--font-mono)', fontSize: 10.5 }}>
                  {line}
                </li>
              ))}
            </ul>
          ) : null}
          <Button icon="undo" onClick={() => { void load(); }}>{t('plugins.refresh')}</Button>
        </div>
      </section>
    );
  }

  return (
    <section aria-label={t('plugins.title')} aria-busy={busy || undefined} style={ROOT}>
      {banner}
      <p id={INTRO_ID} style={OPTIONS_HELP}>{t('plugins.intro')}</p>

      <fieldset
        style={{ border: 0, margin: 0, padding: 0, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 6 }}
        aria-describedby={INTRO_ID}
      >
        <legend style={{ ...OPTIONS_SECTION_LABEL, padding: 0 }}>{t('plugins.decoders')}</legend>
        {rows.length === 0 ? (
          <p style={EMPTY}>{t('plugins.empty')}</p>
        ) : (
          <ul style={OPTIONS_PLUGIN_LIST}>
            {rows.map((row, index) => {
              const state = pluginState(row);
              const ids = rowIds(row.id);
              const userPlugin = canUninstall(row);
              return (
                <li
                  key={row.id}
                  style={index === 0 ? OPTIONS_PLUGIN_ROW : { ...OPTIONS_PLUGIN_ROW, ...OPTIONS_PLUGIN_ROW_RULE }}
                >
                  <input
                    id={ids.radio}
                    type="radio"
                    name={DECODER_GROUP}
                    style={RADIO}
                    checked={row.enabled === true}
                    disabled={busy}
                    aria-describedby={`${ids.meta} ${ids.state}`}
                    onChange={() => setEnabled(row, true)}
                    onKeyDown={event => moveRadioFocus(event, row.id)}
                  />
                  <div style={{ minWidth: 0 }}>
                    <div style={OPTIONS_PLUGIN_NAME}>
                      <label
                        htmlFor={ids.radio}
                        style={OPTIONS_PLUGIN_NAME_TEXT}
                        title={row.name}
                      >
                        {row.name}
                      </label>
                      <span
                        style={userPlugin ? USER_BADGE : OPTIONS_BADGE}
                        title={`${t('plugins.source')}: ${t(userPlugin ? 'plugins.userBadge' : 'plugins.builtinBadge')}`}
                      >
                        {t(userPlugin ? 'plugins.userBadge' : 'plugins.builtinBadge')}
                      </span>
                    </div>
                    <div id={ids.meta} style={OPTIONS_PLUGIN_META}>
                      <span>
                        {t('plugins.id')}{' '}
                        <span style={OPTIONS_PLUGIN_META_VALUE}>{row.id}</span>
                      </span>
                      <span>
                        {t('plugins.version')}{' '}
                        <span style={OPTIONS_PLUGIN_META_VALUE}>{row.version}</span>
                      </span>
                      <span>
                        ABI <span style={OPTIONS_PLUGIN_META_VALUE}>{row.abi}</span>
                      </span>
                    </div>
                  </div>
                  <div style={OPTIONS_PLUGIN_ACTIONS}>
                    {/* The host's exact manifest finding (`manifest_missing`,
                        `manifest_invalid`, ...) stays in the tooltip; the pill
                        itself carries the user-facing state. */}
                    <span
                      id={ids.state}
                      style={{ ...OPTIONS_PILL, ...pillStyle(state) }}
                      title={state === 'invalid' ? row.status : undefined}
                    >
                      {t(pluginStateKey(state))}
                    </span>
                    <div style={OPTIONS_PLUGIN_BUTTONS}>
                      {canDisable(row) ? (
                        <Button size="xs" disabled={busy} onClick={() => setEnabled(row, false)}>
                          {t('plugins.disable')}
                        </Button>
                      ) : (
                        <Button size="xs" disabled={busy} onClick={() => setEnabled(row, true)}>
                          {t('plugins.enable')}
                        </Button>
                      )}
                      {/* While the inline confirmation is open it owns the one
                          Uninstall action; a second identical trigger in the
                          same row would make the choice ambiguous. */}
                      {userPlugin && confirmUninstall !== row.id ? (
                        <Button
                          size="xs"
                          icon="trash"
                          iconSize={12}
                          disabled={busy}
                          title={t('plugins.uninstallConfirm')}
                          onClick={() => setConfirmUninstall(row.id)}
                        >
                          {t('plugins.uninstall')}
                        </Button>
                      ) : null}
                    </div>
                  </div>
                  {confirmUninstall === row.id ? (
                    <div
                      role="group"
                      aria-label={`${t('plugins.uninstall')} ${row.name}`}
                      style={{ ...OPTIONS_CONFIRM, gridColumn: '1 / -1' }}
                    >
                      <p id={ids.confirm} style={OPTIONS_CONFIRM_TEXT}>{t('plugins.uninstallConfirm')}</p>
                      <Button
                        size="xs"
                        aria-describedby={ids.confirm}
                        onClick={() => cancelUninstall(row)}
                      >
                        {t('plugins.cancel')}
                      </Button>
                      <Button
                        ref={uninstallConfirmRef}
                        size="xs"
                        variant="chip"
                        disabled={busy}
                        aria-describedby={ids.confirm}
                        onClick={() => remove(row)}
                      >
                        {t('plugins.uninstall')}
                      </Button>
                    </div>
                  ) : null}
                </li>
              );
            })}
          </ul>
        )}
      </fieldset>

      <div style={INSTALL_ROW}>
        <Button
          ref={installRef}
          variant="chip"
          size="sm"
          icon="plus"
          iconSize={14}
          disabled={busy}
          aria-describedby={INSTALL_HINT_ID}
          onClick={pickFile}
        >
          {t('plugins.install')}
        </Button>
        <p id={INSTALL_HINT_ID} style={{ ...OPTIONS_HELP, flex: '1 1 180px', minWidth: 0 }}>
          {t('plugins.installHint')}
        </p>
        <Button size="sm" icon="undo" iconSize={14} disabled={busy} onClick={() => { void load(true); }}>
          {t('plugins.refresh')}
        </Button>
        <input
          id={FILE_INPUT_ID}
          ref={fileRef}
          type="file"
          accept=".js,.mjs"
          hidden
          onChange={chooseFile}
        />
      </div>

      {pendingFile ? (
        <div role="group" aria-label={t('plugins.install')} style={INSTALL_PANEL}>
          <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8 }}>
            <Icon name="alert" size={16} style={{ color: 'var(--accent-text)', flex: '0 0 auto', marginTop: 1 }} />
            <div style={{ flex: '1 1 auto', minWidth: 0 }}>
              <p id={INSTALL_FILE_ID} style={FILE_NAME} title={pendingFile.name}>{pendingFile.name}</p>
              <p id={INSTALL_WARNING_ID} style={OPTIONS_HELP}>{t('plugins.installWarning')}</p>
            </div>
            <IconButton icon="close" label={t('plugins.close')} size="xs" onClick={cancelInstall} />
          </div>
          {problem ? (
            <p role="alert" className="palette__error" style={{ margin: 0, fontSize: 11 }}>
              {t(pluginFileProblemKey(problem))}
            </p>
          ) : null}
          <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
            <Button ref={installCancelRef} size="xs" onClick={cancelInstall}>{t('plugins.cancel')}</Button>
            <Button
              ref={installConfirmRef}
              size="xs"
              variant="solid"
              disabled={Boolean(problem) || busy}
              aria-busy={busy || undefined}
              aria-describedby={`${INSTALL_FILE_ID} ${INSTALL_WARNING_ID}`}
              onClick={confirmInstall}
            >
              {t('plugins.installConfirm')}
            </Button>
          </div>
        </div>
      ) : null}

      {refusal ? (
        <div role="alert" style={REFUSAL}>
          <p style={{ ...OPTIONS_CONFIRM_TEXT, flex: '0 0 auto', color: 'var(--danger)', margin: 0 }}>
            {t('plugins.errorReason')}
          </p>
          <p className="palette__error" style={{ margin: 0, fontSize: 11 }}>
            {refusal.reason || t('plugins.errorGeneric')}
          </p>
        </div>
      ) : null}

      <section aria-label={t('plugins.diagnostics')} style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <p style={OPTIONS_SECTION_LABEL}>{t('plugins.diagnostics')}</p>
        <div style={OPTIONS_DIAGNOSTICS}>
          {settled ? (
            <p style={{ margin: 0 }}>{t('plugins.appliedCount', { count: settled.applied })}</p>
          ) : (
            <p style={{ margin: 0, color: 'var(--text-muted)' }}>{t('plugins.noDiagnostics')}</p>
          )}
          {settled && settled.diagnostics.length > 0 ? (
            <ul style={OPTIONS_DIAGNOSTICS_LIST}>
              {settled.diagnostics.map((line, index) => (
                <li key={`pass-${index}`}>{line}</li>
              ))}
            </ul>
          ) : null}
          {scan.length > 0 ? (
            <>
              <p style={DIAGNOSTICS_LABEL}>{t('plugins.scanDiagnostics')}</p>
              <ul style={OPTIONS_DIAGNOSTICS_LIST}>
                {scan.map((line, index) => (
                  <li key={`scan-${index}`}>{line}</li>
                ))}
              </ul>
            </>
          ) : null}
        </div>
      </section>
    </section>
  );
}
