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
// The rows themselves live in `BlockList.tsx`, shared with the Tool Palettes'
// Blocks tab: both surfaces list the same definitions and arm the same flow, so
// they word every row and every refusal identically.
//
// The panel is non-modal: no backdrop and no focus trap. It takes the keyboard
// on open, closes on Escape and restores the previous focus.
import React from 'react';
import { Icon } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { blockRows } from '../classic/classic-rows.ts';
import { useSurfacePanelFocus } from './surface-focus.ts';
import { BlockList } from './BlockList.tsx';

interface ClassicInsertPanelProps {
  readonly onClose: () => void;
  /** Arm INSERT with this block name once the point phase completes. */
  readonly onInsert: (name: string) => void;
}

export function ClassicInsertPanel({ onClose, onInsert }: ClassicInsertPanelProps) {
  const { t } = useI18n();
  const shell = useShell();
  const { panelRef } = useSurfacePanelFocus(onClose);
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
        <BlockList
          rows={rows}
          documentOpen={documentOpen}
          loading={loading}
          toolActive={toolActive}
          onInsert={onInsert}
        />
      )}
    </aside>
  );
}
