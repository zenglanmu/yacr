// The mobile top bar: one button, and nothing else.
//
// It replaces the title/QAT row, the ribbon tab strip and the drawing-tab strip on
// the compact layout. The CAD drawing is the thing that must get the space, so the
// bar keeps exactly one control: the 44x44 menu button that opens the ribbon
// drawer (`aria-expanded`/`aria-controls`, the smallest comfortable touch target,
// and the only way into the tools).
//
// The open drawing's name is **not** here. A filename is not a control, and on a
// 390px phone a title line either truncates a name the user cannot read or pushes
// the one button aside; it now lives in the drawer's own header, where a tap on the
// menu button shows the full name and the tools at the same time. The theme switch
// moved to the navigation dock's fixed head (mobile only) for the same reason: one
// row, one control, and it stays reachable without scrolling. Nothing else competes
// with the drawing.
//
// The import strip is the one addition, and it is 2px tall: the desktop puts open
// progress in the drawing-tab strip, which does not exist here, so a long DWG
// import would otherwise be invisible (a blank drawing area with no hint that
// something is running). It reports the host's own progress figure and never
// invents one; a determinate import fills the bar, an indeterminate one slides.
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
