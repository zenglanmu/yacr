#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Viewer,
    Work,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandId {
    OpenDrawing,
    /// Start a new, empty drawing. Host-owned like `OpenDrawing`: the
    /// application layer has no document factory, so it returns `Unsupported`
    /// and a platform host performs the replacement.
    NewDrawing,
    /// Export the active drawing to a vector file (SVG/PDF). Host-owned like
    /// `OpenDrawing`: the application layer has no file chooser, so it returns
    /// `Unsupported` and a platform host performs the export.
    PlotDrawing,
    /// Save the active drawing as a lossy DXF file. Host-owned like
    /// `OpenDrawing`: the application layer has no file chooser, so it returns
    /// `Unsupported` and a platform host performs the save.
    SaveDrawingAs,
    CancelLoading,
    Pan,
    Zoom,
    FitDrawing,
    ResetView,
    ToggleLayer,
    /// Atomically publish a nonempty batch of temporary layer overrides.
    /// Allowed in Viewer and Work; no drawing write, reparse or undo entry.
    SetLayerVisibilities,
    RestoreLayers,
    SwitchSpace,
    Select,
    /// Select visible model-space pick items without editing the drawing.
    /// Requires `CommandPayload::None`; incomplete picking is explicitly refused.
    SelectAll,
    /// Select model-space items in a window/crossing rectangle without editing
    /// the drawing (ADR 0014 B). Allowed in Viewer and Work; requires
    /// `CommandPayload::BoxSelect`.
    SelectInBox,
    Measure,
    /// Confirm the points captured by the active open-ended measurement tool.
    ConfirmMeasurement,
    /// Cancel the active tool without committing anything.
    CancelMeasurement,
    Undo,
    Redo,
    Resources,
    Diagnostics,
    SwitchBackend,
    Switch2d3d,
    StandardView,
    Orbit,
    SwitchProjection,
    /// Switch the session between Viewer and Work mode (audit U02).
    ///
    /// Allowed in both directions and in both modes: the mode change itself is
    /// not a Work-only operation, but it cancels any unconfirmed tool. The
    /// command layer still refuses every Work-only command while in Viewer mode.
    SetMode,
    /// Draw a LINE through two payload points into model space. Work-only.
    CreateLine,
    /// Draw a CIRCLE with a centre and an edge point. The radius is the exact
    /// centre→edge distance and must be positive. Work-only.
    CreateCircle,
    /// Draw an ARC through three points (start, a point on the arc, end) on the
    /// work plane. The three points must be non-collinear and finite; a
    /// degenerate input is an explicit refusal that changes nothing. Work-only.
    CreateArc,
    /// Draw an open LWPOLYLINE through the payload points (at least two, all
    /// finite and not all identical) on the active layer. Straight segments
    /// only (one zero bulge per vertex, `closed == false`); fewer than two
    /// distinct points is an explicit refusal that changes nothing. Work-only.
    CreatePolyline,
    /// Draw an axis-aligned RECTANG from two opposite corners: a closed
    /// four-vertex LWPOLYLINE (one zero bulge per vertex) on the active layer.
    /// Payload is `Points([corner1, corner2])` in click order; the handler
    /// canonicalises the winding and refuses a non-finite, degenerate (zero
    /// width/height) or non-coplanar pair. Work-only.
    CreateRectangle,
    /// Draw a full ELLIPSE by AutoCAD's **axis-endpoint method**: the two
    /// endpoints of the first axis plus a third point giving the distance to the
    /// other axis. Payload is `Points([p1, p2, p3])` in click order; the handler
    /// builds the `SemanticGeometry::Ellipse` (centre, `+Z` normal, longer axis as
    /// `major_axis`, `ratio <= 1`) and refuses a non-finite, degenerate
    /// (coincident endpoints / third point on the first axis) or non-coplanar
    /// triple. The centre method is not implemented. Work-only.
    CreateEllipse,
    /// Draw a closed regular POLYGON **inscribed** in the circle through a centre
    /// pick and a radius point. Payload is `Points([center, radius_point])` in
    /// click order; the side count is the session sysvar
    /// ([`crate::SessionState::polygon_sides`], default 4, range 3..=1024), not a
    /// payload field. The handler builds the closed `n`-vertex
    /// `SemanticGeometry::Polyline` (one zero bulge per vertex) and refuses a
    /// non-finite, zero-radius, non-coplanar or out-of-range input. Work-only.
    CreatePolygon,
    /// Place a single POINT at the payload position. Payload is
    /// `Points([position])` in click order; the handler stores
    /// `SemanticGeometry::Point` and refuses a non-finite position, a
    /// mismatched payload (a `Geometry` or a non-singleton point list) and a
    /// missing active layer. A single point has no coplanarity constraint (it is
    /// trivially coplanar), so the `+Z` refusal is vacuous. AutoCAD documents one
    /// `Specify a point`; **repeat-until-Enter placement is NOT IMPLEMENTED**
    /// (unverified). Work-only.
    CreatePoint,
    /// Draw a fit-point SPLINE through the payload fit points (at least four,
    /// degree 3, clamped chord knots) as one `SemanticGeometry::Spline`. The
    /// handler delegates the interpolation to
    /// `cad_geometry::spline::fit_points_to_cubic_spline` and refuses fewer than
    /// four fit points, non-finite or non-coplanar `+Z` points, a mismatched
    /// payload, a missing active layer, and every degenerate-chord / numerically
    /// singular input the geometry core raises. Work-only.
    CreateSpline,
    /// DIMLINEAR: draw a **horizontal or vertical** linear dimension through two
    /// extension-line origins and a dimension-line location (ADR 0017 decision C,
    /// P5c). Payload is `Points([first, second, location])` in click order. The
    /// handler decides horizontal vs vertical from the location pick — AutoCAD's
    /// rule: the location determines the direction — and refuses a non-plan
    /// (off-one-`+Z`-plane) triple, coincident origins, an ambiguous direction and
    /// a zero measurement. The dimension is built through the shared synthesis
    /// ([`cad_domain::DimensionGeometry::synthesized`]) from the resolved current
    /// style (`Standard`) and inserted on the active layer in one transaction /
    /// one undo step. Non-associative. Work-only.
    DimLinear,
    /// DIMALIGNED: draw a dimension whose line is **parallel** to the line joining
    /// the two extension-line origins (ADR 0017 decision C, P5c). Payload is
    /// `Points([first, second, location])`; the handler measures the true
    /// origin-to-origin distance and refuses a non-plan triple, coincident origins
    /// and a zero measurement. Same single-entity / one-transaction / active-layer
    /// funnel as [`CommandId::DimLinear`]. Non-associative. Work-only.
    DimAligned,
    /// DIMRADIUS: draw a **radius** dimension for a picked arc/circle (ADR 0017
    /// decision C, P5d). The **only** entry point is an entity pick of an arc or
    /// circle (a fillet arc counts), followed by the dimension-line location.
    /// Payload is `DimRadial { target, location }`. The handler resolves the
    /// entity, places the leader point on the circle along the location pick's
    /// direction, stores the centre in the group-10 slot, and refuses a
    /// non-arc/circle pick, an off-plane entity or location, a degenerate radius
    /// and a location at the centre. Built through the shared synthesis from the
    /// resolved current style (`Standard`) and inserted on the active layer in one
    /// transaction / one undo step. Non-associative. Work-only.
    DimRadius,
    /// DIMDIAMETER: draw a **diameter** dimension for a picked arc/circle (ADR
    /// 0017 decision C, P5d). Same two-step capture as [`CommandId::DimRadius`];
    /// the group-10 slot is the **opposite chord point** and the measurement is the
    /// diameter. Same single-entity / one-transaction / active-layer funnel.
    /// Non-associative. Work-only.
    DimDiameter,
    /// DIMCENTER: create the **non-associative centre mark or centrelines** of a
    /// picked arc/circle as **plain geometry** (ADR 0017 decision C/D.7, P5d) —
    /// ordinary LINE entities, **not** a `DIMENSION`. Payload is
    /// `DimCenter(target)`. The resolved `DIMCEN` drives the result: `> 0` a centre
    /// mark, `< 0` centrelines, `0` nothing (an explicit diagnostic with no
    /// transaction). One transaction / one undo step on the active layer; refuses a
    /// non-arc/circle pick, an off-plane entity, a degenerate radius and a missing
    /// active layer. Work-only.
    DimCenter,
    /// Translate one or more selected entities by a world delta. Exactly one
    /// transaction and one undo step for the whole selection. Work-only.
    MoveEntities,
    /// Trim one LINE against LINE/LWPOLYLINE segments (the honest subset of
    /// §3 of `docs/drawing-edit.md`); anything else is an explicit refusal that
    /// changes nothing. Work-only.
    TrimEntity,
    /// Extend one LINE to the nearest LINE/LWPOLYLINE boundary segment beyond
    /// the picked end (the honest subset of EXTEND); anything else is an
    /// explicit refusal that changes nothing. Work-only.
    ExtendEntity,
    /// CHAMFER two straight lines: trim/extend both to the corner of the two
    /// infinite lines (when both distances are 0) or to the offset points, joining
    /// them with one new LINE (ADR 0016 decision C). The two picks are the two
    /// picked lines; the distances come from the session typed channel
    /// ([`crate::draw_tool::ToolParameter::ChamferDistanceA`]/`B`). One
    /// transaction and one undo step. Work-only.
    ChamferLines,
    /// FILLET two straight lines: trim/extend both to the corner (radius 0) or to
    /// the tangent points of a tangent ARC of the session radius, on the pick side
    /// (ADR 0016 decision B). The two picks are the two picked lines; the radius
    /// comes from the session typed channel
    /// ([`crate::draw_tool::ToolParameter::FilletRadius`]). One transaction and one
    /// undo step. Work-only.
    FilletLines,
    /// ARRAYRECT: insert **independent** copies of the current selection on a
    /// rectangular grid, growing up and to the right (ADR 0016 decision D).
    /// Payload is `ArrayRect { refs }`; the row/column counts and spacings are the
    /// session typed channel
    /// ([`crate::draw_tool::ToolParameter::ArrayRows`] … `ArrayColumnSpacing`).
    /// The source objects are **retained** and the base point is the selection's
    /// bounding-box lower-left. Every copy is inserted in one transaction and one
    /// undo step; a total above the explicit cap
    /// ([`crate::app_drawing::MAX_ARRAY_ENTITIES`]) is refused, never truncated.
    /// Associativity is **NOT IMPLEMENTED**. Work-only.
    ArrayRectangular,
    /// ARRAYPOLAR: insert **independent** copies of the current selection around a
    /// captured centre point (ADR 0016 decision D). Payload is
    /// `ArrayPolar { refs, center }`; the item count, fill angle and rotate-items
    /// flag are the session typed channel
    /// ([`crate::draw_tool::ToolParameter::ArrayItems`] … `ArrayRotateItems`). The
    /// base point is the selection's bounding-box lower-left; the placement rule
    /// is our declared (unverified) choice. One transaction and one undo step;
    /// associativity is **NOT IMPLEMENTED**. Work-only.
    ArrayPolar,
    /// ALIGN the current selection from two point pairs (ADR 0016 decision F).
    /// Payload is `Align { refs, source1, dest1, source2, dest2 }`; the
    /// `AlignScale` flag is the session typed channel
    /// ([`crate::draw_tool::ToolParameter::AlignScale`]). The transform translates
    /// `source1 → dest1`, rotates by the angle between the two source/destination
    /// directions, and (only when the flag is true) applies a uniform scale of
    /// `|dest2 − dest1| / |source2 − source1|` about `source1`; it is applied to
    /// the whole selection **in place** in one transaction / one undo step. A
    /// non-finite point, a degenerate source or destination pair, an empty
    /// selection or untransformable geometry is an explicit refusal that changes
    /// nothing. Work-only.
    AlignEntities,
    /// STRETCH the entities inside a **crossing window**: a fully enclosed entity
    /// is translated by the displacement; a partially enclosed LINE/LWPOLYLINE
    /// has only its in-window vertices moved (bulge preserved, UNVERIFIED). The
    /// window is always CROSSING (AutoCAD requires one); the enclosure
    /// classification is recomputed from the rectangle and the geometry — never
    /// stored in the selection. Payload is
    /// `Stretch { rect_logical, base, delta }`. One transaction and one undo
    /// step for the whole command; an in-window entity that cannot be handled
    /// fails the whole command (nothing written). Work-only.
    StretchEntities,
    /// Delete every entity in the current selection in one transaction. An
    /// empty selection is an explicit refusal. Work-only.
    EraseEntities,
    /// EXPLODE the current selection one level at a time (ADR 0016 decision G):
    /// a LWPOLYLINE becomes straight segments plus ARCs (bulge segments through
    /// `bulge_arc_geometry`), a uniformly scaled INSERT becomes its block members
    /// with the placement baked in (nested inserts expanded under a cycle stack
    /// and a depth cap of 32), and a Compound becomes **all** of its children (a
    /// MINSERT yields its Insert instances). Members keep their own
    /// block-definition layers. Refusals (nothing written): a selection with
    /// nothing decomposable; `TEXT`/`SPLINE`/`SHAPE`/`OPAQUE` and mirrored
    /// Arc/Ellipse children via the transform surface; `IMAGE`/`MASK` by policy;
    /// `Dimension`/`MTable`/`MLeader` compounds by `type_key`; a non-uniformly
    /// scaled INSERT; and an INSERT with an untransformable member (the whole
    /// command fails). One transaction and one undo step. Work-only.
    ExplodeEntities,
    /// Clone every entity in the current selection with a fresh id, translating
    /// its geometry by a world delta, in one transaction. An empty selection or
    /// a geometry that cannot be transformed is an explicit refusal. Work-only.
    CopyEntities,
    /// Rotate the current selection about a base point in the XY plane by an
    /// absolute angle (radians). One transaction and one undo step. Work-only.
    RotateEntities,
    /// Scale the current selection uniformly about a base point by a positive
    /// factor. One transaction and one undo step. Work-only.
    ScaleEntities,
    /// Mirror the current selection across the line through two points, keeping
    /// the originals and adding mirrored copies in one transaction. Work-only.
    MirrorEntities,
    /// Capture the current selection into the in-process clipboard. Read-only
    /// (the drawing and history are untouched), so it is allowed in Viewer too.
    CopyToClipboard,
    /// Capture the current selection into the clipboard, then delete it in one
    /// transaction and one undo step. Work-only.
    CutToClipboard,
    /// Insert fresh-id clones of the clipboard entities at their original
    /// coordinates in one transaction (PASTEORIG). Work-only.
    PasteFromClipboard,
    /// Create an offset copy of a single LINE/CIRCLE source, at the distance and
    /// side given by the picked point. Another geometry kind is an explicit
    /// `Unsupported` refusal. Work-only.
    OffsetEntity,
    /// Set the session's active layer for new drawing entities. Session state
    /// only; no database write. Work-only.
    SetActiveLayer,
    /// Flip the idle hover entity preview for this session. Session/display
    /// state only (no database write, no history, no ChangeSet), so it is
    /// allowed in Viewer and Work. It defaults to on (AutoCAD's
    /// `SELECTIONPREVIEW` initial `3`); see
    /// [`crate::SessionState::hover_preview`].
    ToggleHoverPreview,
    /// Flip object snapping for this session (ADR 0014 A.4). Session/display
    /// state only (no database write, no history, no ChangeSet), so it is
    /// allowed in Viewer and Work. It defaults to on; independent of the
    /// persisted `view.overlays.snapHints` preference, which only gates the hint
    /// overlay. See [`crate::SessionState::snap_enabled`].
    ToggleSnap,
}

impl CommandId {
    pub fn requires_work_mode(self) -> bool {
        matches!(
            self,
            Self::Measure
                | Self::ConfirmMeasurement
                | Self::Undo
                | Self::Redo
                | Self::CreateLine
                | Self::CreateCircle
                | Self::CreateArc
                | Self::CreatePolyline
                | Self::CreateRectangle
                | Self::CreateEllipse
                | Self::CreatePolygon
                | Self::CreatePoint
                | Self::CreateSpline
                | Self::DimLinear
                | Self::DimAligned
                | Self::DimRadius
                | Self::DimDiameter
                | Self::DimCenter
                | Self::MoveEntities
                | Self::TrimEntity
                | Self::ExtendEntity
                | Self::ChamferLines
                | Self::FilletLines
                | Self::ArrayRectangular
                | Self::ArrayPolar
                | Self::AlignEntities
                | Self::StretchEntities
                | Self::EraseEntities
                | Self::ExplodeEntities
                | Self::CopyEntities
                | Self::RotateEntities
                | Self::ScaleEntities
                | Self::MirrorEntities
                | Self::CutToClipboard
                | Self::PasteFromClipboard
                | Self::OffsetEntity
                | Self::SetActiveLayer
        )
    }
}
