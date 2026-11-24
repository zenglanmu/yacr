// The AutoCAD 2024 "Drafting & Annotation" tab order (ADR-0053 D7).
//
// Reference order: Home | Insert | Annotate | Parametric | View | Manage |
// Output | Add-ins. `file` is this build's extension (AutoCAD's File is the
// application menu): it stays first so every existing entry point keeps its
// place. Each tab names its reference panels first, then this build's extension
// panels (BIM, standard views, view state, display, diagnostics).
//
// The panel data lives in `panel-items.ts`; this module is only the order. A
// bottom-of-file `ribbonTab()` keeps `shell.ribbonTab` resolution in one place,
// and an unknown id (a profile switch, an old saved state) falls back to Home.
import type { RibbonTab } from './ribbon-model.ts';
import {
  ADDINS_PLUGINS_PANEL,
  ADDINS_VENDOR_PANEL,
  ANNOTATE_DIMENSION_PANEL,
  ANNOTATE_LEADER_PANEL,
  ANNOTATE_MARKUP_PANEL,
  ANNOTATE_SCALING_PANEL,
  ANNOTATE_TABLE_PANEL,
  ANNOTATE_TEXT_PANEL,
  BIM_PANEL,
  DRAW_PANEL,
  FILE_PANEL,
  FILE_UTILITIES_PANEL,
  HOME_ANNOTATION_PANEL,
  HOME_BLOCK_PANEL,
  HOME_CLIPBOARD_PANEL,
  HOME_GROUPS_PANEL,
  HOME_LAYERS_PANEL,
  HOME_MODIFY_PANEL,
  HOME_PROPERTIES_PANEL,
  HOME_UTILITIES_PANEL,
  INSERT_BLOCK_DEFINITION_PANEL,
  INSERT_BLOCK_PANEL,
  INSERT_CONTENT_PANEL,
  INSERT_DATA_PANEL,
  INSERT_IMPORT_PANEL,
  INSERT_LINKING_PANEL,
  INSERT_REFERENCE_PANEL,
  INTERFACE_PANEL,
  MANAGE_APPLICATIONS_PANEL,
  MANAGE_CLEANUP_PANEL,
  MANAGE_CUSTOMIZATION_PANEL,
  MANAGE_DIAGNOSTICS_PANEL,
  MANAGE_RECORDER_PANEL,
  MANAGE_STANDARDS_PANEL,
  OUTPUT_EXPORT_PANEL,
  OUTPUT_PLOT_PANEL,
  OUTPUT_SEND_PANEL,
  PARAMETRIC_DIMENSIONAL_PANEL,
  PARAMETRIC_GEOMETRIC_PANEL,
  PARAMETRIC_MANAGE_PANEL,
  VIEW_COORDINATES_PANEL,
  VIEW_DISPLAY_PANEL,
  VIEW_NAVIGATE_PANEL,
  VIEW_PALETTES_PANEL,
  VIEW_STANDARD_PANEL,
  VIEW_STATE_PANEL,
  VIEW_VIEWPORTS_PANEL,
  VIEW_VISUALIZE_PANEL,
} from './panel-items.ts';

export const RIBBON_TABS: readonly RibbonTab[] = Object.freeze<RibbonTab[]>([
  {
    id: 'file',
    labelKey: 'ribbon.file',
    accent: true,
    panels: [FILE_PANEL, FILE_UTILITIES_PANEL],
  },
  {
    id: 'home',
    labelKey: 'ribbon.home',
    panels: [
      DRAW_PANEL,
      HOME_MODIFY_PANEL,
      HOME_ANNOTATION_PANEL,
      HOME_LAYERS_PANEL,
      HOME_BLOCK_PANEL,
      HOME_PROPERTIES_PANEL,
      HOME_GROUPS_PANEL,
      HOME_UTILITIES_PANEL,
      HOME_CLIPBOARD_PANEL,
      BIM_PANEL,
    ],
  },
  {
    id: 'insert',
    labelKey: 'ribbon.insert',
    panels: [
      INSERT_BLOCK_PANEL,
      INSERT_BLOCK_DEFINITION_PANEL,
      INSERT_REFERENCE_PANEL,
      INSERT_IMPORT_PANEL,
      INSERT_DATA_PANEL,
      INSERT_LINKING_PANEL,
      INSERT_CONTENT_PANEL,
    ],
  },
  {
    id: 'annotate',
    labelKey: 'ribbon.annotate',
    panels: [
      ANNOTATE_TEXT_PANEL,
      ANNOTATE_DIMENSION_PANEL,
      ANNOTATE_LEADER_PANEL,
      ANNOTATE_TABLE_PANEL,
      ANNOTATE_MARKUP_PANEL,
      ANNOTATE_SCALING_PANEL,
    ],
  },
  {
    id: 'parametric',
    labelKey: 'ribbon.parametric',
    panels: [PARAMETRIC_GEOMETRIC_PANEL, PARAMETRIC_DIMENSIONAL_PANEL, PARAMETRIC_MANAGE_PANEL],
  },
  {
    id: 'view',
    labelKey: 'ribbon.view',
    panels: [
      VIEW_NAVIGATE_PANEL,
      VIEW_VISUALIZE_PANEL,
      VIEW_COORDINATES_PANEL,
      VIEW_VIEWPORTS_PANEL,
      VIEW_STANDARD_PANEL,
      VIEW_STATE_PANEL,
      VIEW_DISPLAY_PANEL,
      VIEW_PALETTES_PANEL,
      INTERFACE_PANEL,
    ],
  },
  {
    id: 'manage',
    labelKey: 'ribbon.manage',
    panels: [
      MANAGE_STANDARDS_PANEL,
      MANAGE_RECORDER_PANEL,
      MANAGE_CUSTOMIZATION_PANEL,
      MANAGE_APPLICATIONS_PANEL,
      MANAGE_CLEANUP_PANEL,
      MANAGE_DIAGNOSTICS_PANEL,
    ],
  },
  {
    id: 'output',
    labelKey: 'ribbon.output',
    panels: [OUTPUT_PLOT_PANEL, OUTPUT_EXPORT_PANEL, OUTPUT_SEND_PANEL],
  },
  {
    id: 'addins',
    labelKey: 'ribbon.addins',
    panels: [ADDINS_PLUGINS_PANEL, ADDINS_VENDOR_PANEL],
  },
]);

/** The tab with `id`, or Home for an unknown/absent id. */
export function ribbonTab(id: string): RibbonTab {
  return RIBBON_TABS.find(tab => tab.id === id) ?? RIBBON_TABS[1];
}

/** All command ids referenced by the ribbon (used to keep the catalog honest). */
export function ribbonCommandIds(): string[] {
  const ids = new Set<string>();
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      for (const item of panel.items) {
        if (typeof item.command === 'string') ids.add(item.command);
        for (const variant of item.variants ?? []) {
          if (typeof variant.command === 'string') ids.add(variant.command);
        }
      }
    }
  }
  return [...ids];
}
