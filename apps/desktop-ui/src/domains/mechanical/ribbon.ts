// The mechanical domain's CommandManager ribbon (SolidWorks-derived structure).
//
// SolidWorks' CommandManager is **contextual**: its tab set changes with the
// document type and the active command. This build has no document-type model
// (a `.ymodel` is one drawing database), so this module pins a **preliminary,
// fixed** subset: the part-editing tabs this kernel can actually reach, in the
// reference order — File, Sketch, Features, Assembly, Drawing, Evaluate, View,
// Options. The fixture in `test/fixtures/solidworks-2024-mechanical.json` is the
// contract for that subset; it is not a full transcription of the reference's
// contextual tabs.
//
// A tool is wired only when the kernel has it:
//
//   * the sketch/annotation/solid/edit commands this build really runs
//     (`LINE` … `EXTRUDE`, `CHAMFEREDGE`, `MASSPROP`, the typed view commands)
//     keep their stable token and therefore pass the one shared admission;
//   * revolve/sweep/loft/fillet-edge/hole/pattern/shell, sketch relations,
//     assembly mates and drawing views have **no kernel operation**; they are
//     `RibbonPendingItem`s with their own sentence, never clickable fake
//     buttons.
//
// No profile enables a command (ADR-0053 D5): a wired row is still disabled when
// the document or carrier cannot service it.
import type { RibbonItem, RibbonPanel, RibbonTab } from '../../ribbon/ribbon-model.ts';
import type { RibbonPendingItem } from '../../ribbon/ribbon-model.ts';
import { STANDARD_VIEWS } from '../../cad/view3d.ts';
import { DOMAIN_PANEL } from '../../ribbon/domain-entry.ts';
import { SHELL_ACTIONS } from '../../ribbon/actions.ts';

function panel(id: string, titleKey: string, items: readonly RibbonItem[]): RibbonPanel {
  return Object.freeze({ id, titleKey, items: Object.freeze([...items]) });
}

function tab(id: string, labelKey: string, panels: readonly RibbonPanel[]): RibbonTab {
  return Object.freeze({ id, labelKey, panels: Object.freeze([...panels]) });
}

/** A mechanical tool this build cannot run; the row says so and is disabled. */
function pending(
  id: string,
  labelKey: string,
  icon: string,
  pendingKey: string,
  large = false,
): RibbonPendingItem {
  return {
    id,
    labelKey,
    icon,
    pendingKey,
    ...(large ? { large: true } : {}),
  };
}

/** The shared "no 3D feature kernel operation" sentence. */
const NO_KERNEL = 'mechanical.pending.kernel';
/** The shared "no sketch constraint solver" sentence. */
const NO_CONSTRAINT = 'mechanical.pending.constraint';
/** The shared "no assembly model" sentence. */
const NO_ASSEMBLY = 'mechanical.pending.assembly';
/** The shared "no drawing/sheet model" sentence. */
const NO_DRAWING = 'mechanical.pending.drawing';
/** The shared "no material/appearance model" sentence. */
const NO_MATERIAL = 'mechanical.pending.material';

/** The seven standard views, shared with the drafting band (same typed IPC). */
function standardViewItems(): readonly RibbonItem[] {
  return STANDARD_VIEWS.map(entry => ({ ...entry }));
}

// --- File -----------------------------------------------------------------
//
// The same payload-free file commands the drafting File tab, the BIM File tab
// and the TitleBar QAT dispatch. `SAVE` always writes `.ymodel` and other
// formats go through `EXPORT` (ADR-0041/0052), so there is no Save As row.
const FILE = panel('mechanical.file.actions', 'mechanical.panel.file', [
  { id: 'mechanical.new', labelKey: 'qat.new', icon: 'new', command: 'NEW', large: true },
  { id: 'mechanical.open', labelKey: 'file.open', icon: 'open', command: 'OPEN', large: true },
  { id: 'mechanical.save', labelKey: 'qat.save', icon: 'save', command: 'SAVE', large: true },
  { id: 'mechanical.export', labelKey: 'file.export', icon: 'export', command: 'EXPORT', large: true },
  { id: 'mechanical.plot', labelKey: 'file.plot', icon: 'printer', command: 'PLOT' },
  { id: 'mechanical.close', labelKey: 'doctabs.close', icon: 'close', command: 'CLOSE' },
  { id: 'mechanical.file_options', labelKey: 'file.options', icon: 'settings', action: SHELL_ACTIONS.options },
]);

