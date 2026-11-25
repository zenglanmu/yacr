// BIM-domain ribbon model (Revit-derived structure).
//
// This is the **structure** of the reference application's ribbon — the tab and
// panel order — transcribed into plain data. It is not a screenshot copy: every
// glyph is our own line icon, every label is our own catalog string, and the
// reference product's assets are never shipped.
//
// A tool is wired only when the kernel actually has it:
//
//   * construction elements (walls, doors, windows, floors, roofs, stairs,
//     structural framing, rebar, levels/grids, rooms, spaces, schedules,
//     worksharing, coordination) have **no kernel operation** in this build and
//     are `RibbonPendingItem`s. They keep their Revit position, are disabled,
//     and name the missing BIM element kernel instead of pretending to run;
//   * generic commands that do exist (`MOVE`, `EXTRUDE`, `DIST`, `XATTACH`,
//     `PDFIMPORT`, `PURGE`, `AUDIT`, `PARAMETERS`, the BIM browse panel, ...)
//     keep their stable token and therefore pass the one shared admission.
//
// No profile enables a command (ADR-0053 D5): a wired row is still disabled when
// the document/carrier cannot service it.
import type { RibbonItem, RibbonPanel, RibbonTab } from '../../ribbon/ribbon-model.ts';
import type { RibbonPendingItem } from '../../ribbon/ribbon-model.ts';
import { STANDARD_VIEWS } from '../../cad/view3d.ts';
import { BIM_ACTION } from '../../ribbon/bim-panel.ts';
import { SHELL_ACTIONS } from '../../ribbon/actions.ts';

function panel(id: string, titleKey: string, items: readonly RibbonItem[]): RibbonPanel {
  return Object.freeze({ id, titleKey, items: Object.freeze([...items]) });
}

function tab(id: string, labelKey: string, panels: readonly RibbonPanel[]): RibbonTab {
  return Object.freeze({ id, labelKey, panels: Object.freeze([...panels]) });
}

/** A Revit tool this build cannot run; the row says so and is disabled. */
function pending(
  id: string,
  labelKey: string,
  icon: string,
  large = false,
): RibbonPendingItem {
  return {
    id,
    labelKey,
    icon,
    pendingKey: 'bim.pending.kernel',
    ...(large ? { large: true } : {}),
  };
}

/** The seven standard views, shared with the drafting band (same typed IPC). */
function standardViewItems(): readonly RibbonItem[] {
  return STANDARD_VIEWS.map(entry => ({ ...entry }));
}

// --- Architecture ---------------------------------------------------------

const BUILD = panel('bim.build', 'bim.panel.build', [
  pending('bim.wall', 'bim.item.wall', 'rectang', true),
  pending('bim.door', 'bim.item.door', 'copy', true),
  pending('bim.window', 'bim.item.window', 'rectang', true),
  pending('bim.floor', 'bim.item.floor', 'drawing'),
  pending('bim.roof', 'bim.item.roof', 'home'),
  pending('bim.ceiling', 'bim.item.ceiling', 'grid'),
  pending('bim.stair', 'bim.item.stair', 'arrayrect'),
  pending('bim.ramp', 'bim.item.ramp', 'align'),
]);

const ROOMS = panel('bim.rooms', 'bim.panel.rooms', [
  pending('bim.room', 'bim.item.room', 'area', true),
  pending('bim.area', 'bim.item.area', 'area'),
]);

const DATUM = panel('bim.datum', 'bim.panel.datum', [
  pending('bim.level', 'bim.item.level', 'viewTop', true),
  pending('bim.grid', 'bim.item.grid', 'grid', true),
  pending('bim.reference_plane', 'bim.item.reference_plane', 'axes'),
]);

// Revit's contextual "Modify" tab is a selection-time surface we do not model;
// the real generic editing commands keep a permanent panel on this tab instead
// of a fake contextual tab.
const MODIFY = panel('bim.modify', 'bim.panel.modify', [
  { id: 'bim.move', labelKey: 'draw.kind.move', icon: 'move', command: 'MOVE' },
  { id: 'bim.copy', labelKey: 'draw.kind.copy', icon: 'copy', command: 'COPY' },
  { id: 'bim.rotate', labelKey: 'draw.kind.rotate', icon: 'rotate', command: 'ROTATE' },
  { id: 'bim.mirror', labelKey: 'draw.kind.mirror', icon: 'mirror', command: 'MIRROR' },
  { id: 'bim.arrayrect', labelKey: 'draw.kind.arrayrect', icon: 'arrayrect', command: 'ARRAYRECT' },
  { id: 'bim.align', labelKey: 'draw.kind.align', icon: 'align', command: 'ALIGN' },
  { id: 'bim.trim', labelKey: 'draw.kind.trim', icon: 'trim', command: 'TRIM' },
  { id: 'bim.offset', labelKey: 'draw.kind.offset', icon: 'offset', command: 'OFFSET' },
  { id: 'bim.erase', labelKey: 'draw.kind.erase', icon: 'erase', command: 'ERASE' },
  { id: 'bim.matchprop', labelKey: 'draw.kind.matchprop', icon: 'matchprop', command: 'MATCHPROP' },
  // Real solid operations stand in for Revit's Join/Cut Geometry: they are the
  // kernel's own boolean tools, not a claim that BIM joins are modelled.
  { id: 'bim.union', labelKey: 'draw.kind.union', icon: 'dot', command: 'UNION' },
  { id: 'bim.subtract', labelKey: 'draw.kind.subtract', icon: 'dot', command: 'SUBTRACT' },
  { id: 'bim.intersect', labelKey: 'draw.kind.intersect', icon: 'dot', command: 'INTERSECT' },
  { id: 'bim.slice', labelKey: 'draw.kind.slice', icon: 'trim', command: 'SLICE' },
]);

