//! The canonical command vocabulary and name normalization.
use cad_app::{CommandId, MeasurementToolKind};

use super::annot_arguments::{
    GROUPEDIT_SELECTOR, GROUP_SELECTOR, HATCHSETORIGIN_SELECTOR, LEADER_SELECTOR, QLEADER_SELECTOR,
    TEXTFIT_SELECTOR, UNGROUP_SELECTOR,
};
pub use super::dim_arguments::{
    DIMEDIT_SELECTOR, DIMJOGLINE_SELECTOR, DIMOVERRIDE_SELECTOR, DIMSTYLE_SELECTOR,
    DIMTEDIT_SELECTOR,
};
use super::text_batch::{
    JUSTIFYTEXT_SELECTOR, TCIRCLE_SELECTOR, TCOUNT_SELECTOR, TEXTALIGN_SELECTOR, TSCALE_SELECTOR,
};
use super::text_commands::{TCASE_SELECTOR, TORIENT_SELECTOR};

/// The canonical command vocabulary: every [`CommandId`] variant with the stable
/// uppercase selector a frontend uses to name it.
///
/// This is the single mapping table; it is deliberately exhaustive over the
/// `cad-app` command set so a host never has to special-case a command. A
/// selector whose variant needs a payload still dispatches through the same
/// funnel and is refused explicitly by the application/host layer.
// The rows are kept **one selector per line** on purpose: the documentation gate
// (`scripts/check-command-docs.py`) parses this table line by line, and rustfmt
// would otherwise split a long row (`POLYGON CIRCUMSCRIBED`) across three lines
// and hide it from the gate. Formatting is therefore frozen for this table.
#[rustfmt::skip]
pub const COMMAND_TABLE: &[(&str, CommandId)] = &[
    ("OPEN", CommandId::OpenDrawing),
    ("NEW", CommandId::NewDrawing),
    ("PLOT", CommandId::PlotDrawing),
    ("SAVE", CommandId::SaveDrawingAs),
    ("CANCEL", CommandId::CancelLoading),
    ("PAN", CommandId::Pan),
    ("ZOOM", CommandId::Zoom),
    ("FIT", CommandId::FitDrawing),
    ("RESETVIEW", CommandId::ResetView),
    ("TOGGLELAYER", CommandId::ToggleLayer),
    ("SETLAYERS", CommandId::SetLayerVisibilities),
    ("RESTORELAYERS", CommandId::RestoreLayers),
    // LAYERP is the **previous layer state** (AutoCAD's one-step layer stack), a
    // real command of its own — never an alias of the restore. It pops the newest
    // session layer-override snapshot; an empty history is a named
    // `layers.previous.empty` no-op, not an error.
    ("LAYERP", CommandId::LayerPrevious),
    // LAYISO hides every layer the selection is not on (plus the current layer),
    // as session visibility overrides only — no payload, no database write. The
    // restore half is the existing `RESTORELAYERS` path: `LAYUNISO`/`LAYON` are
    // aliases of it below, so there is exactly one "clear the overrides" path and
    // one "previous state" path (`LAYERP`).
    ("LAYISO", CommandId::IsolateLayers),
    // LAYOFF is its sibling: hide exactly the selection's layers (the current
    // layer is never turned off), again as session visibility overrides only.
    ("LAYOFF", CommandId::LayOff),
    ("SPACE", CommandId::SwitchSpace),
    // `MODEL` (AutoCAD 2026) is the payload-free "switch to the Model tab"
    // selector. It is the same validated session-state switch as `SPACE MODEL`,
    // never a second space path, and takes no argument.
    ("MODEL", CommandId::ModelSpace),
    ("SELECT", CommandId::Select),
    ("SELECTALL", CommandId::SelectAll),
    // SELECTSIMILAR (AutoCAD 2026) is the type-based sibling of SELECTALL: it
    // takes no payload and no argument, requires a non-empty current selection,
    // and does not edit the drawing.
    ("SELECTSIMILAR", CommandId::SelectSimilar),
    ("SELECTINBOX", CommandId::SelectInBox),
    // The 2026-11-07 selection/filter batch. QSELECT/FILTER/GETSEL take a typed
    // `PROPERTY VALUE` argument (the grammar lives in `commands::filters`), SSX
    // and SELECTCOUNT take none, and the find pair is `-FIND <needle>` (report)
    // vs `FIND <needle> [=> replacement]` (report or replace). All names are
    // canonical selectors; none is an alias of another.
    ("QSELECT", CommandId::QSelect),
    ("FILTER", CommandId::FilterSelect),
    ("GETSEL", CommandId::GetSelection),
    ("SSX", CommandId::SelectMatchingProperties),
    ("SELECTCOUNT", CommandId::SelectCount),
    ("-FIND", CommandId::FindReport),
    ("FIND", CommandId::FindReplace),
    ("HIDEOBJECTS", CommandId::HideObjects),
    ("ISOLATEOBJECTS", CommandId::IsolateObjects),
    ("UNISOLATEOBJECTS", CommandId::UnisolateObjects),
    ("MEASURE", CommandId::Measure),
    // `AREA OBJECT` is our **selection-based** exact area of one closed object
    // (AutoCAD's `AREA` → `Object` pick prompt, read from the selection instead).
    // The bare `AREA` selector stays the interactive point measurement below, so
    // the two are distinct names, not aliases.
    ("AREA OBJECT", CommandId::AreaObject),
    // MASSPROP (AutoCAD 2026): read-only mass properties of the **current
    // selection** (our declared divergence from AutoCAD's `Select objects`
    // prompt), limited to closed planar CIRCLE / full ELLIPSE / closed POLYLINE
    // on one common plane. Payload `None`; Viewer-legal.
    ("MASSPROP", CommandId::MassProp),
    // MEASUREGEOM (AutoCAD 2026): the inquiry **router**. The bare name is the
    // named `measuregeom.needs_option` refusal; the typed `MEASUREGEOM <option>`
    // line routes to the existing measurement tools (or the read-only RADIUS
    // report). It is not a second measurement engine.
    ("MEASUREGEOM", CommandId::MeasureGeom),
    ("CONFIRM", CommandId::ConfirmMeasurement),
    ("CANCELMEASURE", CommandId::CancelMeasurement),
    ("UNDO", CommandId::Undo),
    ("REDO", CommandId::Redo),
    ("RESOURCES", CommandId::Resources),
    ("DIAGNOSTICS", CommandId::Diagnostics),
    // AUDIT (AutoCAD 2026) is the **detect-only** drawing audit: it reports every
    // stored entity that fails `cad_db::validate_entity` and repairs nothing
    // (AutoCAD's "corrects some errors" half is NOT IMPLEMENTED). Read-only, so
    // Viewer mode admits it.
    ("AUDIT", CommandId::AuditDrawing),
    ("SWITCHBACKEND", CommandId::SwitchBackend),
    ("SWITCH2D3D", CommandId::Switch2d3d),
    ("STANDARDVIEW", CommandId::StandardView),
    // `PLAN` (AutoCAD 2026) is the payload-free "orthographic view of the XY
    // plane" selector: v1 is the top/plan 2D view only, the same observation as
    // `STANDARDVIEW` → `Top`. This build has no UCS model, so PLAN's UCS/World/
    // Current orientation options are NOT IMPLEMENTED and take no argument.
    ("PLAN", CommandId::PlanView),
    ("ORBIT", CommandId::Orbit),
    ("PROJECTION", CommandId::SwitchProjection),
    ("MODE", CommandId::SetMode),
    ("LINE", CommandId::CreateLine),
    // XLINE/RAY are the construction-line pair: two free points (base, through),
    // stored as one finite segment clipped to the model bounds because `cad-db`
    // has no infinite-line geometry. The bare selectors start their capture.
    ("XLINE", CommandId::CreateXLine),
    ("RAY", CommandId::CreateRay),
    ("CIRCLE", CommandId::CreateCircle),
    // The three CIRCLE construction options are canonical **multi-word**
    // selectors, not aliases of `CIRCLE`: each names a different construction
    // (two diameter endpoints, three points on the circle, a centre plus a
    // diameter point), so they start their own capture and build one Circle
    // through the same create funnel. The bare `C` abbreviation stays the
    // centre+radius `CIRCLE`.
    ("CIRCLE 2P", CommandId::CreateCircle2P),
    ("CIRCLE 3P", CommandId::CreateCircle3P),
    ("CIRCLE D", CommandId::CreateCircleDiameter),
    ("ARC", CommandId::CreateArc),
    ("PLINE", CommandId::CreatePolyline),
    // WIPEOUT is its own open-ended polygon capture: first point, then every
    // further boundary point, Enter finishes. It stores one closed
    // `SemanticGeometry::Mask` (`AcDbWipeout`, `inverted: false`); AutoCAD's
    // `Frames`/`Polyline`/`Erase Polyline` surface is NOT IMPLEMENTED, so no
    // argument form is admitted here.
    ("WIPEOUT", CommandId::Wipeout),
    ("RECTANG", CommandId::CreateRectangle),
    ("ELLIPSE", CommandId::CreateEllipse),
    ("POLYGON", CommandId::CreatePolygon),
    // POLYGON CIRCUMSCRIBED is AutoCAD's `POLYGON` -> `Circumscribed about circle`
    // (`C`) construction as a canonical **multi-word** selector of its own -- not
    // an alias: the second pick is a point on the **tangent** circle and the ring
    // is `n` tangent edges (`R = r / cos(pi/n)`, first edge midpoint on the picked
    // direction), where `POLYGON` is the inscribed ring through the picked point.
    // Both read the session `POLYSIDES` count; neither has an argument form.
    ("POLYGON CIRCUMSCRIBED", CommandId::CreatePolygonCircumscribed),
    ("POINT", CommandId::CreatePoint),
    ("SPLINE", CommandId::CreateSpline),
    // INSERT places a reference to an **existing** named block at one picked
    // insertion point; the bare selector starts the point-then-name capture (the
    // name arrives through the same terminal text channel TEXT uses). `BLOCK`
    // below writes the definition (`WBLOCK` is still not implemented).
    ("INSERT", CommandId::InsertBlock),
    // BLOCK defines a named block from the current selection at one picked base
    // point; the bare selector starts the selection-then-point-then-name
    // capture. Work-only.
    ("BLOCK", CommandId::DefineBlock),
    ("TEXT", CommandId::CreateText),
    // MTEXT: the multiline create command **and** a real two-corner interactive
    // tool; the payload path stays for CLI/host IPC.
    ("MTEXT", CommandId::CreateMText),
    ("MLEADER", CommandId::MLeader),
    ("TEXTEDIT", CommandId::TextEdit),
    ("DIMLINEAR", CommandId::DimLinear),
    ("DIMALIGNED", CommandId::DimAligned),
    ("DIMRADIUS", CommandId::DimRadius),
    ("DIMDIAMETER", CommandId::DimDiameter),
    ("DIMCENTER", CommandId::DimCenter),
    ("DIMANGULAR", CommandId::DimAngular),
    ("DIMORDINATE", CommandId::DimOrdinate),
    ("DIMBASELINE", CommandId::DimBaseline),
    ("DIMCONTINUE", CommandId::DimContinue),
    ("DIMROTATED", CommandId::DimRotated),
    ("DIMSPACE", CommandId::DimSpace),
    // The 2026-11-07 dimension-editing batch. `-DIMSTYLE` is an alias.
    ("DIMSTYLE", CommandId::DimStyle),
    ("DIMTEDIT", CommandId::DimTextEdit),
    ("DIMEDIT", CommandId::DimEdit),
    ("DIMOVERRIDE", CommandId::DimOverride),
    ("DIMREGEN", CommandId::DimRegen),
    ("DIMDISASSOCIATE", CommandId::DimDisassociate),
    ("QDIM", CommandId::QDim),
    ("DIMJOGLINE", CommandId::DimJogLine),
    ("DIMBREAK", CommandId::DimBreak),
    // The 2026-11-07 annotation/group batch. `-GROUP` is an alias of GROUP.
    ("GROUP", CommandId::Group),
    ("UNGROUP", CommandId::Ungroup),
    ("GROUPEDIT", CommandId::GroupEdit),
    ("QLEADER", CommandId::QLeader),
    ("LEADER", CommandId::Leader),
    ("TEXTFIT", CommandId::TextFit),
    ("HATCHSETBOUNDARY", CommandId::HatchSetBoundary),
    ("HATCHSETORIGIN", CommandId::HatchSetOrigin),
    ("XDLIST", CommandId::XdList),
    ("HATCH", CommandId::Hatch),
    // HATCHPOINT: internal-point selector; discovers the smallest visible closed
    // boundary containing the pick. Not an alias of HATCH (input differs).
    ("HATCHPOINT", CommandId::HatchPoint),
    // BOUNDARY: the same discovery, emitting one closed polyline per loop.
    // `-BOUNDARY` options are NOT IMPLEMENTED; BO/BPOLY are not admitted.
    ("BOUNDARY", CommandId::Boundary),
    ("HATCHGENERATEBOUNDARY", CommandId::HatchGenerateBoundary),
    ("HATCHTOBACK", CommandId::HatchToBack),
    // GRADIENT fills the **current selection** through the HATCH path (forced
    // gradient, +Z plane). Not an alias of HATCH; no preview/.pat handling.
    ("GRADIENT", CommandId::Gradient),
    // HATCHEDIT edits one selected model-space hatch with a typed option
    // (`ANGLE|SCALE|SOLID|GRADIENT`); the bare name is a named refusal.
    ("HATCHEDIT", CommandId::HatchEdit),
    ("MOVE", CommandId::MoveEntities),
    ("TRIM", CommandId::TrimEntity),
    ("EXTEND", CommandId::ExtendEntity),
    // BREAK starts its interactive capture; the bare selector fabricates no payload.
    ("BREAK", CommandId::Break),
    // DIVIDE/LENGTHEN act on one preselected curve (no capture point).
    ("DIVIDE", CommandId::Divide),
    ("LENGTHEN", CommandId::Lengthen),
    // SCALETEXT acts on the preselected text with a session typed factor.
    ("SCALETEXT", CommandId::ScaleText),
    // TORIENT/TCASE/TXT2MTXT act on the selection; the first two take an argument.
    ("TORIENT", CommandId::Torient),
    ("TCASE", CommandId::TCase),
    ("TXT2MTXT", CommandId::Txt2MText),
    // TEXTTOFRONT reads the current selection and raises the selected
    // text/leader/dimension draw order above every non-eligible model-space row.
    ("TEXTTOFRONT", CommandId::TextToFront),
    // 2026-11-07 2D text batch: `TJUST`/`MTEDIT`/`-MTEXT` are aliases.
    ("JUSTIFYTEXT", CommandId::JustifyText),
    ("TSCALE", CommandId::TextScale),
    ("TEXTALIGN", CommandId::TextAlignEdges),
    ("TCOUNT", CommandId::TextCount),
    ("TCIRCLE", CommandId::TextCircle),
    // TEXTMASK/TEXTUNMASK carry no argument: the selection is the input.
    ("TEXTMASK", CommandId::TextMask),
    ("TEXTUNMASK", CommandId::TextUnmask),
    ("CHAMFER", CommandId::ChamferLines),
    ("FILLET", CommandId::FilletLines),
    // The **Polyline mode** of the two corner commands. AutoCAD reaches it
    // through the `Polyline` sub-option of the object prompt; the fixed-step
    // capture cannot express "pick the first object *or* the Polyline option",
    // so each mode is a selection-based selector of its own with
    // `CommandPayload::None` (the one selected polyline is the whole input).
    // They are **not** aliases of `CHAMFER`/`FILLET`: those keep their two-pick
    // capture, and an alias would silently route a polyline edit into it.
    ("CHAMFER POLYLINE", CommandId::ChamferPolyline),
    ("FILLET POLYLINE", CommandId::FilletPolyline),
    // REVCLOUD (AutoCAD 2026) is the selection-based **Object mode** v1: the one
    // selected closed straight-segment polyline is the whole input, every edge
    // becomes one outward arc bulge. It is a canonical selector of its own, not
    // an alias: there is no interactive rectangular/polygonal draw mode in this
    // build, so the name must not route into a draw capture.
    ("REVCLOUD", CommandId::RevisionCloud),
    // REVERSE / OOPS / OVERKILL are selection-based; BREAKATPOINT is a pick tool.
    ("REVERSE", CommandId::Reverse),
    // PEDIT (AutoCAD 2026) is the **declared v1 subset** CLOSE/OPEN/REVERSE. The
    // option is typed as the command's argument (`PEDIT CLOSE`), because this
    // build has no in-command option prompt; the one selected model-space
    // LWPOLYLINE is the whole input. A bare `PEDIT` normalizes and is refused by
    // the application with `pedit.needs_option` (listing the implemented
    // options), never defaulted to an option.
    ("PEDIT", CommandId::Pedit),
    ("BREAKATPOINT", CommandId::BreakAtPoint),
    ("OOPS", CommandId::Oops),
    ("OVERKILL", CommandId::Overkill),
    ("ARRAYRECT", CommandId::ArrayRectangular),
    ("ARRAYPOLAR", CommandId::ArrayPolar),
    ("ALIGN", CommandId::AlignEntities),
    ("STRETCH", CommandId::StretchEntities),
    ("ERASE", CommandId::EraseEntities),
    // JOIN merges the **selection** into one entity (collinear LINEs / concentric
    // ARCs in this build); the bare selector is its whole input, so no payload and
    // no IPC. The application refuses a shorter selection by name at apply time.
    ("JOIN", CommandId::Join),
    ("EXPLODE", CommandId::ExplodeEntities),
    ("COPY", CommandId::CopyEntities),
    ("ROTATE", CommandId::RotateEntities),
    ("SCALE", CommandId::ScaleEntities),
    ("MIRROR", CommandId::MirrorEntities),
    ("COPYCLIP", CommandId::CopyToClipboard),
    ("CUTCLIP", CommandId::CutToClipboard),
    // `PASTECLIP` is the **insertion-point** paste: it starts the one-point
    // `PasteClip` capture (the clipboard is the input, so a selection is not
    // required) and translates every clipboard entry by `insertion − base`.
    // `PASTEORIG` is the other half of the AutoCAD pair: it takes no point and
    // inserts at the original coordinates, ignoring the base.
    ("PASTECLIP", CommandId::PasteFromClipboard),
    ("PASTEORIG", CommandId::PasteOriginalCoordinates),
    // `PASTEBLOCK` turns the clipboard into a new auto-named block definition and
    // inserts one reference at a picked point. It is **not** an alias of
    // `PASTECLIP`: the paste shape (a block plus one reference) differs, so it
    // starts the one-point `PasteBlock` capture instead.
    ("PASTEBLOCK", CommandId::PasteBlock),
    // `COPYBASE`/`CUTBASE` are the base-point captures (`COPYCLIP`/`CUTCLIP`
    // record no base). Both start the same one-point capture over the current
    // selection: `COPYBASE` only captures (read-only, Viewer-legal), `CUTBASE`
    // captures **and** deletes in one undo step (work-only). The second kind
    // (`CutBase`) exists only because the typed `BeginDraw` request carries the
    // kind and no `CommandId`; the session records the command so the host maps
    // the payload by it, never by guessing.
    ("COPYBASE", CommandId::CopyToClipboardBase),
    ("CUTBASE", CommandId::CutToClipboardBase),
    ("OFFSET", CommandId::OffsetEntity),
    ("SETACTIVELAYER", CommandId::SetActiveLayer),
    // Layer **table** writes (create/rename/delete). Each needs a payload — the
    // new name (create), `{ layer, name }` (rename) or `{ layer }` (delete) — so
    // the bare selector normalizes and is then refused explicitly by the
    // application layer; the panel/IPC path supplies the typed payload. The
    // layer selector inside the payload is a **name**, resolved against the
    // drawing's own table (exact match first, then a unique case-insensitive one).
    ("CREATELAYER", CommandId::CreateLayer),
    ("RENAMELAYER", CommandId::RenameLayer),
    ("DELETELAYER", CommandId::DeleteLayer),
    // The layer **attribute** write (`off`/`frozen`/`locked`/`plottable`/`color`/
    // `linetype`/`lineweight`/`transparency`) is also payload-requiring:
    // `{ layer, key, value }`. The bare selector normalizes and is refused
    // explicitly by the application layer; the panel/IPC path
    // (`cad_set_layer_property`) supplies the typed payload.
    ("SETLAYERPROPERTY", CommandId::SetLayerProperty),
    ("LAYCUR", CommandId::LayCur),
    ("LAYMCUR", CommandId::LayMCur),
    ("LAYFRZ", CommandId::LayFrz),
    ("LAYTHW", CommandId::LayThw),
    ("LAYLCK", CommandId::LayLck),
    ("LAYULK", CommandId::LayUlk),
    ("SETBYLAYER", CommandId::SetByLayer),
    // The 2026-11-07 layer batch. `LAYMCH`/`LAYMRG`/`LAYDEL` are selection-based
    // (`CommandPayload::None`); the `-` spellings are aliases, not second
    // commands. `LAYERSTATE`, `LTSCALE` and `LAYWALK` take a typed argument;
    // `LAYERPMODE` reports on a bare name and takes ON/OFF as its argument.
    ("LAYMCH", CommandId::LayMCh),
    ("LAYMRG", CommandId::LayMrg),
    ("LAYDEL", CommandId::LayDel),
    ("LAYERSTATE", CommandId::LayerState),
    ("LAYERPMODE", CommandId::LayerPmode),
    ("LTSCALE", CommandId::SetLinetypeScale),
    ("LAYWALK", CommandId::LayWalk),
    // A payload-requiring edit: the selector normalizes, and the application
    // refuses a bare invocation explicitly (the panel/IPC path supplies the
    // `EntityProperty { key, value }` payload).
    ("SETENTITYPROPERTY", CommandId::SetEntityProperty),
    // MATCHPROP is the **selection-based** property painter: the bare selector is
    // the whole input (`CommandPayload::None`), because the current selection's
    // first entity is the source and the rest are destinations. No IPC of its own.
    ("MATCHPROP", CommandId::MatchProp),
    // GRIPSTRETCH is the **payload path of a pointer gesture**: `GripStretch
    // { id, to }` carries the resolved handle id plus the world point the drag
    // resolved to. That payload comes from the host's own pointer routing on the
    // release of an armed grip drag (`cad_host`), never from this table: this
    // layer only maps the selector, and the typed command line carries no handle,
    // so the bare selector normalizes and is then refused explicitly by the
    // application layer — it must never be routed to `BeginDraw` or turned into a
    // fabricated drag.
    ("GRIPSTRETCH", CommandId::GripStretch),
    ("HOVERPREVIEW", CommandId::ToggleHoverPreview),
    // The display grid switch (F7). Grid **snap** is the separate `SNAP`
    // selector below; the two are independent session switches and never alias
    // each other.
    ("GRID", CommandId::ToggleGrid),
    // Grid snap (F9): quantise a captured point to the session `SNAPUNIT`
    // spacing. Independent of object snap (`OSNAP`) and of the display grid
    // (`GRID`); no alias maps onto any of the three.
    ("SNAP", CommandId::ToggleSnapGrid),
    ("OSNAP", CommandId::ToggleSnap),
    // The typed spacing sysvar: `SNAPUNIT <value>`. The bare name carries no
    // value, so the application refuses it explicitly (no fabricated default).
    ("SNAPUNIT", CommandId::SetSnapSpacing),
    // The typed POINT display sysvars write one drawing-header value each; the
    // bare names are refused (no fabricated default, no SETVAR indirection).
    ("PDMODE", CommandId::SetPointDisplayMode),
    ("PDSIZE", CommandId::SetPointDisplaySize),
    // The orthographic drafting constraint (F8). Session/display state only;
    // applied after object snap and grid snap so it still decides the final axis.
    ("ORTHO", CommandId::ToggleOrtho),
    ("LIST", CommandId::List),
    ("STATUS", CommandId::Status),
    ("FS", CommandId::Fs),
    // The 2026-11-07 utility/named-object batch (`-PURGE`/`-RENAME` aliases).
    ("PURGE", CommandId::Purge),
    ("RENAME", CommandId::Rename),
    ("SETVAR", CommandId::SetVar),
    ("TIME", CommandId::Time),
    ("DBLIST", CommandId::DbList),
    ("TREESTAT", CommandId::TreeStat),
    ("MREDO", CommandId::MRedo),
    ("MULTIPLE", CommandId::Multiple),
    // The 2026-11-07 draw/modify batch (`-OVERKILL` aliases the same path).
    ("DONUT", CommandId::Donut),
    ("SOLID", CommandId::Solid),
    ("HELIX", CommandId::Helix),
    ("MLINE", CommandId::Mline),
    ("MLSTYLE", CommandId::MlStyle),
    ("TREX", CommandId::Trex),
    ("EXTRIM", CommandId::Extrim),
    ("MPEDIT", CommandId::Mpedit),
    ("DRAWORDER", CommandId::DrawOrder),

    // The 2026-11-07 view batch: session view state only (Viewer-legal).
    ("REDRAW", CommandId::Redraw),
    ("REDRAWALL", CommandId::RedrawAll),
    ("REGEN", CommandId::Regen),
    ("REGENALL", CommandId::RegenAll),
    ("VIEW", CommandId::View),
    ("VIEWGO", CommandId::ViewGo),
    ("VIEWBACK", CommandId::ViewBack),
    ("VIEWFORWARD", CommandId::ViewForward),
    ("VPOINT", CommandId::VPoint),

    // The 2026-11-07 extras batch. `REGEN3`, `-PAN` and `-LINETYPE` are aliases
    // below (same CommandId/path), never a second implementation.
    ("SKETCH", CommandId::Sketch),
    ("TRACE", CommandId::Trace),
    ("MOCORO", CommandId::Mocoro),
    ("MSTRETCH", CommandId::MStretch),
    ("VIEWRES", CommandId::ViewRes),
    ("VSCURRENT", CommandId::VisualStyle),
    ("LINETYPE", CommandId::Linetype),

    // The 2026-11-07 layout batch; every write form is a named refusal.
    ("LAYOUT", CommandId::Layout),
    ("LAYOUTMERGE", CommandId::LayoutMerge),
    ("LAYOUTWIZARD", CommandId::LayoutWizard),
    ("VPORTS", CommandId::Vports),
    ("VPMAX", CommandId::VpMax),
    ("VPMIN", CommandId::VpMin),
    ("VPCLIP", CommandId::VpClip),
    ("VPSCALE", CommandId::VpScale),
    ("LAYVPI", CommandId::LayVpi),

];

