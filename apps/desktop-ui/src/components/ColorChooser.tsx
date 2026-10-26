// ColorChooser / ColorField: the in-DOM colour chooser both palettes open.
//
// One overlay inside this same React tree — no second Tauri window (superseded by
// ADR 0023 for exactly this reason) and no portal. Like the CAD shortcut menu it is
// `position: fixed`, measured against its trigger's rect and placed by
// `placeContextMenu`'s flip-then-clamp rule, so it paints above the docked palettes
// and the CAD partition on both carriers. It is opaque chrome with no backdrop: a
// press outside is not swallowed, it closes the chooser and still does whatever it
// would have done (a CAD pick, a field, a drag).
//
// What it offers is exactly what the write path accepts, never more:
//
//   * the symbolic entries the **caller** passes (`ByLayer`, `ByBlock` for an
//     entity colour; none for a layer colour, whose grammar refuses both by name);
//   * the standard indexed palette (ACI `1..=255`) as a swatch grid, written as
//     `aci:N` — the very string the host projects, so one parser and no new value
//     form;
//   * optionally a free-form entry, normalised by the caller's own grammar check
//     (the layer palette passes the layer grammar's `#RRGGBB`/`aci:N` routing). The
//     properties palette passes none, because its colour row already *is* the text
//     entry: that field stays the fallback for anything the grid cannot say.
//
// It never writes: `onPick(value)` hands the chosen spelling to the surface that
// owns the write channel (the properties palette's one `shell.applySelectedProperty`
// path, the layers store's single-flight `setProperty`). A host refusal therefore
// surfaces through that surface's existing error line, not through a new toast.
//
// Keyboard and dismissal are the app's popover rules: `Escape` closes and hands
// focus back to the trigger, a press outside closes, `window` blur/resize and a
// scroll of anything but the chooser itself close it (no drifting anchor). The grid
// is a single tab stop with a roving cursor — arrows, Home/End — the same rule the
// layers list follows, so 255 swatches never become 255 tab stops.
import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { IconButton, Button } from './primitives.tsx';
import { useI18n } from '../context.ts';
import { placeContextMenu } from '../commands/context-menu.ts';
import type { MenuPlacement } from '../commands/context-menu.ts';
import type { ComposingKeyEvent } from './key-events.ts';
import {
  COLOR_GRID,
  COLOR_GRID_COLUMNS,
  aciChoice,
  colorSwatch,
  isSelectedSymbol,
  selectedAciIndex,
  swatchCursor,
  symbolLabelKey,
} from '../color/color-value.ts';

/** Gap between the trigger and the chooser, in CSS px. */
const CHOOSER_GAP = 4;

/** The trigger's viewport rect at open time: the chooser's anchor. */
export interface ColorAnchorRect {
  readonly left: number;
  readonly top: number;
  readonly bottom: number;
}

/** What the open chooser needs from the surface that owns it. */
export interface ColorChooserProps {
  /** The trigger's rect at open time (the trigger is the anchor, not an ancestor). */
  readonly anchorRect: ColorAnchorRect;
  /** The trigger itself: a press on it toggles the chooser instead of closing it. */
  readonly trigger: HTMLElement | null;
  /** The surface's current wire value: what the chooser marks as selected. */
  readonly value: string;
  /** The symbolic entries this write path accepts, in order (may be empty). */
  readonly symbols: readonly string[];
  /**
   * The free-form entry's grammar check: the wire value `text` means, or `''` when
   * this surface would not accept it. Omit to render no field at all.
   */
  readonly custom?: (text: string) => string;
  /** One chosen value, already in the surface's own grammar. */
  readonly onPick: (value: string) => void;
  /** `restoreFocus` is `true` when the keyboard closed it. */
  readonly onClose: (options: { readonly restoreFocus?: boolean }) => void;
}

/**
 * The chooser surface itself: the symbolic entries, the indexed palette and the
 * optional free-form entry.
 *
 * @param anchorRect the trigger's rect (measured by [`ColorField`] on open)
 * @param trigger    the trigger element (a press on it is not "outside")
 * @param value      the current wire value, shown as the selected control
 * @param symbols    the symbolic entries this surface's grammar takes
 * @param custom     optional `text -> wire value` check for the free-form entry
 * @param onPick     receives one chosen wire value; the surface writes it
 * @param onClose    receives `{restoreFocus}` (true: focus goes back to the trigger)
 */
