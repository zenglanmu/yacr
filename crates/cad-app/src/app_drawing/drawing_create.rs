use super::*;

impl Application {
    /// Dispatch the four drawing/edit commands.
    pub(crate) fn drawing_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match command.id {
            CommandId::CreateLine => self.create_line(session, command),
            CommandId::CreateCircle => self.create_circle(session, command),
            CommandId::CreateArc => self.create_arc(session, command),
            CommandId::CreatePolyline => self.create_polyline(session, command),
            CommandId::CreateRectangle => self.create_rectangle(session, command),
            CommandId::CreateEllipse => self.create_ellipse(session, command),
            CommandId::CreatePolygon => self.create_polygon(session, command),
            CommandId::CreatePoint => self.create_point(session, command),
            CommandId::CreateSpline => self.create_spline(session, command),
            CommandId::MoveEntities => self.move_entities(command),
            CommandId::TrimEntity => self.trim_entity(command),
            CommandId::ExtendEntity => self.extend_entity(command),
            CommandId::ChamferLines => self.chamfer_lines(session, command),
            CommandId::FilletLines => self.fillet_lines(session, command),
            CommandId::ArrayRectangular => self.array_rect(session, command),
            CommandId::ArrayPolar => self.array_polar(session, command),
            CommandId::EraseEntities => self.erase_entities(session, command),
            CommandId::CopyEntities => self.copy_entities(command),
            CommandId::RotateEntities => self.rotate_entities(command),
            CommandId::ScaleEntities => self.scale_entities(command),
            CommandId::MirrorEntities => self.mirror_entities(command),
            CommandId::CopyToClipboard => self.copy_to_clipboard(session, command),
            CommandId::CutToClipboard => self.cut_to_clipboard(session, command),
            CommandId::PasteFromClipboard => self.paste_from_clipboard(session, command),
            CommandId::OffsetEntity => self.offset_entity(command),
            CommandId::SetActiveLayer => self.set_active_layer(session, command),
            _ => Err(CadError::InvalidInput(
                "not a drawing/edit command".to_string(),
            )),
        }
    }

    /// Set the session's active drawing layer (session state only).
    ///
    /// The layer need not exist yet: the *create* commands are the point where a
    /// missing layer becomes an explicit `InvalidInput`, so a UI may set a
    /// preference before the table is fully known. This command performs no
    /// database write and records no history.
    pub(crate) fn set_active_layer(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ActiveLayer(layer) = command.payload else {
            return Err(CadError::InvalidInput(
                "SetActiveLayer needs an ActiveLayer(LayerId) payload".into(),
            ));
        };
        session.active_layer = layer;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "drawing.active_layer".into(),
                message: format!("活动图层：{}", layer.0),
            }],
            measurement: None,
        })
    }

    /// Insert a LINE into model space on the session's active layer.
    ///
    /// Accepts either `Points([start, end])` (the UI path) or a fully formed
    /// `Geometry(SemanticGeometry::Line { .. })` (the programmatic path). The
    /// geometry is validated by the database before it is committed, so a
    /// non-finite or degenerate line is refused with no state change.
    pub(crate) fn create_line(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let geometry = match &command.payload {
            CommandPayload::Points(points) => {
                let start = *points.first().ok_or_else(|| {
                    CadError::InvalidInput("CreateLine needs a start point".into())
                })?;
                let end = *points.get(1).ok_or_else(|| {
                    CadError::InvalidInput("CreateLine needs an end point".into())
                })?;
                SemanticGeometry::Line { start, end }
            }
            CommandPayload::Geometry(geometry) => match geometry.as_ref() {
                SemanticGeometry::Line { start, end } => SemanticGeometry::Line {
                    start: *start,
                    end: *end,
                },
                other => {
                    return Err(CadError::InvalidInput(format!(
                        "CreateLine needs a Line geometry, got {}",
                        geometry_kind(other)
                    )));
                }
            },
            _ => {
                return Err(CadError::InvalidInput(
                    "CreateLine needs a Points([start, end]) or Geometry payload".into(),
                ));
            }
        };
        self.commit_new_entity(session, command, geometry, "create line")
    }

    /// Insert a CIRCLE with radius = |edge - center| into model space.
    ///
    /// A zero (or non-finite) radius is an explicit `InvalidInput` and leaves
    /// the database revision unchanged, never a degenerate circle.
    pub(crate) fn create_circle(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "CreateCircle needs a Points([center, edge]) payload".into(),
            ));
        };
        let center = *points
            .first()
            .ok_or_else(|| CadError::InvalidInput("CreateCircle needs a center point".into()))?;
        let edge = *points
            .get(1)
            .ok_or_else(|| CadError::InvalidInput("CreateCircle needs an edge point".into()))?;
        let radius = distance(center, edge);
        if !radius.is_finite() || radius <= 0.0 {
            return Err(CadError::InvalidInput(
                "CreateCircle radius must be a positive finite distance".into(),
            ));
        }
        let geometry = SemanticGeometry::Circle {
            center,
            // The zero-radius guard above, plus the finite-radius check, means
            // the normal is only a documented plane hint; a flat circle uses the
            // work-plane +Z normal, never a fabricated tilt.
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius,
        };
        self.commit_new_entity(session, command, geometry, "create circle")
    }

    /// Insert an ARC through three points into model space.
    ///
    /// Accepts `Points([start, through, end])` (the UI path) or a fully formed
    /// `Geometry(SemanticGeometry::Arc { .. })` (the programmatic path). The
    /// circumcircle is a work-plane `+Z` construction; collinear, coincident or
    /// non-finite points are an explicit refusal that creates nothing.
    pub(crate) fn create_arc(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let geometry = match &command.payload {
            CommandPayload::Points(points) => {
                let start = *points.first().ok_or_else(|| {
                    CadError::InvalidInput("CreateArc needs a start point".into())
                })?;
                let through = *points.get(1).ok_or_else(|| {
                    CadError::InvalidInput("CreateArc needs an on-arc point".into())
                })?;
                let end = *points
                    .get(2)
                    .ok_or_else(|| CadError::InvalidInput("CreateArc needs an end point".into()))?;
                let parameters = crate::draw_tool::arc_parameters(start, through, end)?;
                SemanticGeometry::Arc {
                    center: parameters.center,
                    // The circumcircle is a work-plane construction; a flat arc
                    // uses +Z, never a fabricated tilt.
                    normal: Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 1.0,
                    },
                    radius: parameters.radius,
                    start: parameters.start,
                    sweep: parameters.sweep,
                }
            }
            CommandPayload::Geometry(geometry) => match geometry.as_ref() {
                SemanticGeometry::Arc {
                    center,
                    normal,
                    radius,
                    start,
                    sweep,
                } => SemanticGeometry::Arc {
                    center: *center,
                    normal: *normal,
                    radius: *radius,
                    start: *start,
                    sweep: *sweep,
                },
                other => {
                    return Err(CadError::InvalidInput(format!(
                        "CreateArc needs an Arc geometry, got {}",
                        geometry_kind(other)
                    )));
                }
            },
            _ => {
                return Err(CadError::InvalidInput(
                    "CreateArc needs a Points([start, through, end]) or Geometry payload".into(),
                ));
            }
        };
        self.commit_new_entity(session, command, geometry, "create arc")
    }

    /// Insert an open LWPOLYLINE through the payload points into model space.
    ///
    /// Accepts `Points(points)` (the UI/capture path) or a fully formed
    /// `Geometry(SemanticGeometry::Polyline { .. })` (the programmatic path,
    /// mirroring `CreateLine`/`CreateArc`). The honest subset is straight
    /// segments only: one zero bulge per vertex and `closed == false`. Fewer
    /// than two points, any non-finite point, or fewer than two **distinct**
    /// vertices (for example all points identical) is an explicit refusal that
    /// creates nothing — a programmatic closed polyline or one carrying bulges
    /// is refused rather than silently rewritten to the supported subset.
    pub(crate) fn create_polyline(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let geometry = match &command.payload {
            CommandPayload::Points(points) => polyline_geometry(points.clone())?,
            CommandPayload::Geometry(geometry) => match geometry.as_ref() {
                SemanticGeometry::Polyline {
                    points,
                    bulges,
                    closed,
                } => {
                    if *closed || bulges.iter().any(|bulge| bulge.abs() > 0.0) {
                        return Err(CadError::InvalidInput(
                            "CreatePolyline authors neither arc segments (bulges) nor closed \
                             polylines"
                                .into(),
                        ));
                    }
                    polyline_geometry(points.clone())?
                }
                other => {
                    return Err(CadError::InvalidInput(format!(
                        "CreatePolyline needs a Polyline geometry, got {}",
                        geometry_kind(other)
                    )));
                }
            },
            _ => {
                return Err(CadError::InvalidInput(
                    "CreatePolyline needs a Points or Geometry(Polyline) payload".into(),
                ));
            }
        };
        self.commit_new_entity(session, command, geometry, "create polyline")
    }

    /// Insert an axis-aligned rectangle as a **closed** four-vertex LWPOLYLINE.
    ///
    /// Payload choice: `Points([corner1, corner2])` — the two opposite corners the
    /// user clicked, in click order. This is the same shape LINE/PLINE use, and it
    /// is the *input* the capture produced, not an output geometry: RECTANG means
    /// "two corners", so a `Geometry(SemanticGeometry::Polyline)` arm would have to
    /// accept exactly one canonical closed four-vertex straight polyline and
    /// re-derive the two corners from it, adding a second source of truth for a
    /// caller who already holds the finished polyline and does not need a rectangle
    /// command at all. Like `CreateCircle` (also `Points`-only) the payload is the
    /// construction input, so only the `Points` arm is accepted; anything else is an
    /// explicit refusal that changes nothing.
    ///
    /// The geometry is built by [`rectangle_geometry`], the single
    /// construction/validation point, and committed through the shared
    /// [`Self::commit_new_entity`] funnel so the rectangle is one entity, one
    /// transaction and one undo step, with the active-layer check (a missing layer
    /// is refused there) intact.
    pub(crate) fn create_rectangle(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "CreateRectangle needs a Points([corner1, corner2]) payload".into(),
            ));
        };
        let corner1 = *points.first().ok_or_else(|| {
            CadError::InvalidInput("CreateRectangle needs a first corner point".into())
        })?;
        let corner2 = *points.get(1).ok_or_else(|| {
            CadError::InvalidInput("CreateRectangle needs a second corner point".into())
        })?;
        let geometry = rectangle_geometry(corner1, corner2)?;
        self.commit_new_entity(session, command, geometry, "create rectangle")
    }

    /// Insert a full ELLIPSE by the axis-endpoint method as one entity.
    ///
    /// Payload choice: `Points([p1, p2, p3])` — the two endpoints of the first
    /// axis and the third pick that gives the other half-axis distance, exactly
    /// as clicked (the construction input, like `CreateCircle`/`CreateRectangle`;
    /// a pre-built `SemanticGeometry::Ellipse` arm would be a second source of
    /// truth for a caller who already holds the geometry).
    ///
    /// The geometry is built by [`crate::draw_tool::ellipse_parameters`] — the
    /// single construction/validation point shared with the preview — which
    /// stores the **longer** semi-axis as `major_axis` (`ratio <= 1`) and refuses
    /// a non-finite, degenerate (coincident first-axis endpoints, or a third pick
    /// on the first axis) or non-coplanar triple. The ellipse is committed through
    /// the shared [`Self::commit_new_entity`] funnel, so it is one entity, one
    /// transaction and one undo step, with the active-layer check intact. Only a
    /// full ellipse is authored (`start = 0`, `sweep = 2π`); the centre method and
    /// ellipse arcs are NOT IMPLEMENTED.
    pub(crate) fn create_ellipse(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "CreateEllipse needs a Points([p1, p2, p3]) payload".into(),
            ));
        };
        let p1 = *points
            .first()
            .ok_or_else(|| CadError::InvalidInput("CreateEllipse needs p1".into()))?;
        let p2 = *points
            .get(1)
            .ok_or_else(|| CadError::InvalidInput("CreateEllipse needs p2".into()))?;
        let p3 = *points
            .get(2)
            .ok_or_else(|| CadError::InvalidInput("CreateEllipse needs p3".into()))?;
        let frame = crate::draw_tool::ellipse_parameters(p1, p2, p3)?;
        let geometry = SemanticGeometry::Ellipse {
            center: frame.center,
            // The axis-endpoint method is a work-plane construction; a full
            // ellipse uses +Z, never a fabricated tilt.
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            major_axis: frame.major_axis,
            ratio: frame.ratio,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        };
        self.commit_new_entity(session, command, geometry, "create ellipse")
    }

    /// Insert a closed regular POLYGON inscribed in the circle through the two
    /// picks, as one entity.
    ///
    /// Payload choice: `Points([center, radius_point])` — the centre and a point
    /// on the circumscribed circle, exactly as clicked (the construction input,
    /// like `CreateCircle`/`CreateRectangle`). The **side count** is the session
    /// sysvar ([`SessionState::polygon_sides`], ADR 0015 decision C), read here
    /// rather than carried on the payload, so the session is the single authority
    /// for the count and the payload stays the raw click pair.
    ///
    /// The geometry is built by [`crate::draw_tool::polygon_vertices`] — the
    /// single construction/validation point shared with the preview — which
    /// places the first vertex on the centre→radius ray and steps the rest by
    /// `2π/n` about `+Z`, and refuses a non-finite, zero-radius, non-coplanar or
    /// out-of-range input. The polygon is committed through the shared
    /// [`Self::commit_new_entity`] funnel, so it is one entity, one transaction
    /// and one undo step, with the active-layer check intact. Only the inscribed
    /// (AutoCAD default) construction is authored; the `Edge` branch and the
    /// circumscribed mode are NOT IMPLEMENTED.
    pub(crate) fn create_polygon(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "CreatePolygon needs a Points([center, radius_point]) payload".into(),
            ));
        };
        let center = *points
            .first()
            .ok_or_else(|| CadError::InvalidInput("CreatePolygon needs a center point".into()))?;
        let radius_point = *points
            .get(1)
            .ok_or_else(|| CadError::InvalidInput("CreatePolygon needs a radius point".into()))?;
        let vertices =
            crate::draw_tool::polygon_vertices(center, radius_point, session.polygon_sides())?;
        let geometry = SemanticGeometry::Polyline {
            bulges: vec![0.0; vertices.len()],
            points: vertices,
            closed: true,
        };
        self.commit_new_entity(session, command, geometry, "create polygon")
    }

    /// Place a single POINT into model space on the session's active layer.
    ///
    /// Payload choice: `Points([position])` — the construction input, exactly as
    /// clicked (like `CreateCircle`/`CreateRectangle`/`CreatePolygon`). Only a
    /// single point is accepted; a `Geometry` payload or a point list of any other
    /// length is an explicit refusal that changes nothing. The point is committed
    /// through the shared [`Self::commit_new_entity`] funnel, so it is one entity,
    /// one transaction and one undo step, with the active-layer check intact.
    ///
    /// A single point has **no** coplanarity constraint — it is trivially coplanar
    /// on the `+Z` plane through itself — so the ADR's "non-coplanar `+Z`" refusal
    /// is vacuous here rather than a fabricated constraint. AutoCAD documents one
    /// `Specify a point`; repeat-until-Enter placement is NOT IMPLEMENTED (its
    /// behaviour is unverified).
    pub(crate) fn create_point(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "CreatePoint needs a Points([position]) payload".into(),
            ));
        };
        if points.len() != 1 {
            return Err(CadError::InvalidInput(format!(
                "CreatePoint places exactly one point, got {}",
                points.len()
            )));
        }
        let position = points[0];
        if !is_finite_point(position) {
            return Err(CadError::InvalidInput(
                "CreatePoint position must be finite".into(),
            ));
        }
        self.commit_new_entity(
            session,
            command,
            SemanticGeometry::Point(position),
            "create point",
        )
    }

    /// Draw a fit-point SPLINE as one `SemanticGeometry::Spline` entity.
    ///
    /// Payload choice: `Points(fit_points)` — the ordered fit points the user
    /// clicked (the construction input, like `CreateCircle`/`CreatePolygon`; a
    /// pre-built `Spline` geometry would be a second source of truth for a caller
    /// who already holds the geometry). The interpolation is delegated to
    /// [`crate::draw_tool::spline_control_points`], the pure geometry core, so the
    /// handler never reimplements it: fewer than four fit points, a non-finite or
    /// non-coplanar `+Z` point, a mismatched payload, and every degenerate-chord /
    /// numerically singular input the core raises are explicit refusals that
    /// create nothing. The stored spline is degree 3 with clamped chord knots and
    /// unit weights (fit points do **not** round-trip: the domain has no fit-point
    /// field). It is committed through the shared [`Self::commit_new_entity`]
    /// funnel: one entity, one transaction, one undo step, active-layer check
    /// intact.
    pub(crate) fn create_spline(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(fit_points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "CreateSpline needs a Points(fit_points) payload".into(),
            ));
        };
        let (knots, control_points) = crate::draw_tool::spline_control_points(fit_points)?;
        let weights = vec![1.0; control_points.len()];
        let geometry = SemanticGeometry::Spline {
            degree: cad_geometry::FIT_SPLINE_DEGREE,
            knots,
            control_points,
            weights,
        };
        self.commit_new_entity(session, command, geometry, "create spline")
    }
}

