// Status bar: coordinate readout, drafting-aid toggles (only the ones this build
// really has — unavailable aids stay visibly off), and the right-side utilities
// including the real clean-screen / command-line view toggles.
import React from 'react';
import { Icon } from '../icons.tsx';
import { Button, Toggle } from './primitives.tsx';
import { ParameterChoices, ParameterFlag, ParameterStepper, ParameterSwatch } from './ParameterControls.tsx';
import { useI18n, useShell } from '../context.ts';
import type { CadImportProgress } from '../cad/cad-status.ts';
import type { ToolParameterPending } from '../tools/tool-parameters.ts';
import { toolLabelKey } from '../commands/commands.ts';
import { palettesOffered } from '../cad/surface.ts';
import { OPTIONS_KEYS } from '../options/options.ts';
import { formatWorldCursor } from '../cad/coords.ts';
import { dispatchGrid, dispatchGridSnap, dispatchHoverPreview, dispatchObjectSnap, dispatchOrtho, gridSnapToggle, gridToggle, hoverPreviewToggle, objectSnapToggle, orthoToggle, snapKindLabelKey, toolStepPromptKey } from '../cad/cad.ts';
import { importProgressView } from '../cad/cad-status.ts';
import { toolParameterViews } from '../tools/tool-parameters.ts';

// The drafting aids this build has: GRID (display grid, F7), SNAP (grid snap,
// F9), OSNAP (object snap, F3) and ORTHO (orthographic constraint, F8). Each is a
// real session switch projected from `cad_status`; there is no disabled
// placeholder left, so no local list of "unsupported" aids exists any more.
//
// The active tool's parameter controls (see `ParameterControls.tsx`) render only
// while the owning tool is active with a real projected value, and they own no
// value of their own: a step or a typed commit is a dispatch, never a local mirror.

export function StatusBar() {
  const { t } = useI18n();
  const shell = useShell();
  // The drawing-coordinate readout (AutoCAD's habit): the host's real
  // `cursorWorld`, formatted as `X, Y` (plus `Z` when it is non-zero), or the
  // em-dash placeholder when there is no reliable 2D cursor. The DOM pointer's
  // CSS pixels are deliberately **not** a fallback — they are not drawing
  // coordinates, so they must never appear under the "coordinates" label.
  const worldCursor = formatWorldCursor(shell.status);
  const progress = importProgressView(shell.importProgress as CadImportProgress | null, t);
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
  // Grid snap is real host session state (`status.snapGridEnabled`, F9), never a
  // local mirror. The projected `snapSpacing` is the sysvar the quantiser uses;
  // the chip title reports it so the user can see what points snap to.
  const snapGrid = gridSnapToggle(shell.status, shell.canCommand('SNAP'));
  const snapGridTitle = snapGrid && snapGrid.spacing !== null
    ? `${t('status.toggle.snap')} (F9) \u00b7 ${t('status.toggle.snap_spacing', { spacing: snapGrid.spacing })}`
    : `${t('status.toggle.snap')} (F9)`;
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
  // composes rapid numeric steps without moving the displayed number. Each
  // control is keyed by its tool **and** its parameter, so switching tools
  // remounts it: a half-typed draft can never follow the user to another tool
  // that owns the same parameter key (TEXT and MTEXT share `textHeight`).
  const toolParameters = toolParameterViews(shell.status, shell.canCommand, shell.toolParameterPending as ToolParameterPending | null);
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
        title={t(worldCursor.hintKey)}
        aria-label={t('status.coordinates')}
      >
        <Icon name="snap" size={13} />
        <span className="mono">{worldCursor.text}</span>
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

      {/* The active tool's parameter rows. Ten HATCH rows do not fit beside the
          drafting chips and the right-hand utilities on a narrow window, so this
          is the one part of the bar that scrolls: a bounded `min-width` keeps it
          from pushing the utilities off the edge, and `overflow-x` reveals the
          rest instead of clipping it. Each control keeps its own 20px height and
          its own label, so the strip still reads as one line. */}
      <div className={toolParameters.length > 0 ? 'statusbar__params' : 'statusbar__params is-empty'}>
        {toolParameters.map(view => {
          if (view.kind === 'flag') return <ParameterFlag key={`${tool?.key}:${view.key}`} view={view} onToggle={shell.toggleToolParameter} />;
          // A closed set with catalog names (HATCH's predefined pattern, its gradient
          // kind) renders the same row as a named chooser instead of a stepper: the
          // wire value is still the integer code, but the user picks a name.
          if (view.choices) return <ParameterChoices key={`${tool?.key}:${view.key}`} view={view} onChoose={shell.stepToolParameter} />;
          // A colour row (HATCH's two gradient stops) is the same numeric row shown as
          // a swatch: the integer ACI index still travels the one parameter channel.
          if (view.swatch) return <ParameterSwatch key={`${tool?.key}:${view.key}`} view={view} onChoose={shell.stepToolParameter} />;
          return <ParameterStepper key={`${tool?.key}:${view.key}`} view={view} onStep={shell.stepToolParameter} />;
        })}
      </div>

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
        {snapGrid ? (
          <Toggle
            label={t('status.toggle.snap')}
            on={snapGrid.on}
            disabled={snapGrid.disabled}
            title={snapGridTitle}
            onClick={() => dispatchGridSnap(shell.runCommand)}
          />
        ) : null}
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
