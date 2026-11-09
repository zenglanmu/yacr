//! The canonical command vocabulary and name normalization.
use cad_app::{CommandId, ExtendedCommandId, MeasurementToolKind};

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
    // CLOSE closes the current drawing (host-owned like OPEN/NEW); with nothing open the host shows its start page.
    ("CLOSE", CommandId::CloseDrawing),
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
    // LAYERP is the **previous layer state** (AutoCAD's one-step layer stack), a real command of its own — never an alias of the restore. It pops the newest session layer-override snapshot; an empty history is a named `layers.previous.empty` no-op, not an error.
    ("LAYERP", CommandId::LayerPrevious),
    // LAYISO hides every layer the selection is not on (plus the current layer), as session visibility overrides only — no payload, no database write. The restore half is the existing `RESTORELAYERS` path: `LAYUNISO`/`LAYON` are aliases of it below, so there is exactly one "clear the overrides" path and one "previous state" path (`LAYERP`).
    ("LAYISO", CommandId::IsolateLayers),
    // LAYOFF is its sibling: hide exactly the selection's layers (the current layer is never turned off), again as session visibility overrides only.
    ("LAYOFF", CommandId::LayOff),
    ("SPACE", CommandId::SwitchSpace),
    // `MODEL` (AutoCAD 2026) is the payload-free "switch to the Model tab" selector. It is the same validated session-state switch as `SPACE MODEL`, never a second space path, and takes no argument.
    ("MODEL", CommandId::ModelSpace),
    ("SELECT", CommandId::Select),
    ("SELECTALL", CommandId::SelectAll),
    // SELECTSIMILAR (AutoCAD 2026) is the type-based sibling of SELECTALL: it takes no payload and no argument, requires a non-empty current selection, and does not edit the drawing.
    ("SELECTSIMILAR", CommandId::SelectSimilar),
    ("SELECTINBOX", CommandId::SelectInBox),
    // The 2026-11-07 selection/filter batch. QSELECT/FILTER/GETSEL take a typed `PROPERTY VALUE` argument (the grammar lives in `commands::filters`), SSX and SELECTCOUNT take none, and the find pair is `-FIND <needle>` (report) vs `FIND <needle> [=> replacement]` (report or replace). All names are canonical selectors; none is an alias of another.
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
    // `AREA OBJECT` is our **selection-based** exact area of one closed object (AutoCAD's `AREA` → `Object` pick prompt, read from the selection instead). The bare `AREA` selector stays the interactive point measurement below, so the two are distinct names, not aliases.
    ("AREA OBJECT", CommandId::AreaObject),
    // MASSPROP (AutoCAD 2026): read-only mass properties of the **current selection** (our declared divergence from AutoCAD's `Select objects` prompt), limited to closed planar CIRCLE / full ELLIPSE / closed POLYLINE on one common plane. Payload `None`; Viewer-legal.
    ("MASSPROP", CommandId::MassProp),
    // MEASUREGEOM (AutoCAD 2026): the inquiry **router**. The bare name is the named `measuregeom.needs_option` refusal; the typed `MEASUREGEOM <option>` line routes to the existing measurement tools (or the read-only RADIUS report). It is not a second measurement engine.
    ("MEASUREGEOM", CommandId::MeasureGeom),
    ("CONFIRM", CommandId::ConfirmMeasurement),
    ("CANCELMEASURE", CommandId::CancelMeasurement),
    ("UNDO", CommandId::Undo),
    ("REDO", CommandId::Redo),
    ("RESOURCES", CommandId::Resources),
    ("DIAGNOSTICS", CommandId::Diagnostics),
    // AUDIT (AutoCAD 2026) is the **detect-only** drawing audit: it reports every stored entity that fails `cad_db::validate_entity` and repairs nothing (AutoCAD's "corrects some errors" half is NOT IMPLEMENTED). Read-only, so Viewer mode admits it.
    ("AUDIT", CommandId::AuditDrawing),
    ("SWITCHBACKEND", CommandId::SwitchBackend),
    ("SWITCH2D3D", CommandId::Switch2d3d),
    ("STANDARDVIEW", CommandId::StandardView),
    // `PLAN` (AutoCAD 2026) is the payload-free "orthographic view of the XY plane" selector: v1 is the top/plan 2D view only, the same observation as `STANDARDVIEW` → `Top`. This build has no UCS model, so PLAN's UCS/World/ Current orientation options are NOT IMPLEMENTED and take no argument.
    ("PLAN", CommandId::PlanView),
    ("ORBIT", CommandId::Orbit),
    ("PROJECTION", CommandId::SwitchProjection),
    ("MODE", CommandId::SetMode),
    ("LINE", CommandId::CreateLine),
    // XLINE/RAY are the construction-line pair: two free points (base, through), stored as one finite segment clipped to the model bounds because `cad-db` has no infinite-line geometry. The bare selectors start their capture.
    ("XLINE", CommandId::CreateXLine),
    ("RAY", CommandId::CreateRay),
    ("CIRCLE", CommandId::CreateCircle),
    // The three CIRCLE construction options are canonical **multi-word** selectors, not aliases of `CIRCLE`: each names a different construction (two diameter endpoints, three points on the circle, a centre plus a diameter point), so they start their own capture and build one Circle through the same create funnel. The bare `C` abbreviation stays the centre+radius `CIRCLE`.
    ("CIRCLE 2P", CommandId::CreateCircle2P),
    ("CIRCLE 3P", CommandId::CreateCircle3P),
    ("CIRCLE D", CommandId::CreateCircleDiameter),
    ("ARC", CommandId::CreateArc),
    ("PLINE", CommandId::CreatePolyline),
    // WIPEOUT is its own open-ended polygon capture: first point, then every further boundary point, Enter finishes. It stores one closed `SemanticGeometry::Mask` (`AcDbWipeout`, `inverted: false`); AutoCAD's `Frames`/`Polyline`/`Erase Polyline` surface is NOT IMPLEMENTED, so no argument form is admitted here.
    ("WIPEOUT", CommandId::Wipeout),
    ("RECTANG", CommandId::CreateRectangle),
    ("ELLIPSE", CommandId::CreateEllipse),
    ("POLYGON", CommandId::CreatePolygon),
    // POLYGON CIRCUMSCRIBED is AutoCAD's `POLYGON` -> `Circumscribed about circle` (`C`) construction as a canonical **multi-word** selector of its own -- not an alias: the second pick is a point on the **tangent** circle and the ring is `n` tangent edges (`R = r / cos(pi/n)`, first edge midpoint on the picked direction), where `POLYGON` is the inscribed ring through the picked point. Both read the session `POLYSIDES` count; neither has an argument form.
    ("POLYGON CIRCUMSCRIBED", CommandId::CreatePolygonCircumscribed),
    ("POINT", CommandId::CreatePoint),
    ("SPLINE", CommandId::CreateSpline),
    // INSERT places a reference to an **existing** named block at one picked insertion point; the bare selector starts the point-then-name capture (the name arrives through the same terminal text channel TEXT uses). `BLOCK` below writes the definition (`WBLOCK` is still not implemented).
    ("INSERT", CommandId::InsertBlock),
    // BLOCK defines a named block from the current selection at one picked base point; the bare selector starts the selection-then-point-then-name capture. Work-only.
    ("BLOCK", CommandId::DefineBlock),
    ("TEXT", CommandId::CreateText),
    // MTEXT: the multiline create command **and** a real two-corner interactive tool; the payload path stays for CLI/host IPC.
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
    // HATCHPOINT: internal-point selector; discovers the smallest visible closed boundary containing the pick. Not an alias of HATCH (input differs).
    ("HATCHPOINT", CommandId::HatchPoint),
    // BOUNDARY: the same discovery, emitting one closed polyline per loop. `-BOUNDARY` options are NOT IMPLEMENTED; BO/BPOLY are not admitted.
    ("BOUNDARY", CommandId::Boundary),
    ("HATCHGENERATEBOUNDARY", CommandId::HatchGenerateBoundary),
    ("HATCHTOBACK", CommandId::HatchToBack),
    // GRADIENT fills the **current selection** through the HATCH path (forced gradient, +Z plane). Not an alias of HATCH; no preview/.pat handling.
    ("GRADIENT", CommandId::Gradient),
    // HATCHEDIT edits one selected model-space hatch with a typed option (`ANGLE|SCALE|SOLID|GRADIENT`); the bare name is a named refusal.
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
    // TEXTTOFRONT reads the current selection and raises the selected text/leader/dimension draw order above every non-eligible model-space row.
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
    // The **Polyline mode** of the two corner commands. AutoCAD reaches it through the `Polyline` sub-option of the object prompt; the fixed-step capture cannot express "pick the first object *or* the Polyline option", so each mode is a selection-based selector of its own with `CommandPayload::None` (the one selected polyline is the whole input). They are **not** aliases of `CHAMFER`/`FILLET`: those keep their two-pick capture, and an alias would silently route a polyline edit into it.
    ("CHAMFER POLYLINE", CommandId::ChamferPolyline),
    ("FILLET POLYLINE", CommandId::FilletPolyline),
    // REVCLOUD (AutoCAD 2026) is the selection-based **Object mode** v1: the one selected closed straight-segment polyline is the whole input, every edge becomes one outward arc bulge. It is a canonical selector of its own, not an alias: there is no interactive rectangular/polygonal draw mode in this build, so the name must not route into a draw capture.
    ("REVCLOUD", CommandId::RevisionCloud),
    // REVERSE / OOPS / OVERKILL are selection-based; BREAKATPOINT is a pick tool.
    ("REVERSE", CommandId::Reverse),
    // PEDIT (AutoCAD 2026) is the **declared v1 subset** CLOSE/OPEN/REVERSE. The option is typed as the command's argument (`PEDIT CLOSE`), because this build has no in-command option prompt; the one selected model-space LWPOLYLINE is the whole input. A bare `PEDIT` normalizes and is refused by the application with `pedit.needs_option` (listing the implemented options), never defaulted to an option.
    ("PEDIT", CommandId::Pedit),
    ("BREAKATPOINT", CommandId::BreakAtPoint),
    ("OOPS", CommandId::Oops),
    ("OVERKILL", CommandId::Overkill),
    ("ARRAYRECT", CommandId::ArrayRectangular),
    ("ARRAYPOLAR", CommandId::ArrayPolar),
    ("ALIGN", CommandId::AlignEntities),
    ("STRETCH", CommandId::StretchEntities),
    ("ERASE", CommandId::EraseEntities),
    // JOIN merges the **selection** into one entity (collinear LINEs / concentric ARCs in this build); the bare selector is its whole input, so no payload and no IPC. The application refuses a shorter selection by name at apply time.
    ("JOIN", CommandId::Join),
    ("EXPLODE", CommandId::ExplodeEntities),
    ("COPY", CommandId::CopyEntities),
    ("ROTATE", CommandId::RotateEntities),
    ("SCALE", CommandId::ScaleEntities),
    ("MIRROR", CommandId::MirrorEntities),
    ("COPYCLIP", CommandId::CopyToClipboard),
    ("CUTCLIP", CommandId::CutToClipboard),
    // `PASTECLIP` is the **insertion-point** paste: it starts the one-point `PasteClip` capture (the clipboard is the input, so a selection is not required) and translates every clipboard entry by `insertion − base`. `PASTEORIG` is the other half of the AutoCAD pair: it takes no point and inserts at the original coordinates, ignoring the base.
    ("PASTECLIP", CommandId::PasteFromClipboard),
    ("PASTEORIG", CommandId::PasteOriginalCoordinates),
    // `PASTEBLOCK` turns the clipboard into a new auto-named block definition and inserts one reference at a picked point. It is **not** an alias of `PASTECLIP`: the paste shape (a block plus one reference) differs, so it starts the one-point `PasteBlock` capture instead.
    ("PASTEBLOCK", CommandId::PasteBlock),
    // `COPYBASE`/`CUTBASE` are the base-point captures (`COPYCLIP`/`CUTCLIP` record no base). Both start the same one-point capture over the current selection: `COPYBASE` only captures (read-only, Viewer-legal), `CUTBASE` captures **and** deletes in one undo step (work-only). The second kind (`CutBase`) exists only because the typed `BeginDraw` request carries the kind and no `CommandId`; the session records the command so the host maps the payload by it, never by guessing.
    ("COPYBASE", CommandId::CopyToClipboardBase),
    ("CUTBASE", CommandId::CutToClipboardBase),
    ("OFFSET", CommandId::OffsetEntity),
    ("SETACTIVELAYER", CommandId::SetActiveLayer),
    // Layer **table** writes (create/rename/delete). Each needs a payload — the new name (create), `{ layer, name }` (rename) or `{ layer }` (delete) — so the bare selector normalizes and is then refused explicitly by the application layer; the panel/IPC path supplies the typed payload. The layer selector inside the payload is a **name**, resolved against the drawing's own table (exact match first, then a unique case-insensitive one).
    ("CREATELAYER", CommandId::CreateLayer),
    ("RENAMELAYER", CommandId::RenameLayer),
    ("DELETELAYER", CommandId::DeleteLayer),
    // The layer **attribute** write (`off`/`frozen`/`locked`/`plottable`/`color`/ `linetype`/`lineweight`/`transparency`) is also payload-requiring: `{ layer, key, value }`. The bare selector normalizes and is refused explicitly by the application layer; the panel/IPC path (`cad_set_layer_property`) supplies the typed payload.
    ("SETLAYERPROPERTY", CommandId::SetLayerProperty),
    ("LAYCUR", CommandId::LayCur),
    ("LAYMCUR", CommandId::LayMCur),
    ("LAYFRZ", CommandId::LayFrz),
    ("LAYTHW", CommandId::LayThw),
    ("LAYLCK", CommandId::LayLck),
    ("LAYULK", CommandId::LayUlk),
    ("SETBYLAYER", CommandId::SetByLayer),
    // The 2026-11-07 layer batch. `LAYMCH`/`LAYMRG`/`LAYDEL` are selection-based (`CommandPayload::None`); the `-` spellings are aliases, not second commands. `LAYERSTATE`, `LTSCALE` and `LAYWALK` take a typed argument; `LAYERPMODE` reports on a bare name and takes ON/OFF as its argument.
    ("LAYMCH", CommandId::LayMCh),
    ("LAYMRG", CommandId::LayMrg),
    ("LAYDEL", CommandId::LayDel),
    ("LAYERSTATE", CommandId::LayerState),
    ("LAYERPMODE", CommandId::LayerPmode),
    ("LTSCALE", CommandId::SetLinetypeScale),
    ("LAYWALK", CommandId::LayWalk),
    // A payload-requiring edit: the selector normalizes, and the application refuses a bare invocation explicitly (the panel/IPC path supplies the `EntityProperty { key, value }` payload).
    ("SETENTITYPROPERTY", CommandId::SetEntityProperty),
    // MATCHPROP is the **selection-based** property painter: the bare selector is the whole input (`CommandPayload::None`), because the current selection's first entity is the source and the rest are destinations. No IPC of its own.
    ("MATCHPROP", CommandId::MatchProp),
    // GRIPSTRETCH is the **payload path of a pointer gesture**: `GripStretch { id, to }` carries the resolved handle id plus the world point the drag resolved to. That payload comes from the host's own pointer routing on the release of an armed grip drag (`cad_host`), never from this table: this layer only maps the selector, and the typed command line carries no handle, so the bare selector normalizes and is then refused explicitly by the application layer — it must never be routed to `BeginDraw` or turned into a fabricated drag.
    ("GRIPSTRETCH", CommandId::GripStretch),
    ("HOVERPREVIEW", CommandId::ToggleHoverPreview),
    // The display grid switch (F7). Grid **snap** is the separate `SNAP` selector below; the two are independent session switches and never alias each other.
    ("GRID", CommandId::ToggleGrid),
    // Grid snap (F9): quantise a captured point to the session `SNAPUNIT` spacing. Independent of object snap (`OSNAP`) and of the display grid (`GRID`); no alias maps onto any of the three.
    ("SNAP", CommandId::ToggleSnapGrid),
    ("OSNAP", CommandId::ToggleSnap),
    // The typed spacing sysvar: `SNAPUNIT <value>`. The bare name carries no value, so the application refuses it explicitly (no fabricated default).
    ("SNAPUNIT", CommandId::SetSnapSpacing),
    // The typed POINT display sysvars write one drawing-header value each; the bare names are refused (no fabricated default, no SETVAR indirection).
    ("PDMODE", CommandId::SetPointDisplayMode),
    ("PDSIZE", CommandId::SetPointDisplaySize),
    // The orthographic drafting constraint (F8). Session/display state only; applied after object snap and grid snap so it still decides the final axis.
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

    // The 2026-11-07 extras batch. `REGEN3`, `-PAN` and `-LINETYPE` are aliases below (same CommandId/path), never a second implementation.
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

    // The sustained extended-command batch: payload-free session toggles and
    // read-only reports. All ten ride the single `CommandId::Extended` wrapper.
    ("QTEXT", CommandId::Extended(ExtendedCommandId::QText)),
    ("FILL", CommandId::Extended(ExtendedCommandId::Fill)),
    ("TFRAMES", CommandId::Extended(ExtendedCommandId::TFrames)),
    ("REGENAUTO", CommandId::Extended(ExtendedCommandId::RegenAuto)),
    ("ISODRAFT", CommandId::Extended(ExtendedCommandId::IsoDraft)),
    ("BCOUNT", CommandId::Extended(ExtendedCommandId::BlockCount)),
    ("COUNT", CommandId::Extended(ExtendedCommandId::CountObjects)),
    ("EDITTIME", CommandId::Extended(ExtendedCommandId::EditTime)),
    ("JULIAN", CommandId::Extended(ExtendedCommandId::JulianDate)),
    ("XLIST", CommandId::Extended(ExtendedCommandId::XList)),

    // Wave 2 (2026-11-09): thirty more payload-free names. The first ten are
    // session tokens (each reads or flips one `SessionState::drafting_extras`
    // token; no renderer consumer exists), the next eight are read-only reports,
    // and the last twelve are stable named refusals. All 40 extended names ride
    // the single `CommandId::Extended` wrapper and are Viewer-legal.
    ("DRAGMODE", CommandId::Extended(ExtendedCommandId::DragMode)),
    ("APERTURE", CommandId::Extended(ExtendedCommandId::Aperture)),
    ("ISOPLANE", CommandId::Extended(ExtendedCommandId::IsoPlane)),
    ("HIGHLIGHTNEW", CommandId::Extended(ExtendedCommandId::HighlightNew)),
    ("TRAYSETTINGS", CommandId::Extended(ExtendedCommandId::TraySettings)),
    ("TEXTSCR", CommandId::Extended(ExtendedCommandId::TextScreen)),
    ("GRAPHSCR", CommandId::Extended(ExtendedCommandId::GraphScreen)),
    ("FINDINCLUDE", CommandId::Extended(ExtendedCommandId::FindInclude)),
    ("FINDMATCH", CommandId::Extended(ExtendedCommandId::FindMatch)),
    (
        "REVCLOUDPROPERTIES",
        CommandId::Extended(ExtendedCommandId::RevCloudProperties),
    ),
    ("SYSVARMONITOR", CommandId::Extended(ExtendedCommandId::SysVarMonitor)),
    ("SYSVDLG", CommandId::Extended(ExtendedCommandId::SysVarDialog)),
    (
        "VTOPTIONS",
        CommandId::Extended(ExtendedCommandId::ViewTransitionOptions),
    ),
    ("ALIASEDIT", CommandId::Extended(ExtendedCommandId::AliasEdit)),
    ("WHOHAS", CommandId::Extended(ExtendedCommandId::WhoHas)),
    ("ACADINFO", CommandId::Extended(ExtendedCommandId::AcadInfo)),
    ("PTYPE", CommandId::Extended(ExtendedCommandId::PointType)),
    ("SCALELISTEDIT", CommandId::Extended(ExtendedCommandId::ScaleListEdit)),
    (
        "HYPERLINKOPTIONS",
        CommandId::Extended(ExtendedCommandId::HyperlinkOptions),
    ),
    (
        "INPUTSEARCHOPTIONS",
        CommandId::Extended(ExtendedCommandId::InputSearchOptions),
    ),
    (
        "-INPUTSEARCHOPTIONS",
        CommandId::Extended(ExtendedCommandId::DashInputSearchOptions),
    ),
    ("SIGVALIDATE", CommandId::Extended(ExtendedCommandId::SigValidate)),
    ("BASE", CommandId::Extended(ExtendedCommandId::BasePoint)),
    ("LIMITS", CommandId::Extended(ExtendedCommandId::DrawingLimits)),
    (
        "VIEWPLOTDETAILS",
        CommandId::Extended(ExtendedCommandId::ViewPlotDetails),
    ),
    ("VPLAYER", CommandId::Extended(ExtendedCommandId::VPlayer)),
    ("MVIEW", CommandId::Extended(ExtendedCommandId::MView)),
    ("PSBSCALE", CommandId::Extended(ExtendedCommandId::PsbScale)),
    ("PSTSCALE", CommandId::Extended(ExtendedCommandId::PstScale)),
    ("MVSETUP", CommandId::Extended(ExtendedCommandId::MvSetup)),

    // Wave 3 (2026-11-09): thirty more payload-free names. The first eight are
    // reports/session commands (two space switches reusing the existing
    // MODEL/SPACE path, four read-only table reports, two non-associative
    // no-op reports); the last twenty-two are stable named refusals. All 70
    // extended names ride the single `CommandId::Extended` wrapper and are
    // Viewer-legal.
    ("MSPACE", CommandId::Extended(ExtendedCommandId::Mspace)),
    ("PSPACE", CommandId::Extended(ExtendedCommandId::Pspace)),
    ("STYLE", CommandId::Extended(ExtendedCommandId::TextStyle)),
    ("-STYLE", CommandId::Extended(ExtendedCommandId::DashStyle)),
    ("MLEADERSTYLE", CommandId::Extended(ExtendedCommandId::MLeaderStyleList)),
    ("BLOCK?", CommandId::Extended(ExtendedCommandId::BlockList)),
    ("DIMREASSOC", CommandId::Extended(ExtendedCommandId::DimReassoc)),
    ("DIMREASSOCIATE", CommandId::Extended(ExtendedCommandId::DimReassociate)),
    ("DIMJOGGED", CommandId::Extended(ExtendedCommandId::DimJogged)),
    ("DIMINSPECT", CommandId::Extended(ExtendedCommandId::DimInspect)),
    ("-DIMINSPECT", CommandId::Extended(ExtendedCommandId::DashDimInspect)),
    ("ANNORESET", CommandId::Extended(ExtendedCommandId::AnnoReset)),
    ("ANNOUPDATE", CommandId::Extended(ExtendedCommandId::AnnoUpdate)),
    ("OBJECTSCALE", CommandId::Extended(ExtendedCommandId::ObjectScale)),
    ("-OBJECTSCALE", CommandId::Extended(ExtendedCommandId::DashObjectScale)),
    ("QLATTACH", CommandId::Extended(ExtendedCommandId::QlAttach)),
    ("QLATTACHSET", CommandId::Extended(ExtendedCommandId::QlAttachSet)),
    ("QLDETACHSET", CommandId::Extended(ExtendedCommandId::QlDetachSet)),
    ("ARRAYCLOSE", CommandId::Extended(ExtendedCommandId::ArrayClose)),
    ("ARRAYEDIT", CommandId::Extended(ExtendedCommandId::ArrayEdit)),
    ("BEDIT", CommandId::Extended(ExtendedCommandId::BlockEditor)),
    ("-BEDIT", CommandId::Extended(ExtendedCommandId::DashBlockEditor)),
    ("BCLOSE", CommandId::Extended(ExtendedCommandId::BlockEditorClose)),
    ("BCYCLEORDER", CommandId::Extended(ExtendedCommandId::BCycleOrder)),
    ("BGRIPSET", CommandId::Extended(ExtendedCommandId::BGripSet)),
    ("BESETTINGS", CommandId::Extended(ExtendedCommandId::BEditorSettings)),
    ("BTESTBLOCK", CommandId::Extended(ExtendedCommandId::BTestBlock)),
    ("LAYERPALETTE", CommandId::Extended(ExtendedCommandId::LayerPalette)),
    ("CONSTRAINTBAR", CommandId::Extended(ExtendedCommandId::ConstraintBar)),
    ("CONSTRAINTSETTINGS", CommandId::Extended(ExtendedCommandId::ConstraintSettings)),

    // Wave 4 (2026-11-09): the first extended batch with real entity writes.
    // Twelve selection-based 2D edits (CENTERMARK, CENTERLINE, NCOPY,
    // COPYTOLAYER, -COPYTOLAYER, CDORDER, FLATTEN, XPLODE, BTRIM, BEXTEND,
    // CHANGE, CHPROP) are Work-only and reuse the existing planners; the last
    // eighteen are stable named refusals and stay Viewer-legal.
    ("CENTERMARK", CommandId::Extended(ExtendedCommandId::CenterMark)),
    ("CENTERLINE", CommandId::Extended(ExtendedCommandId::CenterLine)),
    ("NCOPY", CommandId::Extended(ExtendedCommandId::NCopy)),
    ("COPYTOLAYER", CommandId::Extended(ExtendedCommandId::CopyToLayer)),
    ("-COPYTOLAYER", CommandId::Extended(ExtendedCommandId::DashCopyToLayer)),
    ("CDORDER", CommandId::Extended(ExtendedCommandId::CDOrder)),
    ("FLATTEN", CommandId::Extended(ExtendedCommandId::Flatten)),
    ("XPLODE", CommandId::Extended(ExtendedCommandId::XPlode)),
    ("BTRIM", CommandId::Extended(ExtendedCommandId::BTrim)),
    ("BEXTEND", CommandId::Extended(ExtendedCommandId::BExtend)),
    ("CHANGE", CommandId::Extended(ExtendedCommandId::Change)),
    ("CHPROP", CommandId::Extended(ExtendedCommandId::ChProp)),
    ("BSCALE", CommandId::Extended(ExtendedCommandId::BScale)),
    ("MINSERT", CommandId::Extended(ExtendedCommandId::MInsert)),
    ("BLOCKREPLACE", CommandId::Extended(ExtendedCommandId::BlockReplace)),
    ("BREPLACE", CommandId::Extended(ExtendedCommandId::BReplace)),
    ("-BREPLACE", CommandId::Extended(ExtendedCommandId::DashBReplace)),
    ("SPLINEDIT", CommandId::Extended(ExtendedCommandId::SplineEdit)),
    ("MLEDIT", CommandId::Extended(ExtendedCommandId::MlEdit)),
    ("-MLEDIT", CommandId::Extended(ExtendedCommandId::DashMlEdit)),
    ("BURST", CommandId::Extended(ExtendedCommandId::Burst)),
    ("CLIP", CommandId::Extended(ExtendedCommandId::Clip)),
    ("CLIPIT", CommandId::Extended(ExtendedCommandId::ClipIt)),
    ("REGION", CommandId::Extended(ExtendedCommandId::Region)),
    ("ADDSELECTED", CommandId::Extended(ExtendedCommandId::AddSelected)),
    ("WBLOCK", CommandId::Extended(ExtendedCommandId::WBlock)),
    ("-WBLOCK", CommandId::Extended(ExtendedCommandId::DashWBlock)),
    ("BSAVE", CommandId::Extended(ExtendedCommandId::BSave)),
    ("BSAVEAS", CommandId::Extended(ExtendedCommandId::BSaveAs)),
    ("CHSPACE", CommandId::Extended(ExtendedCommandId::ChSpace)),
    // Wave 5 (2026-11-09): attribute and external-reference surfaces. ATTDISP is
    // a session token; BATTMAN/BATTORDER/XREF/-XREF/EXTERNALREFERENCES(/CLOSE)
    // are read-only reports; the rest are stable named refusals. All Viewer-legal.
    ("ATTDISP", CommandId::Extended(ExtendedCommandId::AttDisp)),
    ("BATTMAN", CommandId::Extended(ExtendedCommandId::BattMan)),
    ("BATTORDER", CommandId::Extended(ExtendedCommandId::BattOrder)),
    ("XREF", CommandId::Extended(ExtendedCommandId::XRef)),
    ("-XREF", CommandId::Extended(ExtendedCommandId::DashXRef)),
    ("EXTERNALREFERENCES", CommandId::Extended(ExtendedCommandId::ExternalReferences)),
    ("EXTERNALREFERENCESCLOSE", CommandId::Extended(ExtendedCommandId::ExternalReferencesClose)),
    ("ATTDEF", CommandId::Extended(ExtendedCommandId::AttDef)),
    ("-ATTDEF", CommandId::Extended(ExtendedCommandId::DashAttDef)),
    ("ATTEDIT", CommandId::Extended(ExtendedCommandId::AttEdit)),
    ("-ATTEDIT", CommandId::Extended(ExtendedCommandId::DashAttEdit)),
    ("ATTIPEDIT", CommandId::Extended(ExtendedCommandId::AttIpEdit)),
    ("ATTIN", CommandId::Extended(ExtendedCommandId::AttIn)),
    ("ATTOUT", CommandId::Extended(ExtendedCommandId::AttOut)),
    ("ATTEXT", CommandId::Extended(ExtendedCommandId::AttExt)),
    ("-ATTEXT", CommandId::Extended(ExtendedCommandId::DashAttExt)),
    ("EATTEDIT", CommandId::Extended(ExtendedCommandId::EAttEdit)),
    ("EATTEXT", CommandId::Extended(ExtendedCommandId::EAttExt)),
    ("-EATTEXT", CommandId::Extended(ExtendedCommandId::DashEAttExt)),
    ("ATTREDEF", CommandId::Extended(ExtendedCommandId::AttRedef)),
    ("ATTSYNC", CommandId::Extended(ExtendedCommandId::AttSync)),
    ("GATTE", CommandId::Extended(ExtendedCommandId::Gatte)),
    ("XDATA", CommandId::Extended(ExtendedCommandId::XData)),
    ("XBIND", CommandId::Extended(ExtendedCommandId::XBind)),
    ("-XBIND", CommandId::Extended(ExtendedCommandId::DashXBind)),
    ("XATTACH", CommandId::Extended(ExtendedCommandId::XAttach)),
    ("ATTACH", CommandId::Extended(ExtendedCommandId::Attach)),
    ("-ATTACH", CommandId::Extended(ExtendedCommandId::DashAttach)),
    ("XOPEN", CommandId::Extended(ExtendedCommandId::XOpen)),
    ("CLASSICXREF", CommandId::Extended(ExtendedCommandId::ClassicXRef)),
    // Wave 6 (2026-11-09): file / plot / export surfaces. DWGLOG/LOGFILEON/
    // LOGFILEOFF are one session token (no log file is written); DWGPROPS is a
    // read-only document report; DWFFORMAT/EXPORTPDF are read-only capability
    // reports; the rest are stable named refusals. All Viewer-legal.
    ("DWGLOG", CommandId::Extended(ExtendedCommandId::DwgLog)),
    ("LOGFILEON", CommandId::Extended(ExtendedCommandId::LogFileOn)),
    ("LOGFILEOFF", CommandId::Extended(ExtendedCommandId::LogFileOff)),
    ("DWGPROPS", CommandId::Extended(ExtendedCommandId::DwgProps)),
    ("DWFFORMAT", CommandId::Extended(ExtendedCommandId::DwfFormat)),
    ("EXPORTPDF", CommandId::Extended(ExtendedCommandId::ExportPdf)),
    ("SAVEALL", CommandId::Extended(ExtendedCommandId::SaveAll)),
    ("QNEW", CommandId::Extended(ExtendedCommandId::QNew)),
    ("REVERT", CommandId::Extended(ExtendedCommandId::Revert)),
    ("CLOSEALL", CommandId::Extended(ExtendedCommandId::CloseAll)),
    ("CLOSEALLOTHER", CommandId::Extended(ExtendedCommandId::CloseAllOther)),
    ("QQUIT", CommandId::Extended(ExtendedCommandId::QQuit)),
    ("QUIT", CommandId::Extended(ExtendedCommandId::Quit)),
    ("PROPULATE", CommandId::Extended(ExtendedCommandId::Propulate)),
    ("PLOTTERMANAGER", CommandId::Extended(ExtendedCommandId::PlotterManager)),
    ("STYLESMANAGER", CommandId::Extended(ExtendedCommandId::StylesManager)),
    ("EXPORTSETTINGS", CommandId::Extended(ExtendedCommandId::ExportSettings)),
    ("PREVIEW", CommandId::Extended(ExtendedCommandId::Preview)),
    ("EXPORTLAYOUT", CommandId::Extended(ExtendedCommandId::ExportLayout)),
    ("PLOTSTAMP", CommandId::Extended(ExtendedCommandId::PlotStamp)),
    ("-PLOTSTAMP", CommandId::Extended(ExtendedCommandId::DashPlotStamp)),
    ("-PLOT", CommandId::Extended(ExtendedCommandId::DashPlot)),
    ("-EXPORT", CommandId::Extended(ExtendedCommandId::DashExport)),
    ("ETRANSMIT", CommandId::Extended(ExtendedCommandId::ETransmit)),
    ("-ETRANSMIT", CommandId::Extended(ExtendedCommandId::DashETransmit)),
    ("ARCHIVE", CommandId::Extended(ExtendedCommandId::Archive)),
    ("-ARCHIVE", CommandId::Extended(ExtendedCommandId::DashArchive)),
    ("PUBLISH", CommandId::Extended(ExtendedCommandId::Publish)),
    ("-PUBLISH", CommandId::Extended(ExtendedCommandId::DashPublish)),
    ("AUTOPUBLISH", CommandId::Extended(ExtendedCommandId::AutoPublish)),
    // Wave 7 (2026-11-09): dynamic-block and constraint surfaces. BVSTATE and
    // -BVSTATE are read-only reports of each selected INSERT's active
    // dynamic-block visibility state; the rest are stable named refusals. All
    // Viewer-legal.
    ("BVSTATE", CommandId::Extended(ExtendedCommandId::BvState)),
    ("-BVSTATE", CommandId::Extended(ExtendedCommandId::DashBvState)),
    ("BACTION", CommandId::Extended(ExtendedCommandId::BAction)),
    ("BACTIONBAR", CommandId::Extended(ExtendedCommandId::BActionBar)),
    ("BACTIONSET", CommandId::Extended(ExtendedCommandId::BActionSet)),
    ("BACTIONTOOL", CommandId::Extended(ExtendedCommandId::BActionTool)),
    ("BASSOCIATE", CommandId::Extended(ExtendedCommandId::BAssociate)),
    ("BAUTHORPALETTE", CommandId::Extended(ExtendedCommandId::BAuthorPalette)),
    (
        "BAUTHORPALETTECLOSE",
        CommandId::Extended(ExtendedCommandId::BAuthorPaletteClose),
    ),
    ("BCONSTRUCTION", CommandId::Extended(ExtendedCommandId::BConstruction)),
    ("BCPARAMETER", CommandId::Extended(ExtendedCommandId::BCParameter)),
    ("-BCPARAMETER", CommandId::Extended(ExtendedCommandId::DashBCParameter)),
    ("BPARAMETER", CommandId::Extended(ExtendedCommandId::BParameter)),
    ("BLOOKUPTABLE", CommandId::Extended(ExtendedCommandId::BLookupTable)),
    ("BTABLE", CommandId::Extended(ExtendedCommandId::BTable)),
    ("BVHIDE", CommandId::Extended(ExtendedCommandId::BvHide)),
    ("BVSHOW", CommandId::Extended(ExtendedCommandId::BvShow)),
    ("RESETBLOCK", CommandId::Extended(ExtendedCommandId::ResetBlock)),
    ("AUTOCONSTRAIN", CommandId::Extended(ExtendedCommandId::AutoConstrain)),
    ("DELCONSTRAINT", CommandId::Extended(ExtendedCommandId::DelConstraint)),
    ("DIMCONSTRAINT", CommandId::Extended(ExtendedCommandId::DimConstraint)),
    ("GEOMCONSTRAINT", CommandId::Extended(ExtendedCommandId::GeomConstraint)),
    ("DCALIGNED", CommandId::Extended(ExtendedCommandId::DcAligned)),
    ("DCANGULAR", CommandId::Extended(ExtendedCommandId::DcAngular)),
    ("DCCONVERT", CommandId::Extended(ExtendedCommandId::DcConvert)),
    ("DCDIAMETER", CommandId::Extended(ExtendedCommandId::DcDiameter)),
    ("DCDISPLAY", CommandId::Extended(ExtendedCommandId::DcDisplay)),
    ("DCFORM", CommandId::Extended(ExtendedCommandId::DcForm)),
    ("DCHORIZONTAL", CommandId::Extended(ExtendedCommandId::DcHorizontal)),
    ("DCLINEAR", CommandId::Extended(ExtendedCommandId::DcLinear)),
    // Wave 8 (2026-11-09): session tokens and stable named refusals. ELEV is a
    // read-only elevation-token report and UCSICON flips a session flag; the
    // rest refuse the absent UCS/tablet/menu models and host-owned chrome. All
    // Viewer-legal.
    ("ELEV", CommandId::Extended(ExtendedCommandId::Elev)),
    ("UCSICON", CommandId::Extended(ExtendedCommandId::UcsIcon)),
    ("UCS", CommandId::Extended(ExtendedCommandId::Ucs)),
    ("UCSMAN", CommandId::Extended(ExtendedCommandId::UcsMan)),
    ("TABLET", CommandId::Extended(ExtendedCommandId::Tablet)),
    ("TASKBAR", CommandId::Extended(ExtendedCommandId::TaskBar)),
    ("REINIT", CommandId::Extended(ExtendedCommandId::Reinit)),
    ("SYSWINDOWS", CommandId::Extended(ExtendedCommandId::SysWindows)),
    ("MENU", CommandId::Extended(ExtendedCommandId::Menu)),
    ("RIBBON", CommandId::Extended(ExtendedCommandId::Ribbon)),

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

// The alias table lives in `cad-app`: the `ALIASEDIT` read-only report runs
// there and cannot depend on this UI crate. Re-exported here so
// `commands::COMMAND_ALIASES`, `alias_pairs()` and every existing import path
// are unchanged, and there is still exactly one copy of the rows.
pub use cad_app::command_aliases::COMMAND_ALIASES;

// Command-name resolution, selector constants and the admitted set.
mod helpers;
pub use helpers::*;

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