/// The minimum separation (world units) for two polyline vertices to count as
/// distinct. Below this a capture is a repeated single point and has no segment.
const MIN_POLYLINE_VERTEX_SEPARATION: f64 = 1e-12;

/// Build a straight-segment open polyline from a captured point list.
///
/// The single construction/validation point shared by both payload arms, so the
/// UI and programmatic paths cannot drift. Refuses explicitly (never a silent
/// fix-up) when there are fewer than two points, any point is non-finite, or
/// fewer than two vertices are distinct.
fn polyline_geometry(points: Vec<Point3>) -> CadResult<SemanticGeometry> {
    if points.len() < 2 {
        return Err(CadError::InvalidInput(
            "CreatePolyline needs at least two points".into(),
        ));
    }
    if !points.iter().all(|point| is_finite_point(*point)) {
        return Err(CadError::InvalidInput(
            "CreatePolyline points must all be finite".into(),
        ));
    }
    let first = points[0];
    let has_distinct_vertex = points
        .iter()
        .any(|point| distance(first, *point) > MIN_POLYLINE_VERTEX_SEPARATION);
    if !has_distinct_vertex {
        return Err(CadError::InvalidInput(
            "CreatePolyline needs at least two distinct points".into(),
        ));
    }
    let bulges = vec![0.0; points.len()];
    Ok(SemanticGeometry::Polyline {
        points,
        bulges,
        closed: false,
    })
}

