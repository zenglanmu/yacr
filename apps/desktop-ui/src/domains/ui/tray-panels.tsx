// Match Photo, Overlays, Materials and Outliner tray bodies, split out of
// `DomainTray.tsx` for the file-size gate. All are host-read-backed; none of
// the reads writes anything (painting still goes through `PAINT`).

import { useEffect, useState } from 'react';
import { useI18n, useShell } from '../../context.ts';
import { invoke } from '../../host/invoke.ts';
import { readMatchPhoto, readOverlays } from '../concept/panels.ts';
import { BUILTIN_MATERIALS, paintCommand, readMaterials } from '../concept/materials.ts';
import {
  groupCreateCommand,
  groupEditCommand,
  groupRenameCommand,
  outlinerRowKey,
  ungroupCommands,
  readOutliner,
  reorderPlan,
  selectedOutlinerIds,
  visibleOutlinerRows,
} from '../concept/outliner.ts';
import type { OutlinerAnswer, OutlinerRow } from '../concept/outliner.ts';
import type { MaterialsAnswer } from '../concept/materials.ts';
import { selectBimEntities } from '../../bim/selection.ts';
import type { MatchPhotoAnswer, OverlaysAnswer } from '../concept/panels.ts';

/**
 * The Match Photo panel.
 *
 * Photo matching is not modeled: the host read reports `modeled: false` with a
 * stable reason, and the action dispatches `MATCHPHOTO`, whose refusal names the
 * same missing model. The panel never shows a fake photo list.
 */
export function MatchPhotoBody() {
  const { t } = useI18n();
  const shell = useShell();
  const documentLabel = shell.status?.documentLabel ?? null;
  const [state, setState] = useState<{
    phase: 'ready' | 'loading' | 'error';
    answer: MatchPhotoAnswer | null;
    reason: string;
  }>({ phase: 'ready', answer: null, reason: '' });
  useEffect(() => {
    if (documentLabel === null) {
      setState({ phase: 'ready', answer: null, reason: '' });
      return undefined;
    }
    let live = true;
    setState(previous => ({ phase: 'loading', answer: previous.answer, reason: '' }));
    readMatchPhoto(invoke)
      .then(answer => {
        if (live) setState({ phase: 'ready', answer, reason: '' });
      })
      .catch((error: unknown) => {
        if (!live) return;
        const reason = error instanceof Error ? error.message : String(error);
        setState(previous => ({ phase: 'error', answer: previous.answer, reason }));
      });
    return () => { live = false; };
  }, [documentLabel]);
  if (documentLabel === null) {
    return <p className="concept-panel__hint">{t('concept.panels.no_document')}</p>;
  }
  return (
    <>
      <p className="concept-panel__hint">{t('concept.match_photo.hint')}</p>
      {state.phase === 'error' ? (
        <p className="concept-panel__hint" role="alert">
          {t('concept.match_photo.error', { reason: state.reason })}
        </p>
      ) : null}
      {state.answer !== null && !state.answer.modeled ? (
        <p className="concept-panel__hint" role="status">{state.answer.reason}</p>
      ) : null}
      <button
        type="button"
        className="concept-panel__action"
        onClick={() => shell.runCommand('MATCHPHOTO')}
      >
        {t('concept.match_photo.action')}
      </button>
    </>
  );
}

/**
 * The Overlays panel.
 *
 * The rows are the renderer-owned overlay channels this build really draws, with
 * their current on/off state. There is no user-managed overlay definition model,
 * so the management action dispatches `OVERLAYS`, whose refusal says so.
 */
