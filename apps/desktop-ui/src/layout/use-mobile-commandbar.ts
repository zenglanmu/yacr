// The compact command bar's state and reveal rule, for the one region that
// renders it (`components/MobileCommandSurface.tsx`).
//
// The rules themselves live in `mobile-commandbar.ts` (pure, tested without
// React); this hook only reads the shell facts those rules consume. It is a
// separate module for the same reason `shell-shortcuts.ts` is: `App.tsx` sits at
// the file-size ceiling, and "is the bar on screen?" should be answered once, next
// to the surface that obeys it.
//
// The facts are the bar's own, read from the same sources its controller reads:
// a pending plugin prompt (`shell.pluginPrompt`) and the active tool's text step
// (`awaitingToolText`) make the line an answer field, `displayedError` is the
// sentence the bar's banner would show, and `historyOpen` is the sheet that opens
// upward from this bar. None of them is a style preference.
//
// The state resets by unmounting: the region exists only on the compact layout, so
// a breakpoint crossing destroys the flag, and the next mobile session starts
// closed by construction rather than by a reset effect.
import { useState } from 'react';
import { useShell } from '../context.ts';
import { awaitingToolText } from '../tools/text.ts';
import {
  MOBILE_COMMAND_BAR_CLOSED, commandBarMustShow, mobileCommandBarNext, mobileCommandBarVisible,
} from './mobile-commandbar.ts';
import type { MobileCommandBarState } from './mobile-commandbar.ts';

/** What the surface renders from, and the one control that flips it. */
export interface MobileCommandBarWiring {
  /** Whether the bar is on screen (user's choice or a must-see condition). */
  readonly open: boolean;
  /** Whether a must-see condition is holding it revealed. */
  readonly pinned: boolean;
  /** Put the bar away, or bring it back; a must-see condition still reveals it. */
  readonly toggle: () => void;
}

export function useMobileCommandBar(): MobileCommandBarWiring {
  const shell = useShell();
  const [state, setState] = useState<MobileCommandBarState>(MOBILE_COMMAND_BAR_CLOSED);
  const reveal = {
    promptMode: shell.pluginPrompt !== null,
    textMode: awaitingToolText(shell.status),
    notice: shell.displayedError !== '',
    sheet: shell.historyOpen,
  };
  // The history sheet is shell state shared with the desktop panel, so closing the
  // bar closes the sheet it opened; otherwise the flag would outlive its surface
  // and the next reveal would show a sheet the user never asked for again.
  //
  // The press acts on what is **on screen**, not on the flag: a bar revealed by a
  // condition (flag still false) must read as "hide" and become the closed flag,
  // never as "open" — that would leave the button lying about its own effect.
  const setHistoryOpen = shell.setHistoryOpen;
  const toggle = () => {
    setState(value => mobileCommandBarNext(value, mobileCommandBarVisible(value, reveal) ? 'close' : 'open'));
    setHistoryOpen(false);
  };
  return {
    open: mobileCommandBarVisible(state, reveal),
    pinned: commandBarMustShow(reveal),
    toggle,
  };
}