/// The minimum width/height (world units) for a rectangle to have any area.
///
/// A capture whose two corners share an `x` (zero width) or a `y` (zero height)
/// collapses to a segment or a point and has no rectangle; like
/// [`MIN_POLYLINE_VERTEX_SEPARATION`] the threshold is a tiny relative-free world
/// epsilon so an exactly-degenerate click pair is refused while any real span is
/// accepted.
const MIN_RECTANGLE_EXTENT: f64 = 1e-12;

/// Build the closed four-vertex rectangle through two opposite corners.
///
/// The two clicks are two opposite corners and do **not** define a winding, so the
/// four stored vertices are canonicalised to a counter-clockwise outline starting
/// at the lower-left corner:
/// `(min_x, min_y) → (max_x, min_y) → (max_x, max_y) → (min_x, max_y)`, closed.
/// This makes the stored geometry identical whichever diagonal the user clicked
/// (the click order only reaches the DXF round trip through this canonical order),
/// and gives a stable vertex order for an area/measurement consumer. The `z` plane
/// is the `+Z` work plane of the first corner.
///
/// Explicit refusals that create nothing (never a silent fix-up, projection or
/// degenerate entity): a non-finite corner; two corners that are not coplanar on
/// one `+Z` plane; and a zero-width or zero-height capture (a click-click on the
/// same axis line).
fn rectangle_geometry(corner1: Point3, corner2: Point3) -> CadResult<SemanticGeometry> {
    if !is_finite_point(corner1) || !is_finite_point(corner2) {
        return Err(CadError::InvalidInput(
            "CreateRectangle corners must be finite".into(),
        ));
    }
    // Flat `+Z` subset: a rectangle is a planar construction, so a corner pair
    // that is not coplanar on one plane is refused rather than projected onto the
    // first corner's plane (ADR 0015, non-planar input is NOT IMPLEMENTED).
    let z_tol = 1e-9 * (1.0 + corner1.z.abs() + corner2.z.abs());
    if (corner2.z - corner1.z).abs() > z_tol {
        return Err(CadError::InvalidInput(
            "CreateRectangle corners must lie in one +Z plane".into(),
        ));
    }
    let min_x = corner1.x.min(corner2.x);
    let max_x = corner1.x.max(corner2.x);
    let min_y = corner1.y.min(corner2.y);
    let max_y = corner1.y.max(corner2.y);
    if max_x - min_x <= MIN_RECTANGLE_EXTENT {
        return Err(CadError::InvalidInput(
            "CreateRectangle needs two corners with distinct X (zero width)".into(),
        ));
    }
    if max_y - min_y <= MIN_RECTANGLE_EXTENT {
        return Err(CadError::InvalidInput(
            "CreateRectangle needs two corners with distinct Y (zero height)".into(),
        ));
    }
    let z = corner1.z;
    let points = vec![
        Point3 {
            x: min_x,
            y: min_y,
            z,
        },
        Point3 {
            x: max_x,
            y: min_y,
            z,
        },
        Point3 {
            x: max_x,
            y: max_y,
            z,
        },
        Point3 {
            x: min_x,
            y: max_y,
            z,
        },
    ];
    Ok(SemanticGeometry::Polyline {
        bulges: vec![0.0; points.len()],
        points,
        closed: true,
    })
}
