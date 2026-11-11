// Focus/Escape mechanics for the two **non-modal** chrome panels.
//
// The Quick View panels and the workspace selector float over the drawing but do
// not block it: there is no backdrop and no focus trap. They still owe the
// keyboard two things a dialog owes: focus lands on the first card when the
// panel opens, Escape closes it, and the element that opened it gets the
// keyboard back when it goes away. The drawing card is the one exception: choosing
// "the current drawing" focuses the CAD partition instead, because that is what
// the card means.
//
// The modal dialogs (QuickCalc, workspace settings) use `confirm-focus.ts`
// instead: they do have a trap, and Escape is the same single close.
import { useEffect, useRef } from 'react';

const FOCUSABLE = [
  'button:not([disabled])',
  '[href]',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(', ');

/** The CAD partition: the one element a drawing card hands the keyboard to. */
export function focusDrawingSurface(): void {
  const surface = document.querySelector<HTMLElement>('.native-input');
  surface?.focus?.({ preventScroll: true });
}

/** What a panel's close path can ask the hook to do with the keyboard. */
export interface SurfacePanelFocus {
  /** Attach to the panel's root (it should also be `tabIndex={-1}`). */
  readonly panelRef: { readonly current: HTMLDivElement | null };
  /** Close and hand the keyboard to the CAD partition. */
  readonly closeToSurface: () => void;
}

/**
 * Wire one floating panel: initial focus, Escape, and focus restoration.
 *
 * `onClose` must be stable (the surfaces store's own close functions are module
 * constants), otherwise the Escape listener is re-installed on every render.
 */
export function useSurfacePanelFocus(onClose: () => void): SurfacePanelFocus {
  const panelRef = useRef<HTMLDivElement | null>(null);
  const restoreRef = useRef<HTMLElement | null>(null);
  const focusSurfaceRef = useRef(false);

  useEffect(() => {
    const previous = document.activeElement;
    restoreRef.current = previous instanceof HTMLElement ? previous : null;
    const first = panelRef.current?.querySelector<HTMLElement>(FOCUSABLE);
    if (first) first.focus({ preventScroll: true });
    else panelRef.current?.focus({ preventScroll: true });
    return () => {
      const target = restoreRef.current;
      restoreRef.current = null;
      if (focusSurfaceRef.current) {
        focusSurfaceRef.current = false;
        focusDrawingSurface();
        return;
      }
      if (target && document.contains(target) && typeof target.focus === 'function') {
        try {
          target.focus({ preventScroll: true });
        } catch {
          // The node may have unmounted with its owner.
        }
      }
    };
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.isComposing || event.keyCode === 229) return;
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      event.preventDefault();
      onClose();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [onClose]);

  return {
    panelRef,
    closeToSurface: () => {
      focusSurfaceRef.current = true;
      onClose();
    },
  };
}
