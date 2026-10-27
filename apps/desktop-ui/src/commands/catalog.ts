// The chrome's declarative command catalog: every command name the app exposes,
// in AutoCAD-style listing order. Split verbatim out of `commands.ts` (which
// owns the functions that read it) so neither module approaches the 1000-line
// code gate. This is a plain sibling module, not a barrel: importers name it
// directly.

/** One entry of the chrome's display catalog. */
export interface CommandCatalogEntry {
  readonly id: string;
  readonly labelKey: string;
  readonly icon: string;
  readonly group: string;
}

/**
 * Every command name the app exposes in the chrome, in AutoCAD-style listing
 * order (file, draw, modify, view, measure, annotate, classify). `labelKey` is a
 * catalog key; `icon` names an entry in the icon set.
 *
 * This is a **display** table only: whether an entry is enabled comes from the
 * loaded host vocabulary ([`isSupportedCommand`]), so a command missing here can
 * still be typed and admitted when the router knows it.
 */
export const COMMAND_CATALOG: readonly CommandCatalogEntry[] = Object.freeze([
  { id: 'OPEN', labelKey: 'file.open', icon: 'open', group: 'file' },
  { id: 'FIT', labelKey: 'toolbar.fit', icon: 'fit', group: 'view' },
  { id: 'NEW', labelKey: 'qat.new', icon: 'new', group: 'file' },
  { id: 'SAVE', labelKey: 'qat.save', icon: 'save', group: 'file' },
  // PLOT is the same router selector as the `EXPORT`/`PRINT` aliases (the host
  // writes SVG/PDF through the platform save picker and refuses `.png` for the
  // GUI). It sits after SAVE because it is a file-level output action, and it is
  // gated by `canPlot` like the aliases, not by anything local.
  { id: 'PLOT', labelKey: 'file.plot', icon: 'save', group: 'file' },
  { id: 'UNDO', labelKey: 'toolbar.undo', icon: 'undo', group: 'modify' },
  { id: 'REDO', labelKey: 'toolbar.redo', icon: 'redo', group: 'modify' },
  { id: 'LINE', labelKey: 'draw.kind.line', icon: 'line', group: 'draw' },
  { id: 'CIRCLE', labelKey: 'draw.kind.circle', icon: 'circle', group: 'draw' },
  { id: 'ARC', labelKey: 'draw.kind.arc', icon: 'arc', group: 'draw' },
  { id: 'PLINE', labelKey: 'draw.kind.polyline', icon: 'polyline', group: 'draw' },
  { id: 'RECTANG', labelKey: 'draw.kind.rectang', icon: 'rectang', group: 'draw' },
  { id: 'ELLIPSE', labelKey: 'draw.kind.ellipse', icon: 'ellipse', group: 'draw' },
  { id: 'POLYGON', labelKey: 'draw.kind.polygon', icon: 'polygon', group: 'draw' },
  { id: 'POINT', labelKey: 'draw.kind.point', icon: 'point', group: 'draw' },
  { id: 'SPLINE', labelKey: 'draw.kind.spline', icon: 'spline', group: 'draw' },
  // Single-line TEXT (ADR 0019 decision A): a point and then its text, which is
  // the command's terminal step and travels on `cad_set_tool_text`.
  { id: 'TEXT', labelKey: 'draw.kind.text', icon: 'text', group: 'draw' },
  // MTEXT (2026-10-26): the multiline counterpart of TEXT. The bare selector
  // starts the two-corner tool (first corner = insertion point, opposite corner =
  // wrap width) and the text is the same terminal `cad_set_tool_text` step. It
  // has no variant entry: rotation/attachment/line spacing/columns/background are
  // deliberately not implemented, and nothing here may imply them.
  { id: 'MTEXT', labelKey: 'draw.kind.mtext', icon: 'mtext', group: 'draw' },
  // MLEADER (ADR 0019 decision F.2): two points — the arrowhead position, then
  // the landing point — and then that same terminal text step. It carries no
  // variant entry because the command has no `Options` sub-prompt (leader type /
  // content type / max points / angular constraint), and nothing here may imply
  // one.
  { id: 'MLEADER', labelKey: 'draw.kind.mleader', icon: 'mleader', group: 'draw' },
  { id: 'HATCH', labelKey: 'draw.kind.hatch', icon: 'hatch', group: 'draw' },
  { id: 'MOVE', labelKey: 'draw.kind.move', icon: 'move', group: 'modify' },
  { id: 'COPY', labelKey: 'draw.kind.copy', icon: 'copy', group: 'modify' },
  // Clipboard selectors with a base point (2026-10-27), placed directly after the
  // modify `COPY` they refine so no shipped row moves. `COPYBASE`/`CUTBASE` put
  // the selection on the clipboard **with a base point**, so the following
  // `PASTECLIP` is placed relative to that point; `PASTEORIG` pastes at the
  // coordinates the clipboard came from and takes no insertion pick. `PASTEBLOCK`
  // is deliberately **absent**: it is not implemented, so no row may suggest it.
  // All three reuse the `copy` glyph — the icon set has no cut/paste mark, and
  // they are the same clipboard family (the JOIN/AREA OBJECT precedent).
  { id: 'COPYBASE', labelKey: 'draw.kind.copybase', icon: 'copy', group: 'modify' },
  { id: 'CUTBASE', labelKey: 'draw.kind.cutbase', icon: 'copy', group: 'modify' },
  { id: 'PASTEORIG', labelKey: 'draw.kind.pasteorig', icon: 'copy', group: 'modify' },
  { id: 'TRIM', labelKey: 'draw.kind.trim', icon: 'trim', group: 'modify' },
  { id: 'EXTEND', labelKey: 'draw.kind.extend', icon: 'extend', group: 'modify' },
  { id: 'ROTATE', labelKey: 'draw.kind.rotate', icon: 'rotate', group: 'modify' },
  { id: 'SCALE', labelKey: 'draw.kind.scale', icon: 'scale', group: 'modify' },
  { id: 'MIRROR', labelKey: 'draw.kind.mirror', icon: 'mirror', group: 'modify' },
  { id: 'OFFSET', labelKey: 'draw.kind.offset', icon: 'offset', group: 'modify' },
  { id: 'CHAMFER', labelKey: 'draw.kind.chamfer', icon: 'chamfer', group: 'modify' },
  { id: 'FILLET', labelKey: 'draw.kind.fillet', icon: 'fillet', group: 'modify' },
  // The **Polyline modes** (2026-10-27) are selectors of their own, directly
  // after the two-line pair they share a name with and never aliases of it: the
  // two-pick `CHAMFER`/`FILLET` capture is unchanged, and an alias would route a
  // polyline edit into that capture. Each takes `CommandPayload::None` — the one
  // preselected polyline is the whole input — and acts on the current selection,
  // so `SELECTION_REQUIREMENTS` (not a draw tool) decides when it can run. The
  // radius and the two chamfer distances are the existing `filletRadius` /
  // `chamferDistanceA` / `chamferDistanceB` tool parameters: no new setting row.
  //
  // Both ids are **multi-word**, exactly like `AREA OBJECT`: the host
  // canonicalises the whole trimmed line, so the space is part of one name here
  // and in the vocabulary mirror. The glyphs are the parents' own — the operation
  // is the same corner treatment, only its input differs.
  { id: 'CHAMFER POLYLINE', labelKey: 'draw.kind.chamfer_polyline', icon: 'chamfer', group: 'modify' },
  { id: 'FILLET POLYLINE', labelKey: 'draw.kind.fillet_polyline', icon: 'fillet', group: 'modify' },
  { id: 'ARRAYRECT', labelKey: 'draw.kind.arrayrect', icon: 'arrayrect', group: 'modify' },
  { id: 'ARRAYPOLAR', labelKey: 'draw.kind.arraypolar', icon: 'arraypolar', group: 'modify' },
  { id: 'ALIGN', labelKey: 'draw.kind.align', icon: 'align', group: 'modify' },
  { id: 'STRETCH', labelKey: 'draw.kind.stretch', icon: 'stretch', group: 'modify' },
  { id: 'ERASE', labelKey: 'draw.kind.erase', icon: 'erase', group: 'modify' },
  { id: 'EXPLODE', labelKey: 'draw.kind.explode', icon: 'explode', group: 'modify' },
  // MATCHPROP (the selection-based property painter) sits after the other
  // selection-acting modify commands. It is a **payload-free** router command, so
  // it is listed and typeable; whether it can run is decided by
  // `SELECTION_REQUIREMENTS` (it needs a source *and* at least one target), so a
  // one-object selection is refused with the sentence that says so.
  { id: 'MATCHPROP', labelKey: 'draw.kind.matchprop', icon: 'matchprop', group: 'modify' },
  // JOIN (2026-10-26) closes the modify run: welding the current selection into
  // one entity acts on that selection exactly like MATCHPROP, so it is appended
  // after it (no shipped row moves). It is a payload-free router command, and the
  // "at least two objects" rule is `SELECTION_REQUIREMENTS`' — the host refuses a
  // one-object join outright, so the button is never a live affordance that can
  // only fail. The glyph is the polyline one: the icon set has no dedicated join
  // mark yet, and the merge output is a curve either way.
  { id: 'JOIN', labelKey: 'draw.kind.join', icon: 'polyline', group: 'modify' },
  { id: 'DIST', labelKey: 'measure.kind.distance', icon: 'distance', group: 'measure' },
  { id: 'MEASUREPOLYLINE', labelKey: 'measure.kind.polyline', icon: 'polyline', group: 'measure' },
  { id: 'MEASUREANGLE', labelKey: 'measure.kind.angle', icon: 'angle', group: 'measure' },
  { id: 'AREA', labelKey: 'measure.kind.area', icon: 'area', group: 'measure' },
  // AREA OBJECT (2026-10-27) is the selection-based area variant: it takes the
  // current selection (exactly one closed object — a circle, an ellipse or a
  // closed polyline with bulges) instead of a picked boundary, and echoes the
  // exact area under its own algorithm token (`area_object`). Sharing the `area`
  // glyph is deliberate: it is the same measurement, only its input differs.
  //
  // The id is **multi-word**, which matters for the local admission path: the
  // host canonicalises the whole trimmed line, so a space-separated selector is
  // one name there, while `splitCommandText` here reads the first word as the
  // base name (`AREA`) and the rest as a typed argument (`OBJECT`). This entry is
  // therefore the *catalogue* half only — the surface that names and suggests the
  // command; the local admission path still refuses the line because it splits on
  // the space before the vocabulary is consulted (pinned by
  // `test/area-object-layerp.test.ts` and reported, never patched into the parser
  // with a one-off exception). It is listed beside `AREA` because it is the same
  // measurement kind, and no shipped row moves.
  { id: 'AREA OBJECT', labelKey: 'measure.kind.area_object', icon: 'area', group: 'measure' },
  // ID (2026-10-26) is the point-coordinate readout: one point pick, reported as
  // X/Y/Z plus the object snap that produced it. It is the fifth measurement kind
  // and is listed last there so no shipped entry moves. It reuses the `point`
  // glyph (a crosshair on a marker) because the command is "which point is this",
  // and it has no variant popover: there is no options sub-prompt.
  { id: 'ID', labelKey: 'measure.kind.id', icon: 'point', group: 'measure' },
  { id: 'DIMLINEAR', labelKey: 'draw.kind.dimlinear', icon: 'dimlinear', group: 'annotate' },
  { id: 'DIMALIGNED', labelKey: 'draw.kind.dimaligned', icon: 'dimaligned', group: 'annotate' },
  { id: 'DIMRADIUS', labelKey: 'draw.kind.dimradius', icon: 'dimradius', group: 'annotate' },
  { id: 'DIMDIAMETER', labelKey: 'draw.kind.dimdiameter', icon: 'dimdiameter', group: 'annotate' },
  { id: 'DIMCENTER', labelKey: 'draw.kind.dimcenter', icon: 'dimcenter', group: 'annotate' },
  { id: 'DIMANGULAR', labelKey: 'draw.kind.dimangular', icon: 'dimangular', group: 'annotate' },
  { id: 'DIMORDINATE', labelKey: 'draw.kind.dimordinate', icon: 'dimordinate', group: 'annotate' },
  // Baseline/continue dimensions (ADR 0017 decision C, P5f): both pick an
  // existing linear dimension first, then accept any number of points and commit
  // with Enter — never a fixed three-point capture like the linear pair.
  { id: 'DIMBASELINE', labelKey: 'draw.kind.dimbaseline', icon: 'dimbaseline', group: 'annotate' },
  { id: 'DIMCONTINUE', labelKey: 'draw.kind.dimcontinue', icon: 'dimcontinue', group: 'annotate' },
  { id: 'LAYER', labelKey: 'layers.panel', icon: 'layers', group: 'annotate' },
  // Layer isolate / unisolate (2026-10-26). Unlike `LAYER` they are **router**
  // commands (`LAYISO`/`LAYUNISO`), not chrome-local selectors, so they are
  // admitted by the host vocabulary alone. Their glyphs are the layers palette's
  // own visibility pair — isolating hides the other layers (dimmed eye),
  // unisolating shows them again (plain eye) — because the operation is a
  // visibility change, not a palette toggle.
  { id: 'LAYISO', labelKey: 'layers.isolate', icon: 'eyeOff', group: 'annotate' },
  { id: 'LAYUNISO', labelKey: 'layers.unisolate', icon: 'eye', group: 'annotate' },
  // Layer off / on (2026-10-26) close the layer run. `LAYOFF` turns **off** every
  // layer the current selection sits on (the current layer excepted), and `LAYON`
  // is the router's restore selector: it turns every layer back on and takes no
  // selection. Both are **router** commands like the isolate pair — never
  // chrome-local selectors — so the host vocabulary alone admits them. They reuse
  // the same visibility glyph pair for the same reason: turning layers off hides
  // geometry (dimmed eye) and restoring shows it again (plain eye), which is a
  // visibility change, not a palette toggle.
  { id: 'LAYOFF', labelKey: 'draw.kind.layoff', icon: 'eyeOff', group: 'annotate' },
  { id: 'LAYON', labelKey: 'draw.kind.layon', icon: 'eye', group: 'annotate' },
  // LAYERP (2026-10-27) is **no longer an alias of the restore command**: it
  // restores the *previous* layer-override state and names itself (`layers.previous`),
  // so it is a selector in its own right at the same layer run's end. `LAYUNISO`
  // and `LAYON` keep meaning "clear every override"; this one does not, which is
  // why it never borrows `layers.restore` or `draw.kind.layon` as its label.
  { id: 'LAYERP', labelKey: 'layers.previous', icon: 'eye', group: 'annotate' },
  { id: 'PROPERTIES', labelKey: 'properties.panel', icon: 'properties', group: 'annotate' },
  { id: 'SELECTALL', labelKey: 'selection.all', icon: 'selectAll', group: 'annotate' },
  { id: 'ZOOMIN', labelKey: 'toolbar.zoom_in', icon: 'zoomIn', group: 'view' },
  { id: 'ZOOMOUT', labelKey: 'toolbar.zoom_out', icon: 'zoomOut', group: 'view' },
  { id: 'PAN', labelKey: 'toolbar.pan', icon: 'pan', group: 'view' },
  // RESETVIEW is a payload-free router command (`CommandId::ResetView`) that
  // restores the drawing's saved view; it has no capability gate and no
  // document-scoped precondition of its own.
  { id: 'RESETVIEW', labelKey: 'view.reset', icon: 'view', group: 'view' },
  { id: 'ORBIT', labelKey: 'view.orbit_label', icon: 'orbit', group: 'view' },
  // 3D view selectors: both are payload-free router commands (typing them in the
  // command line already works), and both need an open drawing — the ribbon
  // disables them on the start page. `STANDARDVIEW` is deliberately **absent**:
  // it carries a payload and travels on `cad_standard_view`, never on
  // `cad_command`, so it is not a name this catalog may offer for typing.
  { id: 'SWITCH2D3D', labelKey: 'ribbon.command.view_switch2d3d', icon: 'view', group: 'view' },
  { id: 'PROJECTION', labelKey: 'view.projection', icon: 'axes', group: 'view' },
  { id: 'GRID', labelKey: 'overlay.grid', icon: 'grid', group: 'view' },
  { id: 'SNAP', labelKey: 'status.toggle.snap', icon: 'snap', group: 'view' },
  { id: 'ORTHO', labelKey: 'status.toggle.ortho', icon: 'angle', group: 'view' },
  { id: 'OSNAP', labelKey: 'status.toggle.osnap', icon: 'snap', group: 'view' },
  // POINT display style sysvars (2026-10-26). They are **command-line settings**
  // like `SNAPUNIT`, not buttons: typing `PDMODE 3` / `PDSIZE 2.5` on the command
  // line is the whole affordance, so neither gets a ribbon item — the
  // `SNAPUNIT` precedent, which also has no ribbon button. They are listed at
  // the end of the drafting-aid run (after `OSNAP`) because no shipped row may
  // move, and they reuse the `point` glyph: the setting is how POINTs are drawn.
  // Both change the open drawing's in-memory display style only; nothing here
  // may imply undo or that the exporter writes the values.
  { id: 'PDMODE', labelKey: 'point.display.mode', icon: 'point', group: 'view' },
  { id: 'PDSIZE', labelKey: 'point.display.size', icon: 'point', group: 'view' },
]);
