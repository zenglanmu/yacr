// Declarative ribbon definition. Pure data (no React), so both the renderer and
// the tests can walk it. An item either carries a `command` (admitted through
// `cad_command`) or an `action` token (a DOM-only shell action); items with
// neither are labelled but disabled, which is the honest state for a command
// the host does not wire yet.
import { POPUP_KEYS } from './popup.mjs';

/** Shell actions that never touch the CAD protocol. */
export const SHELL_ACTIONS = Object.freeze({
  diagnostics: 'diagnostics',
  cleanScreen: 'cleanScreen',
  commandLine: 'commandLine',
  language: 'language',
  about: 'about',
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
          // About opens a host-owned dialog window (ADR 0021 D1), so it is an
          // `action` token, not a router command: nothing about it goes through
          // `cad_command`. It closes the File tab's utility cluster, where the
          // other chrome-level affordances live.
          { id: 'about', labelKey: POPUP_KEYS.ribbonEntry, icon: 'app', action: 'about' },
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
          // Single-line TEXT (ADR 0019 decision A). It is a small button next to
          // OFFSET, before HATCH: like HATCH it is not a primary creation tool,
          // and it has no variant popover — the command itself opens the
          // command-line text step (there is no in-place canvas editor here).
          { id: 'TEXT', labelKey: 'draw.kind.text', icon: 'text', command: 'TEXT' },
          // MLEADER (ADR 0019 decision F.2) is the other text-carrying
          // annotation tool, so it sits directly beside TEXT and before HATCH. It
          // captures exactly two points — the arrowhead position, then the
          // landing point — and then the same terminal text step TEXT uses. It is
          // a small button with **no variant popover on purpose**: no `Options`
          // sub-prompt exists (leader type / content type / max points / angular
          // constraint), and a caret here would imply choices the command does
          // not have.
          { id: 'MLEADER', labelKey: 'draw.kind.mleader', icon: 'mleader', command: 'MLEADER' },
          // HATCH closes the draw group, the way AutoCAD's Draw panel puts Hatch
          // at its end. It is a small button: unlike LINE/CIRCLE it is not a
          // primary creation tool, and it has no variant popover (the unimplemented
          // pattern-name/gradient forms must not appear as menu entries).
          { id: 'HATCH', labelKey: 'draw.kind.hatch', icon: 'hatch', command: 'HATCH' },
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
          // Same draw-group tail on the Draw tab: TEXT, then MLEADER next to it,
          // then Hatch last — the small-button decisions match the Home panel.
          { id: 'TEXT', labelKey: 'draw.kind.text', icon: 'text', command: 'TEXT' },
          { id: 'MLEADER', labelKey: 'draw.kind.mleader', icon: 'mleader', command: 'MLEADER' },
          { id: 'HATCH', labelKey: 'draw.kind.hatch', icon: 'hatch', command: 'HATCH' },
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
          // Annotate tab puts its Dimensions panel first. The linear/aligned pair
          // are three-pick captures (two extension-line origins, then the
          // dimension-line location); the radial pair are two-pick captures (an
          // arc/circle entity pick, then the dimension-line location); the centre
          // mark is a single arc/circle pick producing plain geometry; the angular
          // dimension is a two-line capture (two straight-line entity picks, then
          // the arc location that selects which angle is measured) and the
          // ordinate dimension is a two-point capture (a feature point, then the
          // leader endpoint).
          // The whole family stays together in one fixed order — linear, aligned,
          // radius, diameter, centre mark, angular, ordinate — so the panel never
          // reorders between sessions and no already-shipped button moves: the
          // angular/ordinate pair is appended in AutoCAD's own angular-before-
          // ordinate sequence instead of slotting between aligned and radius.
          //
          // DIMANGULAR is offered here as the **two-line** form only. The
          // three-point form exists at command level but has no interactive
          // capture, so no part of this button implies a vertex-first workflow.
          //
          // Each command carries its own glyph in the shared 24px stroke language
          // (`dimlinear`/`dimaligned`/`dimradius`/`dimdiameter`/`dimcenter`/
          // `dimangular`/`dimordinate`/`dimbaseline`/`dimcontinue`), so the family
          // reads as one set: the radial pair draws its arc/circle target with the
          // linear pair's dimension-line + arrowhead vocabulary, the centre mark
          // shows the crossed mark at the circle's centre, the angular glyph adds
          // dimension arrowheads to the measure-angle rays-plus-arc, and the
          // ordinate glyph is a feature point whose leader ends in a dimension
          // tick.
          { id: 'DIMLINEAR', labelKey: 'draw.kind.dimlinear', icon: 'dimlinear', command: 'DIMLINEAR', large: true },
          { id: 'DIMALIGNED', labelKey: 'draw.kind.dimaligned', icon: 'dimaligned', command: 'DIMALIGNED', large: true },
          { id: 'DIMRADIUS', labelKey: 'draw.kind.dimradius', icon: 'dimradius', command: 'DIMRADIUS', large: true },
          { id: 'DIMDIAMETER', labelKey: 'draw.kind.dimdiameter', icon: 'dimdiameter', command: 'DIMDIAMETER', large: true },
          { id: 'DIMCENTER', labelKey: 'draw.kind.dimcenter', icon: 'dimcenter', command: 'DIMCENTER', large: true },
          { id: 'DIMANGULAR', labelKey: 'draw.kind.dimangular', icon: 'dimangular', command: 'DIMANGULAR', large: true },
          { id: 'DIMORDINATE', labelKey: 'draw.kind.dimordinate', icon: 'dimordinate', command: 'DIMORDINATE', large: true },
          // Baseline/continue (ADR 0017 decision C, P5f) close the family. Both
          // start by picking an **existing** linear dimension as their base, then
          // take any number of points and commit with Enter, so they are appended
          // after the seven creation commands (no shipped button moves) and last
          // in the family: they consume a dimension that already exists instead of
          // creating the first one.
          { id: 'DIMBASELINE', labelKey: 'draw.kind.dimbaseline', icon: 'dimbaseline', command: 'DIMBASELINE', large: true },
          { id: 'DIMCONTINUE', labelKey: 'draw.kind.dimcontinue', icon: 'dimcontinue', command: 'DIMCONTINUE', large: true },
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
