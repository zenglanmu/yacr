// The AutoCAD profile's panel catalog (ADR-0053 D7/D8).
//
// One panel per reference panel, in reference order inside each tab (see
// `tabs.ts`). Items still name the same command tokens as before: moving a
// button between tabs never changes what it dispatches. A `<name>_PENDING`
// panel is an explicit "not wired in this build" marker — it renders a note
// instead of a fake button, so the reference structure is visible without
// claiming the reference command.
//
// Extension panels (this build's own additions, not in AutoCAD 2024's Drafting
// & Annotation ribbon) are named where they sit: File, BIM, Standard views,
// View state, Display, Diagnostics, Fonts/resources. They stay after the
// reference panels so the reference order is stable.
import type { RibbonItem, RibbonPanel } from './ribbon-model.ts';
import { POPUP_KEYS } from '../popup/popup.ts';
import { OPTIONS_KEYS } from '../options/options.ts';
import { STANDARD_VIEWS } from '../cad/view3d.ts';
import { HOME_MODIFY_ITEMS, SOLID_MODIFY_ITEM } from './ribbon-modify.ts';
import { LAYER_PANEL_ITEMS } from './layers-panel.ts';
import { BIM_PANEL } from './bim-panel.ts';
import { ANNOTATE_TEXT_COMMAND_ITEMS } from './annotate-text-commands.ts';
import { CIRCLE_ITEM, HATCHEDIT_ITEM, MEASUREGEOM_ITEM, POLYGON_ITEM } from './variants.ts';
import { EXPORT_ITEM } from './ribbon-file-export.ts';
import { INTERFACE_PANEL } from './ribbon-interface.ts';
import { SHELL_ACTIONS } from './actions.ts';

function panel(
  id: string,
  titleKey: string,
  items: readonly RibbonItem[],
  extra: { readonly launcher?: boolean; readonly launcherCommand?: string; readonly rows?: number } = {},
): RibbonPanel {
  return Object.freeze({ id, titleKey, items: Object.freeze([...items]), ...extra });
}

function pending(id: string, titleKey: string): RibbonPanel {
  return Object.freeze({ id, titleKey, items: Object.freeze([]), pending: true });
}

// --- File (extension: AutoCAD's File is the application menu) ----------------

export const FILE_PANEL = panel('file', 'ribbon.file', [
  { id: 'OPEN', labelKey: 'file.open', icon: 'open', command: 'OPEN', large: true },
  { id: 'NEW', labelKey: 'qat.new', icon: 'new', command: 'NEW', large: true },
  { id: 'SAVE', labelKey: 'qat.save', icon: 'save', command: 'SAVE', large: true },
  EXPORT_ITEM,
  { id: 'PLOT', labelKey: 'file.plot', icon: 'save', command: 'PLOT', large: true },
  { id: 'CLOSE', labelKey: 'doctabs.close', icon: 'close', command: 'CLOSE' },
]);

export const FILE_UTILITIES_PANEL = panel('utilities', 'ribbon.panel.utilities', [
  { id: 'fileinfo', labelKey: 'fileinfo.title', icon: 'info', action: 'fileinfo' },
  { id: 'diagnostics', labelKey: 'diagnostics.title', icon: 'diagnostics', action: 'diagnostics' },
  { id: 'audit', labelKey: 'audit.title', icon: 'diagnostics', command: 'AUDIT' },
  { id: 'recover', labelKey: 'draw.kind.recover', icon: 'diagnostics', command: 'RECOVER' },
  {
    id: 'clean',
    labelKey: 'status.clean_screen',
    icon: 'cleanScreen',
    action: 'cleanScreen',
    variants: [
      { id: 'clean-on', labelKey: 'status.clean_screen_on', icon: 'cleanScreen', command: 'CLEANSCREENON' },
      { id: 'clean-off', labelKey: 'status.clean_screen_off', icon: 'cleanScreen', command: 'CLEANSCREENOFF' },
    ],
  },
  { id: 'cmdline', labelKey: 'status.command_line', icon: 'terminal', action: 'commandLine' },
  { id: 'about', labelKey: POPUP_KEYS.ribbonEntry, icon: 'app', action: 'about' },
  { id: 'options', labelKey: OPTIONS_KEYS.ribbonEntry, icon: 'settings', action: 'options' },
  { id: 'plugins', labelKey: 'file.plugins', icon: 'settings', action: SHELL_ACTIONS.plugins },
]);

