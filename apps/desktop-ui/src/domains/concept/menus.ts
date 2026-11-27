// SketchUp-derived top menu model (clean-room transcription of the official
// menu names and the commands the official Help pages document).
//
// The rows are the same `RibbonTab`/`RibbonPanel`/`RibbonItem` the drafting
// ribbon uses; `RibbonMenuBar` renders them as a real menubar. A command that
// exists in this build keeps its stable token; a reference tool with no kernel
// operation is a `RibbonPendingItem` that names the missing capability instead
// of pretending to run. No SketchUp icon, screenshot or help prose is copied:
// labels are our own catalog strings and glyphs are our own line icons.
import type { RibbonItem, RibbonPanel, RibbonTab } from '../../ribbon/ribbon-model.ts';
import type { RibbonPendingItem } from '../../ribbon/ribbon-model.ts';
import { STANDARD_VIEWS } from '../../cad/view3d.ts';
import { CIRCLE_ITEM, POLYGON_ITEM } from '../../ribbon/variants.ts';
import { DOMAIN_PANEL } from '../../ribbon/domain-entry.ts';
import { SHELL_ACTIONS } from '../../ribbon/actions.ts';

function panel(id: string, titleKey: string, items: readonly RibbonItem[]): RibbonPanel {
  return Object.freeze({ id, titleKey, items: Object.freeze([...items]) });
}

function tab(id: string, labelKey: string, panels: readonly RibbonPanel[]): RibbonTab {
  return Object.freeze({ id, labelKey, panels: Object.freeze([...panels]) });
}

/** A reference tool this build cannot run; the row says so and is disabled. */
function pending(id: string, labelKey: string, icon: string, pendingKey = 'concept.pending.kernel'): RibbonPendingItem {
  return { id, labelKey, icon, pendingKey };
}

/** The seven standard views, shared by View and Camera (same tokens as the band). */
export function standardViewItems(): readonly RibbonItem[] {
  return STANDARD_VIEWS.map(entry => ({ ...entry }));
}

const FILE = panel('concept.file', 'concept.menu.file', [
  { id: 'NEW', labelKey: 'qat.new', icon: 'new', command: 'NEW' },
  { id: 'OPEN', labelKey: 'file.open', icon: 'open', command: 'OPEN' },
  { id: 'SAVE', labelKey: 'qat.save', icon: 'save', command: 'SAVE' },
  { id: 'EXPORT', labelKey: 'file.export', icon: 'export', command: 'EXPORT' },
  { id: 'PLOT', labelKey: 'file.plot', icon: 'printer', command: 'PLOT' },
  { id: 'CLOSE', labelKey: 'doctabs.close', icon: 'close', command: 'CLOSE' },
]);

const EDIT_BASIC = panel('concept.edit.basic', 'concept.section.basic', [
  { id: 'UNDO', labelKey: 'menu.edit.undo', icon: 'undo', command: 'UNDO' },
  { id: 'REDO', labelKey: 'menu.edit.redo', icon: 'redo', command: 'REDO' },
  { id: 'CUT', labelKey: 'menu.clipboard.cut', icon: 'close', command: 'CUTCLIP' },
  { id: 'COPY', labelKey: 'menu.clipboard.copy', icon: 'copy', command: 'COPYCLIP' },
  { id: 'PASTE', labelKey: 'menu.clipboard.paste', icon: 'copy', command: 'PASTECLIP' },
  { id: 'ERASE', labelKey: 'draw.kind.erase', icon: 'erase', command: 'ERASE' },
]);

const EDIT_SELECTION = panel('concept.edit.selection', 'concept.section.selection', [
  { id: 'SELECTALL', labelKey: 'selection.all', icon: 'selectAll', command: 'SELECTALL' },
]);

const EDIT_GROUPS = panel('concept.edit.groups', 'concept.section.group', [
  { id: 'GROUP', labelKey: 'draw.kind.group', icon: 'copy', command: 'GROUP' },
  { id: 'UNGROUP', labelKey: 'draw.kind.ungroup', icon: 'explode', command: 'UNGROUP' },
  { id: 'EXPLODE', labelKey: 'draw.kind.explode', icon: 'explode', command: 'EXPLODE' },
]);

const VIEW_INTERFACE = panel('concept.view.interface', 'concept.section.interface', [
  { id: 'TOOLBAR', labelKey: 'draw.kind.toolbar', icon: 'menu', command: 'TOOLBAR' },
  { id: 'GRID', labelKey: 'overlay.grid', icon: 'grid', command: 'GRID' },
  { id: 'UCSICON', labelKey: 'draw.kind.ucs', icon: 'axes', command: 'UCS' },
]);

const VIEW_STANDARD = panel('concept.view.standard', 'concept.section.standard_views', standardViewItems());

const VIEW_DISPLAY = panel('concept.view.display', 'concept.section.display', [
  pending('SECTIONPLANES', 'concept.tool.section_plane', 'view'),
  pending('HIDDENGEOMETRY', 'concept.tool.hidden_geometry', 'eyeOff'),
]);