// --- Sketch ---------------------------------------------------------------

const SKETCH_DRAW = panel('mechanical.sketch.draw', 'mechanical.panel.sketch_draw', [
  { id: 'mechanical.line', labelKey: 'draw.kind.line', icon: 'line', command: 'LINE', large: true },
  { id: 'mechanical.rectang', labelKey: 'draw.kind.rectang', icon: 'rectang', command: 'RECTANG', large: true },
  { id: 'mechanical.circle', labelKey: 'draw.kind.circle', icon: 'circle', command: 'CIRCLE', large: true },
  { id: 'mechanical.arc', labelKey: 'draw.kind.arc', icon: 'arc', command: 'ARC' },
  { id: 'mechanical.polygon', labelKey: 'draw.kind.polygon', icon: 'polygon', command: 'POLYGON' },
  { id: 'mechanical.ellipse', labelKey: 'draw.kind.ellipse', icon: 'ellipse', command: 'ELLIPSE' },
  { id: 'mechanical.spline', labelKey: 'draw.kind.spline', icon: 'spline', command: 'SPLINE' },
  { id: 'mechanical.polyline', labelKey: 'draw.kind.polyline', icon: 'polyline', command: 'PLINE' },
  { id: 'mechanical.xline', labelKey: 'draw.kind.xline', icon: 'xline', command: 'XLINE' },
  pending('mechanical.slot', 'mechanical.item.slot', 'rectang', NO_KERNEL),
]);

const SKETCH_MODIFY = panel('mechanical.sketch.modify', 'mechanical.panel.sketch_modify', [
  { id: 'mechanical.trim', labelKey: 'draw.kind.trim', icon: 'trim', command: 'TRIM' },
  { id: 'mechanical.extend', labelKey: 'draw.kind.extend', icon: 'extend', command: 'EXTEND' },
  { id: 'mechanical.offset', labelKey: 'draw.kind.offset', icon: 'offset', command: 'OFFSET' },
  { id: 'mechanical.mirror', labelKey: 'draw.kind.mirror', icon: 'mirror', command: 'MIRROR' },
  { id: 'mechanical.move', labelKey: 'draw.kind.move', icon: 'move', command: 'MOVE' },
  { id: 'mechanical.rotate', labelKey: 'draw.kind.rotate', icon: 'rotate', command: 'ROTATE' },
  { id: 'mechanical.scale', labelKey: 'draw.kind.scale', icon: 'scale', command: 'SCALE' },
  { id: 'mechanical.copy', labelKey: 'draw.kind.copy', icon: 'copy', command: 'COPY' },
  { id: 'mechanical.erase', labelKey: 'draw.kind.erase', icon: 'erase', command: 'ERASE' },
]);

const SKETCH_RELATIONS = panel('mechanical.sketch.relations', 'mechanical.panel.sketch_relations', [
  pending('mechanical.smart_dimension', 'mechanical.item.smart_dimension', 'dimlinear', NO_CONSTRAINT, true),
  pending('mechanical.add_relation', 'mechanical.item.add_relation', 'align', NO_CONSTRAINT),
  pending('mechanical.pierce', 'mechanical.item.pierce', 'dot', NO_CONSTRAINT),
]);

// --- Features -------------------------------------------------------------

const FEATURE_BASE = panel('mechanical.feature.base', 'mechanical.panel.feature_base', [
  { id: 'mechanical.extrude', labelKey: 'draw.kind.extrude', icon: 'pushPull', command: 'EXTRUDE', hintKey: 'draw.kind.extrude.hint', large: true },
  pending('mechanical.revolve', 'draw.kind.revolve', 'rotate', NO_KERNEL, true),
  pending('mechanical.sweep', 'draw.kind.sweep', 'spline', NO_KERNEL),
  pending('mechanical.loft', 'draw.kind.loft', 'polyline', NO_KERNEL),
  pending('mechanical.thicken', 'draw.kind.thicken', 'offset', NO_KERNEL),
]);

