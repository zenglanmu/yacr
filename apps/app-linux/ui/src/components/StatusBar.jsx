// Status bar: coordinate readout, drafting-aid toggles (only the ones this build
// really has — unavailable aids stay visibly off), and the right-side utilities
// including the real clean-screen / command-line view toggles.
import React from 'react';
import { Icon } from '../icons.jsx';
import { useI18n, useShell } from '../context.mjs';
import { commandById, toolLabelKey } from '../commands.mjs';
import { dispatchHoverPreview, dispatchObjectSnap, hoverPreviewToggle, importProgressView, objectSnapToggle, polygonSidesView, snapKindLabelKey } from '../cad.mjs';

// The drafting aids this build still has only as placeholders. OSNAP is a real
// toggle (ADR 0014 A.4) and is rendered separately below.
const DRAFTING = ['GRID', 'SNAP', 'ORTHO'];

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

// The POLYGON side-count stepper. It is a pure view over the host value (no
// local mirror): the rendered number is always the one the last `cad_status`
// carried, and a step only dispatches the change. The effective target — the
// status value plus the bounded pending delta of steps the host has not echoed
// back yet — drives the `-`/`+` targets and the disabled state, so rapid clicks
// compose and the buttons stop at the 3..=1024 bounds and the pending cap while
// the host is not admitting commands. Rendered only while POLYGON is the active
// tool with a real value — nothing at all otherwise.
function PolygonSides({ view, onStep }) {
  const { t } = useI18n();
  const stepDown = view.disabled || !view.canDecrement;
  const stepUp = view.disabled || !view.canIncrement;
  return (
    <span
      className={['status-stepper', view.disabled ? 'is-disabled' : ''].filter(Boolean).join(' ')}
      role="group"
      aria-label={t('status.polygon_sides')}
      title={t('status.polygon_sides.hint')}
    >
      <Icon name="polygon" size={12} />
      <span className="status-stepper__label" aria-hidden="true">{t('status.polygon_sides')}</span>
      <button
        type="button"
        className="status-stepper__button"
        aria-label={t('status.polygon_sides.decrease')}
        title={t('status.polygon_sides.decrease')}
        disabled={stepDown}
        onClick={stepDown ? undefined : () => onStep(view.decrement)}
      >
        <Icon name="minus" size={11} />
      </button>
      <span className="status-stepper__value mono">{view.sides}</span>
      <button
        type="button"
        className="status-stepper__button"
        aria-label={t('status.polygon_sides.increase')}
        title={t('status.polygon_sides.increase')}
        disabled={stepUp}
        onClick={stepUp ? undefined : () => onStep(view.increment)}
      >
        <Icon name="plus" size={11} />
      </button>
    </span>
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
  // The current object-snap candidate kind (ADR 0014 A.7) is real host state
  // (`status.snapKind`). Without a candidate the key is null, so nothing extra
  // is rendered — never a placeholder chip.
  const snapKindKey = snapKindLabelKey(shell.status);
  // The POLYGON side count is a real tool parameter (`status.tool.params.sides`),
  // never a local mirror. It exists only while POLYGON is the active tool with a
  // real in-range value; any other tool or a missing/`null` field renders nothing
  // at all. Admission mirrors the toggle chips (`canCommand`). The bounded
  // pending delta composes rapid steps without moving the displayed number.
  const polygonSides = polygonSidesView(shell.status, shell.canCommand('POLYGON'), shell.polygonSidesPending);
  // Announce only the phase here (rare changes); the steadily-incrementing
  // count/bytes stay in the document-tab bar, so a polite live region never
  // chatters on every tick.
  const stateLabel = shell.error ? t('diagnostics.severity.error')
    : shell.pollError ? t('diagnostics.severity.error')
      : progress ? progress.phase
        : shell.loading ? t('import.panel')
          : shell.documentOpen ? t('status.ready') : t('plot.no_document');
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
          <span className="mono">
            {toolOpenEnded ? t('tool.open_ended') : t('tool.points', { count: toolPoints })}
          </span>
        </span>
      ) : null}

      {polygonSides ? (
        <PolygonSides view={polygonSides} onStep={target => shell.stepPolygonSides(target)} />
      ) : null}

      <div className="statusbar__group" role="group" aria-label={t('status.drafting')}>
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
        <button type="button" className="statusbar__workspace" disabled title={t('status.workspace')}>
          <Icon name="settings" size={13} />
          {t('status.workspace')}
        </button>
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
      </div>
    </div>
  );
}
