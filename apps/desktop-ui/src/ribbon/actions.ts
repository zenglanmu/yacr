// The ribbon's dispatch vocabularies: shell actions and palette carriers.
//
// Split out of `ribbon-model.ts` so the panel catalog can name an action without
// importing the model back (which would make `ribbon-model` <-> `tabs` a runtime
// cycle). Values are unchanged; the model re-exports them for existing readers.

/** Shell actions that never touch the CAD protocol. */
export const SHELL_ACTIONS = Object.freeze({
  fileinfo: 'fileinfo',
  diagnostics: 'diagnostics',
  cleanScreen: 'cleanScreen',
  commandLine: 'commandLine',
  language: 'language',
  about: 'about',
  // Opens the Options dialog: a host-owned window like About, so it is an
  // `action` token and never a router command (`OPTIONS` has no Rust command;
  // the command line intercepts it locally).
  options: 'options',
  // Opens the Options dialog on its Plugins page. This is the direct entry a
  // user looks for in the File tab: the page exists, but a plain Options open
  // always starts on the first settings tab. Like Options it is a shell action
  // with no router command and no host payload; the tab handoff is chrome-local
  // (`options.ts`'s pending-tab slot).
  plugins: 'plugins',
  // Toggles the properties palette window. Like OPTIONS it is chrome-local:
  // `PROPERTIES` has no Rust command, so the ribbon must not send `cad_command`.
  properties: 'properties',
  // Opens the Parameters Manager panel (Parametric > Manage). Chrome-local like
  // the BIM panel: the surface reads `cad_param_list`, and the panel's edits use
  // the `-PARAMETERS` command-line form.
  parameters: 'parameters',
  // Opens the Action Recorder panel (ADR-0056). Chrome-local like Parameters:
  // the panel reads `cad_action_recorder` and sends `ACTRECORD`/`ACTSTOP`/
  // `ACTMANAGER` through `cad_command`; there is no payload-free panel command.
  actionRecorder: 'actionRecorder',
  /**
   * The idle pointer (SketchUp's Select). It is not a router command: with a
   * tool active it sends the same `CANCEL` Escape sends, and with no tool it is
   * already the state. `runItem` handles it before the shell-action table so
   * every surface (band, menu, tray, compact drawer) shares one behaviour.
   */
  select: 'select',
  /**
   * Shows/hides the profile's `DomainTray` (the side is the profile's own
   * `traySide`). Chrome-local like the palettes: no router command, no host
   * payload. The reference apps both offer this (SketchUp: Window > Default Tray
   * > Hide Tray; Revit: View > User Interface), and the tray's own header button
   * runs the same action.
   */
  domainTray: 'domainTray',
  /**
   * Opens the application-domain switcher — the same popover the navigation
   * dock's app button and the typed `DOMAIN` command open. BIM and concept have
   * no command line, so without a menu/ribbon entry those profiles could not
   * reach the switcher at all except through the dock icon; this is the
   * in-profile path (BIM Manage > Settings, concept Window > Preferences).
   */
  domainSwitcher: 'domainSwitcher',
});

/** The shell actions that toggle a desktop palette window. */
export const PALETTE_ACTIONS: readonly string[] = Object.freeze([
  SHELL_ACTIONS.fileinfo,
  SHELL_ACTIONS.diagnostics,
  SHELL_ACTIONS.properties,
]);

/**
 * The palette actions a compact layout can still render — as its own sheet.
 *
 * The properties inspector is the one desktop palette with a compact equivalent
 * (the layout mounts the same `PropertiesDock` as a bottom sheet), so its entry
 * survives the carrier filter below; `fileinfo`, `diagnostics` and the layer run
 * still have no compact surface and are dropped.
 */
export const SHEET_PALETTE_ACTIONS: readonly string[] = Object.freeze([SHELL_ACTIONS.properties]);

/**
 * Palette toggles that dispatch a **command name** instead of a shell action.
 *
 * `LAYER` opens the layers palette but rides the command funnel (it is a
 * chrome-local selector: the palette answers it, no Rust command exists), so its
 * dispatch token is `command` even though the affordance is a palette toggle. For
 * this carrier filter the two spellings mean the same thing.
 *
 * `LAYISO`/`LAYUNISO` are **router** commands, not palette toggles, and they are
 * listed here only for the carrier rule: layer isolation is a state the layers
 * palette is the only surface for, so on a carrier that never renders the palette
 * (`dom-canvas`) the isolate pair would hide geometry with nothing on screen to
 * explain or undo it — a dead affordance, exactly like the toggle itself. They
 * are therefore dropped with the palette rather than left behind as live buttons.
 *
 * The rule this list encodes is therefore not "is it a palette toggle" but
 * **"is its effect only observable in the layers palette"**. `LAYOFF`/`LAYON`
 * (2026-10-26) pass it for the same reason: turning the selection's layers off
 * hides geometry, and turning every layer back on restores it, and the palette is
 * the only place either state is named or reversed. Left behind on a palette-less
 * carrier they would hide geometry with no surface to explain it, so they are
 * dropped with the palette too.
 */
export const PALETTE_COMMANDS: readonly string[] = Object.freeze([
  'LAYER',
  'LAYISO',
  'LAYUNISO',
  // `LAYERP` (2026-10-27) restores the previous layer-override state: the result is
  // again a layer state that only the palette names or reverses, so it is dropped
  // with the palette for the same reason as the rest of the layer run.
  'LAYERP',
  'LAYOFF',
  'LAYON',
  // The AutoCAD 2026 layer-selection run (`LAYCUR`, `LAYMCUR`, `LAYFRZ`,
  // `LAYTHW`, `LAYLCK`, `LAYULK`, `SETBYLAYER`) lives in the Layers panel and
  // changes layer state that only the palette names or reverses, so the whole
  // run follows the same carrier rule as the rest of the panel.
  'LAYCUR',
  'LAYMCUR',
  'LAYFRZ',
  'LAYTHW',
  'LAYLCK',
  'LAYULK',
  'SETBYLAYER',
  // The 2026-11-07 layer batch lives in the same panel and changes layer state,
  // so it follows the same carrier rule.
  'LAYMCH',
  'LAYMRG',
  'LAYDEL',
  'LAYERSTATE',
  'LAYWALK',
  // `FONTS` opens the read-only Font Manager, which is a palette window on the
  // desktop (and a floating sheet on the compact layout). A carrier that never
  // renders the palette layer has no surface for that report, so the button is
  // dropped with it instead of opening a panel that is not mounted.
  'FONTS',
]);