const FEATURE_CUT = panel('mechanical.feature.cut', 'mechanical.panel.feature_cut', [
  { id: 'mechanical.subtract', labelKey: 'draw.kind.subtract', icon: 'dot', command: 'SUBTRACT', hintKey: 'draw.kind.subtract.hint', large: true },
  { id: 'mechanical.union', labelKey: 'draw.kind.union', icon: 'dot', command: 'UNION', hintKey: 'draw.kind.union.hint' },
  { id: 'mechanical.intersect', labelKey: 'draw.kind.intersect', icon: 'dot', command: 'INTERSECT', hintKey: 'draw.kind.intersect.hint' },
  { id: 'mechanical.presspull', labelKey: 'draw.kind.presspull', icon: 'move', command: 'PRESSPULL', hintKey: 'draw.kind.presspull.hint' },
  pending('mechanical.extrude_cut', 'mechanical.item.extrude_cut', 'trim', NO_KERNEL),
]);

const FEATURE_FILLET = panel('mechanical.feature.fillet', 'mechanical.panel.feature_fillet', [
  { id: 'mechanical.chamferedge', labelKey: 'draw.kind.chamferedge', icon: 'chamfer', command: 'CHAMFEREDGE', hintKey: 'draw.kind.chamferedge.hint', large: true },
  pending('mechanical.filletedge', 'mechanical.item.filletedge', 'fillet', NO_KERNEL, true),
]);

const FEATURE_PATTERN = panel('mechanical.feature.pattern', 'mechanical.panel.feature_pattern', [
  { id: 'mechanical.arrayrect', labelKey: 'draw.kind.arrayrect', icon: 'arrayrect', command: 'ARRAYRECT', large: true },
  { id: 'mechanical.arraypolar', labelKey: 'draw.kind.arraypolar', icon: 'arraypolar', command: 'ARRAYPOLAR' },
  pending('mechanical.mirror3d', 'mechanical.item.mirror3d', 'mirror', NO_KERNEL),
]);

const FEATURE_REFERENCE = panel('mechanical.feature.reference', 'mechanical.panel.feature_reference', [
  pending('mechanical.datum_plane', 'mechanical.item.datum_plane', 'axes', NO_KERNEL),
  pending('mechanical.datum_axis', 'mechanical.item.datum_axis', 'axes', NO_KERNEL),
  { id: 'mechanical.hatch', labelKey: 'draw.kind.hatch', icon: 'hatch', command: 'HATCH' },
]);

// --- Assembly -------------------------------------------------------------

const ASSEMBLY_COMPONENTS = panel('mechanical.assembly.components', 'mechanical.panel.assembly_components', [
  { id: 'mechanical.insert', labelKey: 'draw.kind.insertblock', icon: 'copy', command: 'INSERT', large: true },
  { id: 'mechanical.block', labelKey: 'draw.kind.blockdefine', icon: 'copy', command: 'BLOCK' },
  pending('mechanical.insert_component', 'mechanical.item.insert_component', 'open', NO_ASSEMBLY),
  pending('mechanical.new_part', 'mechanical.item.new_part', 'new', NO_ASSEMBLY),
]);

const ASSEMBLY_MATES = panel('mechanical.assembly.mates', 'mechanical.panel.assembly_mates', [
  pending('mechanical.mate', 'mechanical.item.mate', 'align', NO_ASSEMBLY, true),
  pending('mechanical.interference', 'mechanical.item.interference', 'check', NO_ASSEMBLY),
]);

const ASSEMBLY_MOVE = panel('mechanical.assembly.move', 'mechanical.panel.assembly_move', [
  { id: 'mechanical.assembly_move', labelKey: 'draw.kind.move', icon: 'move', command: 'MOVE' },
  { id: 'mechanical.assembly_rotate', labelKey: 'draw.kind.rotate', icon: 'rotate', command: 'ROTATE' },
  pending('mechanical.explode', 'mechanical.item.explode', 'explode', NO_ASSEMBLY),
]);

// --- Drawing --------------------------------------------------------------

