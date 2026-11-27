// The right default-tray slot.
//
// Each panel names a **built-in body** (`TrayPanel.kind`); a plugin can add a
// declarative `pending` panel with its own sentence, but it cannot inject React.
// That keeps the tray's writes reviewable while still giving extensions a place
// to appear.
//
// The bodies are deliberately read-only or launchers:
//
//   * Entity Info shows the host's real `cad_status.selection` rows and points
//     at the full Properties palette for editing;
//   * Components shows `cad_status.blocks` (name + member count);
//   * Styles hosts the one real appearance switch (the host theme);
//   * Tags opens the existing Tags/Layers palette (`LAYER`), which owns the
//     list and the writes;
//   * the mechanical tree reads the host's `cad_mechanical_tree` projection
//     (features then bodies) and never derives geometry itself;
//   * Instructor names the active tool and its real next step;
//   * `pending` names the missing panel instead of drawing an empty one.
import React, { useEffect, useRef, useSyncExternalStore, useState } from 'react';
import { useI18n, useShell } from '../../context.ts';
import { Icon } from '../../icons.tsx';
import { invoke } from '../../host/invoke.ts';
import { blocksOf } from '../../cad/cad-status.ts';
import { componentDefineCommand, componentInsertCommand } from '../concept/components.ts';
import { toolLabelKey } from '../../commands/commands.ts';
import { toolStepPromptKey } from '../../cad/cad.ts';
import { propertyRowLabel } from '../../properties/property-labels.ts';
import { ThemeToggle } from '../../components/ThemeToggle.tsx';
import { BIM_ACTION } from '../../ribbon/bim-panel.ts';
import { currentDomain, domainStoreSnapshot, subscribeDomainStore, toggleTray } from '../store.ts';
import { useDomainUi } from './use-ui-layout.ts';
import { bodyKindKey, featureOperationKey, mechanicalStatusKey, readMechanicalTree } from '../mechanical/tree.ts';
import type { MechanicalTreeAnswer } from '../mechanical/tree.ts';
import type { TrayPanel } from './slots.ts';
import { MatchPhotoBody, OverlaysBody } from './tray-panels.tsx';
import { BUILTIN_MATERIALS, paintCommand } from '../concept/materials.ts';
import { readScenes } from '../concept/scenes.ts';
import type { ScenesAnswer } from '../concept/scenes.ts';
import { readDisplay } from '../concept/display.ts';
import type { DisplayAnswer } from '../concept/display.ts';
import { outlinerRowKey, readOutliner, visibleOutlinerRows } from '../concept/outliner.ts';
import type { OutlinerAnswer, OutlinerRow } from '../concept/outliner.ts';
import { selectBimEntities } from '../../bim/selection.ts';

function EntityInfoBody() {
  const { t } = useI18n();
  const shell = useShell();
  const selection = shell.status?.selection;
  if (!selection || selection.empty) {
    return <p className="concept-panel__hint">{t('concept.entity_info.empty')}</p>;
  }
  return (
    <>
      <p className="concept-panel__hint">{t('concept.entity_info.count', { count: selection.count })}</p>
      <ul className="concept-rows">
        {selection.rows.slice(0, 10).map((row: { readonly key: string; readonly value: string | null; readonly mixed: boolean }) => (
          <li key={row.key}>
            <span className="concept-rows__label">{propertyRowLabel(t, row.key)}</span>
            <span className="concept-rows__value" title={row.value ?? undefined}>
              {row.mixed || row.value === null ? '\u2014' : row.value}
            </span>
          </li>
        ))}
      </ul>
      <button type="button" className="concept-panel__action" onClick={() => shell.runAction('properties')}>
        {t('concept.entity_info.open')}
      </button>
    </>
  );
}