// --- Home: Draw | Modify | Annotation | Layers | Block | Properties | Groups
//     | Utilities | Clipboard (reference order) ------------------------------

export const DRAW_PANEL = panel('draw', 'ribbon.panel.draw', [
  { id: 'LINE', labelKey: 'draw.kind.line', icon: 'line', command: 'LINE', large: true },
  CIRCLE_ITEM,
  { id: 'ARC', labelKey: 'draw.kind.arc', icon: 'arc', command: 'ARC' },
  { id: 'XLINE', labelKey: 'draw.kind.xline', icon: 'xline', command: 'XLINE', hintKey: 'draw.kind.xline.hint' },
  { id: 'RAY', labelKey: 'draw.kind.ray', icon: 'ray', command: 'RAY', hintKey: 'draw.kind.ray.hint' },
  { id: 'PLINE', labelKey: 'draw.kind.polyline', icon: 'polyline', command: 'PLINE' },
  { id: 'RECTANG', labelKey: 'draw.kind.rectang', icon: 'rectang', command: 'RECTANG' },
  { id: 'ELLIPSE', labelKey: 'draw.kind.ellipse', icon: 'ellipse', command: 'ELLIPSE' },
  POLYGON_ITEM,
  { id: 'POINT', labelKey: 'draw.kind.point', icon: 'point', command: 'POINT' },
  { id: 'SPLINE', labelKey: 'draw.kind.spline', icon: 'spline', command: 'SPLINE' },
  { id: 'HATCH', labelKey: 'draw.kind.hatch', icon: 'hatch', command: 'HATCH' },
  { id: 'HATCHPOINT', labelKey: 'draw.kind.hatchpoint', icon: 'hatch', command: 'HATCHPOINT' },
  { id: 'BOUNDARY', labelKey: 'draw.kind.boundary', icon: 'polyline', command: 'BOUNDARY', hintKey: 'draw.kind.boundary.hint' },
  { id: 'HATCHGENERATEBOUNDARY', labelKey: 'draw.kind.hatchgenerateboundary', icon: 'hatch', command: 'HATCHGENERATEBOUNDARY' },
  { id: 'REVCLOUD', labelKey: 'draw.kind.revcloud', icon: 'polyline', command: 'REVCLOUD' },
  { id: 'GRADIENT', labelKey: 'draw.kind.gradient', icon: 'hatch', command: 'GRADIENT' },
  { id: 'WIPEOUT', labelKey: 'draw.kind.wipeout', icon: 'erase', command: 'WIPEOUT' },
]);

export const HOME_MODIFY_PANEL = panel('modify', 'ribbon.panel.modify', [
  ...HOME_MODIFY_ITEMS,
  SOLID_MODIFY_ITEM,
  HATCHEDIT_ITEM,
]);

export const HOME_ANNOTATION_PANEL = panel('annotation', 'ribbon.panel.annotation', [
  { id: 'TEXT', labelKey: 'draw.kind.text', icon: 'text', command: 'TEXT', large: true },
  { id: 'MTEXT', labelKey: 'draw.kind.mtext', icon: 'mtext', command: 'MTEXT' },
  { id: 'SCALETEXT', labelKey: 'draw.kind.scaletext', icon: 'scale', command: 'SCALETEXT' },
  { id: 'DIMLINEAR', labelKey: 'draw.kind.dimlinear', icon: 'dimlinear', command: 'DIMLINEAR', large: true },
  { id: 'MLEADER', labelKey: 'draw.kind.mleader', icon: 'mleader', command: 'MLEADER' },
  { id: 'TABLE', labelKey: 'draw.kind.table', icon: 'properties', command: 'TABLE' },
]);

export const HOME_LAYERS_PANEL = panel('layers', 'layers.panel', LAYER_PANEL_ITEMS, {
  launcher: true,
  launcherCommand: 'LAYER',
});