export function ColorChooser({
  anchorRect,
  trigger,
  value,
  symbols,
  custom,
  onPick,
  onClose,
}: ColorChooserProps) {
  const { t } = useI18n();
  const rootRef = useRef<HTMLDivElement | null>(null);
  const swatchRefs = useRef<Array<HTMLButtonElement | null>>([]);
  // `null` until measured: the first frame is invisible, so the chooser never
  // flashes at an unclamped position.
  const [placed, setPlaced] = useState<MenuPlacement | null>(null);
  const selected = selectedAciIndex(value);
  // The roving cursor: one tab stop. It starts on the current colour when the value
  // is an index, else on the first swatch, so opening with the keyboard lands
  // somewhere meaningful and the arrows work immediately.
  const [cursor, setCursor] = useState(() => {
    const at = COLOR_GRID.findIndex(cell => cell.index === selected);
    return at === -1 ? 0 : at;
  });
  // The free-form entry's draft: the surface's own value until the user types.
  const [draft, setDraft] = useState(value);

  // Measure once per open; a locale change that resizes the labels re-measures the
  // same way the shortcut menu does.
  useLayoutEffect(() => {
    const node = rootRef.current;
    if (!node) return;
    const rect = node.getBoundingClientRect();
    const size = { width: rect.width, height: rect.height };
    const viewport = { width: window.innerWidth, height: window.innerHeight };
    const below = placeContextMenu({
      anchor: { x: anchorRect.left, y: anchorRect.bottom + CHOOSER_GAP },
      size,
      viewport,
    });
    // `placeContextMenu` flips a box that would overflow the window. Flipped, it is
    // anchored to the trigger's bottom edge, which would cover the trigger the user
    // just pressed — so it is re-placed against the trigger's **top** edge instead.
    // Same helper, one extra call: still one flip-then-clamp rule in the app.
    const flipped = below.top < anchorRect.top;
    setPlaced(flipped
      ? placeContextMenu({
        anchor: { x: anchorRect.left, y: anchorRect.top - CHOOSER_GAP - size.height },
        size,
        viewport,
      })
      : below);
  }, [anchorRect, symbols, custom, t]);

  // Focus lands on the current swatch (the roving tab stop), not on the container:
  // the grid is the reason the chooser exists and its arrows need focus. Mount only;
  // later cursor moves are the grid's own focus calls.
  useEffect(() => {
    const node = swatchRefs.current[cursor] ?? rootRef.current;
    node?.focus?.({ preventScroll: true });
  }, []);

  // The chooser owns Escape while it is open, and stops it: the shell's global
  // Escape (a tool cancel, a field blur) must not also fire for the same press.
  const onKeyDown = (event: ComposingKeyEvent<HTMLDivElement>) => {
    if (event.isComposing || event.keyCode === 229) return;
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    onClose({ restoreFocus: true });
  };

  // A press outside closes the chooser but is **not** prevented: it reaches
  // whatever is underneath (the CAD partition forwards it as before) — exactly the
  // shortcut menu's rule. The trigger counts as inside, so its own click toggles the
  // chooser rather than closing and reopening it.
  useEffect(() => {
    const inside = (target: EventTarget | null): boolean => {
      if (!target || typeof target !== 'object') return false;
      const node = target as Node;
      if (rootRef.current?.contains(node)) return true;
      return Boolean(trigger?.contains?.(node));
    };
    const dismiss = () => onClose({ restoreFocus: false });
    const onPointerDown = (event: PointerEvent) => { if (!inside(event.target)) dismiss(); };
    // A chooser taller than the window scrolls inside itself, and that scroll must
    // not dismiss it; a scroll of anything else moves the anchor out from under it.
    const onScroll = (event: Event) => { if (!inside(event.target)) dismiss(); };
    document.addEventListener('pointerdown', onPointerDown, true);
    window.addEventListener('blur', dismiss);
    window.addEventListener('resize', dismiss);
    window.addEventListener('scroll', onScroll, true);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown, true);
      window.removeEventListener('blur', dismiss);
      window.removeEventListener('resize', dismiss);
      window.removeEventListener('scroll', onScroll, true);
    };
  }, [onClose, trigger]);

  // Tab out of the chooser closes it: an open popover whose focus sits elsewhere
  // would be a stranded surface. Focus moving to the trigger is not "out" — the
  // trigger's own press is what closes it then.
  const onBlur = (event: React.FocusEvent<HTMLDivElement>) => {
    const next = event.relatedTarget;
    if (next && event.currentTarget.contains(next as Node)) return;
    if (next && trigger?.contains?.(next as Node)) return;
    onClose({ restoreFocus: false });
  };

  const focusSwatch = (index: number) => {
    setCursor(index);
    swatchRefs.current[index]?.focus?.({ preventScroll: true });
  };

  const onGridKeyDown = (event: ComposingKeyEvent<HTMLDivElement>) => {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    const next = swatchCursor(cursor, event.key, COLOR_GRID_COLUMNS, COLOR_GRID.length);
    if (next === null) return;
    event.preventDefault();
    event.stopPropagation();
    focusSwatch(next);
  };

  // One committed entry is one write. The caller's grammar check normalises the
  // typed text; an unchanged value closes without a write (the host's own no-op
  // rule), and a value the surface would refuse sends nothing at all.
  const normalized = custom ? custom(draft) : '';
  const applyCustom = () => {
    if (!custom) return;
    const next = custom(draft);
    if (!next) return;
    if (next.toLowerCase() === String(value ?? '').trim().toLowerCase()) {
      onClose({ restoreFocus: true });
      return;
    }
    onPick(next);
  };

  const symbolicId = 'color-chooser-symbolic';
  const paletteId = 'color-chooser-palette';
  const customId = 'color-chooser-custom';
  const customHintId = 'color-chooser-custom-hint';

  return (
    <div
      ref={rootRef}
      className={['color-chooser', placed ? '' : 'is-unplaced'].filter(Boolean).join(' ')}
      role="dialog"
      aria-label={t('color.title')}
      tabIndex={-1}
      style={{ left: `${placed?.left ?? 0}px`, top: `${placed?.top ?? 0}px` }}
      onKeyDown={onKeyDown}
      onBlur={onBlur}
      data-yacr-color-chooser="dom"
    >
      <div className="color-chooser__head">
        <span className="color-chooser__title">{t('color.title')}</span>
        <IconButton
          icon="close"
          size="xs"
          label={t('color.close')}
          onClick={() => onClose({ restoreFocus: true })}
        />
      </div>

      {symbols.length > 0 ? (
        <>
          <p className="color-chooser__label" id={symbolicId}>{t('color.symbolic')}</p>
          <div className="color-chooser__symbols" role="group" aria-labelledby={symbolicId}>
            {symbols.map(symbol => {
              const key = symbolLabelKey(symbol);
              return (
                <button
                  key={symbol}
                  type="button"
                  className="color-chooser__symbol"
                  aria-pressed={isSelectedSymbol(value, symbol)}
                  onClick={() => onPick(symbol)}
                >
                  {/* An unknown token is shown verbatim, never relabelled. */}
                  {key ? t(key) : symbol}
                </button>
              );
            })}
          </div>
        </>
      ) : null}

      <p className="color-chooser__label" id={paletteId}>{t('color.palette')}</p>
      <div
        className="color-chooser__grid"
        role="group"
        aria-labelledby={paletteId}
        onKeyDown={onGridKeyDown}
      >
        {COLOR_GRID.map(cell => (
          <button
            key={cell.index}
            type="button"
            ref={node => { swatchRefs.current[cell.index - 1] = node; }}
            className="color-chooser__swatch"
            style={{ backgroundColor: cell.hex }}
            tabIndex={cell.index - 1 === cursor ? 0 : -1}
            aria-label={t('color.index', { index: cell.index })}
            aria-pressed={cell.index === selected}
            title={t('color.index', { index: cell.index })}
            onClick={() => onPick(aciChoice(cell.index))}
            onFocus={() => setCursor(cell.index - 1)}
          />
        ))}
      </div>

      {custom ? (
        <form
          className="color-chooser__custom"
          onSubmit={event => { event.preventDefault(); applyCustom(); }}
        >
          <label className="color-chooser__custom-label" htmlFor={customId}>
            {t('color.custom_label')}
          </label>
          <div className="color-chooser__custom-row">
            <input
              id={customId}
              className="color-chooser__input"
              type="text"
              value={draft}
              aria-describedby={customHintId}
              autoComplete="off"
              spellCheck={false}
              onChange={event => setDraft(event.target.value)}
            />
            <Button
              type="submit"
              variant="ghost"
              size="xs"
              // Not a value this surface's grammar takes: nothing would be sent. It
              // stays focusable (a real `disabled` would drop the keyboard) and the
              // hint below says what the field does take.
              aria-disabled={normalized === '' || undefined}
            >
              {t('color.apply')}
            </Button>
          </div>
          <p className="color-chooser__hint" id={customHintId}>{t('color.custom_hint')}</p>
        </form>
      ) : null}
    </div>
  );
}

