use super::text_policy::{has_line_break, single_line_newline_error};
// The authored-geometry validators of PLINE live next to the draw variants so
// `drawing_create` stays inside the file-size limit; they are the single validation
// point of the programmatic `Geometry` payload arm. ELLIPSE/SPLINE live there too
// (`create_ellipse` / `create_spline`).
use super::draw_variants::authored_polyline_geometry;
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
            CommandId::CreateXLine => self.create_xline(session, command),
            CommandId::CreateRay => self.create_ray(session, command),
            CommandId::CreateCircle => self.create_circle(session, command),
            CommandId::CreateCircle2P => self.create_circle_2p(session, command),
            CommandId::CreateCircle3P => self.create_circle_3p(session, command),
            CommandId::CreateCircleDiameter => self.create_circle_diameter(session, command),
            CommandId::CreateArc => self.create_arc(session, command),
            CommandId::CreatePolyline => self.create_polyline(session, command),
            CommandId::Wipeout => self.create_wipeout(session, command),
            CommandId::CreateRectangle => self.create_rectangle(session, command),
            CommandId::CreateEllipse => self.create_ellipse(session, command),
            CommandId::CreatePolygon => self.create_polygon(session, command),
            CommandId::CreatePolygonCircumscribed => {
                self.create_polygon_circumscribed(session, command)
            }
            CommandId::CreatePoint => self.create_point(session, command),
            CommandId::CreateSpline => self.create_spline(session, command),
            CommandId::CreateText => self.create_text(session, command),
            CommandId::CreateMText => self.create_mtext(session, command),
            CommandId::CreateShape => self.create_shape(session, command),
            CommandId::MLeader => self.create_mleader(session, command),
            CommandId::TextEdit => self.edit_text(command),
            CommandId::InsertBlock => self.insert_block(session, command),
            CommandId::DefineBlock => self.define_block(session, command),
            CommandId::DimLinear => self.dim_linear(session, command),
            CommandId::DimAligned => self.dim_aligned(session, command),
            CommandId::DimRotated => self.dim_rotated(session, command),
            CommandId::DimSpace => self.dim_space(session, command),
            // The 2026-11-07 dimension-editing batch.
            CommandId::DimTextEdit => self.dim_text_edit(session, command),
            CommandId::DimEdit => self.dim_edit(session, command),
            CommandId::DimOverride => self.dim_override(session, command),
            CommandId::QDim => self.qdim(session, command),
            CommandId::DimJogLine => self.dim_jogline(command),
            CommandId::DimBreak => self.dim_break(command),
            CommandId::DimRadius => self.dim_radius(session, command),
            CommandId::DimDiameter => self.dim_diameter(session, command),
            CommandId::DimCenter => self.dim_center(session, command),
            CommandId::DimAngular => self.dim_angular(session, command),
            CommandId::DimOrdinate => self.dim_ordinate(session, command),
            CommandId::DimBaseline => self.dim_baseline(session, command),
            CommandId::DimContinue => self.dim_continue(session, command),
            CommandId::Hatch => self.create_hatch(session, command),
            CommandId::HatchPoint => self.create_hatch_point(session, command),
            CommandId::Boundary => self.create_boundary(session, command),
            CommandId::HatchGenerateBoundary => self.hatch_generate_boundary(session, command),
            CommandId::HatchToBack => self.hatch_to_back(session, command),
            CommandId::Gradient => self.create_gradient(session, command),
            CommandId::HatchEdit => self.hatch_edit(session, command),
            CommandId::MoveEntities => self.move_entities(command),
            CommandId::TrimEntity => self.trim_entity(command),
            CommandId::ExtendEntity => self.extend_entity(command),
            CommandId::Break => self.break_curve(command),
            CommandId::Divide => self.divide_curve(session, command),
            CommandId::Lengthen => self.lengthen_curve(session, command),
            CommandId::ScaleText => self.scale_text(session, command),
            CommandId::Torient => self.torient(session, command),
            CommandId::TCase => self.tcase(session, command),
            CommandId::Txt2MText => self.txt2mtext(session, command),
            CommandId::TextToFront => self.text_to_front(session, command),
            // The 2026-11-07 2D text batch.
            CommandId::JustifyText => self.justify_text(session, command),
            CommandId::TextScale => self.text_scale(session, command),
            CommandId::TextAlignEdges => self.text_align(session, command),
            CommandId::TextCount => self.text_count(session, command),
            CommandId::TextCircle => self.text_circle(session, command),
            CommandId::TextMask => self.text_mask(session, command),
            CommandId::TextUnmask => self.text_unmask(session, command),
            CommandId::ChamferLines => self.chamfer_lines(session, command),
            CommandId::FilletLines => self.fillet_lines(session, command),
            CommandId::ChamferPolyline => self.chamfer_polyline(session, command),
            CommandId::FilletPolyline => self.fillet_polyline(session, command),
            CommandId::RevisionCloud => self.revision_cloud(session, command),
            CommandId::Pedit => self.pedit(session, command),
            CommandId::Reverse => self.reverse_entities(session, command),
            CommandId::Oops => self.oops_restore(session, command),
            CommandId::Overkill => self.overkill(session, command),
            CommandId::BreakAtPoint => self.break_at_point(command),
            CommandId::ArrayRectangular => self.array_rect(session, command),
            CommandId::ArrayPolar => self.array_polar(session, command),
            CommandId::ArrayPath => self.array_path(session, command),
            CommandId::AlignEntities => self.align_entities(session, command),
            CommandId::StretchEntities => self.stretch_entities(session, command),
            CommandId::EraseEntities => self.erase_entities(session, command),
            CommandId::Join => self.join_entities(session, command),
            CommandId::ExplodeEntities => self.explode_entities(session, command),
            CommandId::CopyEntities => self.copy_entities(command),
            CommandId::RotateEntities => self.rotate_entities(command),
            CommandId::ScaleEntities => self.scale_entities(command),
            CommandId::MirrorEntities => self.mirror_entities(command),
            CommandId::CopyToClipboard => self.copy_to_clipboard(session, command),
            CommandId::CutToClipboard => self.cut_to_clipboard(session, command),
            CommandId::PasteFromClipboard => self.paste_from_clipboard(session, command),
            CommandId::CopyToClipboardBase => self.copy_to_clipboard_base(session, command),
            CommandId::CutToClipboardBase => self.cut_to_clipboard_base(session, command),
            CommandId::PasteOriginalCoordinates => {
                self.paste_original_coordinates(session, command)
            }
            CommandId::PasteBlock => self.paste_block(session, command),
            CommandId::OffsetEntity => self.offset_entity(command),
            // The 2026-11-07 draw/modify batch.
            CommandId::Donut => self.donut(session, command),
            CommandId::Solid => self.solid(session, command),
            CommandId::Helix => self.helix(session, command),
            CommandId::Mline => self.mline(session, command),
            CommandId::MlStyle => self.mlstyle(session, command),
            CommandId::Trex => self.trex(session, command),
            CommandId::Extrim => self.extrim(session, command),
            CommandId::Mpedit => self.mpedit(session, command),
            CommandId::DrawOrder => self.draw_order(session, command),
            CommandId::SetActiveLayer => self.set_active_layer(session, command),
            CommandId::SetEntityProperty => self.set_entity_property(session, command),
            CommandId::MatchProp => self.match_prop(session, command),
            CommandId::GripStretch => self.grip_stretch(command),
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
    ///
    /// # A locked, frozen or off layer cannot be made current
    ///
    /// One rule, one site: [`Self::ensure_layer_can_be_current`], consulted before
    /// the session is touched. The refusal travels the ordinary command error path
    /// with a stable reason code and a sentence naming the layer:
    ///
    /// * `layer.current_locked` — a locked layer is **read-only** in this build:
    ///   every entity write on it is already refused by `cad_db::drawing::lock`
    ///   (`layer.locked`), so a locked current layer could never receive the
    ///   geometry this session setting exists to place. (This closes the explicit
    ///   gap that module's docs recorded.)
    /// * `layer.current_frozen` — a frozen layer is invisible
    ///   (`Layer::visible == !(off || frozen)`), while the shared create funnel
    ///   does **not** look at visibility, so a frozen current layer would happily
    ///   store geometry the user can neither see nor pick;
    /// * `layer.current_off` — the same invisible-geometry argument applies to an
    ///   off layer. AutoCAD lets an off layer be current (it does not turn it on
    ///   either), but this build has no "turn it on" side effect and no visibility
    ///   check on creation, so copying that would silently hide what the user just
    ///   drew. **This is this build's documented rule, not a claim about
    ///   AutoCAD.**
    ///
    /// The flags are checked locked → frozen → off, so a layer carrying several of
    /// them always reports the same first reason. A refusal changes **nothing**:
    /// no session write, no revision bump, no history entry. Layer-table writes
    /// stay allowed: a layer can still be locked, frozen or turned off while it is
    /// **not** current, and the success path (session-only, one
    /// `drawing.active_layer` diagnostic) is unchanged.
    ///
    /// # The complementary half
    ///
    /// The mirror-image hole — *turning the current layer off or freezing it after
    /// it became current* — is closed by
    /// `app_drawing::layer_edit`'s `ensure_current_layer_stays_visible`
    /// (`layer.current_off` / `layer.current_frozen`), so the two refusals together
    /// mean the current layer is never invisible: not on the way in, and not
    /// afterwards. Locking the current layer stays allowed there (locking is a
    /// write permission, not visibility) and so is clearing those flags.
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
        self.ensure_layer_can_be_current(command.document, layer)?;
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
            point_report: None,
        })
    }

    /// The one current-layer rule: refuse a **locked**, **frozen** or **off**
    /// layer as the session's active layer.
    ///
    /// Reads the drawing's own stored attributes
    /// ([`DrawingDatabase::layer_attributes`], `Default` — on, thawed, unlocked —
    /// for a layer with no attribute row) and names the layer row in the refusal.
    /// A layer id with no row at all therefore resolves to `Default` and is
    /// allowed, which is the pre-existing "the layer need not exist yet"
    /// allowance: the *create* funnel is what reports the missing layer.
    ///
    /// Nothing is resolved here: the panel and the layer-table commands own the
    /// exact-then-unique-case-insensitive name resolver (`resolve_layer` in
    /// `app_drawing::layer_edit` / `cad_ui_core`'s session equivalent), and this
    /// rule only ever sees their [`LayerId`] output, so there is no second
    /// resolution and no second rule.
    ///
    /// A document that is not open has no layer table to consult, so nothing is
    /// refused here; the panel session refuses that case with
    /// `layer.no_document` before it dispatches.
    pub(super) fn ensure_layer_can_be_current(
        &self,
        document: DocumentId,
        layer: LayerId,
    ) -> CadResult<()> {
        let Some(document) = self.workspace.documents.get(&document) else {
            return Ok(());
        };
        let drawing = document.drawing.as_ref();
        let attributes = drawing.layer_attributes(layer);
        let (code, state, remedy) = if attributes.locked {
            (
                "layer.current_locked",
                "is locked",
                "unlock it before making it the current layer",
            )
        } else if attributes.frozen {
            (
                "layer.current_frozen",
                "is frozen",
                "thaw it before making it the current layer",
            )
        } else if attributes.off {
            (
                "layer.current_off",
                "is off",
                "turn it on before making it the current layer",
            )
        } else {
            return Ok(());
        };
        let name = drawing
            .layer(layer)
            .map(|row| row.name.as_str())
            .unwrap_or("<unknown>");
        Err(CadError::InvalidInput(format!(
            "{code}: layer '{name}' ({}) {state}; {remedy}",
            layer.0
        )))
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

    /// Insert an ARC into model space.
    ///
    /// The click-capture payload `Points([start, through, end])` (the UI path)
    /// builds the circumcircle through three points; the circumcircle is a
    /// work-plane `+Z` construction and collinear, coincident or non-finite points
    /// are an explicit refusal that creates nothing. More than three picks is
    /// refused (`arc.payload_length`): the construction is exactly three, and a
    /// longer list used to be silently truncated into a different arc.
    ///
    /// The programmatic payload `Geometry(SemanticGeometry::Arc { center, normal,
    /// radius, start, sweep })` authors **any** arc of the modeled domain, because
    /// every AutoCAD construction collapses into that one stored tuple: three-point
    /// (`start`/`sweep` from the circumcircle), **Start-Center-End** (radius = the
    /// start↔centre distance, sweep from the two parameter angles),
    /// **Start-Center-Angle** (the sweep *is* the angle) and
    /// **Start-End-Angle/Direction** (a negative `sweep` is the clockwise
    /// direction) are all expressible as `center`/`radius`/`start`/`sweep`, and a
    /// tilted `normal` is stored verbatim. The database's validated write path owns
    /// the degenerate-frame refusals (non-finite or zero normal, non-positive
    /// radius, zero sweep), so the command adds no second rule for them.
    ///
    /// Deliberate gap: **`Continue` from the last arc** needs a stored "last arc"
    /// in the session, and this build has no such state (`SessionState` carries no
    /// previous-arc frame), so it cannot be requested and is not fabricated from the
    /// selection. The construction *methods* above exist as geometry, not as
    /// interactive keyword prompts: the capture state machine in `draw_tool` is
    /// fixed at three picks, and no payload field selects a different prompt order.
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
                if points.len() > 3 {
                    return Err(CadError::InvalidInput(format!(
                        "arc.payload_length: the three-point construction takes exactly three \
                         picks (start, on-arc, end), got {}",
                        points.len()
                    )));
                }
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

    /// Insert an LWPOLYLINE through the payload points into model space.
    ///
    /// The click-capture payload `Points(points)` authors an **open, straight-segment**
    /// polyline (one zero bulge per vertex, `closed == false`): fewer than two
    /// points, any non-finite point, or fewer than two **distinct** vertices (for
    /// example all points identical) is an explicit refusal that creates nothing.
    ///
    /// The programmatic payload `Geometry(SemanticGeometry::Polyline { points,
    /// bulges, closed })` authors the **full modeled polyline**: arc segments with
    /// the stored bulges and a closed ring, through
    /// [`authored_polyline_geometry`] — the single validation point, which documents
    /// every normalisation and every `pline.<reason>` refusal. So PLINE now covers
    /// AutoCAD's straight segments, its `Arc` segment mode (stored as bulge, exactly
    /// as the domain and the DXF round trip model it) and its `Close` option.
    ///
    /// `Width` is the **uniform** polyline width (`0` = hairline). The click
    /// capture reads it from the `polylineWidth` tool parameter (default `0`);
    /// the programmatic payload carries it as `SemanticGeometry::Polyline::width`.
    /// Per-vertex start/end taper is not modeled and is never invented from a
    /// single number. **`Undo` of the last vertex** remains a step of the
    /// interactive capture state machine (`draw_tool`), not a command payload,
    /// so it neither exists nor can be requested here.
    pub(crate) fn create_polyline(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let geometry = match &command.payload {
            CommandPayload::Points(points) => {
                polyline_geometry(points.clone(), polyline_width_from_session(session)?)?
            }
            CommandPayload::Geometry(geometry) => match geometry.as_ref() {
                SemanticGeometry::Polyline {
                    points,
                    bulges,
                    closed,
                    width,
                } => authored_polyline_geometry(points.clone(), bulges.clone(), *closed, *width)?,
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
    /// is refused there) intact. More than two corners is refused
    /// (`rectangle.payload_length`) instead of being silently truncated.
    ///
    /// Deliberate gaps, refused by absence rather than by a fabricated shape:
    /// **`Fillet`** and **`Chamfer`** would need per-corner radii/distances,
    /// **`Rotation`** an angle, and **`Area`/`Dimensions`** a size, and the payload
    /// is the two clicked corners — there is no payload field, session parameter or
    /// typed channel that carries any of them (the session tool-parameter store has
    /// no RECTANG entry), so none can be requested. They are also not silently
    /// approximated: a rotated or filleted rectangle must be authored as a closed
    /// polyline with the matching vertices/bulges (PLINE), where the model can
    /// represent it. `Width` is not part of the RECTANG capture: author a closed
    /// PLINE with the `polylineWidth` parameter to get a wide rectangle outline.
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
        if points.len() > 2 {
            return Err(CadError::InvalidInput(format!(
                "rectangle.payload_length: RECTANG takes exactly two opposite corners, got {}",
                points.len()
            )));
        }
        let geometry = rectangle_geometry(corner1, corner2)?;
        self.commit_new_entity(session, command, geometry, "create rectangle")
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
    /// and one undo step, with the active-layer check intact. More than two picks is
    /// refused (`polygon.payload_length`) instead of being silently truncated.
    ///
    /// Deliberate gaps, refused by absence rather than by a fabricated ring:
    /// AutoCAD's **`Edge`** method takes the two endpoints of one edge, which is
    /// indistinguishable from the inscribed construction's centre + radius point on
    /// a two-pick payload — selecting it needs a mode field that neither the payload
    /// nor the session carries, so the ring it would produce cannot be requested
    /// (and must not be guessed). A **numeric radius** rotating the ring by the snap
    /// rotation angle has no channel either: a typed radius is not a point, and the
    /// session tool-parameter store's only POLYGON entry is the side count. The
    /// circumscribed sibling is its own selector
    /// ([`Self::create_polygon_circumscribed`]).
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
        if points.len() > 2 {
            return Err(CadError::InvalidInput(format!(
                "polygon.payload_length: the inscribed construction takes exactly two picks \
                 (centre and radius point), got {}",
                points.len()
            )));
        }
        let vertices =
            crate::draw_tool::polygon_vertices(center, radius_point, session.polygon_sides())?;
        let geometry = SemanticGeometry::Polyline {
            bulges: vec![0.0; vertices.len()],
            points: vertices,
            closed: true,
            width: 0.0,
        };
        self.commit_new_entity(session, command, geometry, "create polygon")
    }

    /// Place one or more POINTs into model space on the session's active layer.
    ///
    /// Payload choice: `Points(positions)` — the construction input, exactly as
    /// clicked (like `CreateCircle`/`CreateRectangle`/`CreatePolygon`). A
    /// **single** pick is the ordinary POINT; a list of several picks is AutoCAD's
    /// **`Multiple`** placement and stores one `AcDbPoint` per position in the
    /// **same** transaction and the **same** undo step, so the whole run is
    /// all-or-nothing (one non-finite position refuses the batch and writes
    /// nothing). The interactive capture is still one pick per command
    /// (`DrawToolKind::Point`'s fixed arity lives in `draw_tool`, outside this
    /// handler); the repeated-placement contract is the payload's.
    ///
    /// Stable refusals that change nothing: `point.payload_length` for an empty
    /// list, `point.invalid_point` for a non-finite position, and the shared
    /// funnel's active-layer check. A `Geometry` payload is refused: POINT's payload
    /// is the raw click(s), never a second source of truth.
    ///
    /// A point has **no** coplanarity constraint — it is trivially coplanar on the
    /// `+Z` plane through itself — so the ADR's "non-coplanar `+Z`" refusal is
    /// vacuous here rather than a fabricated constraint.
    pub(crate) fn create_point(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "CreatePoint needs a Points([position, ...]) payload".into(),
            ));
        };
        if points.is_empty() {
            return Err(CadError::InvalidInput(
                "point.payload_length: CreatePoint needs at least one position".into(),
            ));
        }
        if !points.iter().all(|position| is_finite_point(*position)) {
            return Err(CadError::InvalidInput(
                "point.invalid_point: every CreatePoint position must be finite".into(),
            ));
        }
        let geometries = points
            .iter()
            .map(|position| SemanticGeometry::Point(*position))
            .collect();
        self.commit_new_entities(session, command, geometries, "create point")
    }

    /// Insert one single-line TEXT into model space on the session's active layer
    /// (ADR 0019 decision F.1, P7a).
    ///
    /// The payload is the **construction input** — the captured insertion point,
    /// the typed text and the height — exactly like `CreateCircle`/`CreatePoint`,
    /// so a caller never has to pre-build a `SemanticGeometry::Text` to run the
    /// command. Every refusal changes nothing:
    ///
    /// * an empty or whitespace-only text (`tool.text_empty` semantics);
    /// * more than [`MAX_TOOL_TEXT_CHARS`] characters, counted in **characters**
    ///   (not bytes), matching the capture bound;
    /// * a text containing `\r`/`\n` (`text.single_line_newline`): an `AcDbText`
    ///   does not interpret `\P`, so storing a multi-line draft here produced an
    ///   entity that rendered over several lines and could never be re-edited by
    ///   `TEXTEDIT`. MTEXT is the multi-line channel;
    /// * a non-finite position, or a non-finite/non-positive height;
    /// * a missing active layer (the shared create funnel).
    ///
    /// The **style** is the drawing's `Standard` text style (the `StyleId(0)`
    /// entry as the documented fallback) and its first resource key becomes the
    /// font key, so the stored text is drawn with the style's face; the resolution
    /// is [`resolve_created_text_style`], the same one MTEXT creation uses. The
    /// **alignment** is the style default (left/baseline): the `Style` table
    /// carries no alignment and TEXT creation has no justify prompt, so a
    /// per-invocation alignment would be an invented value. The **rotation** is
    /// the session typed channel
    /// ([`crate::draw_tool::ToolParameter::TextRotation`], degrees, default `0`),
    /// resolved by [`text_rotation_radians`] and stored in the domain's radians:
    /// the payload carries no rotation field, so the channel is the only source
    /// and nothing invents a second one. A single-line text is always on the `+Z`
    /// plane through its insertion point — there is no plane input to be
    /// non-`+Z` and nothing is projected.
    ///
    /// The entity goes through the shared [`Self::commit_new_entity`] funnel: one
    /// `AcDbText` entity, one transaction, one undo step, active-layer check
    /// intact.
    pub(crate) fn create_text(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Text {
            position,
            text,
            height,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "CreateText needs a Text { position, text, height } payload".into(),
            ));
        };
        if !is_finite_point(*position) {
            return Err(CadError::InvalidInput(
                "CreateText position must be finite".into(),
            ));
        }
        if text.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "tool.text_empty: CreateText needs a non-empty, non-whitespace text".into(),
            ));
        }
        let characters = text.chars().count();
        if characters > MAX_TOOL_TEXT_CHARS {
            return Err(CadError::InvalidInput(format!(
                "tool.text_too_long: CreateText allows at most {MAX_TOOL_TEXT_CHARS} characters, \
                 got {characters}"
            )));
        }
        // TEXT is single-line: a draft with a line break would be stored as an
        // `AcDbText` the renderer draws over several lines and `TEXTEDIT` then
        // refuses. Refuse it here instead of storing a value we cannot edit.
        if has_line_break(text) {
            return Err(single_line_newline_error("CreateText"));
        }
        if !height.is_finite() || *height <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "CreateText height must be finite and positive, got {height}"
            )));
        }
        // The style is resolved from the authoritative drawing, never fabricated:
        // `Standard` by name, else the documented `StyleId(0)` fallback, else no
        // style at all (the import path's own convention). The resolution is
        // shared with MTEXT creation so the two cannot disagree.
        let (style, font) = {
            let document = self
                .workspace
                .documents
                .get(&command.document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            resolve_created_text_style(document.drawing.as_ref())
        };
        let geometry = SemanticGeometry::Text {
            text: text.clone(),
            position: *position,
            style,
            height: *height,
            // The `Text` payload deliberately carries no rotation (adding one
            // would be a second construction input), so the session typed channel
            // `textRotation` is the single source: degrees in, radians stored, one
            // conversion. A session at its default stores `0.0`, exactly as
            // before the parameter existed.
            rotation: text_rotation_radians(session)?,
            font,
            // Style defaults: the `Style` table has no alignment column and TEXT
            // creation has no justify prompt.
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            // Single-line TEXT creation has no width prompt.
            wrap_width: None,
            width_factor: 1.0,
        };
        self.commit_new_entity(session, command, geometry, "create text")
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
fn polyline_geometry(points: Vec<Point3>, width: f64) -> CadResult<SemanticGeometry> {
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
    if !width.is_finite() || width < 0.0 {
        return Err(CadError::InvalidInput(
            "polyline.width_invalid: CreatePolyline needs a finite non-negative width".into(),
        ));
    }
    let bulges = vec![0.0; points.len()];
    Ok(SemanticGeometry::Polyline {
        points,
        bulges,
        closed: false,
        width,
    })
}

/// The typed `polylineWidth` tool parameter as the uniform PLINE width.
///
/// The channel already refuses a negative/non-finite set, so this re-checks the
/// value it reads (a session default or a future wiring bug must never store an
/// unrepresentable width) and names `polyline.width_invalid` otherwise.
fn polyline_width_from_session(session: &SessionState) -> CadResult<f64> {
    let width = session
        .tool_parameter(ToolParameter::PolylineWidth)
        .number()
        .ok_or_else(|| {
            CadError::InvalidInput(
                "polyline.width_invalid: the polylineWidth parameter must be a number".into(),
            )
        })?;
    if !width.is_finite() || width < 0.0 {
        return Err(CadError::InvalidInput(format!(
            "polyline.width_invalid: the polylineWidth parameter must be finite and >= 0, got \
             {width}"
        )));
    }
    Ok(width)
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
        width: 0.0,
    })
}