export const HOME_BLOCK_PANEL = panel('block', 'ribbon.panel.block', [
  { id: 'INSERT', labelKey: 'draw.kind.insertblock', icon: 'copy', command: 'INSERT', large: true },
  { id: 'BLOCK', labelKey: 'draw.kind.blockdefine', icon: 'copy', command: 'BLOCK', large: true },
]);

export const HOME_PROPERTIES_PANEL = panel('properties', 'properties.panel', [
  { id: 'PROPERTIES', labelKey: 'properties.panel', icon: 'properties', action: 'properties', large: true },
  { id: 'LIST', labelKey: 'draw.kind.list', icon: 'properties', command: 'LIST', hintKey: 'draw.kind.list.hint' },
  { id: 'MASSPROP', labelKey: 'draw.kind.massprop', icon: 'properties', command: 'MASSPROP', hintKey: 'draw.kind.massprop.hint' },
], { launcher: true, launcherCommand: 'PROPERTIES' });

export const HOME_GROUPS_PANEL = panel('groups', 'ribbon.panel.groups', [
  { id: 'GROUP', labelKey: 'draw.kind.group', icon: 'properties', command: 'GROUP', large: true },
  { id: 'UNGROUP', labelKey: 'draw.kind.ungroup', icon: 'explode', command: 'UNGROUP' },
  { id: 'GROUPEDIT', labelKey: 'draw.kind.groupedit', icon: 'pencil', command: 'GROUPEDIT' },
]);

export const HOME_UTILITIES_PANEL = panel('utilities', 'ribbon.panel.utilities', [
  { id: 'DIST', labelKey: 'measure.kind.distance', icon: 'distance', command: 'DIST', large: true },
  { id: 'PLINE', labelKey: 'measure.kind.polyline', icon: 'polyline', command: 'MEASUREPOLYLINE', large: true },
  { id: 'ANGLE', labelKey: 'measure.kind.angle', icon: 'angle', command: 'MEASUREANGLE', large: true },
  {
    id: 'AREA',
    labelKey: 'measure.kind.area',
    icon: 'area',
    command: 'AREA',
    large: true,
    variants: [
      { id: 'AREA', labelKey: 'measure.kind.area', icon: 'area', command: 'AREA' },
      { id: 'AREA OBJECT', labelKey: 'measure.kind.area_object', icon: 'area', command: 'AREA OBJECT' },
    ],
  },
  { id: 'ID', labelKey: 'measure.kind.id', icon: 'point', command: 'ID', large: true },
  MEASUREGEOM_ITEM,
  { id: 'STATUS', labelKey: 'draw.kind.status', icon: 'diagnostics', command: 'STATUS', hintKey: 'draw.kind.status.hint' },
]);

export const HOME_CLIPBOARD_PANEL = panel('clipboard', 'ribbon.panel.clipboard', [
  { id: 'SELECTALL', labelKey: 'selection.all', icon: 'selectAll', command: 'SELECTALL', large: true },
  { id: 'COPY', labelKey: 'draw.kind.copy', icon: 'copy', command: 'COPY' },
  { id: 'PASTEBLOCK', labelKey: 'draw.kind.pasteblock', icon: 'copy', command: 'PASTEBLOCK' },
  { id: 'MATCHPROP', labelKey: 'draw.kind.matchprop', icon: 'matchprop', command: 'MATCHPROP' },
]);

// --- Insert: Block | Block Definition | Reference | Import | Data | Linking
//     & Extraction | Content (+ BIM extension) -------------------------------

export const INSERT_BLOCK_PANEL = panel('block', 'ribbon.panel.block', [
  { id: 'INSERT', labelKey: 'draw.kind.insertblock', icon: 'copy', command: 'INSERT', large: true },
]);

export const INSERT_BLOCK_DEFINITION_PANEL = panel('block_definition', 'ribbon.panel.block_definition', [
  { id: 'BLOCK', labelKey: 'draw.kind.blockdefine', icon: 'copy', command: 'BLOCK', large: true },
]);

export const INSERT_REFERENCE_PANEL = panel('reference', 'ribbon.panel.reference', [
  { id: 'XREF', labelKey: 'draw.kind.xref', icon: 'reference', command: 'XREF', large: true },
  { id: 'IMAGE', labelKey: 'draw.kind.image', icon: 'reference', command: 'IMAGE', large: true },
]);

