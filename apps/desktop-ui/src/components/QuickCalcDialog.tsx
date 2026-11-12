// QuickCalc: a non-modal, draggable calculator float with two keypads.
//
// It is deliberately **not** a dialog: no scrim, no focus trap, no
// `aria-modal`. The drawing stays visible and usable around it, Escape closes it
// through the shared panel hook (`surface-focus.ts`) and the keyboard returns to
// whatever opened it. The header is the drag handle, and the window remembers
// where the user put it for the rest of the session (a module-level store, the
// same idea as the palette windows), so close/reopen restores the position.
//
// Three modules own three different jobs:
//   * `chrome/quick-calc.ts` owns the arithmetic (pure, angle-aware);
//   * `layout/quick-calc-window.ts` owns placement/clamping (pure, no DOM);
//   * this file owns the chrome, the pointer session and the keyboard.
//
// The keypad keeps the familiar AutoCAD 4-column numeric block (7 8 9 / 4 5 6 /
// 1 2 3 / ± 0 .) in **both** modes. Scientific mode adds one 5-column function
// strip above it, ordered by family down each column — direct trig, inverse
// trig, logarithms/roots, powers, constants/reciprocal — and a DEG/RAD switch
// beside the mode switch. Every key on screen is implemented by the engine;
// there is no placeholder key and no invented function.
import React, { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '../icons.tsx';
import { Button, IconButton } from './primitives.tsx';
import { useI18n } from '../context.ts';
import { writeClipboardText } from '../clipboard.ts';
import {
  INITIAL_CALC_STATE,
  appendCalcToken,
  applyCalcUnary,
  backspaceCalc,
  calcCopyText,
  calcDisplay,
  calcMemoryActive,
  calcPreview,
  clearCalc,
  evaluateCalc,
  formatCalcNumber,
  memoryAddCalc,
  memoryClearCalc,
  memoryRecallCalc,
  setCalcExpression,
  toggleCalcAngle,
  toggleSignCalc,
} from '../chrome/quick-calc.ts';
import type { CalcState, CalcUnaryKind } from '../chrome/quick-calc.ts';
import {
  QUICKCALC_EDGE_MARGIN,
  clampQuickCalc,
  defaultQuickCalcGeometry,
  moveQuickCalc,
  quickCalcNominalHeight,
  quickCalcWidth,
} from '../layout/quick-calc-window.ts';
import type { QuickCalcMode } from '../layout/quick-calc-window.ts';
import { isDragHandleTarget } from '../layout/palette-window.ts';
import type { DragTargetLike, PaletteGeometry, PaletteLayerSize } from '../layout/palette-window.ts';
import { useSurfacePanelFocus } from './surface-focus.ts';

/** The surface tone of one key; `digit` keys stay plain. */
type KeyTone = 'digit' | 'action' | 'equals' | 'func';

/** One keypad key: a face, its accessible name and the transition it runs. */
interface CalcKey {
  readonly id: string;
  /** What the key shows: a digit, a glyph or a short math name. */
  readonly face: string;
  /** Catalog key for the accessible name; `null` means the face is the name. */
  readonly labelKey: string | null;
  readonly tone: KeyTone;
  readonly run: (state: CalcState) => CalcState;
}

/** Build one key. The id is the label key (or the face for digits). */
function key(
  face: string,
  labelKey: string | null,
  run: (state: CalcState) => CalcState,
  tone: KeyTone,
): CalcKey {
  return { id: labelKey === null ? face : labelKey, face, labelKey, tone, run };
}

/** A key that inserts one engine token. */
function tokenKey(face: string, labelKey: string | null, value: string, tone: KeyTone = 'action'): CalcKey {
  return key(face, labelKey, state => appendCalcToken(state, value), tone);
}

/** A scientific key that inserts a function head or a constant. */
function fnKey(face: string, labelKey: string, value: string): CalcKey {
  return key(face, labelKey, state => appendCalcToken(state, value), 'func');
}

/** A scientific key that transforms the current operand. */
function unaryKey(face: string, labelKey: string, kind: CalcUnaryKind): CalcKey {
  return key(face, labelKey, state => applyCalcUnary(state, kind), 'func');
}

// The shared top rows: memory and parentheses/backspace/divide.
const TOP_ROWS: readonly (readonly CalcKey[])[] = Object.freeze([
  Object.freeze([
    key('MC', 'quickcalc.memory_clear', memoryClearCalc, 'action'),
    key('MR', 'quickcalc.memory_recall', memoryRecallCalc, 'action'),
    key('M+', 'quickcalc.memory_add', memoryAddCalc, 'action'),
    key('C', 'quickcalc.clear', clearCalc, 'action'),
  ]),
  Object.freeze([
    tokenKey('(', 'quickcalc.open_paren', '('),
    tokenKey(')', 'quickcalc.close_paren', ')'),
    key('⌫', 'quickcalc.backspace', backspaceCalc, 'action'),
    tokenKey('/', 'quickcalc.divide', '/'),
  ]),
]);

// The familiar numeric block. It is identical in both modes on purpose: a
// scientific toggle must never move the digits the user already knows.
const NUMERIC_ROWS: readonly (readonly CalcKey[])[] = Object.freeze([
  Object.freeze([
    tokenKey('7', null, '7', 'digit'),
    tokenKey('8', null, '8', 'digit'),
    tokenKey('9', null, '9', 'digit'),
    tokenKey('×', 'quickcalc.multiply', '*'),
  ]),
  Object.freeze([
    tokenKey('4', null, '4', 'digit'),
    tokenKey('5', null, '5', 'digit'),
    tokenKey('6', null, '6', 'digit'),
    tokenKey('-', 'quickcalc.minus', '-'),
  ]),
  Object.freeze([
    tokenKey('1', null, '1', 'digit'),
    tokenKey('2', null, '2', 'digit'),
    tokenKey('3', null, '3', 'digit'),
    tokenKey('+', 'quickcalc.plus', '+'),
  ]),
  Object.freeze([
    key('±', 'quickcalc.plus_minus', toggleSignCalc, 'action'),
    tokenKey('0', null, '0', 'digit'),
    tokenKey('.', 'quickcalc.decimal', '.'),
    key('=', 'quickcalc.equals', evaluateCalc, 'equals'),
  ]),
]);

/**
 * The scientific strip, written as five **columns** and rendered with
 * `grid-auto-flow: column` so each family reads downward:
 * trig, inverse trig, logs/roots, powers, constants/reciprocal.
 */
const FUNCTION_COLUMNS: readonly (readonly CalcKey[])[] = Object.freeze([
  Object.freeze([
    fnKey('sin', 'quickcalc.sci.sin', 'sin('),
    fnKey('cos', 'quickcalc.sci.cos', 'cos('),
    fnKey('tan', 'quickcalc.sci.tan', 'tan('),
  ]),
  Object.freeze([
    fnKey('asin', 'quickcalc.sci.asin', 'asin('),
    fnKey('acos', 'quickcalc.sci.acos', 'acos('),
    fnKey('atan', 'quickcalc.sci.atan', 'atan('),
  ]),
  Object.freeze([
    fnKey('ln', 'quickcalc.sci.ln', 'ln('),
    fnKey('log', 'quickcalc.sci.log', 'log('),
    fnKey('sqrt', 'quickcalc.sci.sqrt', 'sqrt('),
  ]),
  Object.freeze([
    fnKey('abs', 'quickcalc.sci.abs', 'abs('),
    unaryKey('x²', 'quickcalc.sci.square', 'square'),
    fnKey('xʸ', 'quickcalc.sci.power', '^'),
  ]),
  Object.freeze([
    unaryKey('1/x', 'quickcalc.sci.inverse', 'inverse'),
    fnKey('π', 'quickcalc.sci.pi', 'pi'),
    fnKey('e', 'quickcalc.sci.e', 'e'),
  ]),
]);

const FUNCTION_KEYS: readonly CalcKey[] = Object.freeze(FUNCTION_COLUMNS.flat());

/** The numeric keypad flattened: top rows then the familiar digit block. */
const KEYPAD_KEYS: readonly CalcKey[] = Object.freeze([...TOP_ROWS, ...NUMERIC_ROWS].flat());

/** The two mode buttons, in display order. */
const MODES: readonly (readonly [QuickCalcMode, string])[] = Object.freeze([
  Object.freeze(['basic', 'quickcalc.mode.basic'] as const),
  Object.freeze(['scientific', 'quickcalc.mode.scientific'] as const),
]);

type CopyState = 'idle' | 'copied' | 'failed';

/**
 * The smallest height the window may be clamped to. The inline `max-height` is
 * the space below the header, so without a floor dragging the header to the
 * bottom edge would collapse the card to a few pixels. With the floor the body
 * scrolls instead, and the header stays a usable drag target.
 */
const MIN_WINDOW_HEIGHT = 240;

// --- Session placement store ---------------------------------------------
// One QuickCalc window exists per document, so the store is a single value
// rather than the palette layer's keyed map. `moved` distinguishes "the user
// dragged it" (their position wins from now on) from "still at the default"
// (which is re-solved per mode, because the scientific card is taller).

interface Placement {
  readonly geometry: PaletteGeometry;
  readonly moved: boolean;
}

let placement: Placement | null = null;
const PLACEMENT_LISTENERS = new Set<() => void>();

function placementSnapshot(): Placement | null {
  return placement;
}

function subscribePlacement(listener: () => void): () => void {
  PLACEMENT_LISTENERS.add(listener);
  return () => { PLACEMENT_LISTENERS.delete(listener); };
}

/** Remember a user-chosen position for the rest of the document's lifetime. */
function rememberPlacement(geometry: PaletteGeometry): void {
  placement = { geometry, moved: true };
  for (const listener of [...PLACEMENT_LISTENERS]) listener();
}

/** The viewport in CSS px; a host without `window` gets a desktop-sized guess. */
function readViewport(): PaletteLayerSize {
  if (typeof window === 'undefined') return { width: 1280, height: 800 };
  return { width: window.innerWidth, height: window.innerHeight };
}

interface DragSession {
  readonly pointerId: number;
  readonly startX: number;
  readonly startY: number;
  readonly origin: PaletteGeometry;
  readonly viewport: PaletteLayerSize;
}

/** One keypad button. The accessible name is the translated label, always. */
function CalcKeyButton({ item, onRun }: {
  readonly item: CalcKey;
  readonly onRun: (item: CalcKey) => void;
}) {
  const { t } = useI18n();
  const label = item.labelKey === null ? item.face : t(item.labelKey);
  return (
    <button
      type="button"
      className={['quickcalc__key', item.tone === 'digit' ? '' : `is-${item.tone}`].filter(Boolean).join(' ')}
      aria-label={label}
      onClick={() => onRun(item)}
    >
      {item.face}
    </button>
  );
}

interface QuickCalcDialogProps {
  readonly onClose: () => void;
}

export function QuickCalcDialog({ onClose }: QuickCalcDialogProps) {
  const { t } = useI18n();
  const [calc, setCalc] = useState<CalcState>(INITIAL_CALC_STATE);
  const [mode, setMode] = useState<QuickCalcMode>('basic');
  const [copyState, setCopyState] = useState<CopyState>('idle');
  const [dragging, setDragging] = useState(false);
  const [viewport, setViewport] = useState<PaletteLayerSize>(readViewport);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const dragRef = useRef<DragSession | null>(null);
  const { panelRef } = useSurfacePanelFocus(onClose);
  const stored = useSyncExternalStore(subscribePlacement, placementSnapshot, placementSnapshot);

  // Tracking the viewport (not the drawing area) keeps the window on-screen
  // when the OS window is resized or the mobile breakpoint is crossed.
  useEffect(() => {
    if (typeof window === 'undefined') return;
    const refresh = () => setViewport(readViewport());
    window.addEventListener('resize', refresh);
    return () => window.removeEventListener('resize', refresh);
  }, []);

  // The shared panel hook focuses the first control (the mode switch); QuickCalc
  // wants the expression field, exactly like the old dialog did. Declared after
  // the hook so this effect runs last and wins the initial focus.
  useEffect(() => {
    inputRef.current?.focus({ preventScroll: true });
  }, []);

  const scientific = mode === 'scientific';
  const width = quickCalcWidth(mode, viewport.width);
  const height = quickCalcNominalHeight(mode);
  const placed = stored !== null && stored.moved
    ? stored.geometry
    : defaultQuickCalcGeometry(viewport, width, height);
  // Width follows the mode, never the stored geometry (the card is not
  // resizable), and the whole placement is clamped for the current viewport.
  const view = clampQuickCalc({ ...placed, width }, viewport);

  const preview = calcPreview(calc);
  const display = calcDisplay(calc);

  function run(item: CalcKey): void {
    setCopyState('idle');
    setCalc(state => item.run(state));
  }

  async function copyResult(): Promise<void> {
    try {
      const copied = await writeClipboardText(calcCopyText(calc));
      setCopyState(copied ? 'copied' : 'failed');
    } catch {
      setCopyState('failed');
    }
  }

  function onPointerDown(event: React.PointerEvent<HTMLElement>): void {
    if (event.button !== 0) return;
    // A press on a header control stays a click; only header chrome drags.
    if (!isDragHandleTarget(event.target as unknown as DragTargetLike)) return;
    event.preventDefault();
    panelRef.current?.setPointerCapture?.(event.pointerId);
    dragRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      origin: view,
      viewport,
    };
    setDragging(true);
  }

  function onPointerMove(event: React.PointerEvent<HTMLDivElement>): void {
    const session = dragRef.current;
    if (!session || session.pointerId !== event.pointerId) return;
    event.preventDefault();
    rememberPlacement(moveQuickCalc(
      session.origin,
      event.clientX - session.startX,
      event.clientY - session.startY,
      session.viewport,
    ));
  }

  function endDrag(event: React.PointerEvent<HTMLDivElement>): void {
    const session = dragRef.current;
    if (!session || session.pointerId !== event.pointerId) return;
    dragRef.current = null;
    setDragging(false);
    const node = panelRef.current;
    if (node?.hasPointerCapture?.(event.pointerId)) node.releasePointerCapture(event.pointerId);
  }

  const body = (
    <div
      ref={panelRef}
      className={[
        'quickcalc-window',
        scientific ? 'is-scientific' : 'is-basic',
        dragging ? 'is-dragging' : '',
      ].filter(Boolean).join(' ')}
      style={{
        left: view.left,
        top: view.top,
        // Never paint past the viewport bottom: the space below the header caps
        // the card and the body takes the scroller (see MIN_WINDOW_HEIGHT).
        maxHeight: `max(${MIN_WINDOW_HEIGHT}px, calc(100vh - ${view.top}px - ${QUICKCALC_EDGE_MARGIN}px))`,
      }}
      data-yacr-dialog="quickcalc"
      role="dialog"
      aria-label={t('quickcalc.title')}
      tabIndex={-1}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
    >
      <header
        className="quickcalc-window__head"
        title={t('quickcalc.drag_hint')}
        onPointerDown={onPointerDown}
      >
        <h2 className="quickcalc-window__title">
          <Icon name="settings" size={14} />
          {t('quickcalc.title')}
        </h2>
        <div className="quickcalc__modes">
          {MODES.map(([value, labelKey]) => (
            <button
              key={value}
              type="button"
              className="quickcalc__mode"
              aria-pressed={mode === value}
              onClick={() => setMode(value)}
            >
              {t(labelKey)}
            </button>
          ))}
        </div>
        {scientific ? (
          <button
            type="button"
            className="quickcalc__angle"
            aria-label={calc.angle === 'deg' ? t('quickcalc.angle.deg') : t('quickcalc.angle.rad')}
            onClick={() => setCalc(state => toggleCalcAngle(state))}
          >
            {calc.angle === 'deg' ? 'DEG' : 'RAD'}
          </button>
        ) : null}
        <IconButton icon="close" size="xs" label={t('quickcalc.close')} onClick={onClose} />
      </header>
      <div className="quickcalc-window__body">
        <div className="quickcalc">
          <input
            ref={inputRef}
            className="quickcalc__display"
            value={display}
            placeholder="0"
            aria-label={t('quickcalc.expression')}
            inputMode={scientific ? 'text' : 'decimal'}
            autoComplete="off"
            spellCheck={false}
            onChange={event => {
              setCalc(state => setCalcExpression(state, event.target.value));
              setCopyState('idle');
            }}
            onKeyDown={event => {
              if (event.key === 'Enter') {
                event.preventDefault();
                setCalc(state => evaluateCalc(state));
                setCopyState('idle');
              }
            }}
          />
          <div className="quickcalc__meta">
            {calc.error ? (
              <span className="quickcalc__error" role="alert">{t('quickcalc.error')}</span>
            ) : preview !== null ? (
              <span className="quickcalc__preview" aria-live="polite">= {preview}</span>
            ) : (
              <span className="quickcalc__preview is-empty" aria-hidden="true">&nbsp;</span>
            )}
            <span className={['quickcalc__memory', calcMemoryActive(calc) ? 'is-active' : ''].filter(Boolean).join(' ')}>
              {calcMemoryActive(calc) ? `M ${formatCalcNumber(calc.memory)}` : t('quickcalc.memory_empty')}
            </span>
          </div>
          {scientific ? (
            <div className="quickcalc__funcs">
              {FUNCTION_KEYS.map(item => <CalcKeyButton key={item.id} item={item} onRun={run} />)}
            </div>
          ) : null}
          <div className="quickcalc__keys">
            {KEYPAD_KEYS.map(item => <CalcKeyButton key={item.id} item={item} onRun={run} />)}
          </div>
        </div>
      </div>
      <footer className="quickcalc-window__foot">
        {copyState === 'copied' ? <span className="quickcalc__copied" role="status">{t('quickcalc.copied')}</span> : null}
        {copyState === 'failed' ? <span className="quickcalc__copy-failed" role="alert">{t('quickcalc.copy_failed')}</span> : null}
        <Button variant="chip" icon="copy" iconSize={14} onClick={() => { void copyResult(); }}>
          {t('quickcalc.copy')}
        </Button>
        <Button variant="solid" onClick={onClose}>{t('quickcalc.close')}</Button>
      </footer>
    </div>
  );

  return typeof document === 'undefined' ? body : createPortal(body, document.body);
}