/// Interaction-only selectors: canonical uppercase names that carry **no**
/// [`CommandId`] of their own and dispatch a typed
/// [`cad_app::interaction::InteractionRequest`] directly.
///
/// The four measurement selectors name the four real
/// [`MeasurementToolKind`] algorithms, so a measurement button starts *its own*
/// kind instead of every button collapsing onto `MEASURE`/Distance. `DIST`
/// (distance) and `AREA` are the real AutoCAD command names with exactly this
/// meaning — `AREA` is the **interactive point** area; its AutoCAD `Object`
/// sub-prompt is the separate canonical selector `AREA OBJECT`
/// ([`CommandId::AreaObject`], a selection-based measurement, not a point
/// algorithm, so it is not listed here). `MEASUREPOLYLINE` (polyline length) and
/// `MEASUREANGLE` are explicit
/// project selectors: AutoCAD's `PLINE` now names the polyline **draw** command
/// (a [`COMMAND_TABLE`] entry, not a measurement selector) and `ANGLE` is not an
/// AutoCAD command at all, so neither measurement meaning shadows a real
/// command. `MEASURE` remains the default [`MeasurementToolKind::Distance`]
/// selector.
///
/// These names live in this table (never a hand-written UI list):
/// [`admissible_names`] folds them into the single vocabulary a frontend enables
/// from, so a new measurement selector is admitted without a second list.
pub const MEASUREMENT_SELECTORS: &[(&str, MeasurementToolKind)] = &[
    ("DIST", MeasurementToolKind::Distance),
    ("MEASUREPOLYLINE", MeasurementToolKind::PolylineLength),
    ("MEASUREANGLE", MeasurementToolKind::Angle),
    ("AREA", MeasurementToolKind::Area),
    // ID is AutoCAD's own point-coordinate command (`Specify point`). It is an
    // interaction-only selector like the four above: the one pick is resolved by
    // the host (object snap included) and the answer is a point report, not a
    // measurement, so it carries no `CommandId` and never enters the measurement
    // engine.
    ("ID", MeasurementToolKind::Id),
];

