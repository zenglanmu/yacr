// The one call site for SketchUp's plain-key shortcuts.
//
// It is a listener component rather than a branch in `App.tsx` because the
// drafting accelerators and the domain accelerators have different ownership
// rules, and the shell already owns the Ctrl chords. This component only runs
// while the rendered domain is `concept`; a resolved name still passes the
// shared `canCommand` gate and rides `submitCommand`, so nothing here is a
// second admission path.
//
// Space is the Select state: with a tool active it sends the same `CANCEL`
// Escape sends; with no tool it is already the idle pointer, so the press is
// consumed without a wire request.
import { useEffect, useRef } from 'react';
import { useShell } from '../../context.ts';
import { isTextEntryTarget } from '../../defaults.ts';
import { currentDomain } from '../store.ts';
import { conceptShortcut } from './shortcuts.ts';

function typingTarget(target: EventTarget | null): boolean {
  if (target instanceof HTMLElement) {
    if (['INPUT', 'SELECT', 'TEXTAREA'].includes(target.tagName)) return true;
    if (target.isContentEditable) return true;
  }
  return isTextEntryTarget(target);
}

export function ConceptShortcuts() {
  const shell = useShell();
  const active = currentDomain(shell.status) === 'concept';
  // The poll replaces `status` (and therefore `canCommand`) on every tick. A ref
  // keeps the listener installed once per domain while the handler still reads
  // the latest snapshot; re-registering a window listener every 250 ms would be
  // pure churn.
  const latest = useRef(shell);
  latest.current = shell;

  useEffect(() => {
    if (!active) return undefined;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      const { status, submitCommand, canCommand } = latest.current;
      const tool = status?.tool ?? null;
      const result = conceptShortcut(event, {
        typing: typingTarget(event.target),
        popupOpen: Boolean(status?.popup),
        toolActive: Boolean(tool),
        // A tool's terminal text step owns the keyboard: typing a name there is
        // never a tool switch.
        textStep: tool?.awaiting_text === true,
        gripDragActive: status?.gripDrag === true,
      });
      if (result.kind === 'none') return;
      if (result.kind === 'select') {
        event.preventDefault();
        if (tool) void submitCommand('CANCEL');
        return;
      }
      // The shared admission is the authority: a command this build/carrier does
      // not admit is left to the explicit disabled button, never half-dispatched.
      if (!canCommand(result.command)) return;
      event.preventDefault();
      void submitCommand(result.command);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [active]);

  return null;
}
