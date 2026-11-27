// SketchUp Large Tool Set, grouped the way the official "Toolbars" page groups
// the same tools (Getting Started, Drawing, Edit, Construction, Camera, Views,
// Solid Tools, Standard). The column is icon-only like the reference; each cell
// carries the shared command item and its own refusal sentence.
//
// Tools with no kernel operation remain labelled `pending`: they are visible so
// the reference layout is honest, but they never claim to run.
import type { RibbonItem, RibbonPanel } from '../../ribbon/ribbon-model.ts';
import type { RibbonPendingItem } from '../../ribbon/ribbon-model.ts';
import { STANDARD_VIEWS } from '../../cad/view3d.ts';
import { CIRCLE_ITEM, POLYGON_ITEM } from '../../ribbon/variants.ts';
import { SHELL_ACTIONS } from '../../ribbon/actions.ts';

/** The Select state's shared shell action (handled by `runItem`). */
export const CONCEPT_SELECT_ACTION = SHELL_ACTIONS.select;

function group(id: string, titleKey: string, items: readonly RibbonItem[]): RibbonPanel {
  return Object.freeze({ id, titleKey, items: Object.freeze([...items]) });
}

function pending(id: string, labelKey: string, icon: string): RibbonPendingItem {
  return { id, labelKey, icon, pendingKey: 'concept.pending.kernel' };
}

const SELECT: RibbonItem = {
  id: 'concept.select',
  labelKey: 'concept.tool.select',
  icon: 'select',
  hintKey: 'concept.tool.select.hint',
  action: CONCEPT_SELECT_ACTION,
};

const GETTING_STARTED = group('concept.getting_started', 'concept.toolbar.getting_started', [
  SELECT,
  { id: 'ERASE', labelKey: 'draw.kind.erase', icon: 'erase', command: 'ERASE' },
  { id: 'LINE', labelKey: 'draw.kind.line', icon: 'line', command: 'LINE' },
  { id: 'ARC', labelKey: 'draw.kind.arc', icon: 'arc', command: 'ARC' },
  { id: 'RECTANG', labelKey: 'draw.kind.rectang', icon: 'rectang', command: 'RECTANG' },
  { id: 'OFFSET', labelKey: 'draw.kind.offset', icon: 'offset', command: 'OFFSET' },
  { id: 'MOVE', labelKey: 'draw.kind.move', icon: 'move', command: 'MOVE' },
  { id: 'ROTATE', labelKey: 'draw.kind.rotate', icon: 'rotate', command: 'ROTATE' },
  { id: 'SCALE', labelKey: 'draw.kind.scale', icon: 'scale', command: 'SCALE' },
  { id: 'MIRROR', labelKey: 'draw.kind.mirror', icon: 'mirror', command: 'MIRROR' },
  { id: 'DIST', labelKey: 'measure.kind.distance', icon: 'distance', command: 'DIST' },
  { id: 'MATERIALS', labelKey: 'concept.tool.paint_bucket', icon: 'palette', command: 'MATERIALS', hintKey: 'concept.tool.paint_bucket.hint' },
  { id: 'ORBIT', labelKey: 'view.orbit_label', icon: 'orbit', command: 'ORBIT' },
  { id: 'PAN', labelKey: 'toolbar.pan', icon: 'pan', command: 'PAN' },
  { id: 'FIT', labelKey: 'toolbar.zoom_extents', icon: 'fit', command: 'FIT' },
]);

const DRAWING = group('concept.drawing', 'concept.toolbar.drawing', [
  { id: 'LINE', labelKey: 'draw.kind.line', icon: 'line', command: 'LINE' },
  { id: 'FREEHAND', labelKey: 'concept.tool.freehand', icon: 'spline', command: 'SKETCH', hintKey: 'concept.tool.freehand.hint' },
  { id: 'RECTANG', labelKey: 'draw.kind.rectang', icon: 'rectang', command: 'RECTANG' },
  { id: 'RECTANGROTATED', labelKey: 'concept.tool.rotated_rectangle', icon: 'rectang', command: 'RECTANGROTATED', hintKey: 'draw.kind.rectangrotated.hint' },
  CIRCLE_ITEM,
  POLYGON_ITEM,
  { id: 'ARC', labelKey: 'draw.kind.arc', icon: 'arc', command: 'ARC' },
  { id: 'PIE', labelKey: 'concept.tool.pie', icon: 'circle', command: 'PIE', hintKey: 'draw.kind.pie.hint' },
]);

const EDIT = group('concept.edit', 'concept.toolbar.edit', [
  { id: 'MOVE', labelKey: 'draw.kind.move', icon: 'move', command: 'MOVE' },
  { id: 'PRESSPULL', labelKey: 'concept.tool.push_pull', icon: 'pushPull', command: 'PRESSPULL', hintKey: 'concept.tool.push_pull.hint' },
  { id: 'ROTATE', labelKey: 'draw.kind.rotate', icon: 'rotate', command: 'ROTATE' },
  { id: 'FOLLOWME', labelKey: 'concept.tool.follow_me', icon: 'spline', command: 'FOLLOWME', hintKey: 'concept.tool.follow_me.hint' },
  { id: 'SCALE', labelKey: 'draw.kind.scale', icon: 'scale', command: 'SCALE' },
  { id: 'MIRROR', labelKey: 'draw.kind.mirror', icon: 'mirror', command: 'MIRROR' },
  { id: 'OFFSET', labelKey: 'draw.kind.offset', icon: 'offset', command: 'OFFSET' },
]);