/// The zoom-in step factor for the `ZOOMIN` selector.
///
/// A zoom button supplies no cursor, so the session applies this fixed factor
/// about the **viewport centre** (see `CadSession::command`): a real zoom about
/// the viewport centre, never a fabricated no-op.
pub const ZOOM_IN_FACTOR: f64 = 1.25;

/// The zoom-out step factor for the `ZOOMOUT` selector (the reciprocal of
/// [`ZOOM_IN_FACTOR`], so the two steps are exact opposites).
pub const ZOOM_OUT_FACTOR: f64 = 1.0 / ZOOM_IN_FACTOR;

/// Fixed zoom-step selectors: canonical uppercase name -> factor about the
/// viewport centre. Distinct from the payload-bearing `ZOOM` selector, which
/// still requires a caller-supplied factor.
pub const ZOOM_STEP_SELECTORS: &[(&str, f64)] =
    &[("ZOOMIN", ZOOM_IN_FACTOR), ("ZOOMOUT", ZOOM_OUT_FACTOR)];

/// The selectors that accept a **typed argument** on the command line.
///
/// An argument is admitted by its base selector and then validated against that
/// selector's own grammar; it is **not** a new [`CommandId`] and there is no
/// second IPC for it. Every other selector stays name-only, so `LINE 2` is the
/// ordinary unsupported-name refusal.
///
/// * [`ARGUMENT_SELECTOR`] (`ZOOM`): a positive factor or
///   `ALL`/`EXTENTS`/`WINDOW`/`PREVIOUS` ([`parse_zoom_argument`]);
/// * [`SNAPUNIT_SELECTOR`] / [`PDMODE_SELECTOR`] / [`PDSIZE_SELECTOR`]: one
///   positive spacing / one integer / one signed decimal, in that order;
/// * [`SPACE_SELECTOR`] (`SPACE`): `MODEL`, `#<u128>` or a layout display name,
///   resolved against the authoritative drawing by the session;
/// * [`TORIENT_SELECTOR`] (`TORIENT`) / [`TCASE_SELECTOR`] (`TCASE`): one finite
///   angle in degrees / one case mode ([`parse_torient_argument`],
///   [`parse_tcase_argument`]);
///
/// PEDIT/HATCHEDIT/MEASUREGEOM take one declared option; the text batch takes
/// `<H> <V>`, a factor, an edge word, a numbering run or a shape; the layer
/// batch takes the layer-state/laywalk/ltscale/layerpmode grammars. PEDIT,
/// HATCHEDIT, LAYERSTATE and the layer-batch forms refuse a bare selector by
/// name (never default an option); see the parsers re-exported from
/// `super::commands`.
pub const ARGUMENT_SELECTORS: &[&str] = &[
    ARGUMENT_SELECTOR,
    SNAPUNIT_SELECTOR,
    PDMODE_SELECTOR,
    PDSIZE_SELECTOR,
    SPACE_SELECTOR,
    TORIENT_SELECTOR,
    TCASE_SELECTOR,
    PEDIT_SELECTOR,
    HATCHEDIT_SELECTOR,
    MEASUREGEOM_SELECTOR,
    JUSTIFYTEXT_SELECTOR,
    TSCALE_SELECTOR,
    TEXTALIGN_SELECTOR,
    TCOUNT_SELECTOR,
    TCIRCLE_SELECTOR,
    LAYERSTATE_SELECTOR,
    LAYWALK_SELECTOR,
    LTSCALE_SELECTOR,
    LAYERPMODE_SELECTOR,
    QSELECT_SELECTOR,
    FILTER_SELECTOR,
    GETSEL_SELECTOR,
    FIND_SELECTOR,
    FIND_REPORT_SELECTOR,
    // The 2026-11-07 utility batch argument forms.
    PURGE_SELECTOR,
    RENAME_SELECTOR,
    SETVAR_SELECTOR,
    MREDO_SELECTOR,
    MULTIPLE_SELECTOR,
    // The 2026-11-07 draw/modify batch argument forms.
    DONUT_SELECTOR,
    SOLID_SELECTOR,
    HELIX_SELECTOR,
    MLINE_SELECTOR,
    MLSTYLE_SELECTOR,
    TREX_SELECTOR,
    MPEDIT_SELECTOR,
    DRAWORDER_SELECTOR,
    VIEW_SELECTOR,
    VIEWGO_SELECTOR,
    VPOINT_SELECTOR,
    // The 2026-11-07 layout/viewport batch argument forms.
    LAYOUT_SELECTOR,
    LAYOUTMERGE_SELECTOR,
    LAYOUTWIZARD_SELECTOR,
    VPORTS_SELECTOR,
    VPCLIP_SELECTOR,
    VPSCALE_SELECTOR,
    LAYVPI_SELECTOR,
    // The 2026-11-07 dimension-editing batch argument forms.
    DIMSTYLE_SELECTOR,
    DIMTEDIT_SELECTOR,
    DIMEDIT_SELECTOR,
    DIMOVERRIDE_SELECTOR,
    DIMJOGLINE_SELECTOR,
    // The 2026-11-07 annotation/group batch argument forms.
    GROUP_SELECTOR,
    UNGROUP_SELECTOR,
    GROUPEDIT_SELECTOR,
    QLEADER_SELECTOR,
    LEADER_SELECTOR,
    TEXTFIT_SELECTOR,
    HATCHSETORIGIN_SELECTOR,
    // The 2026-11-07 extras batch typed forms. `SKETCH`/`MSTRETCH` have payload
    // paths only; `REGEN3` takes no argument.
    PAN_SELECTOR,
    TRACE_SELECTOR,
    MOCORO_SELECTOR,
    VIEWRES_SELECTOR,
    VSCURRENT_SELECTOR,
    LINETYPE_SELECTOR,
];