export const INSERT_IMPORT_PANEL = panel('import', 'ribbon.panel.import', [
  { id: 'IMPORT', labelKey: 'draw.kind.import', icon: 'open', command: 'IMPORT', large: true },
]);

// --- Annotate: Text | Dimension | Leader | Table | Markup | Scaling ---------

export const ANNOTATE_TEXT_PANEL = panel('text', 'ribbon.panel.text', [
  { id: 'TEXT', labelKey: 'draw.kind.text', icon: 'text', command: 'TEXT', large: true },
  { id: 'MTEXT', labelKey: 'draw.kind.mtext', icon: 'mtext', command: 'MTEXT', large: true },
  { id: 'SCALETEXT', labelKey: 'draw.kind.scaletext', icon: 'scale', command: 'SCALETEXT' },
  ...ANNOTATE_TEXT_COMMAND_ITEMS,
  // Extension: the font registry is an annotation dependency, not a reference
  // panel of its own.
  { id: 'FONTS', labelKey: 'resources.fonts', icon: 'font', command: 'FONTS' },
]);

export const ANNOTATE_DIMENSION_PANEL = panel('dimension', 'ribbon.panel.dimension', [
  { id: 'DIMLINEAR', labelKey: 'draw.kind.dimlinear', icon: 'dimlinear', command: 'DIMLINEAR', large: true },
  { id: 'DIMALIGNED', labelKey: 'draw.kind.dimaligned', icon: 'dimaligned', command: 'DIMALIGNED', large: true },
  { id: 'DIMRADIUS', labelKey: 'draw.kind.dimradius', icon: 'dimradius', command: 'DIMRADIUS', large: true },
  { id: 'DIMDIAMETER', labelKey: 'draw.kind.dimdiameter', icon: 'dimdiameter', command: 'DIMDIAMETER', large: true },
  { id: 'DIMCENTER', labelKey: 'draw.kind.dimcenter', icon: 'dimcenter', command: 'DIMCENTER', large: true },
  { id: 'DIMANGULAR', labelKey: 'draw.kind.dimangular', icon: 'dimangular', command: 'DIMANGULAR', large: true },
  { id: 'DIMORDINATE', labelKey: 'draw.kind.dimordinate', icon: 'dimordinate', command: 'DIMORDINATE', large: true },
  { id: 'DIMBASELINE', labelKey: 'draw.kind.dimbaseline', icon: 'dimbaseline', command: 'DIMBASELINE', large: true },
  { id: 'DIMCONTINUE', labelKey: 'draw.kind.dimcontinue', icon: 'dimcontinue', command: 'DIMCONTINUE', large: true },
  { id: 'DIMROTATED', labelKey: 'draw.kind.dimrotated', icon: 'dimrotated', command: 'DIMROTATED', large: true },
  { id: 'DIMSPACE', labelKey: 'draw.kind.dimspace', icon: 'dimspace', command: 'DIMSPACE', large: true },
]);

export const ANNOTATE_LEADER_PANEL = panel('leader', 'ribbon.panel.leader', [
  { id: 'MLEADER', labelKey: 'draw.kind.mleader', icon: 'mleader', command: 'MLEADER', large: true },
]);

export const ANNOTATE_TABLE_PANEL = panel('table', 'ribbon.panel.table', [
  { id: 'TABLE', labelKey: 'draw.kind.table', icon: 'properties', command: 'TABLE', large: true },
]);

export const ANNOTATE_MARKUP_PANEL = panel('markup', 'ribbon.panel.markup', [
  { id: 'REVCLOUD', labelKey: 'draw.kind.revcloud', icon: 'polyline', command: 'REVCLOUD', large: true },
]);

// --- View: Navigate | Visualize | Coordinates | Viewports | Palettes |
//     Interface (+ Standard views / View state / Display extensions) ----------

