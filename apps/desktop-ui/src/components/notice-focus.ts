// Shared keyboard/focus mechanics for the small host-owned modals
// (`PluginNotice`, `HostNotice`).
//
// A notice owns the keyboard while it is open: focus starts on its one action,
// Tab cannot leave the card, and Escape dismisses exactly once. Both notices
// share this file so the trap cannot drift between them; the visual language
// (backdrop, card, metrics) is shared through the stylesheet.
//
// A composing IME keeps its own keys. `preventDefault` on Escape keeps the key
// from also reaching a focused field.
import { useEffect } from 'react';

const FOCUSABLE = [
  'button:not([disabled])',
  '[href]',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(', ');

/** The two refs a notice needs; structural so a mutable `useRef` fits. */
export interface NoticeFocusRefs {
  readonly dialog: { readonly current: HTMLDivElement | null };
  readonly dismiss: { readonly current: HTMLButtonElement | null };
}

/**
 * Focus the notice's action on mount and keep Tab/Escape inside the card until
 * it unmounts. `onDismiss` must be stable enough to be an effect dependency
 * (the hosts pass a module-level function or a memoized callback).
 */
export function useNoticeFocus({ dialog, dismiss }: NoticeFocusRefs, onDismiss: () => void): void {
  useEffect(() => {
    dismiss.current?.focus();
  }, [dismiss]);

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