/// Split a typed line into an argument-taking selector and its argument.
///
/// `Some((selector, argument))` only when `text` starts with one of
/// [`ARGUMENT_SELECTORS`] followed by one or more spaces/tabs and a non-empty
/// argument (`ZOOM 2`, `  snapunit\t0.5 `); a name-only line (`ZOOM`,
/// `SNAPUNIT`) returns `None` so it keeps the name-only funnel and its existing
/// payload refusal. The argument keeps its internal text: `ZOOM 2 3` reaches
/// [`parse_zoom_argument`] as `2 3` and is refused there rather than being split
/// a second time.
pub fn typed_command_argument(text: &str) -> Option<(&'static str, &str)> {
    let trimmed = text.trim();
    let split = trimmed.find([' ', '\t'])?;
    let (name, rest) = trimmed.split_at(split);
    let argument = rest.trim_start_matches([' ', '\t']).trim_end();
    if argument.is_empty() {
        return None;
    }
    let selector = canonical_command_name(name)?;
    ARGUMENT_SELECTORS
        .contains(&selector)
        .then_some((selector, argument))
}

/// AutoCAD's `ZOOM n` / `ZOOM ALL`: the first argument-taking selector.
///
/// A positive factor reuses the `ZOOMIN`/`ZOOMOUT` step path (anchored at the
/// viewport centre); `ALL`/`EXTENTS` dispatches the ordinary `FIT`; `WINDOW`
/// starts the two-corner capture and `PREVIOUS` restores the last discrete view
/// (see [`ZoomArgument`]). The last two still carry **no** `CommandId` of their
/// own: they ride [`CommandId::Zoom`] with their own payloads.
pub const ARGUMENT_SELECTOR: &str = "ZOOM";

