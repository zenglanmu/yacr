// The chrome-local command line selectors, in one pure table plus one thin
// React hook.
//
// These words never reach the router: the shell answers them itself. Keeping the
// decision here means `App.tsx` owns no per-selector branch and a new selector
// cannot be added without also answering the layout question (a docked panel on
// the compact layout is a refusal in the catalog's own words, never a flag that
// paints nothing).
//
// `FONTS` is deliberately **not** refused on mobile: the compact layout renders
// the same panel as a floating sheet, exactly like `RESOURCES`.
import { useCallback } from 'react';
import type { Translator } from '../i18n.ts';
import type { LayoutMode } from '../layout/layout-mode.ts';
import { DOCKED_PANEL_REFUSAL_KEY, dockedPaletteOpenable } from '../layout/layout-surfaces.ts';
import { isLayersCommand } from '../layers/layers.ts';
import { isOptionsCommand } from '../options/options.ts';
import { isPropertiesCommand } from '../properties/properties.ts';
import { isResourcesCommand } from '../references/references.ts';
import { isFontsCommand } from '../fonts/fonts.ts';

/** One selector's destination. `refused` names the panel the layout cannot show. */
export type LocalSelectorResult =
  | { readonly kind: 'options' }
  | { readonly kind: 'resources' }
  | { readonly kind: 'properties' }
  | { readonly kind: 'layers' }
  | { readonly kind: 'fonts' }
  | { readonly kind: 'refused'; readonly panelKey: string };

/** The panel labels the docked refusal names. */
export const LOCAL_SELECTOR_PANEL_KEYS = Object.freeze({
  properties: 'properties.panel',
  layers: 'layers.panel',
});

/**
 * Resolve typed text to the chrome-local surface it opens, or `null` when the
 * line belongs to the router.
 *
 * The order mirrors the old inline branch: dialog selectors first, then the
 * docked panels. `PROPERTIES`/`LAYER` are refused where the dock is not rendered;
 * `FONTS`/`RESOURCES` have a compact sheet and are never refused by layout.
 */
export function localSelectorFor(text: string, layout: LayoutMode): LocalSelectorResult | null {
  if (isOptionsCommand(text)) return { kind: 'options' };
  if (isResourcesCommand(text)) return { kind: 'resources' };
  if (isFontsCommand(text)) return { kind: 'fonts' };
  if (isPropertiesCommand(text)) {
    return dockedPaletteOpenable(layout)
      ? { kind: 'properties' }
      : { kind: 'refused', panelKey: LOCAL_SELECTOR_PANEL_KEYS.properties };
  }
  if (isLayersCommand(text)) {
    return dockedPaletteOpenable(layout)
      ? { kind: 'layers' }
      : { kind: 'refused', panelKey: LOCAL_SELECTOR_PANEL_KEYS.layers };
  }
  return null;
}

/** What the hook does for each resolved selector. */
export interface LocalSelectorHost {
  readonly layout: LayoutMode;
  readonly translate: Translator;
  readonly clearResult: () => void;
  readonly clearDraft: () => void;
  readonly requestOptions: () => void;
  readonly openResources: () => void;
  readonly openProperties: () => void;
  /** Open the layers palette and take the keyboard (a repeat `LAYER` re-reads). */
  readonly openLayers: () => void;
  readonly openFonts: () => void;
  /** Put a refusal sentence on the chrome's one error line. */
  readonly reportError: (message: string) => void;
}

/**
 * Build the one submit-path interceptor.
 *
 * Returns `true` when the line was answered locally (the caller must then never
 * forward it to `cad_command`), and `false` for every router line. It clears the
 * previous result for any selector, and the draft only for input submissions —
 * the same contract the inline branch had.
 */
export function useLocalSelectors(host: LocalSelectorHost): (text: string, fromInput: boolean) => boolean {
  const {
    layout,
    translate,
    clearResult,
    clearDraft,
    requestOptions,
    openResources,
    openProperties,
    openLayers,
    openFonts,
    reportError,
  } = host;
  return useCallback((text: string, fromInput: boolean): boolean => {
    const result = localSelectorFor(text, layout);
    if (!result) return false;
    clearResult();
    if (result.kind === 'refused') {
      reportError(translate(DOCKED_PANEL_REFUSAL_KEY, { panel: translate(result.panelKey) }));
      return true;
    }
    if (fromInput) clearDraft();
    switch (result.kind) {
      case 'options': requestOptions(); break;
      case 'resources': openResources(); break;
      case 'properties': openProperties(); break;
      case 'layers': openLayers(); break;
      case 'fonts': openFonts(); break;
    }
    return true;
  }, [
    layout,
    translate,
    clearResult,
    clearDraft,
    requestOptions,
    openResources,
    openProperties,
    openLayers,
    openFonts,
    reportError,
  ]);
}
