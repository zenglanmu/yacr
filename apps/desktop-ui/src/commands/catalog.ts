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
  // CLOSE ends the open drawing and returns the chrome to the start page. It is
  // listed directly after the other file-level actions because it is one, and it
  // is document-scoped (`gating.ts`): with no drawing open there is nothing to
  // close, so the start page disables it with the catalog's own "no drawing is
  // open" sentence.
  { id: 'CLOSE', labelKey: 'doctabs.close', icon: 'close', group: 'file' },
  { id: 'UNDO', labelKey: 'toolbar.undo', icon: 'undo', group: 'modify' },
  { id: 'REDO', labelKey: 'toolbar.redo', icon: 'redo', group: 'modify' },
  { id: 'LINE', labelKey: 'draw.kind.line', icon: 'line', group: 'draw' },
  { id: 'CIRCLE', labelKey: 'draw.kind.circle', icon: 'circle', group: 'draw' },
  // Circle definition modes (2026-11-01), listed directly after the `CIRCLE` they
  // belong to: the same command line with a mode word (`CIRCLE 2P`, `CIRCLE 3P`,
  // `CIRCLE D`). Each is a **canonical selector of its own**, never an alias of
  // `CIRCLE`: the capture differs in what it asks for — two diameter endpoints,
  // three points on the circumference, or a centre followed by a point whose
  // distance from that centre is the **diameter** (so the radius is half of it) —
  // and every mode commits exactly one circle.
  //
  // The ids are **multi-word**, exactly like `AREA OBJECT`: the host canonicalises
  // the whole trimmed line, so the space is part of one name here and in the
  // vocabulary mirror. Admission resolves the whole line before the
  // base-plus-argument grammar, so a correctly advertised `CIRCLE 2P` is admitted
  // as one selector instead of being refused as `CIRCLE` with a typed argument.
  //
  // No mode takes a parameter: the picks are the whole input, and no variant may
  // imply a radius/diameter switch on the plain `CIRCLE`. All three reuse the
  // `circle` glyph — every mode produces the same entity and the icon set has no
  // per-mode badge; the label names the definition method (the `AREA` /
  // `AREA OBJECT` precedent).
  { id: 'CIRCLE 2P', labelKey: 'draw.kind.circle2p', icon: 'circle', group: 'draw' },
  { id: 'CIRCLE 3P', labelKey: 'draw.kind.circle3p', icon: 'circle', group: 'draw' },
  { id: 'CIRCLE D', labelKey: 'draw.kind.circlediameter', icon: 'circle', group: 'draw' },
  { id: 'ARC', labelKey: 'draw.kind.arc', icon: 'arc', group: 'draw' },
  // XLINE and RAY (2026-11-01) are the two **construction** captures, listed
  // directly after the primitives they belong with (LINE, CIRCLE, ARC), so no
  // shipped row moves and the draw run still opens with the plain primitives.
  //
  // Both take exactly two points — a base and a through point — and commit **one
  // finite bounded segment** clipped to the model bounds: the entity keeps its
  // construction type key (`AcDbXline` / `AcDbRay`), but there is **no infinite
  // geometry** anywhere in this build. That is why the labels name the type
  // (`draw.kind.xline` says "construction line (XLINE)") and the hint says the
  // drawn span is finite: a reader must never infer an unbounded line from the
  // AutoCAD name.
  //
  // They carry no parameter: two picks are the whole input, there is no options
  // sub-prompt, and neither is a pre-selection tool (the picks are the input).
  // The glyphs are their own (`xline` / `ray`), drawn in the same 24px stroke
  // language as `line` so the three read as one family.
  { id: 'XLINE', labelKey: 'draw.kind.xline', icon: 'xline', group: 'draw' },
  { id: 'RAY', labelKey: 'draw.kind.ray', icon: 'ray', group: 'draw' },
  { id: 'PLINE', labelKey: 'draw.kind.polyline', icon: 'polyline', group: 'draw' },
  { id: 'RECTANG', labelKey: 'draw.kind.rectang', icon: 'rectang', group: 'draw' },
  { id: 'ELLIPSE', labelKey: 'draw.kind.ellipse', icon: 'ellipse', group: 'draw' },
  { id: 'POLYGON', labelKey: 'draw.kind.polygon', icon: 'polygon', group: 'draw' },
  // POLYGON CIRCUMSCRIBED (2026-11-07) is AutoCAD's `POLYGON` →
  // `Circumscribed about circle` (`C`) construction, listed directly after the
  // `POLYGON` it belongs to. It is a **canonical selector of its own**, never an
  // alias: the second pick is a point on the **tangent** circle (not a vertex), the
  // picked direction is the midpoint of the first edge, and the ring is
  // `R = r / cos(π/n)` instead of `r`. The side count is the same session
  // `POLYSIDES` sysvar, so there is no parameter row and no `Edge` variant (that
  // construction is NOT IMPLEMENTED). The id is multi-word exactly like
  // `POLYGON CIRCUMSCRIBED`'s `CIRCLE 2P`/`CHAMFER POLYLINE` neighbours; the glyph
  // is the parent's own (same LWPOLYLINE output, different construction).
  { id: 'POLYGON CIRCUMSCRIBED', labelKey: 'draw.kind.polygon_circumscribed', icon: 'polygon', group: 'draw' },
  { id: 'POINT', labelKey: 'draw.kind.point', icon: 'point', group: 'draw' },
  { id: 'SPLINE', labelKey: 'draw.kind.spline', icon: 'spline', group: 'draw' },
  // INSERT (2026-11-01) places a reference to a block that **already exists** in
  // the drawing: one pick sets the insertion point, then the block name is typed
  // as the terminal text step (the host commits after the text). It is listed at
  // the end of the draw group's creation run, directly after SPLINE, so no
  // shipped row moves. There is no block list/palette in v1 and no
  // scale/rotation/attribute control, so no variant or option row may suggest
  // one. The glyph is `copy` — the icon set has no insert mark, and the output is
  // a reference to the block, the same as a copied definition.
  { id: 'INSERT', labelKey: 'draw.kind.insertblock', icon: 'copy', group: 'draw' },
  // BLOCK (2026-11-01) is the other half of the block pair and sits directly
  // beside INSERT: it turns the **current selection** into a new block definition
  // and replaces it with a reference placed at the picked base point. The flow is
  // selection already made → pick base point → type a new block name (the
  // terminal text step). It is not a pre-selection tool: the base pick is the
  // input, so a preselection entry would hide it. v1 has no attributes, no
  // dynamic-block options and no retain/delete choice — the selected objects move
  // into the new block — so no variant row or option caret may imply otherwise.
  // The glyph is the shared `copy` mark, like INSERT: the icon set has no
  // block-specific name, and the output is a reference to the block definition.
  { id: 'BLOCK', labelKey: 'draw.kind.blockdefine', icon: 'copy', group: 'draw' },
  // Single-line TEXT (ADR 0019 decision A): a point and then its text, which is
  // the command's terminal step and travels on `cad_set_tool_text`.
  { id: 'TEXT', labelKey: 'draw.kind.text', icon: 'text', group: 'draw' },
  // MTEXT (2026-10-26): the multiline counterpart of TEXT. The bare selector
  // starts the two-corner tool (first corner = insertion point, opposite corner =
  // wrap width) and the text is the same terminal `cad_set_tool_text` step. It
  // has no variant entry: rotation/attachment/line spacing/columns/background are
  // deliberately not implemented, and nothing here may imply them.
  { id: 'MTEXT', labelKey: 'draw.kind.mtext', icon: 'mtext', group: 'draw' },
  // SCALETEXT (2026-11-01) scales the glyph height of the **selected** Text/MTEXT
  // objects (and the MTEXT wrap width) by one positive factor. It is a
  // selection-based, **zero-point** command, so it is listed beside the text
  // commands it edits, directly after MTEXT; the later creation and annotation
  // rows keep their relative order. The glyph is the shared `scale` mark — the
  // operation is a scale, applied to text rather than to picked geometry.
  { id: 'SCALETEXT', labelKey: 'draw.kind.scaletext', icon: 'scale', group: 'draw' },
  // MLEADER (ADR 0019 decision F.2): two points — the arrowhead position, then
  // the landing point — and then that same terminal text step. It carries no
  // variant entry because the command has no `Options` sub-prompt (leader type /
  // content type / max points / angular constraint), and nothing here may imply
  // one.
  { id: 'MLEADER', labelKey: 'draw.kind.mleader', icon: 'mleader', group: 'draw' },
  { id: 'HATCH', labelKey: 'draw.kind.hatch', icon: 'hatch', group: 'draw' },
  // HATCHPOINT is HATCH's point-pick entry: one pick inside a closed boundary
  // (circle, full ellipse, closed polyline) and the host finds the containing
  // boundary with its nested islands. It reuses the `hatch` glyph — the icon set
  // has no separate point mark, and the fill output is the same entity — and it
  // sits directly after HATCH so no shipped row moves.
  { id: 'HATCHPOINT', labelKey: 'draw.kind.hatchpoint', icon: 'hatch', group: 'draw' },
  // BOUNDARY (AutoCAD 2026) is the other internal-point selector: it reuses
  // HATCHPOINT's discovery but creates one closed, non-associative polyline per
  // loop on the active layer. It is a canonical selector of its own (never an
  // alias of HATCH/HATCHPOINT; `BO`/`BPOLY` are deliberately not admitted) and it
  // is a capture, not a pre-selection tool. It reuses the `polyline` glyph — the
  // output is a polyline — and sits directly after HATCHPOINT so the HATCH family
  // stays contiguous. `-BOUNDARY` options are NOT IMPLEMENTED; the hint says so.
  { id: 'BOUNDARY', labelKey: 'draw.kind.boundary', icon: 'polyline', group: 'draw' },
  { id: 'MOVE', labelKey: 'draw.kind.move', icon: 'move', group: 'modify' },
  { id: 'COPY', labelKey: 'draw.kind.copy', icon: 'copy', group: 'modify' },
  // Clipboard selectors with a base point (2026-10-27), placed directly after the
  // modify `COPY` they refine so no shipped row moves. `COPYBASE`/`CUTBASE` put
  // the selection on the clipboard **with a base point**, so the following
  // `PASTECLIP` is placed relative to that point; `PASTEORIG` pastes at the
  // coordinates the clipboard came from and takes no insertion pick.
  //
  // PASTEBLOCK (2026-11-01) closes the same run: it pastes the clipboard as a
  // **new auto-named block** (`PasteBlock1`, `PasteBlock2`, …) plus one reference
  // at a picked insertion point. It is a one-point capture like `PASTECLIP`, but
  // it is its own selector: the output is a block reference, not loose geometry,
  // and the clipboard is not cleared (so it may be pasted again). All four reuse
  // the `copy` glyph — the icon set has no cut/paste/block-paste mark, and they
  // are the same clipboard family (the JOIN/AREA OBJECT precedent).
  { id: 'COPYBASE', labelKey: 'draw.kind.copybase', icon: 'copy', group: 'modify' },
  { id: 'CUTBASE', labelKey: 'draw.kind.cutbase', icon: 'copy', group: 'modify' },
  { id: 'PASTEORIG', labelKey: 'draw.kind.pasteorig', icon: 'copy', group: 'modify' },
  { id: 'PASTEBLOCK', labelKey: 'draw.kind.pasteblock', icon: 'copy', group: 'modify' },
  { id: 'TRIM', labelKey: 'draw.kind.trim', icon: 'trim', group: 'modify' },
  { id: 'EXTEND', labelKey: 'draw.kind.extend', icon: 'extend', group: 'modify' },
  // The three single-curve edits (2026-11-01) are listed directly after the
  // TRIM/EXTEND pair they belong with, so no shipped row moves.
  //
  // BREAK takes **two picks**: the first pick selects the curve *and* is the first
  // break point, the second is the other end of the span that is removed. It is
  // therefore **not** a pre-selection tool — the pick is the input. Its glyph is
  // the `trim` mark (scissors), because the operation is a trim between two
  // points; the icon set has no break-specific name.
  //
  // DIVIDE and LENGTHEN take **no point at all**: the one preselected curve is the
  // whole input, exactly like HATCH, and their settings are the existing tool
  // parameters (`divideSegments`; `lengthenMode` + `lengthenValue`) rather than a
  // second settings surface. The glyphs are borrowed: `point` for the division
  // marks DIVIDE creates, `scale` for the length change LENGTHEN makes (the icon
  // set has no divide/lengthen mark; SCALE keeps its own row below).
  //
  // The host refuses a polyline for BREAK and anything beyond one LINE/ARC/CIRCLE/
  // straight-polyline for the other two; nothing here may imply the wider set.
  { id: 'BREAK', labelKey: 'draw.kind.break', icon: 'trim', group: 'modify' },
  { id: 'DIVIDE', labelKey: 'draw.kind.divide', icon: 'point', group: 'modify' },
  { id: 'LENGTHEN', labelKey: 'draw.kind.lengthen', icon: 'scale', group: 'modify' },
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
  // MEASUREGEOM (AutoCAD 2026) is the inquiry **router** over the measurement
  // tools already listed above. The bare name needs an option (`DISTANCE`,
  // `ANGLE`, `AREA`, `POLYLINE`, `RADIUS`, `VOLUME`), which the ribbon sends as
  // the full `MEASUREGEOM <OPTION>` line; `VOLUME` is an explicit refusal (no 3D
  // solid/volume model) and must never be presented as a working measurement.
  { id: 'MEASUREGEOM', labelKey: 'measure.kind.measuregeom', icon: 'distance', group: 'measure' },
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
  // The text and rotated/spaced dimension commands (2026-11-07) close the run.
  { id: 'DIMROTATED', labelKey: 'draw.kind.dimrotated', icon: 'dimrotated', group: 'annotate' },
  { id: 'DIMSPACE', labelKey: 'draw.kind.dimspace', icon: 'dimspace', group: 'annotate' },
  // The 2026-11-07 dimension-editing batch. These are command-line forms first
  // (the host owns the grammar and every refusal): DIMSTYLE/-DIMSTYLE report the
  // style table, DIMTEDIT/DIMEDIT/DIMOVERRIDE edit selected dimensions, and
  // DIMREGEN/DIMDISASSOCIATE are explicit non-associative no-op reports.
  // QDIM is selection-based; DIMJOGLINE/DIMBREAK are catalogued capability
  // refusals so a typed or clicked attempt names the missing domain field.
  { id: 'DIMSTYLE', labelKey: 'draw.kind.dimstyle', icon: 'properties', group: 'annotate' },
  { id: '-DIMSTYLE', labelKey: 'draw.kind.dimstyle_alias', icon: 'terminal', group: 'annotate' },
  { id: 'DIMTEDIT', labelKey: 'draw.kind.dimtedit', icon: 'pencil', group: 'annotate' },
  { id: 'DIMEDIT', labelKey: 'draw.kind.dimedit', icon: 'pencil', group: 'annotate' },
  { id: 'DIMOVERRIDE', labelKey: 'draw.kind.dimoverride', icon: 'properties', group: 'annotate' },
  { id: 'DIMREGEN', labelKey: 'draw.kind.dimregen', icon: 'rotate', group: 'annotate' },
  { id: 'DIMDISASSOCIATE', labelKey: 'draw.kind.dimdisassociate', icon: 'reference', group: 'annotate' },
  { id: 'QDIM', labelKey: 'draw.kind.qdim', icon: 'dimlinear', group: 'annotate' },
  { id: 'DIMJOGLINE', labelKey: 'draw.kind.dimjogline', icon: 'dimlinear', group: 'annotate' },
  { id: 'DIMBREAK', labelKey: 'draw.kind.dimbreak', icon: 'trim', group: 'annotate' },
  // The 2026-11-07 annotation/group batch. The group commands are session-only
  // command-line forms; QLEADER/LEADER create a MULTILEADER; TEXTFIT and the
  // hatch/xdata commands read the selection. `-GROUP` is the host alias spelling.
  { id: 'GROUP', labelKey: 'draw.kind.group', icon: 'properties', group: 'annotate' },
  { id: '-GROUP', labelKey: 'draw.kind.group_alias', icon: 'terminal', group: 'annotate' },
  { id: 'UNGROUP', labelKey: 'draw.kind.ungroup', icon: 'explode', group: 'annotate' },
  { id: 'GROUPEDIT', labelKey: 'draw.kind.groupedit', icon: 'pencil', group: 'annotate' },
  { id: 'QLEADER', labelKey: 'draw.kind.qleader', icon: 'pencil', group: 'annotate' },
  { id: 'LEADER', labelKey: 'draw.kind.leader', icon: 'reference', group: 'annotate' },
  { id: 'TEXTFIT', labelKey: 'draw.kind.textfit', icon: 'pencil', group: 'annotate' },
  { id: 'HATCHSETBOUNDARY', labelKey: 'draw.kind.hatchsetboundary', icon: 'properties', group: 'annotate' },
  { id: 'HATCHSETORIGIN', labelKey: 'draw.kind.hatchsetorigin', icon: 'reference', group: 'annotate' },
  { id: 'XDLIST', labelKey: 'draw.kind.xdlist', icon: 'properties', group: 'annotate' },
  { id: 'TORIENT', labelKey: 'draw.kind.torient', icon: 'rotate', group: 'annotate' },
  { id: 'TCASE', labelKey: 'draw.kind.tcase', icon: 'pencil', group: 'annotate' },
  { id: 'TXT2MTXT', labelKey: 'draw.kind.txt2mtxt', icon: 'copy', group: 'annotate' },
  // The 2026-11-07 2D text batch. The five argument-taking entries are
  // command-line first: clicking one inserts the bare name and the user types the
  // argument (`JUSTIFYTEXT LEFT TOP`, `TSCALE 2`, `TEXTALIGN LEFT`,
  // `TCOUNT 1 1 A -`, `TCIRCLE CIRCLE`); the host refuses a bare argument form by
  // name. TEXTMASK/TEXTUNMASK take no argument and are selection-based.
  { id: 'JUSTIFYTEXT', labelKey: 'draw.kind.justifytext', icon: 'pencil', group: 'annotate' },
  { id: 'TSCALE', labelKey: 'draw.kind.tscale', icon: 'scale', group: 'annotate' },
  // The new entries must not reuse the dedicated `text`/`mtext`/`hatch` glyphs:
  // those names belong to exactly one catalogued command each (the chrome glyph
  // contracts assert it). `align` reads as the shared edge, `pencil` as an edit,
  // `eyeOff`/`eye` as mask/unmask.
  { id: 'TEXTALIGN', labelKey: 'draw.kind.textalign', icon: 'align', group: 'annotate' },
  { id: 'TCOUNT', labelKey: 'draw.kind.tcount', icon: 'pencil', group: 'annotate' },
  { id: 'TCIRCLE', labelKey: 'draw.kind.tcircle', icon: 'circle', group: 'annotate' },
  { id: 'TEXTMASK', labelKey: 'draw.kind.textmask', icon: 'eyeOff', group: 'annotate' },
  { id: 'TEXTUNMASK', labelKey: 'draw.kind.textunmask', icon: 'eye', group: 'annotate' },
  // The two aliases are real advertised names (host `COMMAND_ALIASES`), not
  // second commands: MTEDIT starts the existing TEXTEDIT pick tool and `-MTEXT`
  // starts the existing two-corner MTEXT tool.
  { id: 'MTEDIT', labelKey: 'draw.kind.mtedit', icon: 'pencil', group: 'annotate' },
  { id: '-MTEXT', labelKey: 'draw.kind.mtext', icon: 'copy', group: 'annotate' },
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
  // The AutoCAD 2026 layer-selection run (LAYCUR/LAYMCUR/LAYFRZ/LAYTHW/
  // LAYLCK/LAYULK/SETBYLAYER) is **pre-selection based** in this build: the
  // fixed capture machine cannot ask "Select objects", so the current session
  // selection is the input, exactly like MATCHPROP/LAYISO. All are Work-only
  // router commands in the layers palette's run; their effects are layer state,
  // so they reuse the layer glyphs and are dropped with the palette on a
  // palette-less carrier (`PALETTE_COMMANDS`). `SETBYLAYER` is included here as
  // the layer-property reset it is, not as a general Modify button.
  { id: 'LAYCUR', labelKey: 'draw.kind.laycur', icon: 'layers', group: 'annotate' },
  { id: 'LAYMCUR', labelKey: 'draw.kind.laymcur', icon: 'layers', group: 'annotate' },
  { id: 'LAYFRZ', labelKey: 'draw.kind.layfrz', icon: 'eyeOff', group: 'annotate' },
  { id: 'LAYTHW', labelKey: 'draw.kind.laythw', icon: 'eye', group: 'annotate' },
  { id: 'LAYLCK', labelKey: 'draw.kind.laylck', icon: 'layers', group: 'annotate' },
  { id: 'LAYULK', labelKey: 'draw.kind.layulk', icon: 'layers', group: 'annotate' },
  { id: 'SETBYLAYER', labelKey: 'draw.kind.setbylayer', icon: 'layers', group: 'annotate' },
  // The 2026-11-07 layer batch. LAYMCH/LAYMRG/LAYDEL are pre-selection based;
  // LAYERSTATE, LTSCALE and LAYWALK take typed arguments, so their catalogue rows
  // are the command-line affordance (no variant popover). The `-` spellings are
  // aliases of the same commands and therefore get no row of their own.
  { id: 'LAYMCH', labelKey: 'draw.kind.laymch', icon: 'layers', group: 'annotate' },
  { id: 'LAYMRG', labelKey: 'draw.kind.laymrg', icon: 'layers', group: 'annotate' },
  { id: 'LAYDEL', labelKey: 'draw.kind.laydel', icon: 'erase', group: 'annotate' },
  { id: 'LAYERSTATE', labelKey: 'draw.kind.layerstate', icon: 'layers', group: 'annotate' },
  { id: 'LAYERPMODE', labelKey: 'draw.kind.layerpmode', icon: 'layers', group: 'annotate' },
  { id: 'LTSCALE', labelKey: 'draw.kind.ltscale', icon: 'layers', group: 'annotate' },
  { id: 'LAYWALK', labelKey: 'draw.kind.laywalk', icon: 'eye', group: 'annotate' },
  // FONTS opens the read-only Font Manager (`cad_fonts`). It is a chrome-local
  // selector like LAYER/PROPERTIES, so the command line and the ribbon open the
  // panel without the router ever seeing the word. It closes the annotation run
  // (typefaces are an annotation property) without moving the layer run above it.
  { id: 'FONTS', labelKey: 'resources.fonts', icon: 'font', group: 'annotate' },
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
  // ABOUT (AutoCAD 2026) is a chrome-local selector, not a router command: typing
  // it opens the same host-owned About dialog the File tab's About action opens.
  // It is appended at the very end so no shipped row moves; the `app` glyph is
  // the ribbon entry's own mark for the same dialog.
  { id: 'ABOUT', labelKey: 'file.about', icon: 'app', group: 'file' },
  // CLEANSCREENON (AutoCAD 2026) is a chrome-local selector too: typing it sets
  // the same clean-screen state the File tab's toggle flips (it sets, never
  // toggles), sending nothing to the router. Appended last so no shipped row
  // moves; it reuses the `cleanScreen` glyph of the toggle it shares state with.
  { id: 'CLEANSCREENON', labelKey: 'status.clean_screen_on', icon: 'cleanScreen', group: 'view' },
  // CLEANSCREENOFF (AutoCAD 2026) is the leaving half of the same pair: typing it
  // clears the state CLEANSCREENON set and the File tab's toggle flips (it sets,
  // never toggles), sending nothing to the router. Appended last so no shipped
  // row moves; it reuses the `cleanScreen` glyph and the same shared state.
  { id: 'CLEANSCREENOFF', labelKey: 'status.clean_screen_off', icon: 'cleanScreen', group: 'view' },
  // PROPERTIESCLOSE (AutoCAD 2026) closes the properties window the palette's own
  // close button closes, sending nothing to the router. It is appended last so no
  // shipped row moves, and it reuses the `close` glyph and the `properties.close`
  // label the window's own close control already uses.
  { id: 'PROPERTIESCLOSE', labelKey: 'properties.close', icon: 'close', group: 'annotate' },
  // LAYERCLOSE (AutoCAD 2026) is the layers palette's own close word: typing it
  // closes the same palette the palette's close control closes, sending nothing
  // to the router. Appended last so no shipped row moves, and it reuses the
  // `close` glyph and the `layers.close` label that window already uses.
  { id: 'LAYERCLOSE', labelKey: 'layers.close', icon: 'close', group: 'annotate' },
  // MODEL (AutoCAD 2026) is the payload-free "switch to the Model tab" selector.
  // The real affordance is the drawing-space strip's existing Model tab, so this
  // is the catalog/command-line row only — no duplicate ribbon button. It is
  // appended last so no shipped row moves, and it reuses the `drawing` glyph.
  { id: 'MODEL', labelKey: 'layout.tab.model', icon: 'drawing', group: 'view' },
  // PLAN (AutoCAD 2026) is the typed "orthographic view of the XY plane". v1 is
  // the top/plan 2D view only: this build has no UCS model, so AutoCAD's
  // UCS/World/Current orientation options are NOT IMPLEMENTED and the command
  // takes no argument. It is the payload-free `CommandId::PlanView` router
  // selector (`cad_command`), not the payload-carrying `STANDARDVIEW` lane, so
  // it is a real name the command line admits. Appended last so no shipped row
  // moves, and it reuses the `view` glyph the other view rows use.
  { id: 'PLAN', labelKey: 'view.plan', icon: 'view', group: 'view' },
  // SELECTSIMILAR (AutoCAD 2026) is the **type-based** sibling of SELECTALL: it
  // takes no payload, needs a non-empty current selection, and adds every
  // visible model-space object of the same geometry kind to that selection.
  // v1 does NOT match attributes (layer/colour/linetype/lineweight/
  // transparency), so nothing here may imply a property filter. Appended last so
  // no shipped row moves, and it reuses the `selectAll` glyph because it is the
  // same "grow the selection" gesture.
  { id: 'SELECTSIMILAR', labelKey: 'draw.kind.selectsimilar', icon: 'selectAll', group: 'modify' },
  // REVCLOUD (AutoCAD 2026) is the **selection-based Object mode** v1: the one
  // selected closed straight-segment polyline is the whole input, and every edge
  // becomes one outward arc bulge. It is a router command (`CommandPayload::None`),
  // so nothing is picked interactively; the one-object rule lives in
  // `SELECTION_REQUIREMENTS`. There is no rectangular/polygonal draw mode and no
  // `Arc length` option in v1, so nothing here may imply a capture. Appended last
  // so no shipped row moves, and it reuses the `polyline` glyph: the output is a
  // polyline.
  { id: 'REVCLOUD', labelKey: 'draw.kind.revcloud', icon: 'polyline', group: 'draw' },
  // REVERSE (AutoCAD 2026) reverses the vertex/direction order of the selected
  // LINE / LWPOLYLINE / SPLINE. It is selection-based (`CommandPayload::None`),
  // so it is a payload-free router command with no capture: the one-or-more
  // selected objects are the whole input. v1 does not offer AutoCAD's
  // interactive pick prompt or its helix/3D variants. Appended last so no
  // shipped row moves; the `mirror` glyph is the closest direction glyph in the
  // icon set.
  { id: 'REVERSE', labelKey: 'draw.kind.reverse', icon: 'mirror', group: 'modify' },
  // PEDIT (AutoCAD 2026) is the **declared v1 subset** CLOSE / OPEN / REVERSE,
  // typed as the command's own argument (`PEDIT CLOSE`) because this build has
  // no in-command option prompt. The one selected model-space LWPOLYLINE is the
  // whole input; there is no interactive "Select polyline" prompt and no
  // `Multiple` mode. Join, Width/Halfwidth, Fit, Spline, Decurve, Ltype gen,
  // Undo, Edit vertex and the PEDITACCEPT conversion prompt are NOT
  // IMPLEMENTED, so the hint names the three options and nothing more. Appended
  // last so no shipped row moves; `polyline` is the output-kind glyph.
  { id: 'PEDIT', labelKey: 'draw.kind.pedit', icon: 'polyline', group: 'modify' },
  // OOPS (AutoCAD 2026) restores the objects removed by the last ERASE. It is
  // selection-free and payload-free, so it is a plain router command with no
  // capture and no selection rule (gating count 0). v1 records ERASE only —
  // AutoCAD also restores BLOCK/WBLOCK removals, which this build does not.
  // Appended last so no shipped row moves; `undo` is the closest restore glyph.
  { id: 'OOPS', labelKey: 'draw.kind.oops', icon: 'undo', group: 'modify' },
  // OVERKILL (AutoCAD 2026) v1 deletes **exact duplicate** selected entities.
  // Selection-based and payload-free; the two-object minimum lives in
  // `SELECTION_REQUIREMENTS`. Partial overlaps, collinear merges, arc-over-circle
  // replacement and a tolerance option are NOT IMPLEMENTED, so nothing here may
  // imply them. Appended last; `erase` is the closest removal glyph.
  { id: 'OVERKILL', labelKey: 'draw.kind.overkill', icon: 'erase', group: 'modify' },
  // BREAKATPOINT (AutoCAD 2026) is the **interactive** single-point split: pick
  // the object, then the break point (projected onto it). It is a tool selector
  // (`BeginDraw`), not a selection-based instant command, so `gating.ts` gives it
  // no selection precondition. Only LINE, ARC and open polylines are supported;
  // CIRCLE/closed polylines and SPLINEs are named refusals. Appended last; the
  // `trim` glyph matches BREAK's family.
  { id: 'BREAKATPOINT', labelKey: 'draw.kind.breakatpoint', icon: 'trim', group: 'modify' },
  { id: 'WIPEOUT', labelKey: 'draw.kind.wipeout', icon: 'erase', group: 'draw' },
  // AUDIT (AutoCAD 2026) is the detect-only drawing audit: it reports every
  // stored entity that fails the database's own validity check and repairs
  // nothing (AutoCAD's "corrects some errors" half is NOT IMPLEMENTED). It is a
  // payload-free router command that needs an open drawing, so it is appended
  // last — no shipped row moves — and reuses the `diagnostics` glyph, the same
  // report the File tab's Diagnostics entry opens.
  { id: 'AUDIT', labelKey: 'audit.title', icon: 'diagnostics', group: 'file' },
  // LIST (inquiry): selection-based property report; read-only and Viewer-legal.
  { id: 'LIST', labelKey: 'draw.kind.list', icon: 'properties', group: 'annotate' },
  // MASSPROP (AutoCAD 2026) is the read-only mass-properties report of the
  // current selection (closed planar CIRCLE / full ELLIPSE / closed POLYLINE on
  // one plane). Moments of inertia, principal axes, volume/density and `.mpr`
  // output are NOT implemented; the hint and the host refusal say so.
  { id: 'MASSPROP', labelKey: 'draw.kind.massprop', icon: 'properties', group: 'annotate' },
  // STATUS (inquiry): drawing statistics/modes/extents; document-scoped, read-only.
  { id: 'STATUS', labelKey: 'draw.kind.status', icon: 'diagnostics', group: 'view' },
  // FS (Express tool): command-line only in AutoCAD; direct-touch selection grower.
  { id: 'FS', labelKey: 'draw.kind.fs', icon: 'selectAll', group: 'modify' },
  // HATCHGENERATEBOUNDARY (AutoCAD 2026) creates one non-associative closed
  // polyline per loop of the selected hatch(es), on each hatch's own layer. The
  // selection rule is one hatch (`SELECTION_REQUIREMENTS`); there is no pick
  // prompt and no option prompt. Appended last so no shipped row moves, and it
  // reuses the `hatch` glyph because it belongs to the hatch family. It lives in
  // the Annotate tab, the way AutoCAD exposes it from the Hatch context.
  { id: 'HATCHGENERATEBOUNDARY', labelKey: 'draw.kind.hatchgenerateboundary', icon: 'hatch', group: 'annotate' },
  // HATCHTOBACK (AutoCAD 2026) sends every model-space hatch behind every other
  // object by rewriting the stored draw order. It takes no selection and needs an
  // open drawing (`DOCUMENT_SCOPED_COMMANDS`); paper-space/block hatches are not
  // reordered. Appended after the shipped rows; it reuses the `hatch` glyph.
  { id: 'HATCHTOBACK', labelKey: 'draw.kind.hatchtoback', icon: 'hatch', group: 'modify' },
  // TEXTTOFRONT (AutoCAD 2026) is the selection-based draw-order command: it
  // moves the selected text/leaders/dimensions above every other model-space
  // object. It reads the **current selection** (a declared divergence from the
  // interactive Select objects prompt), so it is an ordinary router button gated
  // by `SELECTION_REQUIREMENTS`, never a capture. It uses the generic `properties`
  // glyph (the icon set has no draw-order mark); the dedicated `text`/`mtext`
  // marks stay owned by the text commands. Paper/block draw order and the
  // all-objects form are NOT IMPLEMENTED.
  { id: 'TEXTTOFRONT', labelKey: 'draw.kind.texttofront', icon: 'properties', group: 'modify' },
  // GRADIENT (AutoCAD 2026) fills the **current selection** with the session
  // gradient parameters (two-colour linear default). It is a router command with
  // no payload and no capture, so its enablement is the one-object selection
  // rule in `gating.ts`; it is a canonical selector of its own, never an alias of
  // HATCH. There is no gradient preview, no `-GRADIENT` option prompt and no
  // .pat handling. Appended last so no shipped row moves.
  { id: 'GRADIENT', labelKey: 'draw.kind.gradient', icon: 'hatch', group: 'draw' },
  // HATCHEDIT (AutoCAD 2026) is the typed v1 hatch edit: exactly one selected
  // model-space hatch plus `HATCHEDIT ANGLE <deg>`, `SCALE <factor>`, `SOLID` or
  // `GRADIENT`. The bare name is a named `hatchedit.needs_option` refusal, and
  // pattern name / colour / properties / draw order / boundary add-remove /
  // associativity are NOT implemented. Router command, selection-gated.
  { id: 'HATCHEDIT', labelKey: 'draw.kind.hatchedit', icon: 'hatch', group: 'annotate' },
  // The 2026-11-07 selection/filter/find/visibility batch. QSELECT/FILTER/
  // GETSEL/-FIND/FIND take a typed argument and have no ribbon button (a button
  // would have to invent the filter or the search text); SSX/SELECTCOUNT and
  // the hide-isolate trio are payload-free and appear in the Modify panels.
  // QSELECT/FILTER/GETSEL replace or grow the selection from a **data** query
  // (model space, layer state does not filter). SSX/SELECTCOUNT compare the
  // first selection's five properties. The find pair searches TEXT/MTEXT/
  // MULTILEADER text; FIND can replace in one transaction. HIDEOBJECTS/
  // ISOLATEOBJECTS/UNISOLATEOBJECTS are session-only visibility state.
  { id: 'QSELECT', labelKey: 'draw.kind.qselect', icon: 'selectAll', group: 'modify' },
  { id: 'FILTER', labelKey: 'draw.kind.filter', icon: 'selectAll', group: 'modify' },
  { id: 'GETSEL', labelKey: 'draw.kind.getsel', icon: 'selectAll', group: 'modify' },
  { id: 'SSX', labelKey: 'draw.kind.ssx', icon: 'selectAll', group: 'modify' },
  { id: 'SELECTCOUNT', labelKey: 'draw.kind.selectcount', icon: 'properties', group: 'modify' },
  { id: '-FIND', labelKey: 'draw.kind.findreport', icon: 'search', group: 'annotate' },
  { id: 'FIND', labelKey: 'draw.kind.find', icon: 'search', group: 'annotate' },
  { id: 'HIDEOBJECTS', labelKey: 'draw.kind.hideobjects', icon: 'clearSelection', group: 'modify' },
  { id: 'ISOLATEOBJECTS', labelKey: 'draw.kind.isolateobjects', icon: 'layers', group: 'modify' },
  { id: 'UNISOLATEOBJECTS', labelKey: 'draw.kind.unisolateobjects', icon: 'layers', group: 'modify' },
  // The 2026-11-07 utility/named-object batch. PURGE/RENAME/SETVAR/TIME/
  // DBLIST/TREESTAT/MREDO/MULTIPLE are real router selectors; the `-` spellings
  // of PURGE/RENAME are aliases and therefore have no catalog row of their own.
  // The typed forms carry their argument on the command line, so a button only
  // starts the (argument-less) selector and the host refuses a missing argument
  // by name rather than inventing one.
  { id: 'PURGE', labelKey: 'draw.kind.purge', icon: 'erase', group: 'file' },
  { id: 'RENAME', labelKey: 'draw.kind.rename', icon: 'properties', group: 'modify' },
  { id: 'SETVAR', labelKey: 'draw.kind.setvar', icon: 'properties', group: 'view' },
  { id: 'TIME', labelKey: 'draw.kind.time', icon: 'diagnostics', group: 'view' },
  { id: 'DBLIST', labelKey: 'draw.kind.dblist', icon: 'properties', group: 'annotate' },
  { id: 'TREESTAT', labelKey: 'draw.kind.treestat', icon: 'diagnostics', group: 'view' },
  { id: 'MREDO', labelKey: 'draw.kind.mredo', icon: 'redo', group: 'modify' },
  { id: 'MULTIPLE', labelKey: 'draw.kind.multiple', icon: 'copy', group: 'modify' },
  // The 2026-11-07 draw/modify batch. DONUT/SOLID/HELIX/MLINE/MLSTYLE are typed
  // creation forms (a bare name is refused by the host, never defaulted);
  // TREX/EXTRIM/MPEDIT/DRAWORDER read the current selection. They are listed in
  // the command catalog so the command line can complete them, and the four
  // selection commands also get ribbon buttons (see `ribbon/ribbon-modify.ts`).
  // `-OVERKILL` is deliberately absent: it is an alias of `OVERKILL`, not a
  // second command row.
  { id: 'DONUT', labelKey: 'draw.kind.donut', icon: 'circle', group: 'draw' },
  { id: 'SOLID', labelKey: 'draw.kind.solid', icon: 'rectang', group: 'draw' },
  { id: 'HELIX', labelKey: 'draw.kind.helix', icon: 'spline', group: 'draw' },
  { id: 'MLINE', labelKey: 'draw.kind.mline', icon: 'line', group: 'draw' },
  { id: 'MLSTYLE', labelKey: 'draw.kind.mlstyle', icon: 'properties', group: 'draw' },
  { id: 'TREX', labelKey: 'draw.kind.trex', icon: 'trim', group: 'modify' },
  { id: 'EXTRIM', labelKey: 'draw.kind.extrim', icon: 'trim', group: 'modify' },
  { id: 'MPEDIT', labelKey: 'draw.kind.mpedit', icon: 'polyline', group: 'modify' },
  { id: 'DRAWORDER', labelKey: 'draw.kind.draworder', icon: 'properties', group: 'modify' },
  // The 2026-11-07 layout/viewport batch. `LAYOUT` takes a typed option
  // (LIST/NEW/RENAME/DELETE/SET); `VPORTS`/`VPCLIP`/`VPSCALE` take their typed
  // argument on the command line. `VPMAX`/`VPMIN` are payload-free session
  // commands. All layout/viewport **writes** are named refusals in this build
  // (cad-db has no post-import write API), so these rows advertise the names the
  // router admits, not a promise that a write will land.
  { id: 'LAYOUT', labelKey: 'draw.kind.layout', icon: 'drawing', group: 'view' },
  { id: 'LAYOUTMERGE', labelKey: 'draw.kind.layoutmerge', icon: 'copy', group: 'view' },
  { id: 'LAYOUTWIZARD', labelKey: 'draw.kind.layoutwizard', icon: 'properties', group: 'view' },
  { id: 'VPORTS', labelKey: 'draw.kind.vports', icon: 'rectang', group: 'view' },
  { id: 'VPMAX', labelKey: 'draw.kind.vpmax', icon: 'view', group: 'view' },
  { id: 'VPMIN', labelKey: 'draw.kind.vpmin', icon: 'view', group: 'view' },
  { id: 'VPCLIP', labelKey: 'draw.kind.vpclip', icon: 'rectang', group: 'view' },
  { id: 'VPSCALE', labelKey: 'draw.kind.vpscale', icon: 'properties', group: 'view' },
  { id: 'LAYVPI', labelKey: 'draw.kind.layvpi', icon: 'layers', group: 'annotate' },

  // The 2026-11-07 view batch. REDRAW/REGEN and their ALL forms are payload-free
  // refresh/regeneration stamps; VIEWBACK/VIEWFORWARD walk the per-viewport view
  // stack. VIEW/VIEWGO/VPOINT are typed-argument selectors: a button cannot
  // invent the name or the direction, so their command-line grammar lives in
  // `gating.ts` and the catalog row only makes them discoverable.
  { id: 'REDRAW', labelKey: 'draw.kind.redraw', icon: 'view', group: 'view' },
  { id: 'REDRAWALL', labelKey: 'draw.kind.redrawall', icon: 'view', group: 'view' },
  { id: 'REGEN', labelKey: 'draw.kind.regen', icon: 'view', group: 'view' },
  { id: 'REGENALL', labelKey: 'draw.kind.regenall', icon: 'view', group: 'view' },
  { id: 'VIEW', labelKey: 'draw.kind.view', icon: 'view', group: 'view' },
  { id: 'VIEWGO', labelKey: 'draw.kind.viewgo', icon: 'viewTop', group: 'view' },
  { id: 'VIEWBACK', labelKey: 'draw.kind.viewback', icon: 'zoomPrevious', group: 'view' },
  { id: 'VIEWFORWARD', labelKey: 'draw.kind.viewforward', icon: 'view', group: 'view' },
  { id: 'VPOINT', labelKey: 'draw.kind.vpoint', icon: 'axes', group: 'view' },

  // The 2026-11-07 extras batch. `-PAN`, `REGEN3` and `-LINETYPE` are aliases
  // of PAN/REGEN/LINETYPE (no rows of their own); SKETCH/MSTRETCH take their
  // complete input from a host/IPC payload, so the command line only advertises
  // them. TRACE/MOCORO/VIEWRES/VSCURRENT/LINETYPE have typed grammars in
  // `gating.ts`.
  { id: 'SKETCH', labelKey: 'draw.kind.sketch', icon: 'spline', group: 'draw' },
  { id: 'TRACE', labelKey: 'draw.kind.trace', icon: 'line', group: 'draw' },
  { id: 'MOCORO', labelKey: 'draw.kind.mocoro', icon: 'copy', group: 'modify' },
  { id: 'MSTRETCH', labelKey: 'draw.kind.mstretch', icon: 'stretch', group: 'modify' },
  { id: 'VIEWRES', labelKey: 'draw.kind.viewres', icon: 'view', group: 'view' },
  { id: 'VSCURRENT', labelKey: 'draw.kind.vscurrent', icon: 'viewTop', group: 'view' },
  { id: 'LINETYPE', labelKey: 'draw.kind.linetype', icon: 'properties', group: 'properties' },

  // The 2026-11-09 pilot batch of payload-free command-line names. QTEXT/FILL/
  // TFRAMES/REGENAUTO/ISODRAFT are session-only tokens with no renderer effect:
  // the host records the word but this build applies no display change, so no row
  // here may promise one. BCOUNT/COUNT/EDITTIME/JULIAN/XLIST are read-only
  // reports over the open drawing. None takes a parameter or a capability gate;
  // all need a drawing open (`gating.ts`). Appended last so no shipped row moves,
  // and the report rows reuse the existing `properties`/`diagnostics` marks while
  // the display toggles reuse `eye`/`reference`/`view`/`axes`.
  { id: 'QTEXT', labelKey: 'draw.kind.qtext', icon: 'terminal', group: 'view' },
  { id: 'FILL', labelKey: 'draw.kind.fill', icon: 'eye', group: 'view' },
  { id: 'TFRAMES', labelKey: 'draw.kind.tframes', icon: 'reference', group: 'view' },
  { id: 'REGENAUTO', labelKey: 'draw.kind.regenauto', icon: 'view', group: 'view' },
  { id: 'ISODRAFT', labelKey: 'draw.kind.isodraft', icon: 'axes', group: 'view' },
  { id: 'BCOUNT', labelKey: 'draw.kind.bcount', icon: 'properties', group: 'annotate' },
  { id: 'COUNT', labelKey: 'draw.kind.count', icon: 'selectAll', group: 'modify' },
  { id: 'EDITTIME', labelKey: 'draw.kind.edittime', icon: 'diagnostics', group: 'view' },
  { id: 'JULIAN', labelKey: 'draw.kind.julian', icon: 'diagnostics', group: 'view' },
  { id: 'XLIST', labelKey: 'draw.kind.xlist', icon: 'properties', group: 'annotate' },

  // The 2026-11-09 Wave 2 batch: ten session tokens, eight read-only reports and
  // twelve named refusals. Tokens flip session state only and change no renderer
  // output; reports read data this build really has and name anything absent;
  // refusals answer with a stable `Unsupported` reason and must never be
  // presented as working features. Appended last so no shipped row moves.
  { id: 'DRAGMODE', labelKey: 'draw.kind.dragmode', icon: 'move', group: 'view' },
  { id: 'APERTURE', labelKey: 'draw.kind.aperture', icon: 'snap', group: 'view' },
  { id: 'ISOPLANE', labelKey: 'draw.kind.isoplane', icon: 'axes', group: 'view' },
  { id: 'HIGHLIGHTNEW', labelKey: 'draw.kind.highlightnew', icon: 'alert', group: 'view' },
  { id: 'TRAYSETTINGS', labelKey: 'draw.kind.traysettings', icon: 'settings', group: 'view' },
  { id: 'TEXTSCR', labelKey: 'draw.kind.textscr', icon: 'terminal', group: 'view' },
  { id: 'GRAPHSCR', labelKey: 'draw.kind.graphscr', icon: 'drawing', group: 'view' },
  { id: 'FINDINCLUDE', labelKey: 'draw.kind.findinclude', icon: 'search', group: 'modify' },
  { id: 'FINDMATCH', labelKey: 'draw.kind.findmatch', icon: 'search', group: 'modify' },
  { id: 'REVCLOUDPROPERTIES', labelKey: 'draw.kind.revcloudproperties', icon: 'properties', group: 'draw' },
  { id: 'SYSVARMONITOR', labelKey: 'draw.kind.sysvarmonitor', icon: 'settings', group: 'properties' },
  { id: 'SYSVDLG', labelKey: 'draw.kind.sysvdlg', icon: 'settings', group: 'properties' },
  { id: 'VTOPTIONS', labelKey: 'draw.kind.vtoptions', icon: 'view', group: 'view' },
  { id: 'ALIASEDIT', labelKey: 'draw.kind.aliasedit', icon: 'terminal', group: 'properties' },
  { id: 'WHOHAS', labelKey: 'draw.kind.whohas', icon: 'info', group: 'view' },
  { id: 'ACADINFO', labelKey: 'draw.kind.acadinfo', icon: 'info', group: 'view' },
  { id: 'PTYPE', labelKey: 'draw.kind.ptype', icon: 'point', group: 'properties' },
  { id: 'SCALELISTEDIT', labelKey: 'draw.kind.scalelistedit', icon: 'scale', group: 'properties' },
  { id: 'HYPERLINKOPTIONS', labelKey: 'draw.kind.hyperlinkoptions', icon: 'alert', group: 'view' },
  { id: 'INPUTSEARCHOPTIONS', labelKey: 'draw.kind.inputsearchoptions', icon: 'search', group: 'view' },
  { id: '-INPUTSEARCHOPTIONS', labelKey: 'draw.kind.dashinputsearchoptions', icon: 'search', group: 'view' },
  { id: 'SIGVALIDATE', labelKey: 'draw.kind.sigvalidate', icon: 'check', group: 'view' },
  { id: 'BASE', labelKey: 'draw.kind.base', icon: 'point', group: 'draw' },
  { id: 'LIMITS', labelKey: 'draw.kind.limits', icon: 'grid', group: 'view' },
  { id: 'VIEWPLOTDETAILS', labelKey: 'draw.kind.viewplotdetails', icon: 'printer', group: 'view' },
  { id: 'VPLAYER', labelKey: 'draw.kind.vplayer', icon: 'layers', group: 'view' },
  { id: 'MVIEW', labelKey: 'draw.kind.mview', icon: 'view', group: 'view' },
  { id: 'PSBSCALE', labelKey: 'draw.kind.psbscale', icon: 'scale', group: 'view' },
  { id: 'PSTSCALE', labelKey: 'draw.kind.pstscale', icon: 'scale', group: 'view' },
  { id: 'MVSETUP', labelKey: 'draw.kind.mvsetup', icon: 'drawing', group: 'view' },

  // The 2026-11-09 Wave 3 batch: eight reports/session commands and twenty-two
  // named refusals. MSPACE/PSPACE reuse the existing MODEL/SPACE session path;
  // STYLE/-STYLE, MLEADERSTYLE and BLOCK? are read-only table reports;
  // DIMREASSOC/DIMREASSOCIATE are non-associative no-ops. The rest answer with a
  // stable `Unsupported` reason and must never be presented as working features.
  // Appended last so no shipped row moves. BLOCK?'s label slug is
  // `block_list` because `?` is not a legal i18n key character.
  { id: 'MSPACE', labelKey: 'draw.kind.mspace', icon: 'view', group: 'view' },
  { id: 'PSPACE', labelKey: 'draw.kind.pspace', icon: 'drawing', group: 'view' },
  { id: 'STYLE', labelKey: 'draw.kind.style', icon: 'font', group: 'properties' },
  { id: '-STYLE', labelKey: 'draw.kind.dashstyle', icon: 'font', group: 'properties' },
  { id: 'MLEADERSTYLE', labelKey: 'draw.kind.mleaderstyle', icon: 'properties', group: 'annotate' },
  { id: 'BLOCK?', labelKey: 'draw.kind.block_list', icon: 'properties', group: 'properties' },
  { id: 'DIMREASSOC', labelKey: 'draw.kind.dimreassoc', icon: 'dimlinear', group: 'annotate' },
  { id: 'DIMREASSOCIATE', labelKey: 'draw.kind.dimreassociate', icon: 'dimlinear', group: 'annotate' },
  { id: 'DIMJOGGED', labelKey: 'draw.kind.dimjogged', icon: 'dimlinear', group: 'annotate' },
  { id: 'DIMINSPECT', labelKey: 'draw.kind.diminspect', icon: 'dimlinear', group: 'annotate' },
  { id: '-DIMINSPECT', labelKey: 'draw.kind.dashdiminspect', icon: 'dimlinear', group: 'annotate' },
  { id: 'ANNORESET', labelKey: 'draw.kind.annoreset', icon: 'font', group: 'annotate' },
  { id: 'ANNOUPDATE', labelKey: 'draw.kind.annoupdate', icon: 'font', group: 'annotate' },
  { id: 'OBJECTSCALE', labelKey: 'draw.kind.objectscale', icon: 'scale', group: 'properties' },
  { id: '-OBJECTSCALE', labelKey: 'draw.kind.dashobjectscale', icon: 'scale', group: 'properties' },
  { id: 'QLATTACH', labelKey: 'draw.kind.qlattach', icon: 'properties', group: 'annotate' },
  { id: 'QLATTACHSET', labelKey: 'draw.kind.qlattachset', icon: 'properties', group: 'annotate' },
  { id: 'QLDETACHSET', labelKey: 'draw.kind.qldetachset', icon: 'properties', group: 'annotate' },
  { id: 'ARRAYCLOSE', labelKey: 'draw.kind.arrayclose', icon: 'arrayrect', group: 'modify' },
  { id: 'ARRAYEDIT', labelKey: 'draw.kind.arrayedit', icon: 'arrayrect', group: 'modify' },
  { id: 'BEDIT', labelKey: 'draw.kind.bedit', icon: 'drawing', group: 'properties' },
  { id: '-BEDIT', labelKey: 'draw.kind.dashbedit', icon: 'drawing', group: 'properties' },
  { id: 'BCLOSE', labelKey: 'draw.kind.bclose', icon: 'drawing', group: 'properties' },
  { id: 'BCYCLEORDER', labelKey: 'draw.kind.bcycleorder', icon: 'drawing', group: 'properties' },
  { id: 'BGRIPSET', labelKey: 'draw.kind.bgripset', icon: 'drawing', group: 'properties' },
  { id: 'BESETTINGS', labelKey: 'draw.kind.besettings', icon: 'settings', group: 'properties' },
  { id: 'BTESTBLOCK', labelKey: 'draw.kind.btestblock', icon: 'drawing', group: 'properties' },
  { id: 'LAYERPALETTE', labelKey: 'draw.kind.layerpalette', icon: 'layers', group: 'view' },
  { id: 'CONSTRAINTBAR', labelKey: 'draw.kind.constraintbar', icon: 'alert', group: 'view' },
  { id: 'CONSTRAINTSETTINGS', labelKey: 'draw.kind.constraintsettings', icon: 'settings', group: 'view' },

  // The 2026-11-09 Wave 4 batch: the first extended batch with real entity
  // writes. The first twelve reuse the existing planners/write paths and are
  // Work-only; the last eighteen are stable named refusals and stay
  // Viewer-legal. Appended last so no shipped row moves.
  { id: 'CENTERMARK', labelKey: 'draw.kind.centermark', icon: 'circle', group: 'draw' },
  { id: 'CENTERLINE', labelKey: 'draw.kind.centerline', icon: 'line', group: 'draw' },
  { id: 'NCOPY', labelKey: 'draw.kind.ncopy', icon: 'copy', group: 'modify' },
  { id: 'COPYTOLAYER', labelKey: 'draw.kind.copytolayer', icon: 'layers', group: 'modify' },
  { id: '-COPYTOLAYER', labelKey: 'draw.kind.dashcopytolayer', icon: 'layers', group: 'modify' },
  { id: 'CDORDER', labelKey: 'draw.kind.cdorder', icon: 'arrayrect', group: 'modify' },
  { id: 'FLATTEN', labelKey: 'draw.kind.flatten', icon: 'move', group: 'modify' },
  { id: 'XPLODE', labelKey: 'draw.kind.xplode', icon: 'explode', group: 'modify' },
  { id: 'BTRIM', labelKey: 'draw.kind.btrim', icon: 'trim', group: 'modify' },
  { id: 'BEXTEND', labelKey: 'draw.kind.bextend', icon: 'extend', group: 'modify' },
  { id: 'CHANGE', labelKey: 'draw.kind.change', icon: 'properties', group: 'modify' },
  { id: 'CHPROP', labelKey: 'draw.kind.chprop', icon: 'properties', group: 'modify' },
  { id: 'BSCALE', labelKey: 'draw.kind.bscale', icon: 'scale', group: 'modify' },
  { id: 'MINSERT', labelKey: 'draw.kind.minsert', icon: 'arrayrect', group: 'modify' },
  { id: 'BLOCKREPLACE', labelKey: 'draw.kind.blockreplace', icon: 'copy', group: 'modify' },
  { id: 'BREPLACE', labelKey: 'draw.kind.breplace', icon: 'copy', group: 'modify' },
  { id: '-BREPLACE', labelKey: 'draw.kind.dashbreplace', icon: 'copy', group: 'modify' },
  { id: 'SPLINEDIT', labelKey: 'draw.kind.splinedit', icon: 'spline', group: 'modify' },
  { id: 'MLEDIT', labelKey: 'draw.kind.mledit', icon: 'line', group: 'modify' },
  { id: '-MLEDIT', labelKey: 'draw.kind.dashmledit', icon: 'line', group: 'modify' },
  { id: 'BURST', labelKey: 'draw.kind.burst', icon: 'explode', group: 'modify' },
  { id: 'CLIP', labelKey: 'draw.kind.clip', icon: 'reference', group: 'modify' },
  { id: 'CLIPIT', labelKey: 'draw.kind.clipit', icon: 'reference', group: 'modify' },
  { id: 'REGION', labelKey: 'draw.kind.region', icon: 'polygon', group: 'draw' },
  { id: 'ADDSELECTED', labelKey: 'draw.kind.addselected', icon: 'selectAll', group: 'modify' },
  { id: 'WBLOCK', labelKey: 'draw.kind.wblock', icon: 'save', group: 'file' },
  { id: '-WBLOCK', labelKey: 'draw.kind.dashwblock', icon: 'save', group: 'file' },
  { id: 'BSAVE', labelKey: 'draw.kind.bsave', icon: 'save', group: 'file' },
  { id: 'BSAVEAS', labelKey: 'draw.kind.bsaveas', icon: 'save', group: 'file' },
  { id: 'CHSPACE', labelKey: 'draw.kind.chspace', icon: 'layers', group: 'modify' },

  // The 2026-11-09 Wave 5 batch: the attribute and external-reference surfaces.
  // ATTDISP is a session token; BATTMAN/BATTORDER/XREF/-XREF/
  // EXTERNALREFERENCES(/CLOSE) are read-only reports; the rest are stable named
  // refusals. Appended last so no shipped row moves.
  { id: 'ATTDISP', labelKey: 'draw.kind.attdisp', icon: 'properties', group: 'view' },
  { id: 'BATTMAN', labelKey: 'draw.kind.battman', icon: 'properties', group: 'properties' },
  { id: 'BATTORDER', labelKey: 'draw.kind.battorder', icon: 'properties', group: 'properties' },
  { id: 'XREF', labelKey: 'draw.kind.xref', icon: 'reference', group: 'file' },
  { id: '-XREF', labelKey: 'draw.kind.dashxref', icon: 'reference', group: 'file' },
  { id: 'EXTERNALREFERENCES', labelKey: 'draw.kind.externalreferences', icon: 'reference', group: 'file' },
  { id: 'EXTERNALREFERENCESCLOSE', labelKey: 'draw.kind.externalreferencesclose', icon: 'reference', group: 'file' },
  { id: 'ATTDEF', labelKey: 'draw.kind.attdef', icon: 'properties', group: 'draw' },
  { id: '-ATTDEF', labelKey: 'draw.kind.dashattdef', icon: 'properties', group: 'draw' },
  { id: 'ATTEDIT', labelKey: 'draw.kind.attedit', icon: 'properties', group: 'modify' },
  { id: '-ATTEDIT', labelKey: 'draw.kind.dashattedit', icon: 'properties', group: 'modify' },
  { id: 'ATTIPEDIT', labelKey: 'draw.kind.attipedit', icon: 'properties', group: 'modify' },
  { id: 'ATTIN', labelKey: 'draw.kind.attin', icon: 'open', group: 'file' },
  { id: 'ATTOUT', labelKey: 'draw.kind.attout', icon: 'save', group: 'file' },
  { id: 'ATTEXT', labelKey: 'draw.kind.attext', icon: 'save', group: 'file' },
  { id: '-ATTEXT', labelKey: 'draw.kind.dashattext', icon: 'save', group: 'file' },
  { id: 'EATTEDIT', labelKey: 'draw.kind.eattedit', icon: 'properties', group: 'modify' },
  { id: 'EATTEXT', labelKey: 'draw.kind.eattext', icon: 'save', group: 'file' },
  { id: '-EATTEXT', labelKey: 'draw.kind.dasheattext', icon: 'save', group: 'file' },
  { id: 'ATTREDEF', labelKey: 'draw.kind.attredef', icon: 'properties', group: 'modify' },
  { id: 'ATTSYNC', labelKey: 'draw.kind.attsync', icon: 'properties', group: 'modify' },
  { id: 'GATTE', labelKey: 'draw.kind.gatte', icon: 'properties', group: 'modify' },
  { id: 'XDATA', labelKey: 'draw.kind.xdata', icon: 'properties', group: 'modify' },
  { id: 'XBIND', labelKey: 'draw.kind.xbind', icon: 'reference', group: 'file' },
  { id: '-XBIND', labelKey: 'draw.kind.dashxbind', icon: 'reference', group: 'file' },
  { id: 'XATTACH', labelKey: 'draw.kind.xattach', icon: 'reference', group: 'file' },
  { id: 'ATTACH', labelKey: 'draw.kind.attach', icon: 'reference', group: 'file' },
  { id: '-ATTACH', labelKey: 'draw.kind.dashattach', icon: 'reference', group: 'file' },
  { id: 'XOPEN', labelKey: 'draw.kind.xopen', icon: 'open', group: 'file' },
  { id: 'CLASSICXREF', labelKey: 'draw.kind.classicxref', icon: 'reference', group: 'file' },

  // The 2026-11-09 Wave 6 batch: the file / plot / export surfaces. DWGLOG,
  // LOGFILEON and LOGFILEOFF are one session token (no log file is written);
  // DWGPROPS is a read-only document report, DWFFORMAT/EXPORTPDF are read-only
  // capability reports; the rest are stable named refusals. Appended last so no
  // shipped row moves.
  { id: 'DWGLOG', labelKey: 'draw.kind.dwglog', icon: 'save', group: 'file' },
  { id: 'LOGFILEON', labelKey: 'draw.kind.logfileon', icon: 'save', group: 'file' },
  { id: 'LOGFILEOFF', labelKey: 'draw.kind.logfileoff', icon: 'save', group: 'file' },
  { id: 'DWGPROPS', labelKey: 'draw.kind.dwgprops', icon: 'properties', group: 'properties' },
  { id: 'DWFFORMAT', labelKey: 'draw.kind.dwfformat', icon: 'printer', group: 'file' },
  { id: 'EXPORTPDF', labelKey: 'draw.kind.exportpdf', icon: 'printer', group: 'file' },
  { id: 'SAVEALL', labelKey: 'draw.kind.saveall', icon: 'save', group: 'file' },
  { id: 'QNEW', labelKey: 'draw.kind.qnew', icon: 'new', group: 'file' },
  { id: 'REVERT', labelKey: 'draw.kind.revert', icon: 'open', group: 'file' },
  { id: 'CLOSEALL', labelKey: 'draw.kind.closeall', icon: 'close', group: 'file' },
  { id: 'CLOSEALLOTHER', labelKey: 'draw.kind.closeallother', icon: 'close', group: 'file' },
  { id: 'QQUIT', labelKey: 'draw.kind.qquit', icon: 'close', group: 'file' },
  { id: 'QUIT', labelKey: 'draw.kind.quit', icon: 'close', group: 'file' },
  { id: 'PROPULATE', labelKey: 'draw.kind.propulate', icon: 'matchprop', group: 'modify' },
  { id: 'PLOTTERMANAGER', labelKey: 'draw.kind.plottermanager', icon: 'printer', group: 'file' },
  { id: 'STYLESMANAGER', labelKey: 'draw.kind.stylesmanager', icon: 'printer', group: 'file' },
  { id: 'EXPORTSETTINGS', labelKey: 'draw.kind.exportsettings', icon: 'settings', group: 'file' },
  { id: 'PREVIEW', labelKey: 'draw.kind.preview', icon: 'eye', group: 'file' },
  { id: 'EXPORTLAYOUT', labelKey: 'draw.kind.exportlayout', icon: 'printer', group: 'file' },
  { id: 'PLOTSTAMP', labelKey: 'draw.kind.plotstamp', icon: 'printer', group: 'file' },
  { id: '-PLOTSTAMP', labelKey: 'draw.kind.dashplotstamp', icon: 'printer', group: 'file' },
  { id: '-PLOT', labelKey: 'draw.kind.dashplot', icon: 'printer', group: 'file' },
  { id: '-EXPORT', labelKey: 'draw.kind.dashexport', icon: 'printer', group: 'file' },
  { id: 'ETRANSMIT', labelKey: 'draw.kind.etransmit', icon: 'save', group: 'file' },
  { id: '-ETRANSMIT', labelKey: 'draw.kind.dashetransmit', icon: 'save', group: 'file' },
  { id: 'ARCHIVE', labelKey: 'draw.kind.archive', icon: 'save', group: 'file' },
  { id: '-ARCHIVE', labelKey: 'draw.kind.dasharchive', icon: 'save', group: 'file' },
  { id: 'PUBLISH', labelKey: 'draw.kind.publish', icon: 'printer', group: 'file' },
  { id: '-PUBLISH', labelKey: 'draw.kind.dashpublish', icon: 'printer', group: 'file' },
  { id: 'AUTOPUBLISH', labelKey: 'draw.kind.autopublish', icon: 'printer', group: 'file' },

  // The 2026-11-09 Wave 7 batch: the dynamic-block / constraint surfaces.
  // BVSTATE and -BVSTATE are read-only reports of each selected INSERT's active
  // visibility state; the rest are stable named refusals. Appended last so no
  // shipped row moves.
  { id: 'BVSTATE', labelKey: 'draw.kind.bvstate', icon: 'properties', group: 'properties' },
  { id: '-BVSTATE', labelKey: 'draw.kind.dashbvstate', icon: 'properties', group: 'properties' },
  { id: 'BACTION', labelKey: 'draw.kind.baction', icon: 'app', group: 'modify' },
  { id: 'BACTIONBAR', labelKey: 'draw.kind.bactionbar', icon: 'app', group: 'modify' },
  { id: 'BACTIONSET', labelKey: 'draw.kind.bactionset', icon: 'app', group: 'modify' },
  { id: 'BACTIONTOOL', labelKey: 'draw.kind.bactiontool', icon: 'app', group: 'modify' },
  { id: 'BASSOCIATE', labelKey: 'draw.kind.bassociate', icon: 'app', group: 'modify' },
  { id: 'BAUTHORPALETTE', labelKey: 'draw.kind.bauthorpalette', icon: 'palette', group: 'modify' },
  { id: 'BAUTHORPALETTECLOSE', labelKey: 'draw.kind.bauthorpaletteclose', icon: 'palette', group: 'modify' },
  { id: 'BCONSTRUCTION', labelKey: 'draw.kind.bconstruction', icon: 'xline', group: 'modify' },
  { id: 'BCPARAMETER', labelKey: 'draw.kind.bcparameter', icon: 'settings', group: 'modify' },
  { id: '-BCPARAMETER', labelKey: 'draw.kind.dashbcparameter', icon: 'settings', group: 'modify' },
  { id: 'BPARAMETER', labelKey: 'draw.kind.bparameter', icon: 'settings', group: 'modify' },
  { id: 'BLOOKUPTABLE', labelKey: 'draw.kind.blookuptable', icon: 'properties', group: 'properties' },
  { id: 'BTABLE', labelKey: 'draw.kind.btable', icon: 'properties', group: 'properties' },
  { id: 'BVHIDE', labelKey: 'draw.kind.bvhide', icon: 'eyeOff', group: 'modify' },
  { id: 'BVSHOW', labelKey: 'draw.kind.bvshow', icon: 'eye', group: 'modify' },
  { id: 'RESETBLOCK', labelKey: 'draw.kind.resetblock', icon: 'settings', group: 'modify' },
  { id: 'AUTOCONSTRAIN', labelKey: 'draw.kind.autoconstrain', icon: 'settings', group: 'modify' },
  { id: 'DELCONSTRAINT', labelKey: 'draw.kind.delconstraint', icon: 'settings', group: 'modify' },
  { id: 'DIMCONSTRAINT', labelKey: 'draw.kind.dimconstraint', icon: 'dimlinear', group: 'annotate' },
  { id: 'GEOMCONSTRAINT', labelKey: 'draw.kind.geomconstraint', icon: 'settings', group: 'modify' },
  { id: 'DCALIGNED', labelKey: 'draw.kind.dcaligned', icon: 'dimaligned', group: 'annotate' },
  { id: 'DCANGULAR', labelKey: 'draw.kind.dcangular', icon: 'dimangular', group: 'annotate' },
  { id: 'DCCONVERT', labelKey: 'draw.kind.dcconvert', icon: 'dimlinear', group: 'annotate' },
  { id: 'DCDIAMETER', labelKey: 'draw.kind.dcdiameter', icon: 'dimdiameter', group: 'annotate' },
  { id: 'DCDISPLAY', labelKey: 'draw.kind.dcdisplay', icon: 'dimlinear', group: 'annotate' },
  { id: 'DCFORM', labelKey: 'draw.kind.dcform', icon: 'dimlinear', group: 'annotate' },
  { id: 'DCHORIZONTAL', labelKey: 'draw.kind.dchorizontal', icon: 'dimlinear', group: 'annotate' },
  { id: 'DCLINEAR', labelKey: 'draw.kind.dclinear', icon: 'dimlinear', group: 'annotate' },

  // The 2026-11-09 Wave 8 batch: two session tokens (ELEV, UCSICON) and eight
  // stable named refusals (UCS through RIBBON). Appended last so no shipped row
  // moves.
  { id: 'ELEV', labelKey: 'draw.kind.elev', icon: 'settings', group: 'view' },
  { id: 'UCSICON', labelKey: 'draw.kind.ucsicon', icon: 'axes', group: 'view' },
  { id: 'UCS', labelKey: 'draw.kind.ucs', icon: 'axes', group: 'view' },
  { id: 'UCSMAN', labelKey: 'draw.kind.ucsman', icon: 'axes', group: 'view' },
  { id: 'TABLET', labelKey: 'draw.kind.tablet', icon: 'pencil', group: 'view' },
  { id: 'TASKBAR', labelKey: 'draw.kind.taskbar', icon: 'app', group: 'view' },
  { id: 'REINIT', labelKey: 'draw.kind.reinit', icon: 'settings', group: 'view' },
  { id: 'SYSWINDOWS', labelKey: 'draw.kind.syswindows', icon: 'app', group: 'view' },
  { id: 'MENU', labelKey: 'draw.kind.menu', icon: 'menu', group: 'view' },
  { id: 'RIBBON', labelKey: 'draw.kind.ribbon', icon: 'menu', group: 'view' },

]);