/// AutoCAD's `SNAPUNIT <value>`: the session grid-snap spacing sysvar.
///
/// The value is a positive decimal in drawing units and is stored as session
/// state only (no database, history or preference write). It is the spacing the
/// `SNAP` (F9) quantiser uses.
pub const SNAPUNIT_SELECTOR: &str = "SNAPUNIT";

/// AutoCAD's `PDMODE <n>`: the drawing-global POINT display **mode** sysvar.
///
/// The value is an integer stored **raw** in the drawing header (`$PDMODE`), so
/// it is a document write (one revision bump, no undo entry), not session state.
/// The display layer interprets the bits.
pub const PDMODE_SELECTOR: &str = "PDMODE";

/// AutoCAD's `PDSIZE <n>`: the drawing-global POINT display **size** sysvar.
///
/// The value is a finite decimal stored **raw** in the drawing header
/// (`$PDSIZE`): `> 0` absolute drawing units, `0` viewport-relative, `< 0` a
/// percentage of the viewport height. A document write like
/// [`PDMODE_SELECTOR`], not session state.
pub const PDSIZE_SELECTOR: &str = "PDSIZE";

/// The `PEDIT <option>` selector: one declared v1 polyline option (AutoCAD
/// 2026).
///
/// AutoCAD's `PEDIT` shows an option prompt after the polyline pick; this build
/// has no in-command option prompt, so the option is typed on the same command
/// line. The grammar ([`parse_pedit_argument`]) is `CLOSE`/`C`, `OPEN`/`O` and
/// `REVERSE`/`R`, case-insensitive. The current selection (exactly one
/// model-space LWPOLYLINE) is still the whole input, so there is no second IPC.
/// A bare `PEDIT` is admitted by name and refused by the application with
/// `pedit.needs_option`; an unknown argument is refused before IPC with
/// [`invalid_pedit_argument_reason`]. `Join`, `Width`/`Halfwidth`, `Fit`,
/// `Spline`, `Decurve`, `Ltype gen`, `Undo`, `Multiple`, `Edit vertex` and the
/// `PEDITACCEPT` conversion prompt are NOT IMPLEMENTED.
pub const PEDIT_SELECTOR: &str = "PEDIT";
/// The `HATCHEDIT <option>` selector: the typed v1 hatch-edit surface (AutoCAD
/// 2026).
///
/// AutoCAD's `HATCHEDIT` opens a dialog; this build has no in-command dialog, so
/// the option is typed on the command line. The grammar
/// ([`super::parse_hatchedit_argument`]) is `ANGLE <degrees>`, `SCALE <factor>`,
/// `SOLID` and `GRADIENT`, case-insensitive. The current selection (exactly one
/// model-space hatch) is still the whole input, so there is no second IPC. A
/// bare `HATCHEDIT` is admitted by name and refused by the application with
/// `hatchedit.needs_option`; an unknown argument is refused before IPC with
/// [`super::invalid_hatchedit_argument_reason`]. `Pattern`, `Color`,
/// `Properties`, `Draw order`, `Add`/`Remove` boundaries and associativity are
/// NOT IMPLEMENTED.
pub const HATCHEDIT_SELECTOR: &str = "HATCHEDIT";
/// AutoCAD's `MEASUREGEOM <option>`: the inquiry router's typed argument form.
///
/// This is an **argument form of the canonical `MEASUREGEOM` selector**, not a
/// new command: `DISTANCE`/`ANGLE`/`AREA`/`POLYLINE` start the existing
/// measurement tools, `RADIUS` dispatches the read-only report and `VOLUME` is
/// an explicit not-implemented refusal. Full contract:
/// [`cad_app::command_id::measuregeom`].
pub const MEASUREGEOM_SELECTOR: &str = "MEASUREGEOM";

