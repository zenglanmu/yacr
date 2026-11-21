// The Home tab's BIM panel: one launcher for the document-wide browse panel
// (ADR-0042 Phase 3). Split out of `ribbon-model.ts`, the same way the Layers
// panel is, so the tab tree stays under the 1000-line code ceiling.
//
// The action token lives beside its panel because it is chrome-local: the panel
// is a React surface over `cad_bim_query`, so nothing here is a
// `cad_command`. It is deliberately **not** in `PALETTE_ACTIONS`: on a carrier
// with no BIM query lane the entry must stay reachable so the panel can show the
// host's `unsupported-runtime: bimQuery` refusal instead of the button vanishing.
import type { RibbonPanel } from './ribbon-model.ts';

/** The action token `runAction` resolves to `openBimPanel()`. */
export const BIM_ACTION = 'bim';

/** The BIM panel in the Home tab. */
export const BIM_PANEL: RibbonPanel = Object.freeze<RibbonPanel>({
  id: 'bim',
  titleKey: 'ribbon.panel.bim',
  items: [
    { id: 'bim', labelKey: 'bim.panel', icon: 'bim', action: BIM_ACTION, large: true },
  ],
});
