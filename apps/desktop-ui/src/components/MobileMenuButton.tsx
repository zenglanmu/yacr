// The compact layout's menu button: one floating control, no top bar.
//
// The compact layout has no title row, no ribbon band and no drawing-tab strip,
// and it no longer spends a 52px strip on the one control the drawing needs. The
// CAD drawing starts at the viewport's first pixel; this 44x44 button floats over
// its **top-left corner** as a fixed overlay (`aria-expanded`/`aria-controls` name
// the ribbon drawer) and is the only way into the tools. 44px square is the
// smallest comfortable touch target, and it is the only painted node: no row, no
// scrim, no wrapper wider than the button, so every pixel around it still reaches
// the CAD surface (`mobile.css` keeps the `.mobile-topbar` class names because the
// web smoke harness and the Android host's layout log select them as the compact
// chrome's anchor; nothing under that name paints a bar any more).
//
// The open drawing's name is **not** here. A filename is not a control, and on a
// 390px phone a title line either truncates a name the user cannot read or pushes
// the one button aside; it lives in the drawer's own header, where a tap on this
// button shows the full name and the tools at the same time. The theme switch
// lives in the navigation dock's fixed head (mobile only) for the same reason: it
// stays reachable without scrolling, and nothing competes with the drawing.
//
// The import strip is the one addition, and it is 2px tall: the desktop puts open
// progress in the drawing-tab strip, which does not exist here, so a long DWG
// import would otherwise be invisible (a blank drawing area with no hint that
// something is running). It reports the host's own progress figure and never
// invents one; a determinate import fills the strip, an indeterminate one slides.
// It is pinned to the drawing's own top edge (below the cutout inset) and takes no
// input, because a progress report must not cost the drawing a press.
import React from 'react';
import { Icon } from '../icons.tsx';
import { useI18n, useShell } from '../context.ts';
import { importProgressView } from '../cad/cad-status.ts';
import type { CadImportProgress } from '../cad/cad-status.ts';

export interface MobileMenuButtonProps {
  /** Whether the ribbon drawer is open (`aria-expanded`). */
  readonly menuOpen: boolean;
  /** Toggle the drawer. */
  readonly onMenu: () => void;
  /** The drawer's DOM id (`aria-controls`). */
  readonly drawerId: string;
}

export function MobileMenuButton({ menuOpen, onMenu, drawerId }: MobileMenuButtonProps) {
  const { t } = useI18n();
  const shell = useShell();
  const progress = importProgressView(
    shell.importProgress as CadImportProgress | null,
    t,
    shell.openProgress,
  );
  const percent = progress && progress.determinate ? Math.round((progress.ratio ?? 1) * 100) : null;
  const running = progress !== null || shell.loading;
  return (
    <div className="mobile-topbar app-region">
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
    </div>
  );
}