/// The `SPACE <argument>` selector: switch the active drawing space.
///
/// This is the same [`CommandId::SwitchSpace`] command the IPC/panel path uses;
/// the typed argument only saves a second IPC for a layout-tab click. `MODEL`
/// selects model space, `#<u128>` names a `LayoutId` directly, and any other
/// whole argument is resolved as a layout **display name** (exact match first,
/// then a unique case-insensitive one). A bare `SPACE` keeps the existing
/// explicit "needs a space payload" refusal.
pub const SPACE_SELECTOR: &str = "SPACE";

/// The canonical name in `table` matching the already-uppercased `key`, if any.
fn table_name<T>(table: &[(&'static str, T)], key: &str) -> Option<&'static str> {
    table
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(name, _)| *name)
}

/// The measurement kind an interaction-only selector names, if any.
///
/// The selector must already be canonical (as returned by
/// [`canonical_command_name`]); an alias never reaches here because aliases map
/// onto a canonical selector first.
pub fn measurement_selector_kind(name: &str) -> Option<MeasurementToolKind> {
    MEASUREMENT_SELECTORS
        .iter()
        .find(|(selector, _)| *selector == name)
        .map(|(_, kind)| *kind)
}

/// The fixed zoom factor a zoom-step selector names, if any.
pub fn zoom_step_factor(name: &str) -> Option<f64> {
    ZOOM_STEP_SELECTORS
        .iter()
        .find(|(selector, _)| *selector == name)
        .map(|(_, factor)| *factor)
}

/// Full-word synonyms and AutoCAD-style abbreviations, always accepted, that map
/// onto a canonical [`COMMAND_TABLE`] selector.
///
/// The single/double-letter rows mirror AutoCAD's default `acad.pgp` command
/// abbreviations **for the commands this build already implements only**: an
/// alias is admitted only when its target is a real selector, so `MA`
/// (`MATCHPROP`) is present now that MATCHPROP exists (and was absent while the
/// command itself was absent). There is no `acad.pgp` file parsing and no
/// user-defined alias support; this table is the whole (curated) subset, and
/// every row is contract-tested to canonicalise onto a [`COMMAND_TABLE`] name,
/// to be unique, and to collide with no canonical name.
///
/// There is no `SNAP` alias row. AutoCAD's `SNAP` is **grid snap** (F9), a real
/// selector of its own ([`CommandId::ToggleSnapGrid`], with the `SNAPUNIT`
/// spacing sysvar); it is a different feature from object snap (`OSNAP`) and
/// from the display grid (`GRID`). Aliasing it onto `ToggleSnap` would silently
/// turn a request for grid snap into an object-snap toggle, and aliasing it onto
/// `ToggleGrid` would silently turn it into a display toggle — neither is grid
/// snap, so no alias exists and the canonical name is the only spelling.
pub const COMMAND_ALIASES: &[(&str, &str)] = &[
    ("EXPORT", "PLOT"),
    ("PRINT", "PLOT"),
    ("SAVEAS", "SAVE"),
    ("QSAVE", "SAVE"),
    ("ZOOM EXTENTS", "FIT"),
    ("ENTER", "CONFIRM"),
    // `DBA`/`DCO` are the conventional AutoCAD abbreviations of the baseline and
    // continue dimension pair. Admitting them is our declared convenience (the
    // AutoCAD command reference documents the full names, not these shorthand
    // forms), and an alias never changes behaviour: it only normalizes a typed
    // name onto the canonical selector (ADR 0017 decision C, P5f).
    // `LAYUNISO` is AutoCAD's "turn layer isolation off", which is exactly what
    // `RESTORELAYERS` does here (drop the session's temporary overrides).
    // `LAYON` ("turn all layers on") is the same restore reached by its own name.
    //
    // `LAYERP` is deliberately **not** in this table any more (2026-10-27): it is a
    // canonical selector of its own ([`CommandId::LayerPrevious`]) that restores
    // the *previous* layer state from the session's bounded override history. An
    // alias onto `RESTORELAYERS` would silently clear every override instead.
    ("LAYUNISO", "RESTORELAYERS"),
    // `LAYON` is AutoCAD's "turn all layers on", which is exactly what the one
    // restore path does here (drop every temporary override). It carries no
    // `CommandId` of its own, like `LAYUNISO`.
    ("LAYON", "RESTORELAYERS"),
    // The command-line (`-`) spellings of the 2026-11-07 layer batch name the
    // same commands: this build has no layer dialog, so the dialog-free form is
    // the whole behaviour and a second `CommandId` would be a second path.
    ("-LAYMCH", "LAYMCH"),
    ("-LAYMRG", "LAYMRG"),
    ("-LAYDEL", "LAYDEL"),
    // `-OVERKILL` is AutoCAD's dialog-free spelling of the same command; this
    // build has one selection-based path, so the alias adds no second engine.
    ("-OVERKILL", "OVERKILL"),
    ("DBA", "DIMBASELINE"),
    ("DCO", "DIMCONTINUE"),
    // `NEWLAYER` is a **project** word synonym for creating a layer, not an
    // AutoCAD command (AutoCAD reaches layer creation through `LAYER` /
    // `-LAYER`); it is admitted only because `CREATELAYER` itself exists, the
    // same rule every other alias row follows. It is the only non-`acad.pgp`
    // spelling in this table, and it changes nothing but the normalized name.
    ("NEWLAYER", "CREATELAYER"),
    // AutoCAD's default `acad.pgp` single/double-letter abbreviations, listed
    // alphabetically (draw, modify, view, annotate, drafting switches). Only the
    // subset whose target selector exists here: `MA` (MATCHPROP) joined the table
    // when MATCHPROP itself was implemented, and inventing an abbreviation for an
    // absent command is exactly what this table must never do.
    ("A", "ARC"),
    ("AL", "ALIGN"),
    // AutoCAD's `AR` is ARRAY; `ARRAYRECT` is the array command this build
    // implements, so the abbreviation points at the implemented selector.
    ("AR", "ARRAYRECT"),
    // AutoCAD's `BR` is BREAK, this build's two-point break.
    ("BR", "BREAK"),
    ("C", "CIRCLE"),
    ("CHA", "CHAMFER"),
    ("CO", "COPY"),
    ("CP", "COPY"),
    ("DAL", "DIMALIGNED"),
    ("DAN", "DIMANGULAR"),
    ("DCE", "DIMCENTER"),
    ("DDI", "DIMDIAMETER"),
    // `DIV` / `LEN` are AutoCAD's abbreviations of the two zero-point single-curve
    // edits implemented here (DIVIDE and LENGTHEN).
    ("DIV", "DIVIDE"),
    ("DLI", "DIMLINEAR"),
    ("DRA", "DIMRADIUS"),
    ("E", "ERASE"),
    ("EL", "ELLIPSE"),
    ("EX", "EXTEND"),
    ("F", "FILLET"),
    ("GR", "GRID"),
    ("H", "HATCH"),
    // AutoCAD's default abbreviation for INSERT (an existing block reference).
    ("I", "INSERT"),
    // AutoCAD's default abbreviation for BLOCK (define a named block).
    ("B", "BLOCK"),
    ("L", "LINE"),
    ("LEN", "LENGTHEN"),
    ("M", "MOVE"),
    ("MA", "MATCHPROP"),
    ("MI", "MIRROR"),
    ("O", "OFFSET"),
    ("OR", "ORTHO"),
    ("OS", "OSNAP"),
    ("P", "PAN"),
    ("PL", "PLINE"),
    ("PO", "POINT"),
    ("POL", "POLYGON"),
    ("REC", "RECTANG"),
    ("RO", "ROTATE"),
    ("S", "STRETCH"),
    ("SC", "SCALE"),
    ("SPL", "SPLINE"),
    ("T", "TEXT"),
    ("TR", "TRIM"),
    ("U", "UNDO"),
    ("X", "EXPLODE"),
    // AutoCAD's default abbreviation for XLINE. `RAY` deliberately has no alias:
    // AutoCAD's `acad.pgp` defines none, and this table never invents spellings.
    ("XL", "XLINE"),
    ("Z", "ZOOM"),
    // The 2026-11-07 text batch aliases. `TJUST` is AutoCAD's `JUSTIFYTEXT`
    // alias (same `CommandId`, same typed `<H> <V>` grammar); `MTEDIT` is
    // AutoCAD's name for editing the selected text and maps onto the existing
    // `TEXTEDIT` pick tool; `-MTEXT` is AutoCAD's command-line MTEXT form,
    // mapping onto the existing two-corner `MTEXT` tool. None is a second path.
    ("TJUST", "JUSTIFYTEXT"),
    ("MTEDIT", "TEXTEDIT"),
    ("-MTEXT", "MTEXT"),
    // The command-line (`-`) spellings of PURGE/RENAME name the same commands:
    // this build has no dialog form, so the dialog-free selector is the whole
    // behaviour and a second `CommandId` would be a second path.
    ("-PURGE", "PURGE"),
    ("-RENAME", "RENAME"),
    // `-VIEW` is AutoCAD's command-line form of `VIEW`. This build has no view
    // dialog, so the dialog-free typed form is the whole behaviour and the alias
    // maps onto the same `CommandId`/path — never a second gallery.
    ("-VIEW", "VIEW"),
    // `-VPORTS` is AutoCAD's command-line spelling of VPORTS; this build has no
    // viewport dialog, so the alias only normalizes the name onto the one
    // selector and its one named refusal.
    ("-VPORTS", "VPORTS"),
    // `-DIMSTYLE` is AutoCAD's dialog-free spelling of the same command; this
    // build has one selector path, so it normalizes onto `DIMSTYLE`.
    ("-DIMSTYLE", "DIMSTYLE"),
    // `-GROUP` is AutoCAD's command-line group form; this build has one
    // session-only group path, so it normalizes onto `GROUP`.
    ("-GROUP", "GROUP"),
    // The 2026-11-07 extras aliases: `REGEN3` is the 3D spelling of `REGEN`, and
    // `-PAN`/`-LINETYPE` are the command-line spellings of the same paths.
    ("REGEN3", "REGEN"),
    ("-PAN", "PAN"),
    ("-LINETYPE", "LINETYPE"),
];

/// The alias table as `(alias, canonical selector)` pairs, resolved through
/// [`canonical_command_name`].
///
/// This is what a host publishes to a frontend (the `cad_command_aliases` IPC):
/// the frontend may offer aliases in its suggestion list without keeping a
/// hand-written mirror of the table. An alias whose target does not resolve is
/// skipped rather than guessed; the alias invariants are contract-tested, so no
/// such row exists.
pub fn alias_pairs() -> Vec<(&'static str, &'static str)> {
    COMMAND_ALIASES
        .iter()
        .filter_map(|(alias, target)| canonical_command_name(target).map(|name| (*alias, name)))
        .collect()
}

/// The canonical, stable uppercase selector a command name resolves to, or
/// `None` when the name maps to no admitted command.
///
/// Covers [`COMMAND_TABLE`], the interaction-only [`MEASUREMENT_SELECTORS`] /
/// [`ZOOM_STEP_SELECTORS`] and [`COMMAND_ALIASES`]. Names are matched
/// case-insensitively after trimming surrounding whitespace; internal whitespace
/// is significant (multi-word selectors keep one space).
pub fn canonical_command_name(text: &str) -> Option<&'static str> {
    let key = text.trim().to_ascii_uppercase();
    if let Some(name) = table_name(COMMAND_TABLE, &key) {
        return Some(name);
    }
    if let Some(name) = table_name(MEASUREMENT_SELECTORS, &key) {
        return Some(name);
    }
    if let Some(name) = table_name(ZOOM_STEP_SELECTORS, &key) {
        return Some(name);
    }
    COMMAND_ALIASES
        .iter()
        .find(|(alias, _)| *alias == key)
        .and_then(|(_, canonical)| canonical_command_name(canonical))
}

/// The canonical selector a [`CommandId`] is named by, if it has one.
///
/// This is the reverse of [`COMMAND_TABLE`] and exists for typed entry points
/// that dispatch a `CommandId` directly without going through a text selector,
/// so the command echo still records the same stable machine key the router
/// would. Interaction-only selectors (`DIST`, `ZOOMIN`, …) carry no
/// `CommandId` and are recorded by the selector path itself.
pub fn command_name(id: CommandId) -> Option<&'static str> {
    COMMAND_TABLE
        .iter()
        .find(|(_, candidate)| *candidate == id)
        .map(|(name, _)| *name)
}

