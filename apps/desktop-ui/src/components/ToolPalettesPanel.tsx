// The Tool Palettes float (`TOOLPALETTES`, `TPNAVIGATE Blocks`).
//
// This build has exactly one tab, Blocks. It lists the same authoritative
// `cad_status.blocks` definitions the classic insert panel lists — through the
// shared `BlockList`, so the rows, the member/base-point metadata, the refusal
// sentences and the Insert control are one implementation — and choosing Insert
// arms the same point-then-name `INSERT` flow.
//
// Nothing about tool content is invented: there is no palette library, no tool
// import and no second tab, and the footer says so. `TPNAVIGATE` names a tab;
// only Blocks opens this panel and every other name is refused on the chrome's
// error line by `commands/local-selectors.ts` (it never becomes a command).
//
// The panel is non-modal: no backdrop and no focus trap. It takes the keyboard
// on open (its tab / first block row), closes on Escape and restores the
// previous focus, exactly like the other chrome floats.
import React, { useEffect, useRef } from 'react';
import { Icon } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { blockRows } from '../classic/classic-rows.ts';
import { useSurfacePanelFocus } from './surface-focus.ts';
import { BlockList } from './BlockList.tsx';

interface ToolPalettesPanelProps {
  /** Bumped by every `TOOLPALETTES`/`TPNAVIGATE Blocks`, so a repeat open re-focuses. */
  readonly focusRevision: number;
  readonly onClose: () => void;
  /** Arm INSERT with this block name once the point phase completes. */
  readonly onInsert: (name: string) => void;
}

export function ToolPalettesPanel({ focusRevision, onClose, onInsert }: ToolPalettesPanelProps) {
  const { t } = useI18n();
  const shell = useShell();
  const { panelRef } = useSurfacePanelFocus(onClose);
  // The panel's identity is its tab, so the keyboard lands there on every open
  // request — including the second `TOOLPALETTES` while the float is already up
  // (the shared focus hook focused on mount only).
  const tabRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    tabRef.current?.focus({ preventScroll: true });
  }, [focusRevision]);
  const rows = blockRows(shell.status);
  const loading = Boolean(shell.status?.loading);
  const documentOpen = Boolean(shell.status?.documentLabel) && !loading;
  const toolActive = Boolean(shell.status?.tool?.key);
  return (
    <aside
      ref={panelRef}
      tabIndex={-1}
      className="quick-view tool-palettes"
      role="dialog"
      aria-label={t('toolpalettes.title')}
    >
      <header className="quick-view__head">
        <span className="quick-view__title">
          <Icon name="palette" size={14} />
          {t('toolpalettes.title')}
        </span>
        <IconButton icon="close" size="xs" label={t('toolpalettes.close')} onClick={onClose} />
      </header>
      {/* One tab, and it is the panel's real state: `TPNAVIGATE Blocks` is this
          tab, so the tablist is one honest row rather than a fake chooser. */}
      <div className="quick-view__tabs" role="tablist">
        <button
          ref={tabRef}
          type="button"
          role="tab"
          aria-selected
          className="quick-view__tab is-active"
          tabIndex={0}
        >
          {t('toolpalettes.tab.blocks')}
        </button>
      </div>
      <p className="quick-view__subtitle">{t('toolpalettes.subtitle')}</p>
      {rows.length === 0 ? (
        <p className="quick-view__empty">{t('toolpalettes.empty')}</p>
      ) : (
        <BlockList
          rows={rows}
          documentOpen={documentOpen}
          loading={loading}
          toolActive={toolActive}
          onInsert={onInsert}
        />
      )}
      <footer className="quick-view__foot">{t('toolpalettes.tab.only_blocks')}</footer>
    </aside>
  );
}