function ComponentsBody() {
  const { t } = useI18n();
  const shell = useShell();
  const [name, setName] = useState('');
  const documentLabel = shell.status?.documentLabel ?? null;
  const blocks = blocksOf(shell.status);
  if (documentLabel === null) {
    return <p className="concept-panel__hint">{t('concept.components.no_document')}</p>;
  }
  const defineCommand = componentDefineCommand(name);
  const canDefine = defineCommand !== null && shell.canCommand('BLOCK');
  const canInsert = shell.canCommand('INSERT');
  return (
    <>
      <p className="concept-panel__hint">{t('concept.components.hint')}</p>
      <div className="concept-scenes__save">
        <input
          type="text"
          className="concept-scenes__name"
          value={name}
          placeholder={t('concept.components.name')}
          aria-label={t('concept.components.name')}
          onChange={event => setName(event.currentTarget.value)}
        />
        <button
          type="button"
          className="concept-panel__action"
          disabled={!canDefine}
          onClick={() => {
            if (defineCommand !== null) {
              shell.runCommand(defineCommand);
              setName('');
            }
          }}
        >
          {t('concept.components.create')}
        </button>
      </div>
      {blocks.length === 0 ? (
        <p className="concept-panel__hint">{t('concept.components.empty')}</p>
      ) : (
        <ul className="concept-rows">
          {blocks.slice(0, 12).map((block, index) => {
            const command = componentInsertCommand(block.name);
            return (
              <li key={`${block.name ?? 'unnamed'}-${index}`}>
                <span className="concept-rows__label">
                  {block.name ?? t('classicinsert.unnamed')}
                </span>
                <span className="concept-rows__value">
                  {t('concept.components.members', { count: block.memberCount })}
                </span>
                <button
                  type="button"
                  className="concept-panel__action"
                  title={t('concept.components.insert')}
                  disabled={command === null || !canInsert}
                  onClick={() => {
                    if (command !== null) shell.runCommand(command);
                  }}
                >
                  {t('concept.components.insert')}
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </>
  );
}

function StylesBody() {
  const { t } = useI18n();
  return (
    <div className="concept-panel__styles">
      <p className="concept-panel__hint">{t('concept.styles.theme')}</p>
      <ThemeToggle />
    </div>
  );
}

function TagsBody() {
  const { t } = useI18n();
  const shell = useShell();
  const available = shell.canCommand('LAYER');
  return (
    <>
      <p className="concept-panel__hint">{t('concept.tags.hint')}</p>
      <button
        type="button"
        className="concept-panel__action"
        disabled={!available}
        title={available ? undefined : t('concept.tags.no_document')}
        onClick={() => shell.runCommand('LAYER')}
      >
        {t('concept.tags.open')}
      </button>
    </>
  );
}

function InstructorBody() {
  const { t } = useI18n();
  const shell = useShell();
  const tool = shell.status?.tool ?? null;
  if (!tool) return <p className="concept-panel__hint">{t('concept.instructor.idle')}</p>;
  const step = toolStepPromptKey(tool);
  const nameKey = toolLabelKey(tool.key);
  return (
    <>
      <p className="concept-panel__name">{t(nameKey ?? 'concept.instructor.active')}</p>
      <p className="concept-panel__hint">{step ? t(step) : t('concept.instructor.active')}</p>
    </>
  );
}

function BimQueryBody() {
  const { t } = useI18n();
  const shell = useShell();
  return (
    <>
      <p className="concept-panel__hint">{t('bim.tray.query_hint')}</p>
      <button type="button" className="concept-panel__action" onClick={() => shell.runAction(BIM_ACTION)}>
        {t('bim.panel')}
      </button>
    </>
  );
}

/** One tree read's state; `tree` is kept across a failed refresh so the last
 * good projection stays visible (never silently blanked). */
type TreeState =
  | { readonly phase: 'ready'; readonly tree: MechanicalTreeAnswer | null; readonly reason: '' }
  | { readonly phase: 'loading'; readonly tree: MechanicalTreeAnswer | null; readonly reason: '' }
  | { readonly phase: 'error'; readonly tree: MechanicalTreeAnswer | null; readonly reason: string };

/**
 * The FeatureManager design tree.
 *
 * It reads the host's `cad_mechanical_tree` projection — never the drawing
 * itself. A row is labelled from the catalog when the build knows its token and
 * shown raw otherwise; a feature's `schema_only` status is the honest statement
 * that this build stores feature definitions but has no evaluator (ADR-0043 P2).
 * The read refreshes when the document changes, when the entity count changes
 * and after each command, plus the explicit refresh button.
 */
function MechanicalTreeBody() {
  const { t } = useI18n();
  const shell = useShell();
  const documentLabel = shell.status?.documentLabel ?? null;
  const entityCount = shell.status?.entityCount ?? null;
  const commandCount = shell.status?.commandLog?.length ?? 0;
  const [nonce, setNonce] = useState(0);
  const [state, setState] = useState<TreeState>({ phase: 'ready', tree: null, reason: '' });
  // A tree belongs to one document: a refresh keeps the last good projection,
  // but another document must not show the previous one's rows while loading.
  const labelRef = useRef<string | null>(null);

  useEffect(() => {
    const sameDocument = labelRef.current === documentLabel;
    labelRef.current = documentLabel;
    if (documentLabel === null) {
      setState({ phase: 'ready', tree: null, reason: '' });
      return undefined;
    }
    let live = true;
    setState(previous => ({
      phase: 'loading',
      tree: sameDocument ? previous.tree : null,
      reason: '',
    }));
    readMechanicalTree(invoke).then(tree => {
      if (live) setState({ phase: 'ready', tree, reason: '' });
    }).catch((error: unknown) => {
      if (!live) return;
      const reason = error instanceof Error ? error.message : String(error);
      setState(previous => ({ phase: 'error', tree: previous.tree, reason }));
    });
    return () => { live = false; };
  }, [documentLabel, entityCount, commandCount, nonce]);

  if (documentLabel === null) {
    return <p className="concept-panel__hint">{t('mechanical.tree.empty')}</p>;
  }

  const tree = state.tree;
  const rows = tree?.rows ?? [];
  return (
    <>
      <div className="concept-panel__styles">
        <button type="button" className="concept-panel__action" onClick={() => setNonce(value => value + 1)}>
          {t('mechanical.tree.refresh')}
        </button>
      </div>
      {state.phase === 'error' ? (
        <p className="concept-panel__hint" role="alert">{t('mechanical.tree.error', { reason: state.reason })}</p>
      ) : null}
      {rows.length === 0 ? (
        <p className="concept-panel__hint">
          {state.phase === 'loading' ? t('mechanical.tree.loading') : t('mechanical.tree.no_rows')}
        </p>
      ) : (
        <ul className="concept-rows">
          {rows.map(row => {
            const labelKey = row.kind === 'feature' ? featureOperationKey(row.token) : row.kind === 'body' ? bodyKindKey(row.token) : null;
            const statusKey = mechanicalStatusKey(row.status);
            return (
              <li key={`${row.kind}:${row.id}`}>
                <span className="concept-rows__label">{labelKey === null ? row.token : t(labelKey)}</span>
                <span className="concept-rows__value" title={statusKey === null ? row.status : undefined}>
                  {statusKey === null ? row.status : t(statusKey)}
                </span>
              </li>
            );
          })}
        </ul>
      )}
      {tree !== null && tree.total > rows.length ? (
        <p className="concept-panel__hint">{t('mechanical.tree.truncated', { shown: rows.length, total: tree.total })}</p>
      ) : null}
    </>
  );
}

/**
 * The built-in material palette: one swatch per flat material.
 *
 * A click dispatches `PAINT <name>` through the ordinary command path, so the
 * paint applies to the **current selection** (the command owns every refusal).
 * Document materials are listed by the `MATERIALS` report this panel can open;
 * the swatches are the built-in palette, not a copy of the document table.
 */
function MaterialsBody() {
  const { t } = useI18n();
  const shell = useShell();
  const available = shell.canCommand('PAINT');
  return (
    <>
      <p className="concept-panel__hint">{t('concept.materials.hint')}</p>
      <ul className="concept-materials">
        {BUILTIN_MATERIALS.map(material => (
          <li key={material.name}>
            <button
              type="button"
              className="concept-materials__swatch"
              style={{ backgroundColor: material.color, opacity: 1 - (material.transparency ?? 0) }}
              disabled={!available}
              title={available ? t('concept.materials.paint', { name: material.name }) : t('concept.materials.no_document')}
              aria-label={material.name}
              onClick={() => shell.runCommand(paintCommand(material.name))}
            >
              <span>{material.name}</span>
            </button>
          </li>
        ))}
      </ul>
      <button
        type="button"
        className="concept-panel__action"
        disabled={!shell.canCommand('MATERIALS')}
        onClick={() => shell.runCommand('MATERIALS')}
      >
        {t('concept.materials.report')}
      </button>
    </>
  );
}

/** One scenes read's state; the last good answer survives a failed refresh. */
type ScenesState =
  | { readonly phase: 'ready'; readonly answer: ScenesAnswer | null; readonly reason: '' }
  | { readonly phase: 'loading'; readonly answer: ScenesAnswer | null; readonly reason: '' }
  | { readonly phase: 'error'; readonly answer: ScenesAnswer | null; readonly reason: string };

/**
 * The Scenes panel (SketchUp pages).
 *
 * Rows come from the host's `cad_scenes` read of `tables/scenes.json`; a row
 * click dispatches `SCENE RESTORE <name>` and the delete button
 * `SCENE DELETE <name>`, so every write goes through the ordinary command path
 * and the panel never mutates the document itself. Saving captures the current
 * camera/mode/space/style with `SCENE SAVE <name>`.
 */
function ScenesBody() {
  const { t } = useI18n();
  const shell = useShell();
  const documentLabel = shell.status?.documentLabel ?? null;
  const entityCount = shell.status?.entityCount ?? null;
  const commandCount = shell.status?.commandLog?.length ?? 0;
  const [name, setName] = useState('');
  const [nonce, setNonce] = useState(0);
  const [state, setState] = useState<ScenesState>({ phase: 'ready', answer: null, reason: '' });
  useEffect(() => {
    if (documentLabel === null) {
      setState({ phase: 'ready', answer: null, reason: '' });
      return undefined;
    }
    let live = true;
    setState(previous => ({ phase: 'loading', answer: previous.answer, reason: '' }));
    readScenes(invoke)
      .then(answer => {
        if (live) setState({ phase: 'ready', answer, reason: '' });
      })
      .catch((error: unknown) => {
        if (!live) return;
        const reason = error instanceof Error ? error.message : String(error);
        setState(previous => ({ phase: 'error', answer: previous.answer, reason }));
      });
    return () => { live = false; };
  }, [documentLabel, entityCount, commandCount, nonce]);

  if (documentLabel === null) {
    return <p className="concept-panel__hint">{t('concept.scenes.no_document')}</p>;
  }
  const scenes = state.answer?.scenes ?? [];
  const trimmed = name.trim();
  const canSave = trimmed !== '' && shell.canCommand('SCENE');
  return (
    <>
      <p className="concept-panel__hint">{t('concept.scenes.hint')}</p>
      <div className="concept-scenes__save">
        <input
          type="text"
          className="concept-scenes__name"
          value={name}
          placeholder={t('concept.scenes.name')}
          aria-label={t('concept.scenes.name')}
          onChange={event => setName(event.currentTarget.value)}
        />
        <button
          type="button"
          className="concept-panel__action"
          disabled={!canSave}
          onClick={() => {
            shell.runCommand(`SCENE SAVE ${trimmed}`);
            setName('');
            setNonce(value => value + 1);
          }}
        >
          {t('concept.scenes.save')}
        </button>
      </div>
      {state.phase === 'error' ? (
        <p className="concept-panel__hint" role="alert">
          {t('concept.scenes.error', { reason: state.reason })}
        </p>
      ) : null}
      {scenes.length === 0 ? (
        <p className="concept-panel__hint">
          {state.phase === 'loading' ? t('concept.scenes.loading') : t('concept.scenes.empty')}
        </p>
      ) : (
        <ul className="concept-rows">
          {scenes.map(scene => (
            <li key={scene.name}>
              <button
                type="button"
                className="concept-rows__label concept-scenes__restore"
                title={t('concept.scenes.restore')}
                onClick={() => shell.runCommand(`SCENE RESTORE ${scene.name}`)}
              >
                {scene.name}
              </button>
              <span className="concept-rows__value">
                {scene.space} · {scene.style}
                {scene.is3d ? ' · 3D' : ''}
              </span>
              <button
                type="button"
                className="concept-panel__action"
                title={t('concept.scenes.delete')}
                onClick={() => {
                  shell.runCommand(`SCENE DELETE ${scene.name}`);
                  setNonce(value => value + 1);
                }}
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      )}
      {state.answer !== null && state.answer.truncated ? (
        <p className="concept-panel__hint">
          {t('concept.scenes.truncated', {
            shown: scenes.length,
            total: state.answer.total,
          })}
        </p>
      ) : null}
    </>
  );
}

/**
 * The Soften Edges panel.
 *
 * SketchUp's dialog equivalent: one angle (degrees) applied to the current
 * selection with `SOFTENEDGES <angle>`. All semantics live in the command (the
 * selection must be 3D bodies, the range is `0..=180`, one undo step); the panel
 * only validates the input enough to disable a meaningless click.
 */

/**
 * The Shadows and Fog panels (one body each, one shared document record).
 *
 * Both write through the ordinary command path (`SHADOWS ...`, `FOG ...`); the
 * panel only validates inputs enough to disable a meaningless click, and the
 * command re-validates everything. The Shadows hint names the real boundary:
 * cast shadows are not implemented, this panel moves the shading sun.
 */
function DisplayBody({ part }: { readonly part: 'shadows' | 'fog' }) {
  const { t } = useI18n();
  const shell = useShell();
  const documentLabel = shell.status?.documentLabel ?? null;
  const entityCount = shell.status?.entityCount ?? null;
  const commandCount = shell.status?.commandLog?.length ?? 0;
  const [nonce, setNonce] = useState(0);
  const [state, setState] = useState<{
    phase: 'ready' | 'loading' | 'error';
    answer: DisplayAnswer | null;
    reason: string;
  }>({ phase: 'ready', answer: null, reason: '' });
  const [azimuth, setAzimuth] = useState<string | null>(null);
  const [elevation, setElevation] = useState<string | null>(null);
  const [start, setStart] = useState<string | null>(null);
  const [end, setEnd] = useState<string | null>(null);
  useEffect(() => {
    if (documentLabel === null) {
      setState({ phase: 'ready', answer: null, reason: '' });
      return undefined;
    }
    let live = true;
    setState(previous => ({ phase: 'loading', answer: previous.answer, reason: '' }));
    readDisplay(invoke)
      .then(answer => {
        if (live) setState({ phase: 'ready', answer, reason: '' });
      })
      .catch((error: unknown) => {
        if (!live) return;
        const reason = error instanceof Error ? error.message : String(error);
        setState(previous => ({ phase: 'error', answer: previous.answer, reason }));
      });
    return () => { live = false; };
  }, [documentLabel, entityCount, commandCount, nonce]);

  if (documentLabel === null) {
    return (
      <p className="concept-panel__hint">
        {t(part === 'shadows' ? 'concept.shadows.no_document' : 'concept.fog.no_document')}
      </p>
    );
  }
  const answer = state.answer;
  const refresh = () => setNonce(value => value + 1);
  const failure = state.phase === 'error' ? (
    <p className="concept-panel__hint" role="alert">
      {t('concept.display.error', { reason: state.reason })}
    </p>
  ) : null;

  if (part === 'shadows') {
    const enabled = answer?.shadows.enabled === true;
    const currentAzimuth = answer?.shadows.azimuth ?? 135;
    const currentElevation = answer?.shadows.elevation ?? 45;
    const azimuthText = azimuth ?? String(currentAzimuth);
    const elevationText = elevation ?? String(currentElevation);
    const azimuthValue = Number(azimuthText);
    const elevationValue = Number(elevationText);
    const valid = azimuthText.trim() !== '' && elevationText.trim() !== ''
      && Number.isFinite(azimuthValue) && azimuthValue >= 0 && azimuthValue < 360
      && Number.isFinite(elevationValue) && elevationValue >= 0 && elevationValue <= 90;
    const available = shell.canCommand('SHADOWS');
    return (
      <>
        <p className="concept-panel__hint">{t('concept.shadows.hint')}</p>
        <label className="concept-panel__hint">
          <input
            type="checkbox"
            checked={enabled}
            disabled={!available}
            onChange={event => {
              shell.runCommand(`SHADOWS ${event.currentTarget.checked ? 'ON' : 'OFF'}`);
              refresh();
            }}
          />{' '}
          {t('concept.shadows.display')}
        </label>
        <div className="concept-scenes__save">
          <input
            type="number"
            min={0}
            max={359.9}
            step={1}
            className="concept-scenes__name"
            value={azimuthText}
            aria-label={t('concept.shadows.azimuth')}
            onChange={event => setAzimuth(event.currentTarget.value)}
          />
          <input
            type="number"
            min={0}
            max={90}
            step={1}
            className="concept-scenes__name"
            value={elevationText}
            aria-label={t('concept.shadows.elevation')}
            onChange={event => setElevation(event.currentTarget.value)}
          />
          <button
            type="button"
            className="concept-panel__action"
            disabled={!valid || !available}
            onClick={() => {
              shell.runCommand(`SHADOWS SUN ${azimuthValue} ${elevationValue}`);
              setAzimuth(null);
              setElevation(null);
              refresh();
            }}
          >
            {t('concept.shadows.apply')}
          </button>
        </div>
        {!valid ? (
          <p className="concept-panel__hint" role="alert">{t('concept.shadows.invalid')}</p>
        ) : null}
        {failure}
      </>
    );
  }

  const enabled = answer?.fog.enabled === true;
  const currentStart = answer?.fog.start ?? 100;
  const currentEnd = answer?.fog.end ?? 1000;
  const color = answer?.fog.color ?? [200, 210, 220];
  const startText = start ?? String(currentStart);
  const endText = end ?? String(currentEnd);
  const startValue = Number(startText);
  const endValue = Number(endText);
  const valid = startText.trim() !== '' && endText.trim() !== ''
    && Number.isFinite(startValue) && startValue >= 0
    && Number.isFinite(endValue) && endValue > startValue;
  const available = shell.canCommand('FOG');
  const hex = `#${color.map(value => value.toString(16).padStart(2, '0')).join('')}`;
  return (
    <>
      <p className="concept-panel__hint">{t('concept.fog.hint')}</p>
      <label className="concept-panel__hint">
        <input
          type="checkbox"
          checked={enabled}
          disabled={!available}
          onChange={event => {
            shell.runCommand(`FOG ${event.currentTarget.checked ? 'ON' : 'OFF'}`);
            refresh();
          }}
        />{' '}
        {t('concept.fog.display')}
      </label>
      <div className="concept-scenes__save">
        <input
          type="number"
          min={0}
          step={1}
          className="concept-scenes__name"
          value={startText}
          aria-label={t('concept.fog.start')}
          onChange={event => setStart(event.currentTarget.value)}
        />
        <input
          type="number"
          min={0}
          step={1}
          className="concept-scenes__name"
          value={endText}
          aria-label={t('concept.fog.end')}
          onChange={event => setEnd(event.currentTarget.value)}
        />
        <button
          type="button"
          className="concept-panel__action"
          disabled={!valid || !available}
          onClick={() => {
            shell.runCommand(`FOG RANGE ${startValue} ${endValue}`);
            setStart(null);
            setEnd(null);
            refresh();
          }}
        >
          {t('concept.fog.apply')}
        </button>
      </div>
      <label className="concept-panel__hint">
        {t('concept.fog.color')}{' '}
        <input
          type="color"
          value={hex}
          disabled={!available}
          onChange={event => {
            const text = event.currentTarget.value;
            const channels = [1, 3, 5].map(index => Number.parseInt(text.slice(index, index + 2), 16));
            if (channels.every(channel => Number.isFinite(channel))) {
              shell.runCommand(`FOG COLOR ${channels.join(' ')}`);
              refresh();
            }
          }}
        />
      </label>
      {!valid ? (
        <p className="concept-panel__hint" role="alert">{t('concept.fog.invalid')}</p>
      ) : null}
      {failure}
    </>
  );
}


/**
 * The Outliner panel.
 *
 * The host's tree is a flat pre-order list (`cad_outliner`); the panel only
 * applies the user's collapse state and renders the indentation. Clicking a
 * selectable row sends its `selectIds` through the one selection wire
 * (`cad_select_entities`); a component member row is a readable leaf and says
 * why it cannot be selected. No local "selected" state: the host's
 * `cad_status.selection` is the authority.
 */
function OutlinerBody() {
  const { t } = useI18n();
  const shell = useShell();
  const documentLabel = shell.status?.documentLabel ?? null;
  const entityCount = shell.status?.entityCount ?? null;
  const commandCount = shell.status?.commandLog?.length ?? 0;
  const [nonce, setNonce] = useState(0);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set<string>());
  const [selectionError, setSelectionError] = useState('');
  const [state, setState] = useState<{
    phase: 'ready' | 'loading' | 'error';
    answer: OutlinerAnswer | null;
    reason: string;
  }>({ phase: 'ready', answer: null, reason: '' });
  useEffect(() => {
    if (documentLabel === null) {
      setState({ phase: 'ready', answer: null, reason: '' });
      return undefined;
    }
    let live = true;
    setState(previous => ({ phase: 'loading', answer: previous.answer, reason: '' }));
    readOutliner(invoke)
      .then(answer => {
        if (live) setState({ phase: 'ready', answer, reason: '' });
      })
      .catch((error: unknown) => {
        if (!live) return;
        const reason = error instanceof Error ? error.message : String(error);
        setState(previous => ({ phase: 'error', answer: previous.answer, reason }));
      });
    return () => { live = false; };
  }, [documentLabel, entityCount, commandCount, nonce]);

  if (documentLabel === null) {
    return <p className="concept-panel__hint">{t('concept.outliner.no_document')}</p>;
  }
  const rows = state.answer?.rows ?? [];
  const visible = visibleOutlinerRows(rows, collapsed);
  const toggle = (row: OutlinerRow) => {
    const key = outlinerRowKey(row);
    setCollapsed(previous => {
      const next = new Set(previous);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };
  const select = (row: OutlinerRow) => {
    if (!row.selectable) return;
    setSelectionError('');
    void selectBimEntities(invoke, row.selectIds).then(result => {
      if (result.kind === 'error') setSelectionError(result.reason);
      else setNonce(value => value + 1);
    });
  };
  return (
    <>
      <p className="concept-panel__hint">{t('concept.outliner.hint')}</p>
      {state.phase === 'error' ? (
        <p className="concept-panel__hint" role="alert">
          {t('concept.outliner.error', { reason: state.reason })}
        </p>
      ) : null}
      {selectionError !== '' ? (
        <p className="concept-panel__hint" role="alert">
          {t('concept.outliner.selection_failed', { reason: selectionError })}
        </p>
      ) : null}
      {rows.length === 0 ? (
        <p className="concept-panel__hint">
          {state.phase === 'loading' ? t('concept.outliner.loading') : t('concept.outliner.empty')}
        </p>
      ) : (
        <ul className="concept-outliner">
          {visible.map(row => {
            const key = outlinerRowKey(row);
            const isCollapsed = collapsed.has(key);
            return (
              <li key={key} style={{ paddingInlineStart: `${row.depth * 14}px` }}>
                {row.hasChildren ? (
                  <button
                    type="button"
                    className="concept-outliner__toggle"
                    aria-label={t(isCollapsed ? 'concept.outliner.expand' : 'concept.outliner.collapse')}
                    onClick={() => toggle(row)}
                  >
                    {isCollapsed ? '\u25B8' : '\u25BE'}
                  </button>
                ) : (
                  <span className="concept-outliner__toggle" aria-hidden="true" />
                )}
                {row.selectable ? (
                  <button
                    type="button"
                    className="concept-rows__label concept-outliner__name"
                    title={t('concept.outliner.select', { name: row.label })}
                    onClick={() => select(row)}
                  >
                    {row.label}
                  </button>
                ) : (
                  <span
                    className="concept-outliner__name concept-outliner__member"
                    title={t('concept.outliner.not_selectable')}
                  >
                    {row.label}
                  </span>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {state.answer !== null && state.answer.truncated ? (
        <p className="concept-panel__hint">
          {t('concept.outliner.truncated', {
            shown: rows.length,
            total: state.answer.total,
          })}
        </p>
      ) : null}
    </>
  );
}


function SoftenBody() {
  const { t } = useI18n();
  const shell = useShell();
  const [angle, setAngle] = useState('20');
  const parsed = Number(angle);
  const valid = angle.trim() !== '' && Number.isFinite(parsed) && parsed >= 0 && parsed <= 180;
  const available = shell.canCommand('SOFTENEDGES');
  return (
    <>
      <p className="concept-panel__hint">{t('concept.soften.hint')}</p>
      <div className="concept-scenes__save">
        <input
          type="number"
          min={0}
          max={180}
          step={1}
          className="concept-scenes__name"
          value={angle}
          aria-label={t('concept.soften.angle')}
          onChange={event => setAngle(event.currentTarget.value)}
        />
        <button
          type="button"
          className="concept-panel__action"
          disabled={!valid || !available}
          onClick={() => shell.runCommand(`SOFTENEDGES ${parsed}`)}
        >
          {t('concept.soften.apply')}
        </button>
      </div>
      {!valid ? (
        <p className="concept-panel__hint" role="alert">{t('concept.soften.invalid')}</p>
      ) : null}
      {!available ? (
        <p className="concept-panel__hint">{t('concept.soften.no_document')}</p>
      ) : null}
    </>
  );
}

function PendingBody({ panel }: { readonly panel: TrayPanel }) {
  const { t } = useI18n();
  return <p className="concept-panel__hint">{panel.pendingText ?? t(panel.pendingKey ?? 'concept.pending.panel')}</p>;
}

function Body({ panel }: { readonly panel: TrayPanel }) {
  switch (panel.kind) {
    case 'entityInfo': return <EntityInfoBody />;
    case 'components': return <ComponentsBody />;
    case 'styles': return <StylesBody />;
    case 'tags': return <TagsBody />;
    case 'instructor': return <InstructorBody />;
    case 'bimQuery': return <BimQueryBody />;
    case 'mechanicalTree': return <MechanicalTreeBody />;
    case 'materials': return <MaterialsBody />;
    case 'scenes': return <ScenesBody />;
    case 'soften': return <SoftenBody />;
    case 'shadows': return <DisplayBody part="shadows" />;
    case 'fog': return <DisplayBody part="fog" />;
    case 'outliner': return <OutlinerBody />;
    case 'matchPhoto': return <MatchPhotoBody />;
    case 'overlays': return <OverlaysBody />;
    case 'pending':
    default: return <PendingBody panel={panel} />;
  }
}

function Section({ panel }: { readonly panel: TrayPanel }) {
  const { t } = useI18n();
  const pending = panel.kind === 'pending';
  // `open` is controlled by the section's own state so a tap/click toggles it:
  // React's DOM types do not expose `defaultOpen`, and a permanently open body
  // would ignore the user.
  const [open, setOpen] = useState(!pending);
  return (
    <details
      className="concept-tray__section"
      data-kind={panel.kind}
      open={open}
      onToggle={event => setOpen(event.currentTarget.open)}
    >
      <summary className="concept-tray__summary">{panel.title ?? t(panel.titleKey)}</summary>
      <div className="concept-tray__body"><Body panel={panel} /></div>
    </details>
  );
}

export interface DomainTrayProps {
  /**
   * The side the chrome placed this tray on. The profile owns the choice
   * (`DomainProfile.traySide`): `bim` follows Revit's left dock, `concept` keeps
   * SketchUp's right tray. The component only flips the edge that faces the
   * drawing and stops reserving the right-edge navigation dock's column when it
   * sits on the left.
   */
  readonly side: 'left' | 'right';
}

export function DomainTray({ side }: DomainTrayProps) {
  const { t } = useI18n();
  const { status } = useShell();
  const hidden = useSyncExternalStore(
    subscribeDomainStore,
    () => domainStoreSnapshot().trayHidden,
    () => domainStoreSnapshot().trayHidden,
  );
  const domain = currentDomain(status);
  const layout = useDomainUi(domain);
  if (hidden || !layout.tray || layout.tray.length === 0) return null;
  return (
    <aside
      className={['concept-tray', side === 'left' ? 'concept-tray--left' : ''].filter(Boolean).join(' ')}
      aria-label={t('domain.tray_label')}
    >
      {/* A real title row with the one hide control, the way both reference
          trays have a title bar: the tray is chrome, so the user can put it away
          and bring it back from the profile's own View surface
          (`SHELL_ACTIONS.domainTray`). Sections below keep their own
          collapse; this hides the whole column. */}
      <div className="concept-tray__head">
        <span className="concept-tray__title">{t('domain.tray_label')}</span>
        <button
          type="button"
          className="concept-tray__hide"
          aria-label={t('domain.tray_hide')}
          title={t('domain.tray_hide')}
          onClick={toggleTray}
        >
          <Icon name="close" size={13} />
        </button>
      </div>
      {layout.tray.map(panel => <Section key={panel.id} panel={panel} />)}
    </aside>
  );
}