const CAMERA_NAVIGATE = panel('concept.camera.navigate', 'concept.section.navigate', [
  { id: 'ORBIT', labelKey: 'view.orbit_label', icon: 'orbit', command: 'ORBIT' },
  { id: 'PAN', labelKey: 'toolbar.pan', icon: 'pan', command: 'PAN' },
  { id: 'ZOOMIN', labelKey: 'toolbar.zoom_in', icon: 'zoomIn', command: 'ZOOMIN' },
  { id: 'ZOOMOUT', labelKey: 'toolbar.zoom_out', icon: 'zoomOut', command: 'ZOOMOUT' },
  { id: 'FIT', labelKey: 'toolbar.zoom_extents', icon: 'fit', command: 'FIT' },
  { id: 'VIEWBACK', labelKey: 'toolbar.zoom_previous', icon: 'zoomPrevious', command: 'VIEWBACK' },
]);

const CAMERA_MORE = panel('concept.camera.more', 'concept.section.other_cameras', [
  pending('CAMERA', 'concept.tool.position_camera', 'view'),
  pending('WALK', 'concept.tool.walk', 'pan'),
  pending('LOOKAROUND', 'concept.tool.look_around', 'eye'),
  pending('MATCHPHOTO', 'concept.tool.match_photo', 'reference'),
]);

const DRAW_LINES = panel('concept.draw.lines', 'concept.section.lines', [
  { id: 'LINE', labelKey: 'draw.kind.line', icon: 'line', command: 'LINE' },
  { id: 'FREEHAND', labelKey: 'concept.tool.freehand', icon: 'spline', command: 'SKETCH', hintKey: 'concept.tool.freehand.hint' },
]);

const DRAW_SHAPES = panel('concept.draw.shapes', 'concept.section.shapes', [
  { id: 'RECTANG', labelKey: 'draw.kind.rectang', icon: 'rectang', command: 'RECTANG' },
  { id: 'RECTANGROTATED', labelKey: 'concept.tool.rotated_rectangle', icon: 'rectang', command: 'RECTANGROTATED', hintKey: 'draw.kind.rectangrotated.hint' },
  CIRCLE_ITEM,
  POLYGON_ITEM,
]);

const DRAW_ARCS = panel('concept.draw.arcs', 'concept.section.arcs', [
  { id: 'ARC', labelKey: 'draw.kind.arc', icon: 'arc', command: 'ARC' },
  pending('ARC2POINT', 'concept.tool.arc_2point', 'arc'),
  pending('ARC3POINT', 'concept.tool.arc_3point', 'arc'),
  { id: 'PIE', labelKey: 'concept.tool.pie', icon: 'circle', command: 'PIE', hintKey: 'draw.kind.pie.hint' },
]);

const TOOLS_SELECT = panel('concept.tools.select', 'concept.section.selection', [
  { id: 'SELECT', labelKey: 'concept.tool.select', icon: 'select', action: SHELL_ACTIONS.select, hintKey: 'concept.tool.select.hint' },
]);

const TOOLS_MODIFY = panel('concept.tools.modify', 'concept.section.modify', [
  { id: 'MOVE', labelKey: 'draw.kind.move', icon: 'move', command: 'MOVE' },
  { id: 'ROTATE', labelKey: 'draw.kind.rotate', icon: 'rotate', command: 'ROTATE' },
  { id: 'SCALE', labelKey: 'draw.kind.scale', icon: 'scale', command: 'SCALE' },
  { id: 'OFFSET', labelKey: 'draw.kind.offset', icon: 'offset', command: 'OFFSET' },
  { id: 'MIRROR', labelKey: 'draw.kind.mirror', icon: 'mirror', command: 'MIRROR' },
  { id: 'PRESSPULL', labelKey: 'concept.tool.push_pull', icon: 'pushPull', command: 'PRESSPULL', hintKey: 'concept.tool.push_pull.hint' },
]);

const TOOLS_MEASURE = panel('concept.tools.measure', 'concept.section.measure', [
  { id: 'DIST', labelKey: 'measure.kind.distance', icon: 'distance', command: 'DIST' },
  { id: 'MEASUREGEOM', labelKey: 'measure.kind.measuregeom', icon: 'angle', command: 'MEASUREGEOM' },
  { id: 'PROTRACTOR', labelKey: 'concept.tool.protractor', icon: 'angle', command: 'PROTRACTOR', hintKey: 'concept.tool.protractor.hint' },
]);

const TOOLS_ANNOTATE = panel('concept.tools.annotate', 'concept.section.annotate', [
  { id: 'TEXT', labelKey: 'draw.kind.text', icon: 'text', command: 'TEXT' },
  { id: 'TEXT3D', labelKey: 'concept.tool.text3d', icon: 'pushPull', command: 'TEXT3D', hintKey: 'concept.tool.text3d.hint' },
  { id: 'DIMLINEAR', labelKey: 'draw.kind.dimlinear', icon: 'dimlinear', command: 'DIMLINEAR' },
]);

