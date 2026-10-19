// Declarative ribbon definition. Pure data (no React), so both the renderer and
// the tests can walk it. An item either carries a `command` (admitted through
// `cad_command`) or an `action` token (a DOM-only shell action); items with
// neither are labelled but disabled, which is the honest state for a command
// the host does not wire yet.

/** Shell actions that never touch the CAD protocol. */
export const SHELL_ACTIONS = Object.freeze({
  diagnostics: 'diagnostics',
  cleanScreen: 'cleanScreen',
  commandLine: 'commandLine',
  language: 'language',
});

export const RIBBON_TABS = Object.freeze([
  {
    id: 'file',
    labelKey: 'ribbon.file',
    accent: true,
    panels: [
      {
        id: 'file',
        titleKey: 'ribbon.file',
        items: [
          { id: 'OPEN', labelKey: 'file.open', icon: 'open', command: 'OPEN', large: true },
          { id: 'NEW', labelKey: 'qat.new', icon: 'new', command: 'NEW', large: true },
          { id: 'SAVE', labelKey: 'qat.save', icon: 'save', command: 'SAVE', large: true },
        ],
      },
      {
        id: 'utilities',
        titleKey: 'ribbon.panel.utilities',
        items: [
          { id: 'diagnostics', labelKey: 'diagnostics.title', icon: 'diagnostics', action: 'diagnostics' },
          { id: 'clean', labelKey: 'status.clean_screen', icon: 'cleanScreen', action: 'cleanScreen' },
          { id: 'cmdline', labelKey: 'status.command_line', icon: 'terminal', action: 'commandLine' },
        ],
      },
    ],
  },
  {
    id: 'home',
    labelKey: 'ribbon.home',
    panels: [
      {
        id: 'clipboard',
        titleKey: 'ribbon.panel.clipboard',
        items: [
          { id: 'SELECTALL', labelKey: 'selection.all', icon: 'selectAll', command: 'SELECTALL', large: true },
          { id: 'COPY', labelKey: 'draw.kind.copy', icon: 'copy', command: 'COPY' },
          { id: 'MOVE', labelKey: 'draw.kind.move', icon: 'move', command: 'MOVE' },
          { id: 'ROTATE', labelKey: 'draw.kind.rotate', icon: 'rotate', command: 'ROTATE' },
        ],
      },
      {
        id: 'draw',
        titleKey: 'ribbon.panel.draw',
        items: [
          { id: 'LINE', labelKey: 'draw.kind.line', icon: 'line', command: 'LINE', large: true },
          { id: 'CIRCLE', labelKey: 'draw.kind.circle', icon: 'circle', command: 'CIRCLE', large: true },
          { id: 'ARC', labelKey: 'draw.kind.arc', icon: 'arc', command: 'ARC' },
          { id: 'PLINE', labelKey: 'draw.kind.polyline', icon: 'polyline', command: 'PLINE' },
          { id: 'RECTANG', labelKey: 'draw.kind.rectang', icon: 'rectang', command: 'RECTANG' },
          { id: 'ELLIPSE', labelKey: 'draw.kind.ellipse', icon: 'ellipse', command: 'ELLIPSE' },
          { id: 'POLYGON', labelKey: 'draw.kind.polygon', icon: 'polygon', command: 'POLYGON' },
          { id: 'POINT', labelKey: 'draw.kind.point', icon: 'point', command: 'POINT' },
          { id: 'SPLINE', labelKey: 'draw.kind.spline', icon: 'spline', command: 'SPLINE' },
          { id: 'OFFSET', labelKey: 'draw.kind.offset', icon: 'offset', command: 'OFFSET' },
        ],
      },
      {
        id: 'modify',
        titleKey: 'ribbon.panel.modify',
        items: [
          { id: 'TRIM', labelKey: 'draw.kind.trim', icon: 'trim', command: 'TRIM', large: true },
          { id: 'ERASE', labelKey: 'draw.kind.erase', icon: 'erase', command: 'ERASE' },
          { id: 'EXPLODE', labelKey: 'draw.kind.explode', icon: 'explode', command: 'EXPLODE' },
          { id: 'EXTEND', labelKey: 'draw.kind.extend', icon: 'extend', command: 'EXTEND' },
          { id: 'CHAMFER', labelKey: 'draw.kind.chamfer', icon: 'chamfer', command: 'CHAMFER' },
          { id: 'FILLET', labelKey: 'draw.kind.fillet', icon: 'fillet', command: 'FILLET' },
          { id: 'ARRAYRECT', labelKey: 'draw.kind.arrayrect', icon: 'arrayrect', command: 'ARRAYRECT' },
          { id: 'ARRAYPOLAR', labelKey: 'draw.kind.arraypolar', icon: 'arraypolar', command: 'ARRAYPOLAR' },
          { id: 'ALIGN', labelKey: 'draw.kind.align', icon: 'align', command: 'ALIGN' },
          { id: 'STRETCH', labelKey: 'draw.kind.stretch', icon: 'stretch', command: 'STRETCH' },
          { id: 'MIRROR', labelKey: 'draw.kind.mirror', command: 'MIRROR', icon: 'mirror' },
          { id: 'SCALE', labelKey: 'draw.kind.scale', command: 'SCALE', icon: 'scale' },
        ],
      },
      {
        id: 'layers',
        titleKey: 'layers.panel',
        launcher: true,
        items: [
          { id: 'LAYER', labelKey: 'layers.panel', icon: 'layers', command: 'LAYER', large: true },
          { id: 'PROPERTIES', labelKey: 'properties.panel', icon: 'properties', command: 'PROPERTIES' },
        ],
      },
      {
        id: 'navigate',
        titleKey: 'ribbon.panel.navigate',
        items: [
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
        ],
      },
    ],
  },
  {
    id: 'draw',
    labelKey: 'ribbon.panel.draw',
    panels: [
      {
        id: 'draw',
        titleKey: 'ribbon.panel.draw',
        items: [
          { id: 'LINE', labelKey: 'draw.kind.line', icon: 'line', command: 'LINE', large: true },
          { id: 'CIRCLE', labelKey: 'draw.kind.circle', icon: 'circle', command: 'CIRCLE', large: true },
          { id: 'ARC', labelKey: 'draw.kind.arc', icon: 'arc', command: 'ARC', large: true },
          { id: 'PLINE', labelKey: 'draw.kind.polyline', icon: 'polyline', command: 'PLINE' },
          { id: 'RECTANG', labelKey: 'draw.kind.rectang', icon: 'rectang', command: 'RECTANG' },
          { id: 'ELLIPSE', labelKey: 'draw.kind.ellipse', icon: 'ellipse', command: 'ELLIPSE' },
          { id: 'POLYGON', labelKey: 'draw.kind.polygon', icon: 'polygon', command: 'POLYGON' },
          { id: 'POINT', labelKey: 'draw.kind.point', icon: 'point', command: 'POINT' },
          { id: 'SPLINE', labelKey: 'draw.kind.spline', icon: 'spline', command: 'SPLINE' },
          { id: 'OFFSET', labelKey: 'draw.kind.offset', icon: 'offset', command: 'OFFSET' },
        ],
      },
      {
        id: 'modify',
        titleKey: 'ribbon.panel.modify',
        launcher: true,
        items: [
          { id: 'ERASE', labelKey: 'draw.kind.erase', icon: 'erase', command: 'ERASE' },
          { id: 'EXPLODE', labelKey: 'draw.kind.explode', icon: 'explode', command: 'EXPLODE' },
          { id: 'MOVE', labelKey: 'draw.kind.move', icon: 'move', command: 'MOVE' },
          { id: 'COPY', labelKey: 'draw.kind.copy', icon: 'copy', command: 'COPY' },
          { id: 'TRIM', labelKey: 'draw.kind.trim', icon: 'trim', command: 'TRIM' },
          { id: 'EXTEND', labelKey: 'draw.kind.extend', icon: 'extend', command: 'EXTEND' },
          { id: 'CHAMFER', labelKey: 'draw.kind.chamfer', icon: 'chamfer', command: 'CHAMFER' },
          { id: 'FILLET', labelKey: 'draw.kind.fillet', icon: 'fillet', command: 'FILLET' },
          { id: 'ARRAYRECT', labelKey: 'draw.kind.arrayrect', icon: 'arrayrect', command: 'ARRAYRECT' },
          { id: 'ARRAYPOLAR', labelKey: 'draw.kind.arraypolar', icon: 'arraypolar', command: 'ARRAYPOLAR' },
          { id: 'ALIGN', labelKey: 'draw.kind.align', icon: 'align', command: 'ALIGN' },
          { id: 'STRETCH', labelKey: 'draw.kind.stretch', icon: 'stretch', command: 'STRETCH' },
          { id: 'ROTATE', labelKey: 'draw.kind.rotate', icon: 'rotate', command: 'ROTATE' },
          { id: 'SCALE', labelKey: 'draw.kind.scale', icon: 'scale', command: 'SCALE' },
          { id: 'MIRROR', labelKey: 'draw.kind.mirror', icon: 'mirror', command: 'MIRROR' },
        ],
      },
    ],
  },
  {
    id: 'measure',
    labelKey: 'ribbon.measure',
    panels: [
      {
        id: 'measure',
        titleKey: 'measure.panel',
        launcher: true,
        items: [
          { id: 'DIST', labelKey: 'measure.kind.distance', icon: 'distance', command: 'DIST', large: true },
          { id: 'PLINE', labelKey: 'measure.kind.polyline', icon: 'polyline', command: 'MEASUREPOLYLINE', large: true },
          { id: 'ANGLE', labelKey: 'measure.kind.angle', icon: 'angle', command: 'MEASUREANGLE', large: true },
          { id: 'AREA', labelKey: 'measure.kind.area', icon: 'area', command: 'AREA', large: true },
        ],
      },
    ],
  },
  {
    id: 'annotate',
    labelKey: 'ribbon.annotate',
    panels: [
      {
        id: 'annotation',
        titleKey: 'ribbon.panel.annotation',
        items: [
          // Dimension creation leads the annotate panel, the way AutoCAD's
          // Annotate tab puts its Dimensions panel first. Both are three-pick
          // captures (two extension-line origins, then the dimension-line
          // location); the linear/aligned pair stays together and in a fixed
          // order so the panel never reorders between sessions.
          { id: 'DIMLINEAR', labelKey: 'draw.kind.dimlinear', icon: 'dimlinear', command: 'DIMLINEAR', large: true },
          { id: 'DIMALIGNED', labelKey: 'draw.kind.dimaligned', icon: 'dimaligned', command: 'DIMALIGNED', large: true },
          { id: 'PROPERTIES', labelKey: 'properties.panel', icon: 'properties', command: 'PROPERTIES', large: true },
          { id: 'LAYER', labelKey: 'layers.panel', icon: 'layers', command: 'LAYER', large: true },
          { id: 'SELECTALL', labelKey: 'selection.all', icon: 'selectAll', command: 'SELECTALL' },
        ],
      },
    ],
  },
  {
    id: 'view',
    labelKey: 'ribbon.view',
    panels: [
      {
        id: 'navigate',
        titleKey: 'ribbon.panel.navigate',
        launcher: true,
        items: [
          {
            id: 'view',
            labelKey: 'ribbon.command.view_standard',
            icon: 'view',
            large: true,
            variants: [
              { id: 'viewTop', labelKey: 'view.standard.top', icon: 'view' },
              { id: 'viewFront', labelKey: 'view.standard.front', icon: 'view' },
              { id: 'viewIso', labelKey: 'view.standard.isometric', icon: 'view' },
            ],
          },
          { id: 'ZOOMIN', labelKey: 'toolbar.zoom_in', icon: 'zoomIn', command: 'ZOOMIN' },
          { id: 'ZOOMOUT', labelKey: 'toolbar.zoom_out', icon: 'zoomOut', command: 'ZOOMOUT' },
          { id: 'PAN', labelKey: 'toolbar.pan', icon: 'pan', command: 'PAN' },
          { id: 'FIT', labelKey: 'toolbar.fit', icon: 'fit', command: 'FIT' },
        ],
      },
      {
        id: 'display',
        titleKey: 'ribbon.panel.display',
        items: [
          { id: 'GRID', labelKey: 'overlay.grid', icon: 'grid', command: 'GRID' },
          { id: 'SNAP', labelKey: 'status.toggle.snap', icon: 'snap', command: 'SNAP' },
          { id: 'ORTHO', labelKey: 'status.toggle.ortho', icon: 'angle', command: 'ORTHO' },
        ],
      },
      {
        id: 'palettes',
        titleKey: 'ribbon.panel.utilities',
        items: [
          { id: 'diagnostics', labelKey: 'diagnostics.title', icon: 'diagnostics', action: 'diagnostics', large: true },
          { id: 'clean', labelKey: 'status.clean_screen', icon: 'cleanScreen', action: 'cleanScreen' },
        ],
      },
    ],
  },
]);

export function ribbonTab(id) {
  return RIBBON_TABS.find(tab => tab.id === id) ?? RIBBON_TABS[1];
}

/** All command ids referenced by the ribbon (used to keep the catalog honest). */
export function ribbonCommandIds() {
  const ids = new Set();
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      for (const item of panel.items) {
        if (item.command) ids.add(item.command);
        for (const variant of item.variants ?? []) if (variant.command) ids.add(variant.command);
      }
    }
  }
  return [...ids];
}
