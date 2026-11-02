// Catalog keys for the chrome's **structural limits**.
//
// Every affordance below is permanently disabled, and none of them is missing
// wiring: one drawing per session is a real limitation, and a ribbon panel
// launcher with no target has no dialog to open yet. The sentence a disabled
// button shows must say that, so it is not allowed to reuse
// `command.unsupported` ("not supported yet") or a `*.switch_unwired` key
// ("not wired"), which claim a missing implementation where the product has a
// design decision.
//
// The keys are declared here as constants, not as inline translation literals,
// because they are the chrome's structural limits and belong to one place: the
// catalogs (`crates/cad-ui-core/i18n/*.json`) own the wording, this module owns
// the key names the React tree reads. `test/chrome-affordances.test.ts` pins both
// facts (the keys resolve in both locales, and the component sources never spell
// the literals inline).
//
// Copy (kept in the catalogs, same placeholders):
//
//   doctabs.one_document
//     zh-CN  每个会话只打开一张图纸
//     en     One drawing per session
//   ribbon.panel_dialog_missing   ({panel} = the panel's own title)
//     zh-CN  此面板暂无可打开的对话框：{panel}
//     en     No dialog exists for this panel yet: {panel}

/** No second drawing in a session: the doc-tab `+` can never be a live tab. */
export const ONE_DOCUMENT_KEY: string = 'doctabs.one_document';

/** A panel launcher whose panel has no dialog behind it. */
export const PANEL_DIALOG_MISSING_KEY: string = 'ribbon.panel_dialog_missing';

/** The keys this module needs from the catalog, for the review that adds them. */
export const PENDING_CATALOG_KEYS: readonly string[] = Object.freeze([
  ONE_DOCUMENT_KEY,
  PANEL_DIALOG_MISSING_KEY,
]);