/// The `CommandId` a command name names, if any.
///
/// Covers the full [`COMMAND_TABLE`] vocabulary (plus [`COMMAND_ALIASES`]); an
/// unmapped name — including the interaction-only [`MEASUREMENT_SELECTORS`] /
/// [`ZOOM_STEP_SELECTORS`], which carry no `CommandId` — returns `None` so the
/// caller can dispatch or refuse it explicitly.
pub fn normalize_command(text: &str) -> Option<CommandId> {
    let name = canonical_command_name(text)?;
    COMMAND_TABLE
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, id)| *id)
}

/// The stable rejection reason for a name that maps to no command.
pub fn unsupported_command_reason(text: &str) -> String {
    format!("unsupported CAD command: {}", text.trim())
}

/// Every name the host admits, for a frontend that must disable what cannot run
/// instead of keeping a hand-written mirror that drifts.
///
/// Canonical [`COMMAND_TABLE`] selectors come first, then the interaction-only
/// selectors ([`MEASUREMENT_SELECTORS`], [`ZOOM_STEP_SELECTORS`]), then every
/// alias that is not already canonical. Order is stable, so a frontend may sort
/// it for display without losing the canonical-first grouping.
pub fn admissible_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = COMMAND_TABLE.iter().map(|(name, _)| *name).collect();
    for (name, _) in MEASUREMENT_SELECTORS {
        if !names.contains(name) {
            names.push(name);
        }
    }
    for (name, _) in ZOOM_STEP_SELECTORS {
        if !names.contains(name) {
            names.push(name);
        }
    }
    for (alias, _) in COMMAND_ALIASES {
        if !names.contains(alias) {
            names.push(alias);
        }
    }
    names
}

// The typed-argument grammars and their stable refusals (split out for the
// code-size gate). Re-exported so every `commands::parse_*` path is unchanged.
mod arguments;
pub use arguments::*;
pub(crate) use arguments::{parse_positive_decimal, parse_signed_decimal};

// The selection/filter batch grammars (2026-11-07): `QSELECT`/`FILTER`/`GETSEL`
// and `-FIND`/`FIND`. Re-exported so `commands::parse_*` paths resolve.
mod filters;
pub use filters::*;

// The 2026-11-07 draw/modify batch grammars (`DONUT` … `DRAWORDER`).
mod draw2;
pub use draw2::*;

// The 2026-11-07 layout/viewport grammars (`LAYOUT` … `LAYVPI`).
mod layout;
pub use layout::*;

// The 2026-11-07 extras grammars (`-PAN`, `TRACE` … `LINETYPE`).
mod extras;
pub use extras::*;

// The selector -> interactive-tool routing table (split out for the code-size
// gate). Re-exported so `commands::interaction_request_for` still resolves.
mod routing;
pub(super) use routing::interaction_request_for;