const TOOLS_SOLID = panel('concept.tools.solid', 'concept.section.solid', [
  { id: 'UNION', labelKey: 'draw.kind.union', icon: 'dot', command: 'UNION' },
  { id: 'SUBTRACT', labelKey: 'draw.kind.subtract', icon: 'dot', command: 'SUBTRACT' },
  { id: 'INTERSECT', labelKey: 'draw.kind.intersect', icon: 'dot', command: 'INTERSECT' },
  { id: 'OUTERSHELL', labelKey: 'concept.tool.outer_shell', icon: 'dot', command: 'OUTERSHELL', hintKey: 'concept.tool.outer_shell.hint' },
  pending('TRIMSOLID', 'concept.tool.trim_solid', 'trim'),
  pending('SPLITSOLID', 'concept.tool.split_solid', 'explode'),
]);

const TOOLS_OTHER = panel('concept.tools.other', 'concept.section.other', [
  { id: 'FOLLOWME', labelKey: 'concept.tool.follow_me', icon: 'spline', command: 'FOLLOWME', hintKey: 'concept.tool.follow_me.hint' },
  { id: 'LAYER', labelKey: 'layers.panel', icon: 'layers', command: 'LAYER' },
]);

const WINDOW_INFO = panel('concept.window.info', 'concept.section.info', [
  { id: 'FILEINFO', labelKey: 'fileinfo.title', icon: 'info', action: SHELL_ACTIONS.fileinfo },
  { id: 'DIAGNOSTICS', labelKey: 'diagnostics.title', icon: 'diagnostics', action: SHELL_ACTIONS.diagnostics },
]);

const WINDOW_PREFERENCES = panel('concept.window.preferences', 'concept.section.preferences', [
  // SketchUp's Window > Default Tray owns the tray's visibility. This chrome has
  // one tray slot for every profile, so the row toggles it (the tray's own
  // header runs the same `SHELL_ACTIONS.domainTray`).
  { id: 'DEFAULTTRAY', labelKey: 'concept.tool.default_tray', icon: 'palette', action: SHELL_ACTIONS.domainTray },
  { id: 'OPTIONS', labelKey: 'file.options', icon: 'settings', action: SHELL_ACTIONS.options },
  { id: 'PLUGINS', labelKey: 'file.plugins', icon: 'settings', action: SHELL_ACTIONS.plugins },
]);

const WINDOW_PANELS = panel('concept.window.panels', 'concept.section.panels', [
  { id: 'MATERIALS', labelKey: 'concept.tool.materials', icon: 'palette', command: 'MATERIALS', hintKey: 'concept.tool.materials.hint' },
  pending('OUTLINER', 'concept.tool.outliner', 'layers'),
]);

const EXTENSIONS = panel('concept.extensions.manage', 'concept.section.extensions', [
  { id: 'PLUGINS2', labelKey: 'concept.tool.customize', icon: 'settings', action: SHELL_ACTIONS.plugins },
  pending('EXTENSIONWAREHOUSE', 'concept.tool.extension_warehouse', 'open'),
]);

const HELP = panel('concept.help.about', 'concept.section.about', [
  { id: 'ABOUT', labelKey: 'file.about', icon: 'app', action: SHELL_ACTIONS.about },
  { id: 'DIAGNOSTICS2', labelKey: 'diagnostics.title', icon: 'diagnostics', action: SHELL_ACTIONS.diagnostics },
  pending('SHORTCUTS', 'concept.tool.shortcuts', 'terminal'),
]);

/** The nine official top-level menus, in the documented order. */
export const CONCEPT_MENUS: readonly RibbonTab[] = Object.freeze([
  tab('file', 'concept.menu.file', [FILE, DOMAIN_PANEL]),
  tab('edit', 'concept.menu.edit', [EDIT_BASIC, EDIT_SELECTION, EDIT_GROUPS]),
  tab('view', 'concept.menu.view', [VIEW_INTERFACE, VIEW_STANDARD, VIEW_DISPLAY]),
  tab('camera', 'concept.menu.camera', [CAMERA_NAVIGATE, VIEW_STANDARD, CAMERA_MORE]),
  tab('draw', 'concept.menu.draw', [DRAW_LINES, DRAW_SHAPES, DRAW_ARCS]),
  tab('tools', 'concept.menu.tools', [TOOLS_SELECT, TOOLS_MEASURE, TOOLS_MODIFY, TOOLS_ANNOTATE, TOOLS_SOLID, TOOLS_OTHER]),
  tab('window', 'concept.menu.window', [WINDOW_INFO, WINDOW_PREFERENCES, WINDOW_PANELS]),
  tab('extensions', 'concept.menu.extensions', [EXTENSIONS]),
  tab('help', 'concept.menu.help', [HELP]),
]);
