// The classic block-insertion panel (`CLASSICINSERT`).
//
// Every row is one authoritative `cad_status.blocks[]` definition projected by
// `blockRows`: stored name (or `null`), real member count and the stored base
// point (or `null`). Nothing is resolved or invented here.
//
// `Insert` arms the existing point-then-name `INSERT` flow: the session starts
// the real tool, and once the host reports its terminal text step the chosen
// block name is sent through the same `cad_set_tool_text` channel the command
// line uses. A block without stored metadata cannot be addressed by INSERT, so
// its action stays disabled with the stable reason.
//
// The panel is non-modal: no backdrop and no focus trap. It takes the keyboard
// on open, closes on Escape and restores the previous focus.
import React, { useRef, useState } from 'react';
import { Icon } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { blockRows } from '../classic/classic-rows.ts';
import type { BlockRow } from '../classic/classic-rows.ts';
import { useSurfacePanelFocus } from './surface-focus.ts';

interface ClassicInsertPanelProps {
  readonly onClose: () => void;
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

export function ClassicInsertPanel({ onClose, onInsert }: ClassicInsertPanelProps) {
  const { t } = useI18n();
  const shell = useShell();
  const { panelRef } = useSurfacePanelFocus(onClose);
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  const rows = blockRows(shell.status);
  const loading = Boolean(shell.status?.loading);
  const documentOpen = Boolean(shell.status?.documentLabel) && !loading;
  const toolActive = Boolean(shell.status?.tool?.key);
  return (
    <aside
      ref={panelRef}
      tabIndex={-1}
      className="quick-view classic-insert"
      role="dialog"
      aria-label={t('classicinsert.title')}
    >
      <header className="quick-view__head">
        <span className="quick-view__title">
          <Icon name="copy" size={14} />
          {t('classicinsert.title')}
        </span>
        <IconButton icon="close" size="xs" label={t('resources.close')} onClick={onClose} />
      </header>
      <p className="quick-view__subtitle">{t('classicinsert.subtitle')}</p>
      {rows.length === 0 ? (
        <p className="quick-view__empty">{t('classicinsert.empty')}</p>
      ) : (
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
      )}
    </aside>
  );
}
