// One floating container for every left-column palette.
//
// The window is an absolutely positioned overlay inside `.palette-layer`, which
// itself sits inside `.workspace__canvas` after `.native-input`. That is the
// whole point: opening, closing, dragging or resizing a palette never changes
// the `.native-input` rect, so the host receives no resize report and never
// re-presents the CAD surface. The layer only paints; its windows alone take
// pointer input.
//
// The palette's own `.palette__head` is the title bar - no second header is
// added. Pressing header chrome starts a drag; pressing a control inside it
// (close/create, a form field) is left to the control. A right-edge grab strip
// resizes the width, with ArrowLeft/ArrowRight when it has keyboard focus.
//
// Geometry is a module-level store keyed by `windowKey`, read through
// `useSyncExternalStore`, so closing and reopening a palette restores where the
// user left it. There is deliberately no `localStorage` persistence: the store
// lives for the lifetime of the document, and a reload starts from the default
// docked position again.
import React, { useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react';
import { useI18n } from '../context.ts';
import {
  PALETTE_MIN_WIDTH,
  PALETTE_RESIZE_STEP,
  PALETTE_Z_BASE,
  clampGeometry,
  defaultPaletteGeometry,
  defaultPaletteWidth,
  isDragHandleTarget,
  moveGeometry,
  resizeGeometry,
} from '../layout/palette-window.ts';
import type { DragTargetLike, PaletteGeometry, PaletteLayerSize } from '../layout/palette-window.ts';

export interface PaletteWindowProps {
  /** Stable identity for the geometry store; survives close/reopen. */
  readonly windowKey: string;
  /** Cascade slot for the first placement (0 = top-left, 1 = +24px, …). */
  readonly initialIndex?: number;
  readonly children: React.ReactNode;
}

interface WindowState {
  readonly geometry: PaletteGeometry;
  readonly z: number;
  /** The user moved or resized this window; its width is never overwritten. */
  readonly touched: boolean;
  /** The first real layer measurement has already chosen the default width. */
  readonly sized: boolean;
}

const STATES = new Map<string, WindowState>();
const LISTENERS = new Set<() => void>();
let nextZ = PALETTE_Z_BASE;
// The width the last workspace preset asked for, applied to every existing window
// and every window created afterwards. `null` keeps the responsive default.
let presetWidth: number | null = null;

function notify(): void {
  for (const listener of [...LISTENERS]) listener();
}

/**
 * The state of one window. The first read creates it from the default, which is
 * what "initialise on first open" means here. The same object is returned until
 * a drag/resize/press changes it, as `useSyncExternalStore` requires.
 */
function stateOf(windowKey: string, initialIndex: number): WindowState {
  const existing = STATES.get(windowKey);
  if (existing) return existing;
  const index = Number.isFinite(initialIndex) ? Math.max(0, Math.floor(initialIndex)) : 0;
  const geometry = defaultPaletteGeometry(index);
  const created: WindowState = {
    geometry: presetWidth === null ? geometry : { ...geometry, width: presetWidth },
    // Properties (index 0) starts on top; the cascade already keeps every header
    // visible, and a press raises the pressed window above all of them.
    z: PALETTE_Z_BASE - index,
    touched: false,
    sized: false,
  };
  STATES.set(windowKey, created);
  return created;
}

function subscribe(listener: () => void): () => void {
  LISTENERS.add(listener);
  return () => { LISTENERS.delete(listener); };
}

function setGeometry(windowKey: string, initialIndex: number, geometry: PaletteGeometry): void {
  const current = stateOf(windowKey, initialIndex);
  // Any explicit geometry change is the user's: once touched, the responsive
  // first-measure default must never overwrite the window again.
  STATES.set(windowKey, { ...current, geometry, touched: true, sized: true });
  notify();
}

/**
 * Apply the responsive default width once, on the first usable layer
 * measurement, and only while the user has done nothing to the window. The
 * `sized` flag makes it a one-time decision, so a later layer resize does not
 * rewrite a width the user never set either.
 */
function applyResponsiveDefault(windowKey: string, initialIndex: number, layerWidth: number): void {
  if (!Number.isFinite(layerWidth) || layerWidth <= 0) return;
  const current = stateOf(windowKey, initialIndex);
  if (current.touched || current.sized) return;
  const width = defaultPaletteWidth(layerWidth);
  const changed = width !== current.geometry.width;
  STATES.set(windowKey, {
    ...current,
    geometry: changed ? { ...current.geometry, width } : current.geometry,
    sized: true,
  });
  if (changed) notify();
}

function raise(windowKey: string, initialIndex: number): void {
  const current = stateOf(windowKey, initialIndex);
  // Pressing the window that is already on top is not a stacking change, so it
  // costs no re-render (and cannot disturb a focused control).
  if (current.z === nextZ) return;
  nextZ += 1;
  STATES.set(windowKey, { ...current, z: nextZ });
  notify();
}

type DragMode = 'move' | 'resize';

/**
 * Apply a workspace preset's panel width to every palette window.
 *
 * The width is stored as the default for windows created later too, which is
 * what makes a preset survive its own palettes being closed and reopened. The
 * value is clamped at render time by `clampGeometry`, so an over-wide preset on
 * a narrow drawing is narrowed rather than clipped.
 */
export function applyPaletteWidthPreset(width: number): void {
  if (!Number.isFinite(width) || width <= 0) return;
  presetWidth = Math.round(width);
  for (const [key, state] of STATES) {
    STATES.set(key, {
      ...state,
      geometry: { ...state.geometry, width: presetWidth },
      touched: true,
      sized: true,
    });
  }
  notify();
}

/** Clear the preset width (tests; a fresh document starts responsive again). */
export function clearPaletteWidthPreset(): void {
  presetWidth = null;
}

interface DragSession {
  readonly mode: DragMode;
  readonly pointerId: number;
  readonly startX: number;
  readonly startY: number;
  readonly origin: PaletteGeometry;
  readonly layer: PaletteLayerSize;
}

/** The structural part of a pointer event the drag maths needs. */
interface PointerStart {
  readonly clientX: number;
  readonly clientY: number;
  readonly pointerId: number;
  preventDefault(): void;
}

/** The layer's content box, measured at interaction time. */
function measureLayer(element: HTMLElement | null): PaletteLayerSize | null {
  const layer = element?.parentElement;
  if (!layer) return null;
  return { width: layer.clientWidth, height: layer.clientHeight };
}

export function PaletteWindow({ windowKey, initialIndex = 0, children }: PaletteWindowProps) {
  const { t } = useI18n();
  const windowRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<DragSession | null>(null);
  const [dragging, setDragging] = useState(false);
  const [layer, setLayer] = useState<PaletteLayerSize | null>(null);
  const state = useSyncExternalStore(
    subscribe,
    () => stateOf(windowKey, initialIndex),
    () => stateOf(windowKey, initialIndex),
  );

  useLayoutEffect(() => {
    const parent = windowRef.current?.parentElement ?? null;
    if (!parent) return;
    const refresh = () => setLayer(measureLayer(windowRef.current));
    refresh();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(refresh);
    observer.observe(parent);
    return () => observer.disconnect();
  }, []);

  // First real measurement chooses the responsive default width. It runs once
  // per window (`sized`) and never while `touched`, so a drag/resize is final.
  const measuredWidth = layer?.width ?? 0;
  useLayoutEffect(() => {
    applyResponsiveDefault(windowKey, initialIndex, measuredWidth);
  }, [measuredWidth, windowKey, initialIndex]);

  // The displayed placement is always clamped to the measured layer, so a
  // window that was moved on a wide drawing stays reachable after a resize; the
  // stored geometry is left alone and is restored when the layer grows again.
  const view = layer === null ? state.geometry : clampGeometry(state.geometry, layer);
  const maximumWidth = layer === null
    ? Math.max(PALETTE_MIN_WIDTH, view.width)
    : Math.max(PALETTE_MIN_WIDTH, Math.round(layer.width));

  function startSession(mode: DragMode, event: PointerStart): void {
    const size = measureLayer(windowRef.current) ?? layer;
    if (!size) return;
    event.preventDefault();
    windowRef.current?.setPointerCapture?.(event.pointerId);
    dragRef.current = {
      mode,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      origin: clampGeometry(stateOf(windowKey, initialIndex).geometry, size),
      layer: size,
    };
    setDragging(true);
  }

  function onPointerDown(event: React.PointerEvent<HTMLDivElement>): void {
    if (event.button !== 0) return;
    const target = event.target as unknown as Element;
    if (!isDragHandleTarget(event.target as unknown as DragTargetLike)) return;
    if (!target.closest?.('.palette__head')) return;
    startSession('move', event);
  }

  function onResizePointerDown(event: React.PointerEvent<HTMLDivElement>): void {
    if (event.button !== 0) return;
    startSession('resize', event);
  }

  function onPointerMove(event: React.PointerEvent<HTMLDivElement>): void {
    const session = dragRef.current;
    if (!session || session.pointerId !== event.pointerId) return;
    event.preventDefault();
    const deltaX = event.clientX - session.startX;
    const deltaY = event.clientY - session.startY;
    const next = session.mode === 'move'
      ? moveGeometry(session.origin, deltaX, deltaY, session.layer)
      : resizeGeometry(session.origin, session.origin.width + deltaX, session.layer);
    setGeometry(windowKey, initialIndex, next);
  }

  function endSession(event: React.PointerEvent<HTMLDivElement>): void {
    const session = dragRef.current;
    if (!session || session.pointerId !== event.pointerId) return;
    dragRef.current = null;
    setDragging(false);
    const element = windowRef.current;
    if (element?.hasPointerCapture?.(event.pointerId)) {
      element.releasePointerCapture(event.pointerId);
    }
  }

  function onResizeKeyDown(event: React.KeyboardEvent<HTMLDivElement>): void {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    const size = measureLayer(windowRef.current) ?? layer;
    if (!size) return;
    event.preventDefault();
    const current = clampGeometry(stateOf(windowKey, initialIndex).geometry, size);
    const delta = event.key === 'ArrowRight' ? PALETTE_RESIZE_STEP : -PALETTE_RESIZE_STEP;
    setGeometry(windowKey, initialIndex, resizeGeometry(current, current.width + delta, size));
  }

  return (
    <div
      ref={windowRef}
      className={['palette-window', dragging ? 'is-dragging' : ''].filter(Boolean).join(' ')}
      style={{
        left: view.left,
        top: view.top,
        width: view.width,
        // The content never runs past the layer's bottom edge; the palette body
        // is its own scroller.
        maxHeight: `calc(100% - ${view.top}px)`,
        zIndex: state.z,
      }}
      onPointerDownCapture={() => raise(windowKey, initialIndex)}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endSession}
      onPointerCancel={endSession}
    >
      {children}
      <div
        className="palette-window__resize"
        role="separator"
        aria-orientation="vertical"
        aria-label={t('palette.resize_width')}
        aria-valuemin={PALETTE_MIN_WIDTH}
        aria-valuemax={maximumWidth}
        aria-valuenow={Math.round(view.width)}
        tabIndex={0}
        onPointerDown={onResizePointerDown}
        onKeyDown={onResizeKeyDown}
      />
    </div>
  );
}