export const VIEW_NAVIGATE_PANEL = panel('navigate', 'ribbon.panel.navigate', [
  {
    id: 'FIT',
    labelKey: 'toolbar.fit',
    icon: 'fit',
    command: 'FIT',
    large: true,
    variants: [
      { id: 'FIT', labelKey: 'toolbar.zoom_extents', icon: 'fit', command: 'FIT' },
      { id: 'ZOOMIN', labelKey: 'toolbar.zoom_in', icon: 'zoomIn', command: 'ZOOMIN' },
      { id: 'ZOOMOUT', labelKey: 'toolbar.zoom_out', icon: 'zoomOut', command: 'ZOOMOUT' },
    ],
  },
  { id: 'PAN', labelKey: 'toolbar.pan', icon: 'pan', command: 'PAN' },
  { id: 'ORBIT', labelKey: 'view.orbit_label', icon: 'orbit', command: 'ORBIT' },
  { id: 'ZOOMIN', labelKey: 'toolbar.zoom_in', icon: 'zoomIn', command: 'ZOOMIN' },
  { id: 'ZOOMOUT', labelKey: 'toolbar.zoom_out', icon: 'zoomOut', command: 'ZOOMOUT' },
  { id: 'ZOOM WINDOW', labelKey: 'toolbar.zoom_window', icon: 'zoomWindow', command: 'ZOOM WINDOW', hintKey: 'toolbar.zoom_window.hint' },
  { id: 'ZOOM OBJECT', labelKey: 'toolbar.zoom_object', icon: 'selectAll', command: 'ZOOM OBJECT', hintKey: 'toolbar.zoom_object.hint' },
  { id: 'ZOOM PREVIOUS', labelKey: 'toolbar.zoom_previous', icon: 'zoomPrevious', command: 'ZOOM PREVIOUS', hintKey: 'toolbar.zoom_previous.hint' },
  { id: 'RESETVIEW', labelKey: 'view.reset', icon: 'view', command: 'RESETVIEW' },
], { launcher: true });

export const VIEW_VISUALIZE_PANEL = panel('view3d', 'ribbon.panel.visualize', [
  { id: 'SWITCH2D3D', labelKey: 'ribbon.command.view_switch2d3d', icon: 'view', command: 'SWITCH2D3D', large: true },
  { id: 'PROJECTION', labelKey: 'view.projection', icon: 'axes', command: 'PROJECTION' },
  { id: 'IBL', labelKey: 'ribbon.command.ibl', icon: 'sun', command: 'IBL', hintKey: 'ribbon.command.ibl.hint', activeKey: 'iblEnabled' },
]);

export const VIEW_STANDARD_PANEL = panel('standard', 'ribbon.panel.standard', [
  ...STANDARD_VIEWS.map(entry => ({ ...entry })),
  { id: 'PLAN', labelKey: 'view.plan', icon: 'view', command: 'PLAN' },
], { rows: 2 });

export const VIEW_STATE_PANEL = panel('viewstate', 'ribbon.panel.viewstate', [
  { id: 'REDRAW', labelKey: 'draw.kind.redraw', icon: 'view', command: 'REDRAW', hintKey: 'draw.kind.redraw.hint' },
  { id: 'REDRAWALL', labelKey: 'draw.kind.redrawall', icon: 'view', command: 'REDRAWALL', hintKey: 'draw.kind.redrawall.hint' },
  { id: 'REGEN', labelKey: 'draw.kind.regen', icon: 'view', command: 'REGEN', hintKey: 'draw.kind.regen.hint' },
  { id: 'REGENALL', labelKey: 'draw.kind.regenall', icon: 'view', command: 'REGENALL', hintKey: 'draw.kind.regenall.hint' },
  { id: 'VIEWBACK', labelKey: 'draw.kind.viewback', icon: 'zoomPrevious', command: 'VIEWBACK', hintKey: 'draw.kind.viewback.hint' },
  { id: 'VIEWFORWARD', labelKey: 'draw.kind.viewforward', icon: 'view', command: 'VIEWFORWARD', hintKey: 'draw.kind.viewforward.hint' },
]);

export const VIEW_DISPLAY_PANEL = panel('display', 'ribbon.panel.display', [
  { id: 'GRID', labelKey: 'overlay.grid', icon: 'grid', command: 'GRID' },
  { id: 'SNAP', labelKey: 'status.toggle.snap', icon: 'snap', command: 'SNAP' },
  { id: 'ORTHO', labelKey: 'status.toggle.ortho', icon: 'angle', command: 'ORTHO' },
]);