const DRAWING_VIEWS = panel('mechanical.drawing.views', 'mechanical.panel.drawing_views', [
  pending('mechanical.model_view', 'mechanical.item.model_view', 'view', NO_DRAWING, true),
  pending('mechanical.projected_view', 'mechanical.item.projected_view', 'viewFront', NO_DRAWING),
  pending('mechanical.standard_3_view', 'mechanical.item.standard_3_view', 'viewTop', NO_DRAWING),
  { id: 'mechanical.vports', labelKey: 'draw.kind.vports', icon: 'rectang', command: 'VPORTS', hintKey: 'draw.kind.vports.hint' },
]);

const DRAWING_ANNOTATE = panel('mechanical.drawing.annotate', 'mechanical.panel.drawing_annotate', [
  { id: 'mechanical.dimlinear', labelKey: 'draw.kind.dimlinear', icon: 'dimlinear', command: 'DIMLINEAR', large: true },
  { id: 'mechanical.dimaligned', labelKey: 'draw.kind.dimaligned', icon: 'dimaligned', command: 'DIMALIGNED' },
  { id: 'mechanical.dimradius', labelKey: 'draw.kind.dimradius', icon: 'dimradius', command: 'DIMRADIUS' },
  { id: 'mechanical.text', labelKey: 'draw.kind.text', icon: 'text', command: 'TEXT' },
  { id: 'mechanical.mtext', labelKey: 'draw.kind.mtext', icon: 'mtext', command: 'MTEXT' },
  { id: 'mechanical.qleader', labelKey: 'draw.kind.qleader', icon: 'pencil', command: 'QLEADER' },
]);

// --- Evaluate -------------------------------------------------------------

const EVALUATE_MASS = panel('mechanical.evaluate.mass', 'mechanical.panel.evaluate_mass', [
  { id: 'mechanical.massprop', labelKey: 'draw.kind.massprop', icon: 'properties', command: 'MASSPROP', hintKey: 'draw.kind.massprop.hint', large: true },
  pending('mechanical.section_props', 'mechanical.item.section_props', 'area', NO_KERNEL),
]);

const EVALUATE_MEASURE = panel('mechanical.evaluate.measure', 'mechanical.panel.evaluate_measure', [
  { id: 'mechanical.dist', labelKey: 'measure.kind.distance', icon: 'distance', command: 'DIST' },
  { id: 'mechanical.area', labelKey: 'measure.kind.area', icon: 'area', command: 'AREA' },
  { id: 'mechanical.measuregeom', labelKey: 'measure.kind.measuregeom', icon: 'angle', command: 'MEASUREGEOM' },
]);

const EVALUATE_CHECK = panel('mechanical.evaluate.check', 'mechanical.panel.evaluate_check', [
  pending('mechanical.interference_check', 'mechanical.item.interference', 'check', NO_ASSEMBLY),
  pending('mechanical.draft_check', 'mechanical.item.draft_check', 'check', NO_KERNEL),
]);

// --- View -----------------------------------------------------------------

const VIEW_DISPLAY = panel('mechanical.view.display', 'mechanical.panel.view_display', [
  { id: 'mechanical.visual_style', labelKey: 'draw.kind.vscurrent', icon: 'view', command: 'VSCURRENT', large: true },
  { id: 'mechanical.projection', labelKey: 'view.projection', icon: 'axes', command: 'PROJECTION' },
  { id: 'mechanical.grid_toggle', labelKey: 'overlay.grid', icon: 'grid', command: 'GRID' },
  { id: 'mechanical.clean_screen', labelKey: 'status.clean_screen', icon: 'cleanScreen', action: SHELL_ACTIONS.cleanScreen },
  pending('mechanical.material', 'mechanical.item.material', 'palette', NO_MATERIAL),
]);

const VIEW_STANDARD = panel('mechanical.view.standard', 'mechanical.panel.view_standard', standardViewItems());

