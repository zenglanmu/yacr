// The chrome-local command line selectors, in one pure table plus one thin
// React hook.
//
// These words never reach the router: the shell answers them itself. Keeping the
// decision here means `App.tsx` owns no per-selector branch and a new selector
// cannot be added without also answering the layout question (a palette window on
// the compact layout is a refusal in the catalog's own words, never a flag that
// paints nothing).
//
// `FONTS` is deliberately **not** refused on mobile: the compact layout renders
// the same panel as a floating sheet, exactly like `RESOURCES`. `ABOUT` opens the
// same dialog the File tab's About action opens, so it needs no layout either.
// `CLEANSCREENON` sets the existing clean-screen state, so it needs no layout;
// `CLEANSCREENOFF` clears that same state and needs no layout either.
// `PROPERTIESCLOSE` closes the properties window, so it likewise needs no layout:
// closing a window that is already closed is an idempotent success, not a
// refusal (unlike `PROPERTIES`, which cannot open on the compact layout).
// `LAYERCLOSE` closes the layers palette for the same reason, and is resolved
// right after it: a second close on any layout is still a success.
import { useCallback } from 'react';
import type { Translator } from '../i18n.ts';
import type { LayoutMode } from '../layout/layout-mode.ts';
import { DOCKED_PANEL_REFUSAL_KEY, dockedPaletteOpenable } from '../layout/layout-surfaces.ts';
import { isLayersCommand } from '../layers/layers.ts';
import { isOptionsCommand } from '../options/options.ts';
import { isPropertiesCommand } from '../properties/properties.ts';
import { isResourcesCommand } from '../references/references.ts';
import { isFontsCommand } from '../fonts/fonts.ts';
import {
  dashToolbarVisibility,
  isAboutCommand,
  isClassicGroupCommand,
  isClassicImageCommand,
  isClassicInsertCommand,
  isClassicLayerCommand,
  isClassicXRefCommand,
  isCleanScreenOffCommand,
  isCleanScreenOnCommand,
  isCommandLineCommand,
  isCommandLineHideCommand,
  isDSettingsCommand,
  isFileTabCloseCommand,
  isFileTabCommand,
  isHidePalettesCommand,
  isLayerCloseCommand,
  isPmToggleCommand,
  isPropertiesCloseCommand,
  isQuickCalcCloseCommand,
  isQuickCalcCommand,
  isQuickPropertiesCommand,
  isQuickViewDrawingCloseCommand,
  isQuickViewDrawingCommand,
  isQuickViewLayoutCloseCommand,
  isQuickViewLayoutCommand,
  isRibbonCloseCommand,
  isRibbonCommand,
  isShowPalettesCommand,
  isToolbarCommand,
  isUpdateThumbsNowCommand,
  isWorkspaceCommand,
  isWorkspaceSaveCommand,
  isWorkspaceSettingsCommand,
} from './chrome-commands.ts';
import type { ClassicPanelKind, QuickViewKind, WorkspaceDialogMode } from '../chrome/surfaces.ts';