export const VIEW_PALETTES_PANEL = panel('palettes', 'ribbon.panel.palettes', [
  { id: 'fileinfo', labelKey: 'fileinfo.title', icon: 'info', action: 'fileinfo', large: true },
  { id: 'diagnostics', labelKey: 'diagnostics.title', icon: 'diagnostics', action: 'diagnostics', large: true },
  { id: 'STATUS', labelKey: 'draw.kind.status', icon: 'diagnostics', command: 'STATUS', hintKey: 'draw.kind.status.hint' },
  { id: 'clean', labelKey: 'status.clean_screen', icon: 'cleanScreen', action: 'cleanScreen' },
]);

// --- Manage: CAD Standards | Action Recorder | Customization | Applications
//     (+ Cleanup / Diagnostics extensions) ------------------------------------

export const MANAGE_APPLICATIONS_PANEL = panel('applications', 'ribbon.panel.applications', [
  { id: 'options', labelKey: OPTIONS_KEYS.ribbonEntry, icon: 'settings', action: 'options', large: true },
  { id: 'plugins', labelKey: 'file.plugins', icon: 'settings', action: SHELL_ACTIONS.plugins },
  { id: 'about', labelKey: POPUP_KEYS.ribbonEntry, icon: 'app', action: 'about' },
]);

export const MANAGE_CLEANUP_PANEL = panel('cleanup', 'ribbon.panel.cleanup', [
  { id: 'AUDIT', labelKey: 'audit.title', icon: 'diagnostics', command: 'AUDIT', large: true },
  { id: 'RECOVER', labelKey: 'draw.kind.recover', icon: 'diagnostics', command: 'RECOVER' },
  { id: 'PURGE', labelKey: 'draw.kind.purge', icon: 'erase', command: 'PURGE' },
  { id: 'OVERKILL', labelKey: 'draw.kind.overkill', icon: 'erase', command: 'OVERKILL' },
]);

export const MANAGE_DIAGNOSTICS_PANEL = panel('diagnostics', 'diagnostics.title', [
  { id: 'fileinfo', labelKey: 'fileinfo.title', icon: 'info', action: 'fileinfo' },
  { id: 'diagnostics', labelKey: 'diagnostics.title', icon: 'diagnostics', action: 'diagnostics' },
  { id: 'STATUS', labelKey: 'draw.kind.status', icon: 'diagnostics', command: 'STATUS', hintKey: 'draw.kind.status.hint' },
]);

// --- Output: Plot | Export | Send -------------------------------------------

export const OUTPUT_PLOT_PANEL = panel('plot', 'ribbon.panel.plot', [
  { id: 'PLOT', labelKey: 'file.plot', icon: 'save', command: 'PLOT', large: true },
]);

export const OUTPUT_EXPORT_PANEL = panel('export', 'ribbon.panel.export', [
  EXPORT_ITEM,
]);

// --- Add-ins ----------------------------------------------------------------

export const ADDINS_PLUGINS_PANEL = panel('addins', 'ribbon.panel.addins', [
  { id: 'plugins', labelKey: 'file.plugins', icon: 'settings', action: SHELL_ACTIONS.plugins, large: true },
]);

// --- Pending reference panels ------------------------------------------------