/** What one colour cell needs from the palette that owns the write channel. */
export interface ColorFieldProps {
  /** The surface's current wire value. */
  readonly value: string;
  /** The trigger's accessible name (catalog copy owned by the caller). */
  readonly label: string;
  /** The symbolic entries this write path accepts (may be empty). */
  readonly symbols?: readonly string[];
  /** Optional free-form grammar check; omitted means no text entry in the popover. */
  readonly custom?: (text: string) => string;
  /** A write is in flight: the trigger is announced unavailable and drops presses. */
  readonly busy?: boolean;
  /**
   * The colour the trigger paints, when it is not the one its wire value names.
   *
   * The layers palette hands in `layerSwatch().color`: the drawing's stored colour,
   * or the name-derived marker when the host reported none. It only affects the
   * painted swatch — what the chooser marks as selected is still `value`.
   */
  readonly paint?: string;
  /** One chosen wire value; the caller writes it through its own channel. */
  readonly onPick: (value: string) => void;
  /** The surface's own editor (the properties row's text field), kept beside it. */
  readonly children?: React.ReactNode;
}

/**
 * A colour cell: the swatch trigger, the caller's own field beside it, and the
 * chooser while it is open.
 *
 * The trigger is a real button with `aria-haspopup`/`aria-expanded`, so a keyboard
 * reaches the chooser with Enter/Space and learns whether it is open. Its swatch
 * paints the value's own colour, and a symbolic value (`ByLayer`/`ByBlock`) is a
 * neutral marker: the trigger never paints a colour the drawing would not store,
 * and the row's own value text is what says which token it is.
 */