/** One selector's destination. `refused` names the panel the layout cannot show. */
export type LocalSelectorResult =
  | { readonly kind: 'options' }
  | { readonly kind: 'resources' }
  | { readonly kind: 'properties' }
  | { readonly kind: 'layers' }
  | { readonly kind: 'fonts' }
  | { readonly kind: 'about' }
  /** Set the chrome's clean-screen state to `value` (never a toggle). */
  | { readonly kind: 'cleanScreen'; readonly value: boolean }
  /** Close the properties window; already closed is a success, not a refusal. */
  | { readonly kind: 'propertiesClose' }
  /** Close the layers palette; already closed is a success, not a refusal. */
  | { readonly kind: 'layersClose' }
  /** Set the command line's visibility (COMMANDLINE shows, COMMANDLINEHIDE hides). */
  | { readonly kind: 'commandLine'; readonly value: boolean }
  /** Set the ribbon's visibility (RIBBON shows, RIBBONCLOSE hides). */
  | { readonly kind: 'ribbon'; readonly value: boolean }
  /** Set ribbon + palettes + drawing tabs together (HIDEPALETTES/SHOWPALETTES). */
  | { readonly kind: 'paletteChrome'; readonly value: boolean }
  /** Flip that same palette-chrome state (`PMTOGGLE`; two calls restore it). */
  | { readonly kind: 'paletteToggle' }
  /** Open (or re-focus) the classic external-references panel (`CLASSICXREF`). */
  | { readonly kind: 'classicXref' }
  /** Open one classic block/group/image panel (`CLASSICINSERT`/`GROUP`/`IMAGE`). */
  | { readonly kind: 'classicPanel'; readonly panel: ClassicPanelKind }
  /** Set the drawing-tab strip's visibility (FILETAB shows, FILETABCLOSE hides). */
  | { readonly kind: 'fileTabs'; readonly value: boolean }
  /** Open one Quick View panel (a repeat open re-focuses it). */
  | { readonly kind: 'quickView'; readonly panel: QuickViewKind }
  /** Close one Quick View panel; already closed is a success, not a refusal. */
  | { readonly kind: 'quickViewClose'; readonly panel: QuickViewKind }
  /** Refresh the Quick View preview cards now (no file writes). */
  | { readonly kind: 'updateThumbs' }
  /** Open (`true`) or close (`false`) the QuickCalc dialog. */
  | { readonly kind: 'quickCalc'; readonly value: boolean }
  /** Set the quick-access toolbar strip's visibility (TOOLBAR shows, -TOOLBAR HIDE hides). */
  | { readonly kind: 'toolbar'; readonly value: boolean }
  /** Open the workspace selector. */
  | { readonly kind: 'workspace' }
  /** Open the workspace settings dialog in list or save-current mode. */
  | { readonly kind: 'workspaceSettings'; readonly mode: WorkspaceDialogMode }
  | { readonly kind: 'refused'; readonly panelKey: string };

/** The panel labels the layout refusal names. */
export const LOCAL_SELECTOR_PANEL_KEYS = Object.freeze({
  properties: 'properties.panel',
  layers: 'layers.panel',
});

/**
 * Resolve typed text to the chrome-local surface it opens, or `null` when the
 * line belongs to the router.
 *
 * The order mirrors the old inline branch: dialog selectors first, then the
 * desktop palette windows. `PROPERTIES`/`LAYER` are refused where the palette
 * layer is not rendered; `FONTS`/`RESOURCES` have a compact sheet and are never
 * refused by layout. `ABOUT` opens the existing host-owned dialog and is
 * likewise layout-independent. `CLEANSCREENON` only sets the chrome's own
 * clean-screen state, so it too needs no layout; `CLEANSCREENOFF` clears that
 * same state and is resolved directly after it. `PROPERTIESCLOSE` closes the
 * properties window and is resolved last: it is layout-independent because
 * closing an already-closed window is a valid no-op on every layout;
 * `LAYERCLOSE` closes the layers palette right after it for the same reason.
 *
 * The UI-visibility and window selectors (`RIBBON*`, `FILETAB*`,
 * `HIDEPALETTES`/`SHOWPALETTES`, `TOOLBAR`/`-TOOLBAR`, the two Quick View
 * pairs, `QUICKCALC`/`QCCLOSE`, `UPDATETHUMBSNOW`, `WORKSPACE*`) only set chrome
 * state or open a chrome-owned surface, so none of them depends on the layout or
 * on a document.
 */