export const PARAMETRIC_GEOMETRIC_PANEL = pending('geometric', 'ribbon.panel.geometric');
export const PARAMETRIC_DIMENSIONAL_PANEL = pending('dimensional', 'ribbon.panel.dimensional');
export const PARAMETRIC_MANAGE_PANEL = panel('parametric_manage', 'ribbon.panel.parametric_manage', [
  // The Parameters Manager surface. It is chrome-local (`SHELL_ACTIONS.parameters`)
  // because the panel reads `cad_param_list` and writes through the
  // `-PARAMETERS` command line; there is no payload-free `PARAMETERS` router
  // command, so the ribbon must never send `cad_command` for it.
  { id: 'PARAMETERS', labelKey: 'parameters.open', icon: 'properties', action: SHELL_ACTIONS.parameters, large: true },
]);
export const INSERT_DATA_PANEL = pending('data', 'ribbon.panel.data');
export const INSERT_LINKING_PANEL = pending('linking', 'ribbon.panel.linking');
// Content: the Tool Palettes float is this build's real Blocks surface (the
// reference's Blocks palette/DesignCenter remain named refusals, so they are
// not offered as dead entries).
export const INSERT_CONTENT_PANEL = panel('content', 'ribbon.panel.content', [
  { id: 'TOOLPALETTES', labelKey: 'draw.kind.toolpalettes', icon: 'palette', command: 'TOOLPALETTES', large: true, hintKey: 'draw.kind.toolpalettes.hint' },
]);
// Annotation scaling: the read-only Scale List report is the one implemented
// member of the reference panel; OBJECTSCALE/ANNOSCALE are named refusals.
export const ANNOTATE_SCALING_PANEL = panel('annotation_scaling', 'ribbon.panel.annotation_scaling', [
  { id: 'SCALELISTEDIT', labelKey: 'draw.kind.scalelistedit', icon: 'scale', command: 'SCALELISTEDIT', large: true, hintKey: 'draw.kind.scalelistedit.hint' },
]);
// Coordinates: UCSICON is the one implemented UCS-family session token. UCS and
// UCSMAN stay named refusals (no UCS model), so they are not shown as buttons.
export const VIEW_COORDINATES_PANEL = panel('coordinates', 'ribbon.panel.coordinates', [
  { id: 'UCSICON', labelKey: 'draw.kind.ucsicon', icon: 'axes', command: 'UCSICON', hintKey: 'draw.kind.ucsicon.hint' },
]);
// Viewports: every button here is a real router command (VPMIN/VPMAX are
// payload-free session moves, the rest take their documented typed argument or
// pick flow). MVSETUP stays a named refusal and is deliberately absent.
export const VIEW_VIEWPORTS_PANEL = panel('viewports', 'ribbon.panel.viewports', [
  { id: 'VPORTS', labelKey: 'draw.kind.vports', icon: 'rectang', command: 'VPORTS', large: true, hintKey: 'draw.kind.vports.hint' },
  { id: 'MVIEW', labelKey: 'draw.kind.mview', icon: 'view', command: 'MVIEW', hintKey: 'draw.kind.mview.hint' },
  { id: 'VPCLIP', labelKey: 'draw.kind.vpclip', icon: 'rectang', command: 'VPCLIP', hintKey: 'draw.kind.vpclip.hint' },
  { id: 'VPMAX', labelKey: 'draw.kind.vpmax', icon: 'view', command: 'VPMAX' },
  { id: 'VPMIN', labelKey: 'draw.kind.vpmin', icon: 'view', command: 'VPMIN' },
  { id: 'VPSCALE', labelKey: 'draw.kind.vpscale', icon: 'properties', command: 'VPSCALE', hintKey: 'draw.kind.vpscale.hint' },
  { id: 'LAYVPI', labelKey: 'draw.kind.layvpi', icon: 'layers', command: 'LAYVPI', hintKey: 'draw.kind.layvpi.hint' },
]);
export const MANAGE_STANDARDS_PANEL = pending('cad_standards', 'ribbon.panel.cad_standards');
export const MANAGE_RECORDER_PANEL = panel('action_recorder', 'ribbon.panel.action_recorder', [
  // The panel is chrome-local (the same shape as Parameters): it reads
  // `cad_action_recorder` and writes through the typed ACT* command lines. No
  // dialog is offered because none exists; the typed path is the file surface.
  { id: 'ACTIONRECORDER', labelKey: 'recorder.title', icon: 'settings', action: SHELL_ACTIONS.actionRecorder, large: true },
]);
export const MANAGE_CUSTOMIZATION_PANEL = pending('customization', 'ribbon.panel.customization');
export const OUTPUT_SEND_PANEL = pending('send', 'ribbon.panel.send');
export const ADDINS_VENDOR_PANEL = pending('vendor', 'ribbon.panel.vendor');

// Panels imported whole from their own modules.
export { BIM_PANEL, INTERFACE_PANEL };
