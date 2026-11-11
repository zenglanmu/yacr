// ViewCube: the AutoCAD-style orientation control, docked above the navigation
// rail at the right edge of the drawing area.
//
// It is DOM chrome **in the dock column**, never a floating overlay across the
// native CAD surface (the host composites that surface below the webview and
// cannot put a decorative layer over it). The cube itself is a CSS 3D transform:
// six real `<button>` faces plus a Home target, rotated from the host's own
// camera basis (`cad_status.view` via the volatile store, so a free orbit
// re-renders only this leaf and never the shell). The host stays the authority
// on which standard views exist; the buttons use the same admission helper as the
// ribbon, so a carrier without `cad_standard_view` disables them with the same
// reason.
//
// A `null` view is rendered as an explicit unavailable state: the cube does not
// rotate from a guessed orientation. Edge/corner regions are not implemented,
// which the cube's title states rather than pretending they work.
import React, { useSyncExternalStore } from 'react';
import { Icon } from '../icons.tsx';
import { useI18n, useShell } from '../context.ts';
import { volatileStatusStore } from '../cad/status-cadence.ts';
import { navigationSettings, viewCubeMatrix, viewCubeVisible } from '../cad/viewcube.ts';
import { standardViewAvailable, standardViewUnavailableReasonKey } from '../cad/view3d.ts';
import type { StandardViewName } from '../cad/view3d.ts';
import './viewcube.css';

/** One clickable face: the short painted label and its catalog sentence. */
interface CubeFace {
  /** CSS modifier and DOM identity (`front`, `back`, …). */
  readonly id: string;
  /** Short painted label; the full name lives in `aria-label`/`title`. */
  readonly short: string;
  readonly labelKey: string;
  /** The host's standard-view key the face sends. */
  readonly view: StandardViewName;
}

/**
 * The six faces in DOM order. The painted labels are short (three letters) so
 * they survive isometric shear on a small face; `viewcube.face.*` is the
 * accessible name and the tooltip.
 */
const FACES: readonly CubeFace[] = [
  { id: 'front', short: 'FRT', labelKey: 'viewcube.face.front', view: 'front' },
  { id: 'back', short: 'BCK', labelKey: 'viewcube.face.back', view: 'back' },
  { id: 'left', short: 'LFT', labelKey: 'viewcube.face.left', view: 'left' },
  { id: 'right', short: 'RGT', labelKey: 'viewcube.face.right', view: 'right' },
  { id: 'top', short: 'TOP', labelKey: 'viewcube.face.top', view: 'top' },
  { id: 'bottom', short: 'BOT', labelKey: 'viewcube.face.bottom', view: 'bottom' },
];

/** The Home/isometric target, clickable in both cube states. */
function HomeButton() {
  const { t } = useI18n();
  const shell = useShell();
  const enabled = standardViewAvailable(shell, 'isometric');
  const label = t('viewcube.home');
  const refusal = t(standardViewUnavailableReasonKey(shell, 'isometric'), { command: label });
  return (
    <button
      type="button"
      className="viewcube__home"
      disabled={!enabled}
      aria-label={label}
      title={enabled ? label : `${label} — ${refusal}`}
      onClick={() => shell.runStandardView('isometric')}
    >
      <Icon name="home" size={15} />
    </button>
  );
}

/**
 * The docked orientation control.
 *
 * Visibility is host truth: `NAVVCUBEDISPLAY`'s class bit plus the matching
 * `DISPLAYVIEWCUBEIN2D/3D` gate. Hidden means **nothing** is rendered (no empty
 * frame, no fake cube). Before the first status answer there is no host truth to
 * apply, so the cube is not rendered either.
 */
export function ViewCube() {
  const { t } = useI18n();
  const shell = useShell();
  // The live basis only: subscribing here keeps a 60 Hz orbit out of the shell.
  const view = useSyncExternalStore(
    volatileStatusStore.subscribe,
    volatileStatusStore.getView,
    volatileStatusStore.getView,
  );
  if (!shell.status) return null;
  // The start page has no drawing and therefore no authoritative viewport;
  // there is nothing for a cube to orient. The rail still renders beside it.
  if (!shell.documentVisible) return null;
  if (!viewCubeVisible(view, navigationSettings(shell.status.navigation))) return null;
  const matrix = viewCubeMatrix(view);
  if (!view || !matrix) {
    return (
      <div className="viewcube viewcube--unavailable" role="group" aria-label={t('viewcube.title')}>
        <div
          className="viewcube__unavailable"
          role="img"
          aria-label={t('viewcube.orientation_unavailable')}
          title={t('viewcube.orientation_unavailable')}
        >
          <Icon name="view" size={20} />
        </div>
        <HomeButton />
      </div>
    );
  }
  const title = t('viewcube.title');
  return (
    <div
      className="viewcube"
      role="group"
      aria-label={title}
      title={`${title} — ${t('viewcube.standard_only')}`}
    >
      <div
        className="viewcube__stage"
        data-projection={view.projection}
      >
        {/* The rotation is the host basis, computed in `cad/viewcube.ts`. Each
            face below is a real button; the stage only carries the 3D transform. */}
        <div className="viewcube__cube" style={{ transform: matrix }}>
          {FACES.map(face => {
            const enabled = standardViewAvailable(shell, face.view);
            const label = t(face.labelKey);
            const refusal = t(standardViewUnavailableReasonKey(shell, face.view), { command: label });
            return (
              <button
                key={face.id}
                type="button"
                className={`viewcube__face viewcube__face--${face.id}`}
                disabled={!enabled}
                aria-label={label}
                title={enabled ? label : `${label} — ${refusal}`}
                onClick={() => shell.runStandardView(face.view)}
              >
                <span className="viewcube__label">{face.short}</span>
              </button>
            );
          })}
        </div>
      </div>
      <HomeButton />
    </div>
  );
}