const VIEW_WINDOW = panel('mechanical.view.window', 'mechanical.panel.view_window', [
  { id: 'mechanical.orbit', labelKey: 'view.orbit_label', icon: 'orbit', command: 'ORBIT', large: true },
  { id: 'mechanical.pan', labelKey: 'toolbar.pan', icon: 'pan', command: 'PAN' },
  { id: 'mechanical.zoomin', labelKey: 'toolbar.zoom_in', icon: 'zoomIn', command: 'ZOOMIN' },
  { id: 'mechanical.zoomout', labelKey: 'toolbar.zoom_out', icon: 'zoomOut', command: 'ZOOMOUT' },
  { id: 'mechanical.fit', labelKey: 'toolbar.zoom_extents', icon: 'fit', command: 'FIT' },
  { id: 'mechanical.viewback', labelKey: 'toolbar.zoom_previous', icon: 'zoomPrevious', command: 'VIEWBACK' },
  { id: 'mechanical.switch2d3d', labelKey: 'ribbon.command.view_switch2d3d', icon: 'view', command: 'SWITCH2D3D' },
  // The FeatureManager tray's visibility lives where the reference's View menu
  // keeps its window controls; the tray header runs the same action.
  { id: 'mechanical.domain_tray', labelKey: 'domain.tray_toggle', icon: 'menu', action: SHELL_ACTIONS.domainTray },
]);

// --- Options --------------------------------------------------------------

const OPTIONS_APP = panel('mechanical.options.app', 'mechanical.panel.options_app', [
  { id: 'mechanical.options', labelKey: 'file.options', icon: 'settings', action: SHELL_ACTIONS.options, large: true },
  { id: 'mechanical.parameters', labelKey: 'parameters.open', icon: 'properties', action: SHELL_ACTIONS.parameters, large: true },
  { id: 'mechanical.properties', labelKey: 'properties.panel', icon: 'properties', action: SHELL_ACTIONS.properties },
  { id: 'mechanical.fileinfo', labelKey: 'fileinfo.title', icon: 'info', action: SHELL_ACTIONS.fileinfo },
]);

const OPTIONS_REFERENCES = panel('mechanical.options.references', 'mechanical.panel.options_references', [
  { id: 'mechanical.layers', labelKey: 'layers.panel', icon: 'layers', command: 'LAYER' },
  { id: 'mechanical.dimstyle', labelKey: 'draw.kind.dimstyle', icon: 'dimlinear', command: 'DIMSTYLE' },
  pending('mechanical.appearance', 'mechanical.item.appearance', 'palette', NO_MATERIAL),
]);

const OPTIONS_INQUIRY = panel('mechanical.options.inquiry', 'mechanical.panel.options_inquiry', [
  { id: 'mechanical.status', labelKey: 'draw.kind.status', icon: 'info', command: 'STATUS' },
  { id: 'mechanical.audit', labelKey: 'audit.title', icon: 'diagnostics', command: 'AUDIT' },
  { id: 'mechanical.purge', labelKey: 'draw.kind.purge', icon: 'erase', command: 'PURGE' },
]);

/** The eight fixed tabs, in the reference order (File first, accent). */
export const MECHANICAL_RIBBON_TABS: readonly RibbonTab[] = Object.freeze([
  { id: 'mechanical.file', labelKey: 'mechanical.tab.file', accent: true, panels: [FILE, DOMAIN_PANEL] },
  tab('mechanical.sketch', 'mechanical.tab.sketch', [SKETCH_DRAW, SKETCH_MODIFY, SKETCH_RELATIONS]),
  tab('mechanical.features', 'mechanical.tab.features', [FEATURE_BASE, FEATURE_CUT, FEATURE_FILLET, FEATURE_PATTERN, FEATURE_REFERENCE]),
  tab('mechanical.assembly', 'mechanical.tab.assembly', [ASSEMBLY_COMPONENTS, ASSEMBLY_MATES, ASSEMBLY_MOVE]),
  tab('mechanical.drawing', 'mechanical.tab.drawing', [DRAWING_VIEWS, DRAWING_ANNOTATE]),
  tab('mechanical.evaluate', 'mechanical.tab.evaluate', [EVALUATE_MASS, EVALUATE_MEASURE, EVALUATE_CHECK]),
  tab('mechanical.view', 'mechanical.tab.view', [VIEW_DISPLAY, VIEW_STANDARD, VIEW_WINDOW]),
  tab('mechanical.options', 'mechanical.tab.options', [OPTIONS_APP, OPTIONS_REFERENCES, OPTIONS_INQUIRY]),
]);