// --- Structure ------------------------------------------------------------

const FRAMING = panel('bim.framing', 'bim.panel.framing', [
  pending('bim.beam', 'bim.item.beam', 'line', true),
  pending('bim.brace', 'bim.item.brace', 'align', true),
  pending('bim.column', 'bim.item.column', 'axes', true),
  pending('bim.truss', 'bim.item.truss', 'polyline'),
]);

const FOUNDATION = panel('bim.foundation', 'bim.panel.foundation', [
  pending('bim.isolated_foundation', 'bim.item.isolated_foundation', 'rectang'),
  pending('bim.wall_foundation', 'bim.item.wall_foundation', 'polyline'),
  pending('bim.pile_foundation', 'bim.item.pile_foundation', 'dot'),
]);

const REINFORCEMENT = panel('bim.reinforcement', 'bim.panel.reinforcement', [
  pending('bim.rebar', 'bim.item.rebar', 'line', true),
  pending('bim.rebar_set', 'bim.item.rebar_set', 'arrayrect'),
  pending('bim.cover', 'bim.item.cover', 'offset'),
]);

const CONNECTIONS = panel('bim.connections', 'bim.panel.connections', [
  pending('bim.connection', 'bim.item.connection', 'dot', true),
  pending('bim.modify_connection', 'bim.item.modify_connection', 'settings'),
]);

// --- Insert ---------------------------------------------------------------

const LINK = panel('bim.link', 'bim.panel.link', [
  pending('bim.link_revit', 'bim.item.link_revit', 'reference', true),
  pending('bim.link_ifc', 'bim.item.link_ifc', 'bim', true),
  { id: 'bim.link_cad', labelKey: 'bim.item.link_cad', icon: 'open', command: 'XATTACH' },
]);

const IMPORT = panel('bim.import', 'bim.panel.import', [
  { id: 'bim.import_pdf', labelKey: 'bim.item.import_pdf', icon: 'open', command: 'PDFIMPORT' },
  { id: 'bim.import_image', labelKey: 'bim.item.import_image', icon: 'reference', command: 'IMAGEATTACH' },
  { id: 'bim.attach_dgn', labelKey: 'draw.kind.dgnattach', icon: 'reference', command: 'DGNATTACH' },
]);

const FAMILY = panel('bim.family', 'bim.panel.family', [
  { id: 'bim.block', labelKey: 'draw.kind.blockdefine', icon: 'copy', command: 'BLOCK', large: true },
  { id: 'bim.insert', labelKey: 'draw.kind.insertblock', icon: 'copy', command: 'INSERT', large: true },
  { id: 'bim.bedit', labelKey: 'draw.kind.bedit', icon: 'pencil', command: 'BEDIT' },
  { id: 'bim.wblock', labelKey: 'draw.kind.wblock', icon: 'save', command: 'WBLOCK' },
]);

const LIBRARY = panel('bim.library', 'bim.panel.library', [
  pending('bim.load_family', 'bim.item.load_family', 'open', true),
  pending('bim.load_family_type', 'bim.item.load_family_type', 'copy'),
]);

// --- Annotate -------------------------------------------------------------

const DIMENSION = panel('bim.dimension', 'bim.panel.dimension', [
  { id: 'bim.dimlinear', labelKey: 'draw.kind.dimlinear', icon: 'dimlinear', command: 'DIMLINEAR', large: true },
  { id: 'bim.dimaligned', labelKey: 'draw.kind.dimaligned', icon: 'dimaligned', command: 'DIMALIGNED' },
  { id: 'bim.dimangular', labelKey: 'draw.kind.dimangular', icon: 'dimangular', command: 'DIMANGULAR' },
  { id: 'bim.dimradius', labelKey: 'draw.kind.dimradius', icon: 'dimradius', command: 'DIMRADIUS' },
  { id: 'bim.dimbaseline', labelKey: 'draw.kind.dimbaseline', icon: 'dimbaseline', command: 'DIMBASELINE' },
  { id: 'bim.dimcontinue', labelKey: 'draw.kind.dimcontinue', icon: 'dimcontinue', command: 'DIMCONTINUE' },
]);

