// Status bar: coordinate readout, drafting-aid toggles (only the ones this build
// really has — unavailable aids stay visibly off), and the right-side utilities
// including the real clean-screen / command-line view toggles.
import React from 'react';
import { Icon } from '../icons.jsx';
import { Button } from './primitives.jsx';
import { useI18n, useShell } from '../context.mjs';
import { commandById, toolLabelKey } from '../commands.mjs';
import { palettesOffered } from '../surface.mjs';
import { OPTIONS_KEYS } from '../options.mjs';
import { dispatchGrid, dispatchHoverPreview, dispatchObjectSnap, dispatchOrtho, gridToggle, hoverPreviewToggle, importProgressView, objectSnapToggle, orthoToggle, snapKindLabelKey, toolParameterViews, toolStepPromptKey } from '../cad.mjs';

// The drafting aids this build still has only as placeholders. GRID (display
// grid, F7), OSNAP (object snap, F3) and ORTHO (orthographic constraint, F8) are
// real toggles and are rendered separately below. SNAP (grid snap, F9) is a
// different, unimplemented feature and stays a visibly disabled placeholder.
const DRAFTING = ['SNAP'];

function Toggle({ label, on, onClick, title, disabled }) {
  return (
    <button
      type="button"
      className={['status-toggle', on ? 'is-on' : '', disabled ? 'is-disabled' : ''].filter(Boolean).join(' ')}
      aria-pressed={Boolean(on)}
      disabled={disabled}
      title={title}
      onClick={disabled ? undefined : onClick}
    >
      {label}
    </button>
  );
}

// The bounded numeric stepper for one active-tool parameter. It is a pure view
// over the host value (no local mirror): the rendered number is always the one
// the last `cad_status` carried, and a step only dispatches the change. The
// effective target — the status value plus the bounded pending delta of steps
// the host has not echoed back yet — drives the `-`/`+` targets and the disabled
// state, so rapid clicks compose and the buttons stop at the parameter's own
// bounds (and the pending cap) while the host is not admitting commands. Rendered
// only while the owning tool is active with a real projected value — nothing at
// all otherwise.
function ParameterStepper({ view, onStep }) {
  const { t } = useI18n();
  const name = t(view.labelKey);
  const stepDown = view.disabled || !view.canDecrement;
  const stepUp = view.disabled || !view.canIncrement;
  return (
    <span
      className={['status-stepper', view.disabled ? 'is-disabled' : ''].filter(Boolean).join(' ')}
      role="group"
      aria-label={name}
      title={t(view.hintKey)}
    >
      {view.icon ? <Icon name={view.icon} size={12} /> : null}
      <span className="status-stepper__label" aria-hidden="true">{name}</span>
      <button
        type="button"
        className="status-stepper__button"
        aria-label={t('status.tool_parameter.decrease', { name })}
        title={t('status.tool_parameter.decrease', { name })}
        disabled={stepDown}
        onClick={stepDown ? undefined : () => onStep(view.key, view.decrement)}
      >
        <Icon name="minus" size={11} />
      </button>
      <span className="status-stepper__value mono">{view.text}</span>
      <button
        type="button"
        className="status-stepper__button"
        aria-label={t('status.tool_parameter.increase', { name })}
        title={t('status.tool_parameter.increase', { name })}
        disabled={stepUp}
        onClick={stepUp ? undefined : () => onStep(view.key, view.increment)}
      >
        <Icon name="plus" size={11} />
      </button>
    </span>
  );
}

// The toggle chip for one active-tool flag parameter, using the same status-bar
// chip vocabulary as OSNAP / hover preview. It dispatches the *opposite* of the
// value the last `cad_status` carried — never a fabricated one.
function ParameterFlag({ view, onToggle }) {
  const { t } = useI18n();
  return (
    <Toggle
      label={t(view.labelKey)}
      on={view.value}
      disabled={view.disabled}
      title={t(view.hintKey)}
      onClick={() => onToggle(view.key, view.next)}
    />
  );
}