const CONSTRUCTION = group('concept.construction', 'concept.toolbar.construction', [
  { id: 'DIST', labelKey: 'concept.tool.tape_measure', icon: 'distance', command: 'DIST' },
  { id: 'DIMLINEAR', labelKey: 'draw.kind.dimlinear', icon: 'dimlinear', command: 'DIMLINEAR' },
  { id: 'PROTRACTOR', labelKey: 'concept.tool.protractor', icon: 'angle', command: 'PROTRACTOR', hintKey: 'concept.tool.protractor.hint' },
  { id: 'TEXT', labelKey: 'draw.kind.text', icon: 'text', command: 'TEXT' },
  { id: 'UCS', labelKey: 'concept.tool.axes', icon: 'axes', command: 'UCS' },
  { id: 'TEXT3D', labelKey: 'concept.tool.text3d', icon: 'pushPull', command: 'TEXT3D', hintKey: 'concept.tool.text3d.hint' },
]);

const CAMERA = group('concept.camera', 'concept.toolbar.camera', [
  { id: 'ORBIT', labelKey: 'view.orbit_label', icon: 'orbit', command: 'ORBIT' },
  { id: 'PAN', labelKey: 'toolbar.pan', icon: 'pan', command: 'PAN' },
  { id: 'ZOOMIN', labelKey: 'toolbar.zoom_in', icon: 'zoomIn', command: 'ZOOMIN' },
  { id: 'ZOOMOUT', labelKey: 'toolbar.zoom_out', icon: 'zoomOut', command: 'ZOOMOUT' },
  { id: 'FIT', labelKey: 'toolbar.zoom_extents', icon: 'fit', command: 'FIT' },
  { id: 'VIEWBACK', labelKey: 'toolbar.zoom_previous', icon: 'zoomPrevious', command: 'VIEWBACK' },
  pending('CAMERA', 'concept.tool.position_camera', 'view'),
  pending('WALK', 'concept.tool.walk', 'pan'),
  pending('LOOKAROUND', 'concept.tool.look_around', 'eye'),
]);

const VIEWS = group('concept.views', 'concept.toolbar.views', STANDARD_VIEWS.map(entry => ({ ...entry })));

const SOLID = group('concept.solid', 'concept.toolbar.solid', [
  { id: 'UNION', labelKey: 'draw.kind.union', icon: 'dot', command: 'UNION' },
  { id: 'SUBTRACT', labelKey: 'draw.kind.subtract', icon: 'dot', command: 'SUBTRACT' },
  { id: 'INTERSECT', labelKey: 'draw.kind.intersect', icon: 'dot', command: 'INTERSECT' },
  pending('OUTERSHELL', 'concept.tool.outer_shell', 'dot'),
  pending('TRIMSOLID', 'concept.tool.trim_solid', 'trim'),
  pending('SPLITSOLID', 'concept.tool.split_solid', 'explode'),
]);

const STANDARD = group('concept.standard', 'concept.toolbar.standard', [
  { id: 'NEW', labelKey: 'qat.new', icon: 'new', command: 'NEW' },
  { id: 'OPEN', labelKey: 'file.open', icon: 'open', command: 'OPEN' },
  { id: 'SAVE', labelKey: 'qat.save', icon: 'save', command: 'SAVE' },
  { id: 'CUT', labelKey: 'menu.clipboard.cut', icon: 'close', command: 'CUTCLIP' },
  { id: 'COPY', labelKey: 'menu.clipboard.copy', icon: 'copy', command: 'COPYCLIP' },
  { id: 'PASTE', labelKey: 'menu.clipboard.paste', icon: 'copy', command: 'PASTECLIP' },
  { id: 'ERASE', labelKey: 'draw.kind.erase', icon: 'erase', command: 'ERASE' },
  { id: 'UNDO', labelKey: 'menu.edit.undo', icon: 'undo', command: 'UNDO' },
  { id: 'REDO', labelKey: 'menu.edit.redo', icon: 'redo', command: 'REDO' },
  { id: 'PLOT', labelKey: 'file.plot', icon: 'printer', command: 'PLOT' },
  { id: 'TOOLBAR', labelKey: 'draw.kind.toolbar', icon: 'menu', command: 'TOOLBAR' },
]);

/** The column, top to bottom: the official toolbar groups in documented order. */
export const CONCEPT_TOOLBAR: readonly RibbonPanel[] = Object.freeze([
  GETTING_STARTED,
  DRAWING,
  EDIT,
  CONSTRUCTION,
  CAMERA,
  VIEWS,
  SOLID,
  STANDARD,
]);
