// The one block list both block surfaces render.
//
// `CLASSICINSERT` and the Tool Palettes' Blocks tab read the same authoritative
// `cad_status.blocks` projection (`blockRows`) and arm the same point-then-name
// `INSERT` flow (`useInsertBlock`). Keeping the rows, their focus/Insert controls
// and the three refusal sentences here means a block the classic panel cannot
// insert is worded identically in the Tool Palettes — never a second guess.
//
// `Insert` is disabled (with the stable reason) exactly when the host data or
// the session state forbids the flow: no open drawing, a load, another active
// tool, or a definition without a stored name/base point.
import React, { useRef, useState } from 'react';
import { useI18n } from '../context.ts';
import type { BlockRow } from '../classic/classic-rows.ts';

interface BlockListProps {
  readonly rows: readonly BlockRow[];
  /** The host holds an open, loaded drawing. */
  readonly documentOpen: boolean;
  /** An import is still running. */
  readonly loading: boolean;
  /** Another tool owns the picks, so INSERT cannot start. */
  readonly toolActive: boolean;
  /** Arm INSERT with this block name once the point phase completes. */
  readonly onInsert: (name: string) => void;
}

function formatPoint(point: readonly [number, number, number]): string {
  return point.map(value => (Number.isInteger(value) ? String(value) : value.toFixed(6))).join(', ');
}

function BlockRowView({
  row,
  focused,
  reason,
  onFocus,
  onInsert,
}: {
  readonly row: BlockRow;
  readonly focused: boolean;
  readonly reason: string | null;
  readonly onFocus: () => void;
  readonly onInsert: () => void;
}) {
  const { t } = useI18n();
  const rowRef = useRef<HTMLLIElement | null>(null);
  const name = row.name ?? t('classicinsert.unnamed');
  return (
    <li
      ref={rowRef}
      tabIndex={-1}
      className={['references__row', 'is-muted', focused ? 'is-focused' : ''].filter(Boolean).join(' ')}
      aria-current={focused ? 'true' : undefined}
    >
      <div className="references__head">
        <span className="references__name" title={name}>{name}</span>
        <button
          type="button"
          className="quick-view__badge is-muted"
          onClick={() => { onFocus(); rowRef.current?.focus({ preventScroll: true }); }}
        >
          {t('classicinsert.focus')}
        </button>
        <button
          type="button"
          disabled={reason !== null}
          title={reason ?? t('classicinsert.action.insert')}
          onClick={() => { onFocus(); onInsert(); }}
        >
          {t('classicinsert.action.insert')}
        </button>
      </div>
      <p className="references__meta">
        <span>{t('classicinsert.memberCount', { count: row.memberCount })}</span>
        {row.basePoint === null ? null : (
          <span>{t('classicinsert.basePoint')}: <code>{formatPoint(row.basePoint)}</code></span>
        )}
        <span>{focused ? t('classicinsert.focused') : ''}</span>
      </p>
      {row.insertable ? null : <p className="references__hint">{t('classicinsert.refusal.no_metadata')}</p>}
    </li>
  );
}

export function BlockList({ rows, documentOpen, loading, toolActive, onInsert }: BlockListProps) {
  const { t } = useI18n();
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  return (
    <div className="quick-view__grid">
      <ul className="references__list">
        {rows.map((row, index) => {
          const key = `${row.name ?? ''}:${row.memberCount}:${index}`;
          const reason = !documentOpen
            ? t('classicinsert.refusal.no_document')
            : loading || toolActive
              ? t('classicinsert.refusal.busy')
              : row.insertable ? null : t('classicinsert.refusal.no_metadata');
          return (
            <BlockRowView
              key={key}
              row={row}
              focused={focusedKey === key}
              reason={reason}
              onFocus={() => setFocusedKey(key)}
              onInsert={() => { if (row.name !== null) onInsert(row.name); }}
            />
          );
        })}
      </ul>
    </div>
  );
}
