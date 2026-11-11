// The chrome's UI visibility flags, as one pure value.
//
// These are **chrome only**: they decide which regions the React tree paints and
// never touch CAD state. `CLEANSCREENON/OFF` own the older clean-screen state;
// this value is the finer-grained pair (`RIBBON`/`RIBBONCLOSE`,
// `FILETAB`/`FILETABCLOSE`, `HIDEPALETTES`/`SHOWPALETTES`) plus the snapshot a
// workspace preset stores.
//
// Every transition here is an explicit **set**, never a toggle: the typed words
// say "show" or "hide", and a repeat must leave the same state rather than flip
// it back. The default is today's chrome: ribbon, palettes and drawing tabs all
// visible.

/** Which chrome regions are painted. */
export interface ChromeVisibility {
  readonly ribbon: boolean;
  readonly palettes: boolean;
  readonly fileTabs: boolean;
}

/** The startup chrome: every region visible. */
export const DEFAULT_CHROME_VISIBILITY: ChromeVisibility = Object.freeze({
  ribbon: true,
  palettes: true,
  fileTabs: true,
});

/** The `HIDEPALETTES` state: ribbon, palettes and drawing tabs all hidden. */
export const HIDDEN_CHROME_VISIBILITY: ChromeVisibility = Object.freeze({
  ribbon: false,
  palettes: false,
  fileTabs: false,
});

/** Set only the ribbon flag. */
export function withRibbon(current: ChromeVisibility, visible: boolean): ChromeVisibility {
  return Object.freeze({ ...current, ribbon: visible });
}

/** Set only the palette-layer flag. */
export function withPalettes(current: ChromeVisibility, visible: boolean): ChromeVisibility {
  return Object.freeze({ ...current, palettes: visible });
}

/** Set only the drawing-tab flag. */
export function withFileTabs(current: ChromeVisibility, visible: boolean): ChromeVisibility {
  return Object.freeze({ ...current, fileTabs: visible });
}

/** The value `HIDEPALETTES` (`false`) or `SHOWPALETTES` (`true`) sets. */
export function paletteChrome(visible: boolean): ChromeVisibility {
  return visible ? DEFAULT_CHROME_VISIBILITY : HIDDEN_CHROME_VISIBILITY;
}
