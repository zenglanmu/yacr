// Keyboard/focus mechanics for the error report dialog.
//
// Same shape as `notice-focus.ts` (focus the one obvious action on open, keep Tab
// in the card, Escape dismisses once), plus the one thing the error dialog needs
// that a notice does not: the element that had focus before the dialog opened is
// restored when it closes. The focus effect depends only on the refs, which are
// stable, so navigating older/later reports never yanks focus away from a
// selection the reader is making in the stack.
import { useEffect, useRef } from 'react';

const FOCUSABLE = [
  'button:not([disabled])',
  '[href]',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(', ');

/** The refs the dialog owns; structural so a mutable `useRef` fits. */
export interface ErrorDialogFocusRefs {
  readonly dialog: { readonly current: HTMLDivElement | null };
  readonly initial: { readonly current: HTMLButtonElement | null };
}

/**
 * Focus `initial` on open, restore the previous element on close, and keep
 * Tab/Escape inside the card until it unmounts. `onDismiss` must be stable (the
 * dialog passes the module-level `dismissErrorReport`).
 */
export function useErrorDialogFocus({ dialog, initial }: ErrorDialogFocusRefs, onDismiss: () => void): void {
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
  }, [initial]);

  useEffect(() => {
    const node = dialog.current;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.isComposing || event.keyCode === 229) return;
      if (event.key === 'Escape') {
        if (event.defaultPrevented) return;
        event.preventDefault();
        onDismiss();
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
  }, [dialog, onDismiss]);
}