const TEXT = panel('bim.text', 'bim.panel.text', [
  { id: 'bim.text', labelKey: 'draw.kind.text', icon: 'text', command: 'TEXT', large: true },
  { id: 'bim.mtext', labelKey: 'draw.kind.mtext', icon: 'mtext', command: 'MTEXT', large: true },
]);

const TAG = panel('bim.tag', 'bim.panel.tag', [
  pending('bim.tag', 'bim.item.tag', 'filter', true),
  pending('bim.material_tag', 'bim.item.material_tag', 'hatch'),
  { id: 'bim.mleader', labelKey: 'draw.kind.mleader', icon: 'mleader', command: 'MLEADER' },
  { id: 'bim.qleader', labelKey: 'draw.kind.qleader', icon: 'pencil', command: 'QLEADER' },
]);

const DETAIL = panel('bim.detail', 'bim.panel.detail', [
  { id: 'bim.detail_line', labelKey: 'draw.kind.line', icon: 'line', command: 'LINE', large: true },
  { id: 'bim.hatch', labelKey: 'draw.kind.hatch', icon: 'hatch', command: 'HATCH', large: true },
  { id: 'bim.revcloud', labelKey: 'draw.kind.revcloud', icon: 'polyline', command: 'REVCLOUD' },
]);

// --- Analyze --------------------------------------------------------------

const SCHEDULES = panel('bim.schedules', 'bim.panel.schedules', [
  { id: 'bim.browse', labelKey: 'bim.panel', icon: 'bim', action: BIM_ACTION, large: true },
  pending('bim.schedule', 'bim.item.schedule', 'properties', true),
]);

const MEASURE = panel('bim.measure', 'bim.panel.measure', [
  { id: 'bim.dist', labelKey: 'measure.kind.distance', icon: 'distance', command: 'DIST' },
  { id: 'bim.area_measure', labelKey: 'measure.kind.area', icon: 'area', command: 'AREA' },
  { id: 'bim.measuregeom', labelKey: 'measure.kind.measuregeom', icon: 'angle', command: 'MEASUREGEOM' },
]);

const SPACES = panel('bim.spaces', 'bim.panel.spaces', [
  pending('bim.space', 'bim.item.space', 'area', true),
  pending('bim.zone', 'bim.item.zone', 'filter'),
]);

// --- Massing & Site -------------------------------------------------------

const MASSING = panel('bim.massing', 'bim.panel.massing', [
  { id: 'bim.extrude', labelKey: 'draw.kind.extrude', icon: 'pushPull', command: 'EXTRUDE', large: true },
  { id: 'bim.revolve', labelKey: 'draw.kind.revolve', icon: 'rotate', command: 'REVOLVE' },
  { id: 'bim.sweep', labelKey: 'draw.kind.sweep', icon: 'spline', command: 'SWEEP' },
  { id: 'bim.loft', labelKey: 'draw.kind.loft', icon: 'polyline', command: 'LOFT' },
  pending('bim.mass_insitu', 'bim.item.mass_insitu', 'bim'),
]);

const SITE = panel('bim.site', 'bim.panel.site', [
  pending('bim.toposurface', 'bim.item.toposurface', 'drawing', true),
  pending('bim.building_pad', 'bim.item.building_pad', 'rectang'),
  pending('bim.site_split', 'bim.item.site_split', 'explode'),
]);

// --- Collaborate ----------------------------------------------------------

const COORDINATION = panel('bim.coordination', 'bim.panel.coordination', [
  pending('bim.copy_monitor', 'bim.item.copy_monitor', 'copy'),
  pending('bim.coordination_review', 'bim.item.coordination_review', 'check'),
]);

const WORKSHARING = panel('bim.worksharing', 'bim.panel.worksharing', [
  pending('bim.worksets', 'bim.item.worksets', 'layers', true),
  pending('bim.sync', 'bim.item.sync', 'refresh', true),
  pending('bim.reload_latest', 'bim.item.reload_latest', 'open'),
]);

const LINK_MODELS = panel('bim.link_models', 'bim.panel.link_models', [
  pending('bim.link_revit_model', 'bim.item.link_revit_model', 'reference', true),
  pending('bim.link_ifc_model', 'bim.item.link_ifc_model', 'bim', true),
]);

// --- View -----------------------------------------------------------------