export function ColorField({
  value,
  label,
  symbols = [],
  custom,
  busy = false,
  paint,
  onPick,
  children,
}: ColorFieldProps) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [anchorRect, setAnchorRect] = useState<ColorAnchorRect | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const swatch = colorSwatch(value);
  // The painted colour: the caller's own when it hands one in (the layer palette's
  // stored colour or name marker), otherwise the value's own colour. A symbolic
  // value has neither, so the trigger shows the neutral marker instead of a colour
  // the drawing would not store.
  const painted = paint ?? swatch.hex;

  const close = (restore: boolean) => {
    setOpen(false);
    setAnchorRect(null);
    if (restore) triggerRef.current?.focus?.({ preventScroll: true });
  };

  const toggle = () => {
    if (busy) return;
    if (open) {
      close(true);
      return;
    }
    const rect = triggerRef.current?.getBoundingClientRect?.();
    setAnchorRect(rect
      ? { left: rect.left, top: rect.top, bottom: rect.bottom }
      : { left: 0, top: 0, bottom: 0 });
    setOpen(true);
  };

  return (
    <div className="color-field">
      <button
        ref={triggerRef}
        type="button"
        className={['color-field__trigger', painted ? '' : 'is-symbolic'].filter(Boolean).join(' ')}
        style={painted ? { backgroundColor: painted } : undefined}
        aria-label={label}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-disabled={busy || undefined}
        title={t('color.open')}
        onClick={toggle}
      />
      {children}
      {open && anchorRect ? (
        <ColorChooser
          anchorRect={anchorRect}
          trigger={triggerRef.current}
          value={value}
          symbols={symbols}
          custom={custom}
          onPick={next => { onPick(next); close(true); }}
          onClose={({ restoreFocus }) => close(restoreFocus === true)}
        />
      ) : null}
    </div>
  );
}
