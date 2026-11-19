// The compact layout's command-line surface: the bar, and the floating trigger
// that reveals it — in the one place that owns the flag.
//
// They are mounted together (App renders this region where the docked command line
// sits on the desktop), so the bar's open state cannot outlive the layout: a
// breakpoint crossing unmounts the region and the next mobile session starts closed
// (`layout/use-mobile-commandbar.ts`). The bar itself is mounted only while it is
// revealed, so a hidden bar takes no height from the drawing and its absolute
// sheet/echo cannot paint; the trigger is the only always-visible part.
//
// The drawing gains that height through the ordinary `.shell` flex chain: the bar
// is `flex: 0 0 auto` and absent, so `<Workspace />` (the row that is left) grows
// into it. There is no reserved strip.
import React from 'react';
import { MobileCommandBar, MOBILE_COMMAND_BAR_ID } from './MobileCommandBar.tsx';
import { MobileCommandBarToggle } from './MobileCommandBarToggle.tsx';
import { useMobileCommandBar } from '../layout/use-mobile-commandbar.ts';

export function MobileCommandSurface() {
  const bar = useMobileCommandBar();
  return (
    <>
      {bar.open ? <MobileCommandBar /> : null}
      <MobileCommandBarToggle
        expanded={bar.open}
        pinned={bar.pinned}
        onToggle={bar.toggle}
        barId={MOBILE_COMMAND_BAR_ID}
      />
    </>
  );
}