const GRAPHICS = panel('bim.graphics', 'bim.panel.graphics', [
  { id: 'bim.visual_style', labelKey: 'draw.kind.vscurrent', icon: 'view', command: 'VSCURRENT', large: true },
  { id: 'bim.projection', labelKey: 'view.projection', icon: 'axes', command: 'PROJECTION' },
  { id: 'bim.grid_toggle', labelKey: 'overlay.grid', icon: 'grid', command: 'GRID' },
  { id: 'bim.clean_screen', labelKey: 'status.clean_screen', icon: 'cleanScreen', action: SHELL_ACTIONS.cleanScreen },
]);

const NAVIGATE = panel('bim.navigate', 'bim.panel.navigate', [
  { id: 'bim.orbit', labelKey: 'view.orbit_label', icon: 'orbit', command: 'ORBIT', large: true },
  { id: 'bim.pan', labelKey: 'toolbar.pan', icon: 'pan', command: 'PAN' },
  { id: 'bim.zoomin', labelKey: 'toolbar.zoom_in', icon: 'zoomIn', command: 'ZOOMIN' },
  { id: 'bim.zoomout', labelKey: 'toolbar.zoom_out', icon: 'zoomOut', command: 'ZOOMOUT' },
  { id: 'bim.fit', labelKey: 'toolbar.zoom_extents', icon: 'fit', command: 'FIT' },
  { id: 'bim.viewback', labelKey: 'toolbar.zoom_previous', icon: 'zoomPrevious', command: 'VIEWBACK' },
  { id: 'bim.switch2d3d', labelKey: 'ribbon.command.view_switch2d3d', icon: 'view', command: 'SWITCH2D3D' },
]);

const STANDARD = panel('bim.standard_views', 'bim.panel.standard_views', standardViewItems());

const WINDOWS = panel('bim.windows', 'bim.panel.windows', [
  { id: 'bim.vports', labelKey: 'draw.kind.vports', icon: 'rectang', command: 'VPORTS', large: true },
]);

// --- Manage ---------------------------------------------------------------

const PARAMETERS = panel('bim.parameters', 'bim.panel.parameters', [
  { id: 'bim.parameters', labelKey: 'parameters.open', icon: 'properties', action: SHELL_ACTIONS.parameters, large: true },
  { id: 'bim.properties', labelKey: 'properties.panel', icon: 'properties', action: SHELL_ACTIONS.properties, large: true },
]);

const PROJECT = panel('bim.project', 'bim.panel.project', [
  { id: 'bim.fileinfo', labelKey: 'fileinfo.title', icon: 'info', action: SHELL_ACTIONS.fileinfo },
  { id: 'bim.purge', labelKey: 'draw.kind.purge', icon: 'erase', command: 'PURGE' },
  { id: 'bim.audit', labelKey: 'audit.title', icon: 'diagnostics', command: 'AUDIT' },
  { id: 'bim.status', labelKey: 'draw.kind.status', icon: 'info', command: 'STATUS' },
]);

const SETTINGS = panel('bim.settings', 'bim.panel.settings', [
  { id: 'bim.options', labelKey: 'file.options', icon: 'settings', action: SHELL_ACTIONS.options, large: true },
  { id: 'bim.dimstyle', labelKey: 'draw.kind.dimstyle', icon: 'dimlinear', command: 'DIMSTYLE' },
  { id: 'bim.sysvdlg', labelKey: 'draw.kind.sysvdlg', icon: 'terminal', command: 'SYSVDLG' },
]);

const INQUIRY = panel('bim.inquiry', 'bim.panel.inquiry', [
  { id: 'bim.layers', labelKey: 'layers.panel', icon: 'layers', command: 'LAYER' },
  { id: 'bim.block_list', labelKey: 'draw.kind.block_list', icon: 'properties', command: 'BLOCK?' },
]);

/** The nine Revit-derived tabs, in the reference order. */
export const BIM_RIBBON_TABS: readonly RibbonTab[] = Object.freeze([
  tab('bim.architecture', 'bim.tab.architecture', [BUILD, ROOMS, DATUM, MODIFY]),
  tab('bim.structure', 'bim.tab.structure', [FRAMING, FOUNDATION, REINFORCEMENT, CONNECTIONS]),
  tab('bim.insert', 'bim.tab.insert', [LINK, IMPORT, FAMILY, LIBRARY]),
  tab('bim.annotate', 'bim.tab.annotate', [DIMENSION, TEXT, TAG, DETAIL]),
  tab('bim.analyze', 'bim.tab.analyze', [SCHEDULES, MEASURE, SPACES]),
  tab('bim.massing', 'bim.tab.massing', [MASSING, SITE]),
  tab('bim.collaborate', 'bim.tab.collaborate', [COORDINATION, WORKSHARING, LINK_MODELS]),
  tab('bim.view', 'bim.tab.view', [GRAPHICS, NAVIGATE, STANDARD, WINDOWS]),
  tab('bim.manage', 'bim.tab.manage', [PARAMETERS, PROJECT, SETTINGS, INQUIRY]),
]);
