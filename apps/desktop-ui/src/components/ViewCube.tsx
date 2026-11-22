// ViewCube: the orientation ball, a floating overlay over the drawing area's
// top-right corner (left of the navigation rail), never part of the dock column.
//
// It is DOM chrome painted over the CAD surface, but it is its own layer
// (`.viewball-layer`, `z-index: 6`): above the CAD carriers and the dock (`5`),
// below the floating palettes (`10`) and the notice layers. The 56px rail is
// unchanged, so the ball never takes drawing width or widens the dock; on the
// compact layout it moves to the drawing's top-left corner, away from the dock
// card at the top-right.
//
// The ball itself is a **2D canvas**: six standard-view caps on an opaque sphere,
// drawn from the host's own camera basis (`cad_status.view` through the volatile
// store, so a free orbit re-renders only this leaf and never the shell). A new
// `Vec3` basis per host frame is the only redraw trigger; there is no animation
// loop and no invented camera.
//
// Interaction is split in two, like AutoCAD's own cube:
//
//   * a click on a visible cap (a pointer movement under `CLICK_SLOP_PX`) is the
//     corresponding **standard view**, sent through `runStandardView` with the
//     same admission rules as the ribbon;
//   * a primary-button drag is a real **orbit** (`cad_orbit`), coalesced to one
//     request per animation frame, and only when `orbitAvailable` says the carrier
//     and the current view allow it. A refused request stops the gesture; the
//     host's own `cad_status.error` is the eventual verdict either way.
//
// The canvas is decoration (`aria-hidden`): the six directions stay real,
// focusable buttons in an `sr-only` list, so keyboard and screen-reader users get
// the same labels, titles and disabled reasons as before, and focusing one
// highlights its cap on the canvas. A `null`/degenerate basis, or a canvas that
// cannot get a 2D context, renders an explicit unavailable state — never a blank
// box.
import React, { useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react';
import { Icon } from '../icons.tsx';
import { useI18n, useShell } from '../context.ts';
import { volatileStatusStore } from '../cad/status-cadence.ts';
import { navigationSettings, viewCubeBasis, viewCubeVisible } from '../cad/viewcube.ts';
import type { ViewCubeBasis } from '../cad/viewcube.ts';
import type { CadViewOrientation } from '../cad/cad-status.ts';
import { drawSphere, pickSphere, readSpherePalette, SPHERE_DIRECTIONS } from '../cad/viewcube-canvas.ts';
import type { SphereDirectionId, SpherePalette, SpherePoint } from '../cad/viewcube-canvas.ts';
import { OrbitDrag, orbitAvailable, orbitDelta, orbitRadiansPerPixel, orbitUnavailableReasonKey, ORBIT_NEEDS_3D_KEY } from '../cad/orbit.ts';
import { standardViewAvailable, standardViewUnavailableReasonKey } from '../cad/view3d.ts';
import { THEME_ATTRIBUTE } from '../theme/theme.ts';
import './viewcube.css';

/** A pointer that travels less than this many CSS px is a click, not a drag. */
const CLICK_SLOP_PX = 4;

/** The catalog key of each direction's accessible name (the painted glyph is ASCII). */
const DIRECTION_LABEL_KEYS: Readonly<Record<SphereDirectionId, string>> = {
  front: 'viewcube.face.front',
  back: 'viewcube.face.back',
  left: 'viewcube.face.left',
  right: 'viewcube.face.right',
  top: 'viewcube.face.top',
  bottom: 'viewcube.face.bottom',
};

/** The direction definition by painted id (the canvas table is the single source). */
const DIRECTION_BY_ID: ReadonlyMap<SphereDirectionId, (typeof SPHERE_DIRECTIONS)[number]> =
  new Map(SPHERE_DIRECTIONS.map(direction => [direction.id, direction]));

/** One in-progress pointer gesture on the canvas. */
interface PointerGesture {
  readonly pointerId: number;
  readonly startX: number;
  readonly startY: number;
  lastX: number;
  lastY: number;
  /** Whether the pointer passed the click slop (a drag, not a click). */
  dragging: boolean;
  /** The coalescer, or `null` when this carrier/view cannot orbit. */
  readonly orbit: OrbitDrag | null;
}

/** The Home/isometric target, clickable in both ball states. */
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

/** The honest "nothing can be drawn" frame: an icon and the explicit sentence. */
function Fallback({ message }: { readonly message: string }) {
  return (
    <div className="viewcube__unavailable" role="img" aria-label={message} title={message}>
      <Icon name="view" size={20} />
    </div>
  );
}

/**
 * The floating orientation ball.
 *
 * Visibility is host truth: `NAVVCUBEDISPLAY`'s class bit plus the matching
 * `DISPLAYVIEWCUBEIN2D/3D` gate. Hidden means **nothing** is rendered (no empty
 * frame, no fake sphere). Before the first status answer there is no host truth
 * to apply, so the ball is not rendered either.
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
  // there is nothing for the ball to orient. The rail still renders beside it.
  if (!shell.documentVisible) return null;
  if (!viewCubeVisible(view, navigationSettings(shell.status.navigation))) return null;
  const title = t('viewcube.title');
  const basis = viewCubeBasis(view);
  if (!view || !basis) {
    return (
      <div className="viewcube viewcube--unavailable" role="group" aria-label={title}>
        <Fallback message={t('viewcube.orientation_unavailable')} />
        <HomeButton />
      </div>
    );
  }
  return <SphereStage basis={basis} view={view} />;
}

/**
 * The live ball: canvas, hidden direction buttons and Home.
 *
 * A separate component so the render/resize/theme hooks always run in the same
 * order: the parent returns early for the unavailable states.
 */
function SphereStage({ basis, view }: { readonly basis: ViewCubeBasis; readonly view: CadViewOrientation }) {
  const { t } = useI18n();
  const shell = useShell();
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const gestureRef = useRef<PointerGesture | null>(null);
  const [size, setSize] = useState(0);
  const [palette, setPalette] = useState<SpherePalette | null>(null);
  const [fontFamily, setFontFamily] = useState('');
  const [themeRead, setThemeRead] = useState(false);
  const [failed, setFailed] = useState(false);
  const [hover, setHover] = useState<SphereDirectionId | null>(null);
  const [focus, setFocus] = useState<SphereDirectionId | null>(null);

  const title = t('viewcube.title');
  const orbitEnabled = orbitAvailable({ ...shell, view });
  const orbitReasonKey = orbitUnavailableReasonKey({ ...shell, view });
  // The user's 3D drag rate (`cad_status.navigation`), a single multiplier for
  // the mouse and for touch. A host that predates the field keeps the base rate
  // (`?? 100`), and `orbitRadiansPerPixel` refuses a malformed percentage.
  const orbitRate = orbitRadiansPerPixel(
    shell.status?.navigation?.orbitSensitivityPercent ?? 100,
  );
  // The ball's own 3D sentence is more useful here than the shared ribbon one;
  // every other reason (no document, carrier refusal) is already cube-neutral.
  const orbitHint = orbitEnabled
    ? t('viewcube.orbit_hint')
    : t(orbitReasonKey === ORBIT_NEEDS_3D_KEY ? 'viewcube.orbit_needs_3d' : orbitReasonKey, {
      command: t('view.orbit_label'),
    });
  const groupTitle = `${title} — ${orbitHint} — ${t('viewcube.standard_only')}`;
  const disabled = SPHERE_DIRECTIONS
    .filter(direction => !standardViewAvailable(shell, direction.view))
    .map(direction => direction.id);
  const disabledKey = disabled.join(',');

  // The canvas' CSS size is the drawing's coordinate space: `projectSphere` and a
  // pointer position both use CSS px inside this square. Reading it once on
  // mount covers a canvas that is already laid out; the observer covers a layout
  // change (desktop <-> compact) and a window/DOM zoom.
  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const measure = (): void => {
      const width = canvas.getBoundingClientRect().width;
      if (!(width > 0)) return;
      setSize(current => (Math.abs(current - width) < 0.01 ? current : width));
    };
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, []);

  // The theme is a document attribute, and canvas colours are not inherited from
  // CSS: the tokens are re-read whenever `<html data-theme>` changes, so the
  // light theme is real rather than the dark palette painted once at mount. The
  // label font comes from `--font-ui` (a custom property on `:root`); `html`'s
  // own computed `font-family` is the UA default, not the chrome's.
  useLayoutEffect(() => {
    if (typeof window === 'undefined' || typeof document === 'undefined') return;
    const read = (): void => {
      const style = window.getComputedStyle(document.documentElement);
      setPalette(readSpherePalette(token => style.getPropertyValue(token)));
      setFontFamily(style.getPropertyValue('--font-ui').trim());
      setThemeRead(true);
    };
    read();
    if (typeof MutationObserver === 'undefined') return;
    const observer = new MutationObserver(read);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: [THEME_ATTRIBUTE] });
    return () => observer.disconnect();
  }, []);

  // One paint per changed input (host basis, box size, hover/focus, theme,
  // admission). No rAF loop: an idle ball costs nothing.
  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || size <= 0 || !palette) return;
    const ratio = typeof window === 'undefined' ? 1 : window.devicePixelRatio || 1;
    const physical = Math.max(1, Math.round(size * ratio));
    if (canvas.width !== physical) canvas.width = physical;
    if (canvas.height !== physical) canvas.height = physical;
    let ctx: CanvasRenderingContext2D | null = null;
    try {
      ctx = canvas.getContext('2d');
    } catch {
      ctx = null;
    }
    if (!ctx) {
      setFailed(true);
      return;
    }
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    drawSphere(ctx, basis, size, palette, { hover, focus, disabled, fontFamily });
  }, [basis, size, palette, fontFamily, hover, focus, disabledKey]);

  // An unmount mid-drag (the layer hides the ball, the view switches to 2D) must
  // stop the coalescer: no queued frame may fire against a gone canvas.
  useLayoutEffect(() => () => {
    gestureRef.current?.orbit?.end();
    gestureRef.current = null;
  }, []);

  /** A pointer position in the canvas' CSS-px coordinate space. */
  const localPoint = (event: React.PointerEvent<HTMLCanvasElement>): SpherePoint => {
    const rect = event.currentTarget.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  };

  const handlePointerDown = (event: React.PointerEvent<HTMLCanvasElement>): void => {
    // Primary button/pen only: a right-click is the context menu, never an orbit.
    // A second touch contact is ignored outright so a pinch cannot hijack (or
    // orphan) the gesture already in progress.
    if (event.button !== 0 || !event.isPrimary || gestureRef.current) return;
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      // Capture is a nicety; the pointerup/lostpointercapture pair still ends
      // the gesture if the engine refuses it.
    }
    gestureRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      lastX: event.clientX,
      lastY: event.clientY,
      dragging: false,
      orbit: orbitEnabled ? new OrbitDrag({ send: (yaw, pitch) => shell.runOrbit(yaw, pitch) }) : null,
    };
  };

  const handlePointerMove = (event: React.PointerEvent<HTMLCanvasElement>): void => {
    const gesture = gestureRef.current;
    if (!gesture || gesture.pointerId !== event.pointerId) {
      const next = pickSphere(basis, localPoint(event), size);
      setHover(current => (current === next ? current : next));
      return;
    }
    const from: readonly [number, number] = [gesture.lastX, gesture.lastY];
    const to: readonly [number, number] = [event.clientX, event.clientY];
    gesture.lastX = to[0];
    gesture.lastY = to[1];
    if (!gesture.dragging) {
      const travelled = Math.hypot(to[0] - gesture.startX, to[1] - gesture.startY);
      if (travelled < CLICK_SLOP_PX) return;
      gesture.dragging = true;
      setHover(null);
    }
    if (!gesture.orbit) return;
    const delta = orbitDelta(from, to, orbitRate);
    gesture.orbit.push(delta.yaw, delta.pitch);
  };

  const handlePointerUp = (event: React.PointerEvent<HTMLCanvasElement>): void => {
    const gesture = gestureRef.current;
    if (!gesture || gesture.pointerId !== event.pointerId) return;
    gestureRef.current = null;
    if (gesture.orbit) {
      // Drain the movement the last frame had not sent yet, then end the gesture
      // for good. A refusal already stopped the coalescer, so this is not a retry.
      gesture.orbit.flush();
      gesture.orbit.end();
    }
    if (gesture.dragging) return;
    const id = pickSphere(basis, localPoint(event), size);
    if (!id) return;
    const direction = DIRECTION_BY_ID.get(id);
    if (!direction) return;
    // Same admission as the hidden buttons: a direction the carrier refuses is inert.
    if (!standardViewAvailable(shell, direction.view)) return;
    shell.runStandardView(direction.view);
  };

  const endGesture = (): void => {
    const gesture = gestureRef.current;
    gestureRef.current = null;
    // A cancelled capture ends the drag without inventing a camera move.
    gesture?.orbit?.end();
  };

  return (
    <div className="viewcube" role="group" aria-label={title} title={groupTitle}>
      {failed || (themeRead && !palette) ? (
        <Fallback message={t('viewcube.render_unavailable')} />
      ) : (
        <div className="viewcube__stage">
          <canvas
            ref={canvasRef}
            className="viewcube__canvas"
            aria-hidden="true"
            onPointerDown={handlePointerDown}
            onPointerMove={handlePointerMove}
            onPointerUp={handlePointerUp}
            onPointerCancel={endGesture}
            onLostPointerCapture={endGesture}
            onPointerLeave={() => setHover(null)}
          />
        </div>
      )}
      {/* Real, focusable directions for keyboard and AT users. They are visually
          hidden; focusing one highlights its cap on the canvas, so focus stays
          visible. */}
      <div className="sr-only">
        {SPHERE_DIRECTIONS.map(direction => {
          const enabled = standardViewAvailable(shell, direction.view);
          const label = t(DIRECTION_LABEL_KEYS[direction.id]);
          const refusal = t(standardViewUnavailableReasonKey(shell, direction.view), { command: label });
          return (
            <button
              key={direction.id}
              type="button"
              disabled={!enabled}
              aria-label={label}
              title={enabled ? label : `${label} — ${refusal}`}
              onFocus={() => setFocus(direction.id)}
              onBlur={() => setFocus(current => (current === direction.id ? null : current))}
              onClick={() => shell.runStandardView(direction.view)}
            >
              {label}
            </button>
          );
        })}
      </div>
      <HomeButton />
    </div>
  );
}
