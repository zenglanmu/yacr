// Keyboard/focus mechanics for a two-action confirmation.
//
// Same contract as `notice-focus.ts` (focus one safe action on open, keep Tab in
// the card, Escape dismisses once) plus the error dialog's focus restoration:
// the element that opened the confirmation gets the keyboard back when it goes
// away, so a cancelled SAVE does not strand focus on a removed button.
//
// The initial action is a parameter on purpose: the DWG save confirmation must
// start on Cancel (the safe answer), not on the action that writes a file.
import { useEffect, useRef } from 'react';

const FOCUSABLE = [
  'button:not([disabled])',
  '[href]',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(', ');

/** The refs the confirmation owns; structural so a mutable `useRef` fits. */
export interface ConfirmFocusRefs {
  readonly dialog: { readonly current: HTMLDivElement | null };
  /** The element focused on open (a button, e.g. the safe Cancel answer). */
  readonly initial: { readonly current: HTMLElement | null };
}

/**
 * Focus `initial` on open, restore the previous element on close, and keep
 * Tab/Escape inside the card until it unmounts. `onCancel` must be stable (the
 * shell passes a memoized callback).
 */
export function useConfirmDialogFocus({ dialog, initial }: ConfirmFocusRefs, onCancel: () => void): void {
  const restoreRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    const previous = document.activeElement;
    restoreRef.current = previous instanceof HTMLElement ? previous : null;
    initial.current?.focus({ preventScroll: true });
    return () => {
      const target = restoreRef.current;
      restoreRef.current = null;
      if (target && document.contains(target) && typeof target.focus === 'function') {
        try {
          target.focus({ preventScroll: true });
        } catch {
          // The node may have unmounted with its owner.
        }
      }
    };
  // The two refs are stable `useRef` objects, so the ref object itself never
  // changes identity; depending on it would re-focus and re-capture the restore
  // target on every render (which loses focus restoration on unmount).
  }, []);

  useEffect(() => {
    const node = dialog.current;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.isComposing || event.keyCode === 229) return;
      if (event.key === 'Escape') {
        if (event.defaultPrevented) return;
        event.preventDefault();
        onCancel();
        return;
      }
      if (event.key !== 'Tab' || !node) return;
      const focusable = Array.from(node.querySelectorAll<HTMLElement>(FOCUSABLE))
        .filter(element => element.getClientRects().length > 0);
      if (focusable.length === 0) {
        event.preventDefault();
        node.focus();
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      const inside = active !== null && node.contains(active);
      if (event.shiftKey && (!inside || active === first)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (!inside || active === last)) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [dialog, onCancel]);
}
