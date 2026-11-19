// The compact layout's command-line trigger: one floating 44px button at the
// drawing's bottom-right corner.
//
// The bar is hidden by default (`layout/mobile-commandbar.ts`), so this is the
// only always-on-screen part of that surface: a `terminal` glyph while the bar is
// away (tap to reveal it), a chevron-down while it is on screen (tap to put it
// away). While the bar is open it reserves this button's column, so the same fixed
// corner reads as the bar's own collapse control instead of covering Send.
//
// A chrome overlay, not a carrier: the button itself is the only painted, hit-
// testable node (no wrapper, no full-size layer), so the drawing's press lane is
// untouched everywhere around it. `aria-expanded` reports what is really on
// screen, and `aria-controls` names the bar's own DOM id.
//
// `pinned` means a must-see condition is holding the bar open (a prompt, a text
// step, a failure or the history sheet). The button stays visible and pressed, and
// it **names the condition instead of an action it cannot perform**: while it is
// pinned, pressing it may only clear the user's own flag (the bar leaves as soon as
// the condition clears, e.g. dismissing the history sheet), so a "hide" label would
// be a promise the tap does not keep. The reveal itself belongs to the pure module
// (`layout/mobile-commandbar.ts`), not to this button.
import React from 'react';
import { Icon } from '../icons.tsx';
import { useI18n } from '../context.ts';

export interface MobileCommandBarToggleProps {
  /** Whether the bar is on screen (the button's `aria-expanded`). */
  readonly expanded: boolean;
  /** Whether a must-see condition keeps it revealed. */
  readonly pinned: boolean;
  /** Flip the user's own flag; the reveal rule decides what is rendered. */
  readonly onToggle: () => void;
  /** The bar's DOM id (`aria-controls`). */
  readonly barId: string;
}

export function MobileCommandBarToggle({ expanded, pinned, onToggle, barId }: MobileCommandBarToggleProps) {
  const { t } = useI18n();
  // The action name, unless a must-see condition holds: then the button says what
  // is holding the bar open, because tapping it cannot put the bar away yet.
  const label = pinned ? t('mobile.commandbar.pinned') : t(expanded ? 'mobile.commandbar.hide' : 'mobile.commandbar.show');
  return (
    <button
      type="button"
      className={['mobile-commandbar-toggle', expanded ? 'is-open' : '', pinned ? 'is-pinned' : ''].filter(Boolean).join(' ')}
      aria-label={label}
      aria-expanded={expanded}
      aria-controls={barId}
      title={label}
      onClick={onToggle}
    >
      <Icon name={expanded ? 'chevronDown' : 'terminal'} size={20} />
    </button>
  );
}