export function OverlaysBody() {
  const { t } = useI18n();
  const shell = useShell();
  const documentLabel = shell.status?.documentLabel ?? null;
  const [state, setState] = useState<{
    phase: 'ready' | 'loading' | 'error';
    answer: OverlaysAnswer | null;
    reason: string;
  }>({ phase: 'ready', answer: null, reason: '' });
  useEffect(() => {
    if (documentLabel === null) {
      setState({ phase: 'ready', answer: null, reason: '' });
      return undefined;
    }
    let live = true;
    setState(previous => ({ phase: 'loading', answer: previous.answer, reason: '' }));
    readOverlays(invoke)
      .then(answer => {
        if (live) setState({ phase: 'ready', answer, reason: '' });
      })
      .catch((error: unknown) => {
        if (!live) return;
        const reason = error instanceof Error ? error.message : String(error);
        setState(previous => ({ phase: 'error', answer: previous.answer, reason }));
      });
    return () => { live = false; };
  }, [documentLabel]);
  if (documentLabel === null) {
    return <p className="concept-panel__hint">{t('concept.panels.no_document')}</p>;
  }
  const rows = state.answer?.rows ?? [];
  return (
    <>
      <p className="concept-panel__hint">{t('concept.overlays.hint')}</p>
      {state.phase === 'error' ? (
        <p className="concept-panel__hint" role="alert">
          {t('concept.overlays.error', { reason: state.reason })}
        </p>
      ) : null}
      {rows.length === 0 ? (
        <p className="concept-panel__hint">
          {state.phase === 'loading' ? t('concept.panels.loading') : t('concept.overlays.hint')}
        </p>
      ) : (
        <ul className="concept-rows">
          {rows.map(row => (
            <li key={row.key}>
              <span className="concept-rows__label">{t(`concept.overlays.row.${row.key}`)}</span>
              <span className="concept-rows__value">
                {t(row.active ? 'concept.overlays.on' : 'concept.overlays.off')}
              </span>
            </li>
          ))}
        </ul>
      )}
      <button
        type="button"
        className="concept-panel__action"
        onClick={() => shell.runCommand('OVERLAYS')}
      >
        {t('concept.overlays.manage')}
      </button>
    </>
  );
}


/** One materials read's state; the last good answer survives a failed refresh. */
type MaterialsState =
  | { readonly phase: 'ready'; readonly answer: MaterialsAnswer | null; readonly reason: '' }
  | { readonly phase: 'loading'; readonly answer: MaterialsAnswer | null; readonly reason: '' }
  | { readonly phase: 'error'; readonly answer: MaterialsAnswer | null; readonly reason: string };

