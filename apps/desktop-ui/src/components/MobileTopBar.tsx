// The mobile top bar: one menu button and the drawing's name.
//
// It replaces the title/QAT row, the ribbon tab strip and the drawing-tab strip
// on the compact layout. Deliberately almost empty: the CAD drawing is the thing
// that must get the width, so nothing here competes with it. The menu button is
// the one control (44x44 CSS px, the smallest comfortable touch target, and the
// only way into the ribbon), and the label is one ellipsized line.
//
// The import strip is the one addition, and it is 2px tall: the desktop puts
// open progress in the drawing-tab strip, which does not exist here, so a long
// DWG import would otherwise be invisible (a blank drawing area with no hint
// that something is running). It reports the host's own progress figure and
// never invents one; a determinate import fills the bar, an indeterminate one
// slides.
import React from 'react';
import { Icon } from '../icons.tsx';
import { useI18n, useShell } from '../context.ts';
import { importProgressView } from '../cad/cad-status.ts';
import type { CadImportProgress } from '../cad/cad-status.ts';

export interface MobileTopBarProps {
  /** Whether the ribbon drawer is open (`aria-expanded`). */
  readonly menuOpen: boolean;
  /** Toggle the drawer. */
  readonly onMenu: () => void;
  /** The drawer's DOM id (`aria-controls`). */
  readonly drawerId: string;
}

export function MobileTopBar({ menuOpen, onMenu, drawerId }: MobileTopBarProps) {
  const { t } = useI18n();
  const shell = useShell();
  // The drawing's own label while one is open; on the start state the title is
  // the same "Get started" the desktop start panel and doc tab already use, so
  // the bar never shows an empty slot or an invented file name.
  const label = shell.documentLabel ?? t('start.title');
  const progress = importProgressView(shell.importProgress as CadImportProgress | null, t);
  const percent = progress && progress.determinate ? Math.round((progress.ratio ?? 1) * 100) : null;
  const running = progress !== null || shell.loading;
  return (
    <header className="mobile-topbar app-region" role="banner">
      <button
        type="button"
        className={['mobile-topbar__menu', menuOpen ? 'is-open' : ''].filter(Boolean).join(' ')}
        aria-label={t('mobile.menu')}
        aria-expanded={menuOpen}
        aria-controls={drawerId}
        onClick={onMenu}
      >
        <Icon name="menu" size={22} />
      </button>
      <span className="mobile-topbar__title" title={label}>{label}</span>
      {running ? (
        <span
          className={['mobile-topbar__progress', progress && progress.determinate ? '' : 'is-indeterminate'].filter(Boolean).join(' ')}
          role="progressbar"
          aria-label={progress ? progress.phase : t('import.panel')}
          aria-valuemin={progress && progress.determinate ? 0 : undefined}
          aria-valuemax={progress && progress.determinate ? 100 : undefined}
          aria-valuenow={progress && progress.determinate && percent !== null ? percent : undefined}
        >
          <span
            className="mobile-topbar__progress-fill"
            style={progress && progress.determinate ? { width: `${percent}%` } : undefined}
          />
        </span>
      ) : null}
    </header>
  );
}