export function localSelectorFor(text: string, layout: LayoutMode): LocalSelectorResult | null {
  if (isOptionsCommand(text)) return { kind: 'options' };
  if (isDSettingsCommand(text)) return { kind: 'options' };
  if (isCommandLineCommand(text)) return { kind: 'commandLine', value: true };
  if (isCommandLineHideCommand(text)) return { kind: 'commandLine', value: false };
  if (isRibbonCommand(text)) return { kind: 'ribbon', value: true };
  if (isRibbonCloseCommand(text)) return { kind: 'ribbon', value: false };
  if (isFileTabCommand(text)) return { kind: 'fileTabs', value: true };
  if (isFileTabCloseCommand(text)) return { kind: 'fileTabs', value: false };
  if (isHidePalettesCommand(text)) return { kind: 'paletteChrome', value: false };
  if (isShowPalettesCommand(text)) return { kind: 'paletteChrome', value: true };
  if (isPmToggleCommand(text)) return { kind: 'paletteToggle' };
  if (isClassicXRefCommand(text)) return { kind: 'classicXref' };
  if (isClassicInsertCommand(text)) return { kind: 'classicPanel', panel: 'insert' };
  if (isClassicGroupCommand(text)) return { kind: 'classicPanel', panel: 'group' };
  if (isClassicImageCommand(text)) return { kind: 'classicPanel', panel: 'image' };
  if (isQuickViewDrawingCommand(text)) return { kind: 'quickView', panel: 'drawings' };
  if (isQuickViewDrawingCloseCommand(text)) return { kind: 'quickViewClose', panel: 'drawings' };
  if (isQuickViewLayoutCommand(text)) return { kind: 'quickView', panel: 'layouts' };
  if (isQuickViewLayoutCloseCommand(text)) return { kind: 'quickViewClose', panel: 'layouts' };
  if (isQuickCalcCommand(text)) return { kind: 'quickCalc', value: true };
  if (isQuickCalcCloseCommand(text)) return { kind: 'quickCalc', value: false };
  if (isUpdateThumbsNowCommand(text)) return { kind: 'updateThumbs' };
  // `-TOOLBAR SHOW|HIDE` first: its bare form is a different word (`TOOLBAR`).
  const toolbar = dashToolbarVisibility(text);
  if (toolbar !== null) return { kind: 'toolbar', value: toolbar };
  if (isToolbarCommand(text)) return { kind: 'toolbar', value: true };
  if (isWorkspaceCommand(text)) return { kind: 'workspace' };
  if (isWorkspaceSaveCommand(text)) return { kind: 'workspaceSettings', mode: 'save' };
  if (isWorkspaceSettingsCommand(text)) return { kind: 'workspaceSettings', mode: 'settings' };
  if (isResourcesCommand(text)) return { kind: 'resources' };
  if (isFontsCommand(text)) return { kind: 'fonts' };
  if (isPropertiesCommand(text)) {
    return dockedPaletteOpenable(layout)
      ? { kind: 'properties' }
      : { kind: 'refused', panelKey: LOCAL_SELECTOR_PANEL_KEYS.properties };
  }
  if (isQuickPropertiesCommand(text)) {
    return dockedPaletteOpenable(layout)
      ? { kind: 'properties' }
      : { kind: 'refused', panelKey: LOCAL_SELECTOR_PANEL_KEYS.properties };
  }
  if (isLayersCommand(text)) {
    return dockedPaletteOpenable(layout)
      ? { kind: 'layers' }
      : { kind: 'refused', panelKey: LOCAL_SELECTOR_PANEL_KEYS.layers };
  }
  if (isClassicLayerCommand(text)) {
    return dockedPaletteOpenable(layout)
      ? { kind: 'layers' }
      : { kind: 'refused', panelKey: LOCAL_SELECTOR_PANEL_KEYS.layers };
  }
  if (isAboutCommand(text)) return { kind: 'about' };
  if (isCleanScreenOnCommand(text)) return { kind: 'cleanScreen', value: true };
  if (isCleanScreenOffCommand(text)) return { kind: 'cleanScreen', value: false };
  if (isPropertiesCloseCommand(text)) return { kind: 'propertiesClose' };
  if (isLayerCloseCommand(text)) return { kind: 'layersClose' };
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
  /** Open the existing host-owned About dialog (`requestOpenPopup('about')`). */
  readonly openAbout: () => void;
  /** Set the chrome's clean-screen state (`CLEANSCREENON`/`CLEANSCREENOFF` set, never toggle). */
  readonly setCleanScreen: (value: boolean) => void;
  /** Close the properties window (`PROPERTIESCLOSE`; a no-op when already closed). */
  readonly closeProperties: () => void;
  /** Close the layers palette (`LAYERCLOSE`; a no-op when already closed). */
  readonly closeLayers: () => void;
  /** Set the command line's visibility (`COMMANDLINE`/`COMMANDLINEHIDE`). */
  readonly setCommandLineOpen: (value: boolean) => void;
  /** Set the ribbon's visibility (`RIBBON`/`RIBBONCLOSE`; a set, never a toggle). */
  readonly setRibbonVisible: (value: boolean) => void;
  /** Set the drawing-tab strip's visibility (`FILETAB`/`FILETABCLOSE`). */
  readonly setFileTabsVisible: (value: boolean) => void;
  /** Set ribbon + palettes + drawing tabs together (`HIDEPALETTES`/`SHOWPALETTES`). */
  readonly setPaletteChrome: (value: boolean) => void;
  /** Flip that same palette-chrome state (`PMTOGGLE`). */
  readonly togglePaletteChrome: () => void;
  /** Open (or re-focus) the classic external-references panel (`CLASSICXREF`). */
  readonly openClassicXref: () => void;
  /** Open one classic block/group/image panel (a repeat open re-focuses it). */
  readonly openClassicPanel: (panel: ClassicPanelKind) => void;
  /** Open one Quick View panel (a repeat open re-focuses it). */
  readonly setQuickView: (panel: QuickViewKind) => void;
  /** Close one Quick View panel (`QVDRAWINGCLOSE`/`QVLAYOUTCLOSE`; idempotent). */
  readonly closeQuickView: (panel: QuickViewKind) => void;
  /** Recompute the Quick View preview cards now (`UPDATETHUMBSNOW`; no file writes). */
  readonly refreshQuickView: () => void;
  /** Open (`true`) or close (`false`) the QuickCalc dialog (`QUICKCALC`/`QCCLOSE`). */
  readonly setQuickCalc: (open: boolean) => void;
  /** Set the quick-access toolbar strip's visibility (`TOOLBAR`/`-TOOLBAR`). */
  readonly setToolbarVisible: (value: boolean) => void;
  /** Open the workspace selector. */
  readonly openWorkspace: () => void;
  /** Open the workspace settings dialog (`WSSETTINGS` lists, `WSSAVE` saves). */
  readonly openWorkspaceSettings: (mode: WorkspaceDialogMode) => void;
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
    openAbout,
    setCleanScreen,
    closeProperties,
    closeLayers,
    setCommandLineOpen,
    setRibbonVisible,
    setFileTabsVisible,
    setPaletteChrome,
    togglePaletteChrome,
    openClassicXref,
    openClassicPanel,
    setQuickView,
    closeQuickView,
    refreshQuickView,
    setQuickCalc,
    setToolbarVisible,
    openWorkspace,
    openWorkspaceSettings,
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
      case 'about': openAbout(); break;
      case 'cleanScreen': setCleanScreen(result.value); break;
      case 'propertiesClose': closeProperties(); break;
      case 'layersClose': closeLayers(); break;
      case 'commandLine': setCommandLineOpen(result.value); break;
      case 'ribbon': setRibbonVisible(result.value); break;
      case 'fileTabs': setFileTabsVisible(result.value); break;
      case 'paletteChrome': setPaletteChrome(result.value); break;
      case 'paletteToggle': togglePaletteChrome(); break;
      case 'classicXref': openClassicXref(); break;
      case 'classicPanel': openClassicPanel(result.panel); break;
      case 'quickView': setQuickView(result.panel); break;
      case 'quickViewClose': closeQuickView(result.panel); break;
      case 'updateThumbs': refreshQuickView(); break;
      case 'quickCalc': setQuickCalc(result.value); break;
      case 'toolbar': setToolbarVisible(result.value); break;
      case 'workspace': openWorkspace(); break;
      case 'workspaceSettings': openWorkspaceSettings(result.mode); break;
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
    openAbout,
    setCleanScreen,
    closeProperties,
    closeLayers,
    setCommandLineOpen,
    setRibbonVisible,
    setFileTabsVisible,
    setPaletteChrome,
    togglePaletteChrome,
    openClassicXref,
    openClassicPanel,
    setQuickView,
    closeQuickView,
    refreshQuickView,
    setQuickCalc,
    setToolbarVisible,
    openWorkspace,
    openWorkspaceSettings,
    reportError,
  ]);
}