export function MaterialsBody() {
  const { t } = useI18n();
  const shell = useShell();
  const documentLabel = shell.status?.documentLabel ?? null;
  const entityCount = shell.status?.entityCount ?? null;
  const commandCount = shell.status?.commandLog?.length ?? 0;
  const [state, setState] = useState<MaterialsState>({ phase: 'ready', answer: null, reason: '' });
  useEffect(() => {
    if (documentLabel === null) {
      setState({ phase: 'ready', answer: null, reason: '' });
      return undefined;
    }
    let live = true;
    setState(previous => ({ phase: 'loading', answer: previous.answer, reason: '' }));
    readMaterials(invoke)
      .then(answer => {
        if (live) setState({ phase: 'ready', answer, reason: '' });
      })
      .catch((error: unknown) => {
        if (!live) return;
        const reason = error instanceof Error ? error.message : String(error);
        setState(previous => ({ phase: 'error', answer: previous.answer, reason }));
      });
    return () => { live = false; };
  }, [documentLabel, entityCount, commandCount]);

  const available = shell.canCommand('PAINT');
  const documentMaterials = state.answer?.materials ?? [];
  return (
    <>
      <p className="concept-panel__hint">{t('concept.materials.hint')}</p>
      <p className="concept-panel__hint">{t('concept.materials.document')}</p>
      {state.phase === 'error' ? (
        <p className="concept-panel__hint" role="alert">
          {t('concept.materials.error')}: {state.reason}
        </p>
      ) : documentLabel === null ? (
        <p className="concept-panel__hint">{t('concept.materials.no_document')}</p>
      ) : state.phase === 'loading' && state.answer === null ? (
        <p className="concept-panel__hint">{t('concept.materials.loading')}</p>
      ) : documentMaterials.length === 0 ? (
        <p className="concept-panel__hint">{t('concept.materials.empty')}</p>
      ) : (
        <ul className="concept-materials">
          {documentMaterials.map(material => (
            <li key={material.name}>
              <button
                type="button"
                className="concept-materials__swatch"
                style={{ backgroundColor: material.color, opacity: 1 - material.transparency }}
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
      )}
      <p className="concept-panel__hint">{t('concept.materials.builtin')}</p>
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
export function OutlinerBody() {
  const { t } = useI18n();
  const shell = useShell();
  const documentLabel = shell.status?.documentLabel ?? null;
  const entityCount = shell.status?.entityCount ?? null;
  const commandCount = shell.status?.commandLog?.length ?? 0;
  const [nonce, setNonce] = useState(0);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set<string>());
  const [selectionError, setSelectionError] = useState('');
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [selected, setSelected] = useState<ReadonlySet<string>>(() => new Set<string>());
  const [groupName, setGroupName] = useState('');
  const [dragKeys, setDragKeys] = useState<ReadonlySet<string>>(() => new Set<string>());
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
  const select = (row: OutlinerRow, additive: boolean) => {
    if (!row.selectable) return;
    setSelectionError('');
    const key = outlinerRowKey(row);
    const next = new Set(additive ? selected : []);
    if (additive && next.has(key)) next.delete(key);
    else next.add(key);
    setSelected(next);
    const ids = selectedOutlinerIds(rows, next);
    if (ids.length === 0) return;
    void selectBimEntities(invoke, ids).then(result => {
      if (result.kind === 'error') setSelectionError(result.reason);
      else setNonce(value => value + 1);
    });
  };
  const draggedRows = () => rows.filter(candidate =>
    dragKeys.has(outlinerRowKey(candidate))
    && candidate.selectable
    && candidate.kind !== 'group');
  const reorder = (row: OutlinerRow, direction: 'up' | 'down') => {
    const plan = reorderPlan(rows, outlinerRowKey(row), direction);
    if (plan === null) return;
    setSelectionError('');
    void selectBimEntities(invoke, plan.ids).then(result => {
      if (result.kind === 'error') {
        setSelectionError(result.reason);
        return;
      }
      shell.runCommand(plan.command);
      setNonce(value => value + 1);
    });
  };
  const dropToUngroup = () => {
    const commands = ungroupCommands(rows, dragKeys);
    if (commands.length === 0) return;
    for (const command of commands) shell.runCommand(command);
    setDragKeys(new Set());
    setNonce(value => value + 1);
  };
  const dropOnGroup = (row: OutlinerRow) => {
    if (row.kind !== 'group') return;
    const dragged = draggedRows();
    if (dragged.length === 0) return;
    const target = row.label;
    // A real regroup moves the rows: remove them from their current groups first,
    // then add them to the target. The two GROUPEDIT commands are two undo steps.
    const moving = dragged.filter(candidate => candidate.parentGroup !== target);
    const addCommand = groupEditCommand(
      target,
      'ADD',
      selectedOutlinerIds(rows, new Set(
        moving.map(candidate => outlinerRowKey(candidate)),
      )),
    );
    if (addCommand === null) return;
    const parents = new Set<string>();
    for (const candidate of moving) {
      if (candidate.parentGroup !== null) parents.add(candidate.parentGroup);
    }
    for (const parent of parents) {
      const removeCommand = groupEditCommand(
        parent,
        'REMOVE',
        selectedOutlinerIds(rows, new Set(
          moving
            .filter(candidate => candidate.parentGroup === parent)
            .map(candidate => outlinerRowKey(candidate)),
        )),
      );
      if (removeCommand !== null) shell.runCommand(removeCommand);
    }
    shell.runCommand(addCommand);
    setDragKeys(new Set());
    setNonce(value => value + 1);
  };
  const groupSelected = () => {
    const ids = selectedOutlinerIds(rows, selected);
    const command = groupCreateCommand(groupName);
    if (ids.length === 0 || command === null) return;
    setSelectionError('');
    void selectBimEntities(invoke, ids).then(result => {
      if (result.kind === 'error') {
        setSelectionError(result.reason);
        return;
      }
      shell.runCommand(command);
      setGroupName('');
      setSelected(new Set());
      setNonce(value => value + 1);
    });
  };
  const hide = (row: OutlinerRow) => {
    if (!row.selectable || row.hidden) return;
    setSelectionError('');
    void selectBimEntities(invoke, row.selectIds).then(result => {
      if (result.kind === 'error') {
        setSelectionError(result.reason);
        return;
      }
      shell.runCommand('HIDEOBJECTS');
      setNonce(value => value + 1);
    });
  };
  const show = (row: OutlinerRow) => {
    if (!row.selectable || !row.hidden) return;
    setSelectionError('');
    void selectBimEntities(invoke, row.selectIds).then(result => {
      if (result.kind === 'error') {
        setSelectionError(result.reason);
        return;
      }
      shell.runCommand('SHOWOBJECTS');
      setNonce(value => value + 1);
    });
  };
  const beginRename = (row: OutlinerRow) => {
    setSelectionError('');
    setRenameValue(row.label);
    setRenaming(outlinerRowKey(row));
  };
  const commitRename = (row: OutlinerRow) => {
    const command = groupRenameCommand(row.label, renameValue);
    if (command === null) return;
    shell.runCommand(command);
    setRenaming(null);
  };
  return (
    <>
      <p className="concept-panel__hint">{t('concept.outliner.hint')}</p>
      <p className="concept-panel__hint">{t('concept.outliner.regroup_hint')}</p>
      <div className="concept-scenes__save">
        <input
          type="text"
          className="concept-scenes__name"
          value={groupName}
          placeholder={t('concept.outliner.group_name')}
          aria-label={t('concept.outliner.group_name')}
          onChange={event => setGroupName(event.currentTarget.value)}
        />
        <button
          type="button"
          className="concept-panel__action"
          disabled={selectedOutlinerIds(rows, selected).length === 0
            || groupCreateCommand(groupName) === null
            || !shell.canCommand('GROUP')}
          onClick={groupSelected}
        >
          {t('concept.outliner.group_selected')}
        </button>
      </div>
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
              <li
                key={key}
                style={{ paddingInlineStart: `${row.depth * 14}px` }}
                draggable={row.kind !== 'group' && row.selectable}
                onDragStart={() => {
                  if (row.kind === 'group' || !row.selectable) return;
                  setDragKeys(selected.has(key) ? selected : new Set([key]));
                }}
                onDragOver={row.kind === 'group'
                  ? event => {
                    if (groupEditCommand(
                      row.label,
                      'ADD',
                      selectedOutlinerIds(rows, dragKeys),
                    ) !== null) {
                      event.preventDefault();
                    }
                  }
                  : undefined}
                onDrop={row.kind === 'group'
                  ? event => {
                    event.preventDefault();
                    event.stopPropagation();
                    dropOnGroup(row);
                  }
                  : undefined}
              >
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
                {renaming === key ? (
                  <>
                    <input
                      type="text"
                      className="concept-scenes__name"
                      value={renameValue}
                      aria-label={t('concept.outliner.rename')}
                      onChange={event => setRenameValue(event.currentTarget.value)}
                      onKeyDown={event => {
                        if (event.key === 'Enter') commitRename(row);
                        if (event.key === 'Escape') setRenaming(null);
                      }}
                    />
                    <button
                      type="button"
                      className="concept-panel__action"
                      disabled={groupRenameCommand(row.label, renameValue) === null
                        || !shell.canCommand('GROUPEDIT')}
                      onClick={() => commitRename(row)}
                    >
                      {t('concept.outliner.rename_save')}
                    </button>
                    <button
                      type="button"
                      className="concept-panel__action"
                      onClick={() => setRenaming(null)}
                    >
                      {t('concept.outliner.rename_cancel')}
                    </button>
                  </>
                ) : row.selectable ? (
                  <button
                    type="button"
                    className="concept-rows__label concept-outliner__name"
                    title={t('concept.outliner.select', { name: row.label })}
                    aria-pressed={selected.has(key)}
                    onClick={event => select(
                      row,
                      event.ctrlKey || event.metaKey || event.shiftKey,
                    )}
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
                {row.hidden ? (
                  <span className="concept-outliner__hidden">{t('concept.outliner.hidden')}</span>
                ) : null}
                {row.hidden && row.selectable ? (
                  <button
                    type="button"
                    className="concept-panel__action"
                    disabled={!shell.canCommand('SHOWOBJECTS')}
                    title={t('concept.outliner.show_named', { name: row.label })}
                    onClick={() => show(row)}
                  >
                    {t('concept.outliner.show')}
                  </button>
                ) : null}
                {row.selectable && !row.hidden ? (
                  <button
                    type="button"
                    className="concept-panel__action"
                    disabled={!shell.canCommand('HIDEOBJECTS')}
                    title={t('concept.outliner.hide_named', { name: row.label })}
                    onClick={() => hide(row)}
                  >
                    {t('concept.outliner.hide')}
                  </button>
                ) : null}
                {row.parentGroup !== null && row.selectable ? (
                  <button
                    type="button"
                    className="concept-panel__action"
                    disabled={groupEditCommand(row.parentGroup, 'REMOVE', row.selectIds) === null
                      || !shell.canCommand('GROUPEDIT')}
                    title={t('concept.outliner.remove_named', { name: row.parentGroup })}
                    onClick={() => {
                      if (row.parentGroup === null) return;
                      const command = groupEditCommand(
                        row.parentGroup,
                        'REMOVE',
                        row.selectIds,
                      );
                      if (command !== null) {
                        shell.runCommand(command);
                        setNonce(value => value + 1);
                      }
                    }}
                  >
                    {t('concept.outliner.remove_from_group')}
                  </button>
                ) : null}
                {(row.depth === 0 && row.kind !== 'group' && row.selectable)
                  || (row.depth === 1 && row.kind === 'entity' && row.parentGroup !== null) ? (
                  <>
                    <button
                      type="button"
                      className="concept-panel__action"
                      disabled={reorderPlan(rows, key, 'up') === null
                        || !shell.canCommand('DRAWORDER')}
                      title={t('concept.outliner.move_up')}
                      onClick={() => reorder(row, 'up')}
                    >
                      {'\u2191'}
                    </button>
                    <button
                      type="button"
                      className="concept-panel__action"
                      disabled={reorderPlan(rows, key, 'down') === null
                        || !shell.canCommand('DRAWORDER')}
                      title={t('concept.outliner.move_down')}
                      onClick={() => reorder(row, 'down')}
                    >
                      {'\u2193'}
                    </button>
                  </>
                ) : null}
                {renaming !== key && row.kind === 'group'
                  && groupRenameCommand(row.label, row.label) !== null ? (
                  <button
                    type="button"
                    className="concept-panel__action"
                    disabled={!shell.canCommand('GROUPEDIT')}
                    onClick={() => beginRename(row)}
                  >
                    {t('concept.outliner.rename')}
                  </button>
                ) : null}
              </li>
            );
          })}
        </ul>
      )}
      <div
        className="concept-outliner__ungroup"
        onDragOver={event => {
          if (ungroupCommands(rows, dragKeys).length > 0) event.preventDefault();
        }}
        onDrop={event => {
          event.preventDefault();
          dropToUngroup();
        }}
      >
        {t('concept.outliner.drop_to_ungroup')}
      </div>
      {state.answer !== null && state.answer.truncated ? (
        <p className="concept-panel__hint">
          {t('concept.outliner.truncated', {
            shown: rows.length,
            total: state.answer.total,
          })}
        </p>
      ) : null}
      {rows.some(row => row.hidden) ? (
        <button
          type="button"
          className="concept-panel__action"
          disabled={!shell.canCommand('UNISOLATEOBJECTS')}
          onClick={() => {
            shell.runCommand('UNISOLATEOBJECTS');
            setNonce(value => value + 1);
          }}
        >
          {t('concept.outliner.show_all')}
        </button>
      ) : null}
    </>
  );
}


