// Catalog keys for the chrome's **structural limits**.
//
// The affordance below is permanently disabled, and it is not missing wiring: a
// ribbon panel launcher with no target has no dialog to open yet. The sentence a
// disabled button shows must say that, so it is not allowed to reuse
// `command.unsupported` ("not supported yet") or a `*.switch_unwired` key
// ("not wired"), which claim a missing implementation where the product has a
// design decision.
//
// The key is declared here as a constant, not as an inline translation literal,
// because it is one of the chrome's structural limits and belongs to one place:
// the catalogs (`crates/cad-ui-core/i18n/*.json`) own the wording, this module
// owns the key name the React tree reads. `test/chrome-affordances.test.ts` pins
// both facts (the key resolves in both locales, and the component sources never
// spell the literal inline).
//
// Copy (kept in the catalogs, same placeholder):
//
//   ribbon.panel_dialog_missing   ({panel} = the panel's own title)
//     zh-CN  此面板暂无可打开的对话框：{panel}
//     en     No dialog exists for this panel yet: {panel}

/** A panel launcher whose panel has no dialog behind it. */
export const PANEL_DIALOG_MISSING_KEY: string = 'ribbon.panel_dialog_missing';

/** The keys this module needs from the catalog, for the review that adds them. */
export const PENDING_CATALOG_KEYS: readonly string[] = Object.freeze([
  PANEL_DIALOG_MISSING_KEY,
]);