export function StatusBar() {
  const { t } = useI18n();
  const shell = useShell();
  const cursor = shell.cursor;
  const coordinate = cursor ? `${cursor.x.toFixed(0)}, ${cursor.y.toFixed(0)}` : '\u2014, \u2014';
  const progress = importProgressView(shell.importProgress, t);
  // The interactive tool the host reports as active (`status.tool`:
  // `{key, points, ready} | null`). A missing/null block means no tool, so
  // nothing extra is rendered — never a placeholder chip.
  const tool = shell.status?.tool ?? null;
  const toolNameKey = tool ? toolLabelKey(tool.key) : null;
  const toolPoints = tool && Number.isSafeInteger(tool.points) ? tool.points : 0;
  // The next expected pick for a tool whose steps have names (the dimension
  // family). `null` for every other tool, so its chip is unchanged. Derived from
  // the host's real `tool.key` + captured count, never a local guess.
  const toolStepKey = tool ? toolStepPromptKey(tool) : null;
  // Whether the host reports this capture as open-ended (`status.tool.openEnded`,
  // ADR 0015 decision B). Read from the status only — never inferred from the
  // point count — so an open tool shows the honest "more points accepted, Enter
  // finishes" copy instead of a numeric count that implies a fixed complement.
  const toolOpenEnded = tool?.openEnded === true;
  // Idle hover preview is real host state (`status.hoverPreview`), never a local
  // mirror. An older payload without the field renders no chip at all.
  const hoverPreview = hoverPreviewToggle(shell.status, shell.canCommand('HOVERPREVIEW'));
  // The object-snap master switch is real host session state (`status.snapEnabled`),
  // never a local mirror (ADR 0014 A.4). An older payload without the field renders
  // no chip at all, exactly like the hover-preview chip above.
  const objectSnap = objectSnapToggle(shell.status, shell.canCommand('OSNAP'));
  // The display-grid switch is real host session state (`status.gridEnabled`),
  // never a local mirror (`GRID`, F7). Turning it on does not by itself draw the
  // grid: the host also needs the persisted `view.overlays.grid` preference.
  const grid = gridToggle(shell.status, shell.canCommand('GRID'));
  // The orthographic drafting constraint is real host session state
  // (`status.orthoEnabled`), never a local mirror (`ORTHO`, F8). While on, the
  // host constrains the next point entry from the previous point; it is not grid
  // snap (F9) and never moves a point in Z.
  const ortho = orthoToggle(shell.status, shell.canCommand('ORTHO'));
  // The current object-snap candidate kind (ADR 0014 A.7) is real host state
  // (`status.snapKind`). Without a candidate the key is null, so nothing extra
  // is rendered — never a placeholder chip.
  const snapKindKey = snapKindLabelKey(shell.status);
  // The active tool's typed parameters (`status.tool.params`), never a local
  // mirror: only the ones with a real projected value render, so a tool that
  // projects none shows no control at all. Admission mirrors the toggle chips
  // (`canCommand` on the owning tool's command). The bounded pending delta
  // composes rapid numeric steps without moving the displayed number.
  const toolParameters = toolParameterViews(shell.status, shell.canCommand, shell.toolParameterPending);
  // Announce only the phase here (rare changes); the steadily-incrementing
  // count/bytes stay in the document-tab bar, so a polite live region never
  // chatters on every tick.
  const stateLabel = shell.error ? t('diagnostics.severity.error')
    : shell.pollError ? t('diagnostics.severity.error')
      : progress ? progress.phase
        : shell.loading ? t('import.panel')
          : shell.documentVisible ? t('status.ready') : t('plot.no_document');
  return (
    <div className="region region--status statusbar app-region" role="contentinfo">
      <output
        className="statusbar__coords"
        title={t('status.coordinate_hint')}
        aria-label={t('status.coordinates')}
      >
        <Icon name="snap" size={13} />
        <span className="mono">{coordinate}</span>
      </output>

      <span className="statusbar__state" aria-live="polite">{stateLabel}</span>

      {tool && toolNameKey ? (
        <span
          className="statusbar__tool"
          title={`${t('tool.prompt.confirm')} (Enter) \u00b7 ${t('tool.prompt.cancel')} (Esc)`}
        >
          <Icon name="snap" size={13} />
          {t(toolNameKey)}
          {toolStepKey ? (
            <span className="statusbar__tool-step">{t(toolStepKey)}</span>
          ) : null}
          <span className="mono">
            {toolOpenEnded ? t('tool.open_ended') : t('tool.points', { count: toolPoints })}
          </span>
        </span>
      ) : null}

      {toolParameters.map(view => (view.kind === 'flag'
        ? <ParameterFlag key={view.key} view={view} onToggle={shell.toggleToolParameter} />
        : <ParameterStepper key={view.key} view={view} onStep={shell.stepToolParameter} />))}

      <div className="statusbar__group" role="group" aria-label={t('status.drafting')}>
        {grid ? (
          <Toggle
            label={t('status.toggle.grid')}
            on={grid.on}
            disabled={grid.disabled}
            title={`${t('status.toggle.grid')} (F7)`}
            onClick={() => dispatchGrid(shell.runCommand)}
          />
        ) : null}
        {ortho ? (
          <Toggle
            label={t('status.toggle.ortho')}
            on={ortho.on}
            disabled={ortho.disabled}
            title={`${t('status.toggle.ortho')} (F8)`}
            onClick={() => dispatchOrtho(shell.runCommand)}
          />
        ) : null}
        {DRAFTING.map(id => {
          const command = commandById(id);
          const name = t(command.labelKey);
          return (
            <Toggle
              key={id}
              label={name}
              on={false}
              disabled
              title={t('status.toggle_unsupported', { name })}
            />
          );
        })}
        {objectSnap ? (
          <Toggle
            label={t('status.toggle.osnap')}
            on={objectSnap.on}
            disabled={objectSnap.disabled}
            title={`${t('status.toggle.osnap')} (F3)`}
            onClick={() => dispatchObjectSnap(shell.runCommand)}
          />
        ) : null}
        {snapKindKey ? (
          <span className="statusbar__snap-kind" title={t('overlay.snap_hints')}>
            {t(snapKindKey)}
          </span>
        ) : null}
        {hoverPreview ? (
          <Toggle
            label={t('status.toggle.hover_preview')}
            on={hoverPreview.on}
            disabled={hoverPreview.disabled}
            title={t(hoverPreview.on ? 'status.toggle.hover_preview_on' : 'status.toggle.hover_preview_off')}
            onClick={() => dispatchHoverPreview(shell.runCommand)}
          />
        ) : null}
      </div>

      <span className="statusbar__spacer" />

      {shell.errorText ? (
        <span className="statusbar__error" role="alert" title={shell.errorText}>
          <Icon name="alert" size={13} />
          {shell.errorText}
        </span>
      ) : null}

      <div className="statusbar__group statusbar__group--end">
        {/* The gear opens the real Options window (the same host-owned popup the
            File ribbon entry and the typed `OPTIONS` selector open). */}
        <Button
          className="statusbar__workspace"
          variant="ghost"
          size="sm"
          icon="settings"
          iconSize={13}
          title={`${t(OPTIONS_KEYS.title)} (OPTIONS)`}
          onClick={shell.openOptionsPopup}
        >
          {t(OPTIONS_KEYS.title)}
        </Button>
        <Toggle
          label={t('status.command_line')}
          on={shell.commandLineOpen}
          title={`${t('status.command_line')} (Ctrl+9)`}
          onClick={() => shell.setCommandLineOpen(!shell.commandLineOpen)}
        />
        <Toggle
          label={t('status.clean_screen')}
          on={shell.cleanScreen}
          title={`${t('status.clean_screen')} (Ctrl+0)`}
          onClick={() => shell.setCleanScreen(!shell.cleanScreen)}
        />
        {/* A `dom-canvas` carrier has no host-side document state for these two
            palettes to project, so their toggles are not offered at all — an
            absent toggle, never a chip that opens an empty panel. The carrier rule
            is the shared `palettesOffered` helper (the docked panels behind these
            chips ask the same one). Any other carrier (and an older chrome state
            without the flag) keeps them. */}
        {palettesOffered(shell) ? (
          <>
            <Toggle
              label={t('properties.panel')}
              on={shell.propertiesOpen}
              title={t('properties.panel')}
              onClick={() => shell.setPropertiesOpen(!shell.propertiesOpen)}
            />
            <Toggle
              label={t('diagnostics.title')}
              on={shell.diagnosticsOpen}
              title={t('diagnostics.title')}
              onClick={() => shell.setDiagnosticsOpen(!shell.diagnosticsOpen)}
            />
          </>
        ) : null}
      </div>
    </div>
  );
}
