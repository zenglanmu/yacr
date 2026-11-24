//! Entity-to-semantic-geometry dispatch, proxy-graphics replay and the shared
//! style-id/linetype/font lookups used while converting.
//!
//! Submodule of [`crate::entity`].

use super::*;

/// The named, per-entity reasons an `MTEXT → Text` fold is `Partial` (ADR 0019
/// decision H).
///
/// The domain `Text` variant has no columns, background fill or drawing
/// direction, so those source properties are dropped. The old `Complete` claim
/// was false; these three stable machine keys are the honest reasons, and the
/// same set feeds a deduplicated import diagnostic.
///
/// Line spacing (DXF groups 44 + 73) used to be a fourth reason; it is now part
/// of the domain model ([`TextLineSpacing`]) and is carried, not dropped. The
/// only residual line-spacing gap is a degenerate factor, which has its own
/// narrower reason ([`DEGENERATE_LINE_SPACING_FACTOR`]).
pub(crate) const MTEXT_UNMODELLED: [&str; 3] = [
    "text.background_unsupported",
    "text.columns_unsupported",
    "text.drawing_direction_unsupported",
];

/// The named reason for an MTEXT whose line-spacing factor (DXF group 44) is
/// zero, negative or non-finite: such a value cannot be stored (it would
/// overlap or invert the baselines), so the neutral factor is used and the loss
/// is reported.
pub(crate) const DEGENERATE_LINE_SPACING_FACTOR: &str = "text.line_spacing_factor_degenerate";

/// Map DXF MTEXT line spacing into the domain model.
///
/// Group 44 is the factor and group 73 the style. The style has only the two
/// documented values and acadrust normalises every other group-73 value to "at
/// least", so no style is unrepresentable. A factor that is zero, negative or
/// non-finite cannot be stored (the write validator rejects it): the neutral
/// factor is used and `degenerate` is `true` so the caller can widen its
/// completeness and report the loss.
pub(crate) fn map_mtext_line_spacing(
    factor: f64,
    style: acadrust::entities::LineSpacingStyle,
) -> (TextLineSpacing, bool) {
    let style = match style {
        acadrust::entities::LineSpacingStyle::AtLeast => TextLineSpacingStyle::AtLeast,
        acadrust::entities::LineSpacingStyle::Exactly => TextLineSpacingStyle::Exactly,
    };
    match text_line_spacing_factor_from_dxf(factor) {
        Some(factor) => (TextLineSpacing { factor, style }, false),
        None => (
            TextLineSpacing {
                factor: DEFAULT_TEXT_LINE_SPACING_FACTOR,
                style,
            },
            true,
        ),
    }
}

impl<'a> ImporterBuilder<'a> {
    /// [`map_mtext_line_spacing`] plus the once-per-import report of a
    /// degenerate factor.
    pub(crate) fn mtext_line_spacing(
        &mut self,
        factor: f64,
        style: acadrust::entities::LineSpacingStyle,
    ) -> (TextLineSpacing, bool) {
        let (spacing, degenerate) = map_mtext_line_spacing(factor, style);
        if degenerate
            && self
                .reported_reasons
                .insert(DEGENERATE_LINE_SPACING_FACTOR.to_string())
        {
            self.diagnostics.push(Diagnostic {
                object: None,
                code: "import.text_partial".into(),
                message: format!(
                    "the MTEXT line-spacing factor (group 44) is {factor}, which cannot be stored; \
                     the neutral factor {DEFAULT_TEXT_LINE_SPACING_FACTOR} is used instead"
                ),
            });
        }
        (spacing, degenerate)
    }

    pub(crate) fn convert(
        &mut self,
        entity: &EntityType,
        common: &EntityCommon,
    ) -> (SemanticGeometry, Completeness) {
        match entity {
            EntityType::Line(l) => (
                SemanticGeometry::Line {
                    start: p3(l.start),
                    end: p3(l.end),
                },
                Completeness::Complete,
            ),
            EntityType::Circle(c) => (
                SemanticGeometry::Circle {
                    // A CIRCLE stores its centre in OCS; the extrusion normal
                    // defines the plane. `center_wcs` runs the AutoCAD
                    // arbitrary-axis frame so a tilted circle is placed in its
                    // own plane, not flattened onto world XY (audit B23/B31).
                    center: p3(c.center_wcs()),
                    normal: p3(c.normal),
                    radius: c.radius,
                },
                Completeness::Complete,
            ),
            EntityType::Arc(a) => (
                SemanticGeometry::Arc {
                    center: p3(a.center_wcs()),
                    normal: p3(a.normal),
                    radius: a.radius,
                    start: a.start_angle,
                    // Angles are measured in the OCS frame `arbitrary_axis`
                    // rebuilds from the same normal, so the arc keeps its
                    // sweep and side.
                    sweep: normalize_sweep(a.end_angle - a.start_angle),
                },
                Completeness::Complete,
            ),
            EntityType::Ellipse(e) => ellipse_semantics(e),
            EntityType::Point(p) => (
                SemanticGeometry::Point(p3(p.location)),
                Completeness::Complete,
            ),
            EntityType::LwPolyline(pl) => {
                let normal = p3(pl.normal);
                let points = polyline_ocs_points(
                    normal,
                    pl.elevation,
                    pl.vertices.iter().map(|v| (v.location.x, v.location.y)),
                );
                let raw_bulges: Vec<f64> = pl.vertices.iter().map(|v| v.bulge).collect();
                let bulges = polyline_ocs_bulges(normal, &raw_bulges);
                let has_taper = pl
                    .vertices
                    .iter()
                    .any(|v| v.start_width.abs() > 1e-12 || v.end_width.abs() > 1e-12);
                let (width, width_reason) = polyline_uniform_width(pl.constant_width, has_taper);
                let mut completeness = polyline_completeness(normal, pl.vertices.len(), &bulges);
                if let Some(reason) = width_reason {
                    completeness = completeness.combine(Completeness::Partial(vec![reason.into()]));
                }
                (
                    SemanticGeometry::Polyline {
                        points,
                        bulges,
                        closed: pl.is_closed,
                        width,
                    },
                    completeness,
                )
            }
            EntityType::Polyline2D(pl) => {
                let normal = p3(pl.normal);
                let points = polyline_ocs_points(
                    normal,
                    pl.elevation,
                    pl.vertices.iter().map(|v| (v.location.x, v.location.y)),
                );
                let raw_bulges: Vec<f64> = pl.vertices.iter().map(|v| v.bulge).collect();
                let bulges = polyline_ocs_bulges(normal, &raw_bulges);
                let closed = pl.flags.bits() & 1 != 0;
                // The 2D POLYLINE header carries a default start/end width pair;
                // it is a uniform width only when the two are equal and no vertex
                // overrides it. Anything else is a taper and keeps 0 + a partial
                // diagnostic rather than an invented uniform value.
                let header_uniform = (pl.start_width - pl.end_width).abs() <= 1e-12;
                let vertex_taper = pl
                    .vertices
                    .iter()
                    .any(|v| v.start_width.abs() > 1e-12 || v.end_width.abs() > 1e-12);
                let constant = if header_uniform { pl.start_width } else { 0.0 };
                let (width, width_reason) =
                    polyline_uniform_width(constant, !header_uniform || vertex_taper);
                let mut completeness = polyline_completeness(normal, pl.vertices.len(), &bulges);
                if let Some(reason) = width_reason {
                    completeness = completeness.combine(Completeness::Partial(vec![reason.into()]));
                }
                (
                    SemanticGeometry::Polyline {
                        points,
                        bulges,
                        closed,
                        width,
                    },
                    completeness,
                )
            }
            EntityType::Polyline3D(pl) => {
                let points: Vec<Point3> = pl.vertices.iter().map(|v| p3(v.position)).collect();
                (
                    SemanticGeometry::Polyline {
                        points,
                        bulges: Vec::new(),
                        closed: pl.flags.closed,
                        width: 0.0,
                    },
                    Completeness::Complete,
                )
            }
            EntityType::Text(t) => {
                let (h_align, v_align) = (
                    map_h_align(t.horizontal_alignment),
                    map_v_align(t.vertical_alignment),
                );
                // DXF places aligned text at the alignment point when one is set.
                let position = if t.alignment_point.is_some()
                    && (h_align != TextAlignH::Left || v_align != TextAlignV::Baseline)
                {
                    t.alignment_point.unwrap_or(t.insertion_point)
                } else {
                    t.insertion_point
                };
                // TEXT stores its alignment point and rotation in OCS; a `-Z`
                // extrusion mirrors both (see `text_ocs_placement`).
                let (position, rotation) =
                    text_ocs_placement(p3(t.normal), p3(position), t.rotation);
                // DXF group 41 is the horizontal width scale. A zero, negative
                // or non-finite value cannot be stored (the write validator
                // rejects it), so it is reported and falls back to the neutral
                // factor instead of being silently kept or dropped.
                let (width_factor, width_completeness) =
                    match text_width_factor_from_dxf(t.width_factor) {
                        Some(factor) => (factor, Completeness::Complete),
                        None => {
                            let reason = "text.width_factor_degenerate";
                            if self.reported_reasons.insert(reason.to_string()) {
                                self.diagnostics.push(Diagnostic {
                                    object: None,
                                    code: "import.text_partial".into(),
                                    message: format!(
                                        "the TEXT width factor (group 41) is {0}, which cannot \
                                         be stored; the neutral factor {1} is used instead",
                                        t.width_factor, DEFAULT_TEXT_WIDTH_FACTOR
                                    ),
                                });
                            }
                            (
                                DEFAULT_TEXT_WIDTH_FACTOR,
                                Completeness::Partial(vec![reason.into()]),
                            )
                        }
                    };
                (
                    SemanticGeometry::Text {
                        text: t.value.clone(),
                        position,
                        style: self.style_id(&t.style),
                        height: t.height,
                        rotation,
                        font: self.style_font(&t.style),
                        h_align,
                        v_align,
                        // DXF TEXT has no width boundary.
                        wrap_width: None,
                        width_factor,
                        // A single-line TEXT has no line-spacing groups.
                        line_spacing: TextLineSpacing::default(),
                    },
                    width_completeness,
                )
            }
            EntityType::MText(t) => {
                let (h_align, v_align) = attach_align(t.attachment_point);
                // ADR 0019 decision H: the domain `Text` variant has no columns,
                // background fill or drawing direction, so folding MTEXT into it
                // is lossy. Report `Partial` with the named reasons instead of
                // the old (false) `Complete`. Line spacing is no longer one of
                // them: it is modelled and carried below. Each reason is
                // reported once per import so a text-heavy drawing does not bury
                // the report in duplicates.
                let mut completeness = Completeness::Partial(
                    MTEXT_UNMODELLED.iter().map(|r| (*r).to_string()).collect(),
                );
                for reason in MTEXT_UNMODELLED {
                    if self.reported_reasons.insert(reason.to_string()) {
                        self.diagnostics.push(Diagnostic {
                            object: None,
                            code: "import.text_partial".into(),
                            message: reason.to_string(),
                        });
                    }
                }
                // DXF groups 44 (factor) and 73 (style).
                let (line_spacing, degenerate_line_spacing) =
                    self.mtext_line_spacing(t.line_spacing_factor, t.line_spacing_style);
                if degenerate_line_spacing {
                    completeness = completeness.combine(Completeness::Partial(vec![
                        DEGENERATE_LINE_SPACING_FACTOR.into(),
                    ]));
                }
                (
                    SemanticGeometry::Text {
                        text: t.value.clone(),
                        position: p3(t.insertion_point),
                        style: self.style_id(&t.style),
                        height: t.height,
                        rotation: t.rotation,
                        font: self.style_font(&t.style),
                        h_align,
                        v_align,
                        // MTEXT's reference-rectangle width (group 41) is still
                        // dropped by this importer; only MLEADER populates the
                        // wrap boundary for now.
                        wrap_width: None,
                        // MTEXT has no width-factor group; the factor is neutral.
                        width_factor: DEFAULT_TEXT_WIDTH_FACTOR,
                        line_spacing,
                    },
                    completeness,
                )
            }
            EntityType::Spline(s) => spline_semantics(s),
            EntityType::Solid(s) => {
                let (geometry, completeness) = solid_mesh_semantics(s);
                // The shared helper labels a non-finite corner with the combined
                // `AcDbTrace` class. Keep SOLID and TRACE apart even in the
                // unrepresentable placeholder, so nothing downstream (the export
                // drop report included) sees the wrong class.
                let geometry = match geometry {
                    SemanticGeometry::Opaque {
                        version, payload, ..
                    } => SemanticGeometry::Opaque {
                        type_key: entity_class_name(entity),
                        version,
                        payload,
                    },
                    other => other,
                };
                (geometry, completeness)
            }
            EntityType::Face3D(f) => {
                // A 3DFACE stores its corners in WCS and in boundary order
                // (first, second, third, fourth), unlike SOLID/TRACE
                // (docs/layouts.md §4, audit B31).
                let m = quad_mesh([
                    p3(f.first_corner),
                    p3(f.second_corner),
                    p3(f.third_corner),
                    p3(f.fourth_corner),
                ]);
                (SemanticGeometry::Mesh(m), Completeness::Complete)
            }
            EntityType::Insert(i) => self.insert_semantics(i),
            EntityType::Unknown(u) => {
                self.proxy_geometry(&u.dxf_name, common, u.raw_dwg_data.as_deref())
            }
            EntityType::Extended(x) if x.class_name() == "ACAD_PROXY_ENTITY" => {
                self.proxy_geometry(x.class_name(), common, None)
            }
            EntityType::Extended(x) => self.extended_semantics(x),
            EntityType::Hatch(h) => self.hatch_semantics(h),
            // A DIMENSION becomes a `SemanticGeometry::Dimension`: with a
            // pre-rendered `*D...` block its display is the expanded picture,
            // otherwise it is our synthesis (ADR 0017 A.6).
            EntityType::Dimension(d) => self.dimension_semantics(d),
            EntityType::Leader(l) => self.leader_semantics(l),
            EntityType::Polyline(pl) => (
                polyline_semantics(
                    pl.vertices.iter().map(|v| p3(v.location)).collect(),
                    pl.is_closed(),
                ),
                Completeness::Complete,
            ),
            // ADR-0055: an ATTDEF is a block-definition member of its own class,
            // not a flattened TEXT. The definition carries the tag, prompt,
            // default, flags and the placement in the block's local frame.
            EntityType::AttributeDefinition(a) => self.attribute_definition_semantics(a),
            // An ATTRIB that reached the entity walk with no owning INSERT keeps
            // its pre-M2 Text fallback, but the loss is declared: ownership is
            // never guessed. `insert_semantics` consumes the values acadrust
            // attached to a reference, so this arm is the genuinely orphan case.
            EntityType::AttributeEntity(a) => {
                let (position, rotation) =
                    text_ocs_placement(p3(a.normal), p3(a.insertion_point), a.rotation);
                let geometry = self.attribute_fallback_text(
                    &a.text_style,
                    a.value.clone(),
                    position,
                    a.height,
                    rotation,
                );
                let reason =
                    "attribute value has no owning INSERT; drawn as plain TEXT".to_string();
                if self.reported_reasons.insert(reason.clone()) {
                    self.diagnostics.push(Diagnostic {
                        object: None,
                        code: "import.orphan_attrib".into(),
                        message: format!(
                            "attribute {:?} (handle {:X}) has no owning INSERT; it is drawn as a \
                             plain TEXT and its tag/flags are not stored",
                            a.tag,
                            a.common.handle.value()
                        ),
                    });
                }
                (geometry, Completeness::Partial(vec![reason]))
            }
            // The three mesh kinds keep only vertices and face topology, so
            // `Complete` was a false claim. Each dropped *class* of data is a
            // named reason, reported once per import like the MTEXT fold.
            //
            // A SUBD MESH draws its base control cage: `subdivision_level` is
            // ignored, so the smoothed surface is not what appears on screen.
            EntityType::Mesh(m) => (
                subd_mesh_semantics(m),
                self.named_partial("import.subd_mesh_partial", &SUBD_MESH_UNMODELLED),
            ),
            // A POLYFACE MESH erases the negative-index invisible-edge flag.
            EntityType::PolyfaceMesh(m) => (
                polyface_mesh_semantics(m),
                self.named_partial("import.polyface_mesh_partial", &POLYFACE_MESH_UNMODELLED),
            ),
            // A POLYGON MESH is the flat M x N grid: closed/smoothed forms are
            // not stitched or smoothed.
            EntityType::PolygonMesh(m) => (
                polygon_mesh_semantics(m),
                self.named_partial("import.polygon_mesh_partial", &POLYGON_MESH_UNMODELLED),
            ),
            EntityType::Wipeout(w) => wipeout_semantics(w),
            EntityType::Helix(h) => spline_semantics(&h.spline),
            // A VIEWPORT's drawn frame is its clip boundary when it has one
            // (VPCLIP), so the frame follows the boundary entity (group 340)
            // instead of always being the viewport rectangle.
            EntityType::Viewport(v) => viewport_frame_semantics(v, self.acad),
            EntityType::MLine(m) => self.mline_semantics(m),
            EntityType::Tolerance(t) => {
                let (frame, _width, _height) = tolerance_frame(t);
                let style_name = self.dim_text_style(&t.dimension_style_name);
                let text = self.attribute_fallback_text(
                    &style_name,
                    t.text.clone(),
                    p3(t.insertion_point),
                    t.text_height,
                    0.0,
                );
                (
                    SemanticGeometry::Compound(vec![frame, text]),
                    Completeness::Partial(vec![
                        "tolerance frame width is estimated from the text length".into(),
                    ]),
                )
            }
            EntityType::MultiLeader(ml) => self.multileader_semantics(ml),
            EntityType::Ray(r) => {
                self.construct_line_semantics(p3(r.base_point), p3(r.direction), true)
            }
            EntityType::XLine(x) => {
                self.construct_line_semantics(p3(x.base_point), p3(x.direction), false)
            }
            EntityType::Table(t) => self.table_semantics(t),
            EntityType::RasterImage(img) => raster_image_semantics(img),
            EntityType::Shape(s) => {
                let code = if s.shape_number > 0 {
                    s.shape_number as u32
                } else {
                    0
                };
                let completeness = if code == 0 && !s.shape_name.is_empty() {
                    Completeness::Partial(vec![
                        "shape is referenced by name; only shape codes are resolved".into(),
                    ])
                } else {
                    Completeness::Complete
                };
                (
                    SemanticGeometry::Shape {
                        shape_name: s.shape_name.clone(),
                        code,
                        position: p3(s.insertion_point),
                        size: s.size,
                        rotation: s.rotation,
                        font: self.style_font(&s.style_name),
                    },
                    completeness,
                )
            }
            EntityType::Ole2Frame(f) => ole2frame_semantics(f),
            EntityType::ViewBorder(v) => view_border_semantics(v),
            EntityType::SectionSymbol(s) => section_symbol_semantics(s),
            EntityType::Underlay(u) => underlay_semantics(u),
            EntityType::Light(l) => light_semantics(l),
            EntityType::Solid3D(s) => self.acis_semantics(entity, &s.acis_data),
            EntityType::Region(r) => self.acis_semantics(entity, &r.acis_data),
            EntityType::Body(b) => self.acis_semantics(entity, &b.acis_data),
            EntityType::Surface(s) => self.acis_semantics(entity, &s.acis_data),
            other => (
                SemanticGeometry::Opaque {
                    type_key: entity_class_name(other),
                    version: 1,
                    payload: Vec::new(),
                },
                Completeness::Unverified,
            ),
        }
    }

    /// Record each named `Partial` reason as one import diagnostic and return
    /// the verdict that carries them.
    ///
    /// Mirrors the MTEXT fold: `reported_reasons` keeps a mesh-heavy drawing
    /// from burying the report in thousands of identical diagnostics, while
    /// every entity still declares its own `Partial` reasons.
    pub(crate) fn named_partial(&mut self, code: &str, reasons: &[&str]) -> Completeness {
        for reason in reasons {
            if self.reported_reasons.insert((*reason).to_string()) {
                self.diagnostics.push(Diagnostic {
                    object: None,
                    code: code.into(),
                    message: (*reason).to_string(),
                });
            }
        }
        Completeness::Partial(reasons.iter().map(|r| (*r).to_string()).collect())
    }

    pub(crate) fn proxy_geometry(
        &mut self,
        class_name: &str,
        common: &EntityCommon,
        raw: Option<&[u8]>,
    ) -> (SemanticGeometry, Completeness) {
        let source = ProxySource {
            handle: format!("{:X}", common.handle.value()),
            class_name: class_name.to_string(),
            application: None,
            dwg_version: format!("{:?}", self.stats.source_version),
        };
        match common.graphic_data.as_deref() {
            Some(data) => match self.proxy.replay(&source, data) {
                Ok(out) if !out.geometry.is_empty() => {
                    self.diagnostics.extend(out.diagnostics);
                    let completeness = out.completeness.clone();
                    // Keep *every* decoded fragment (audit B21): `.next()` used
                    // to silently drop all but the first proxy record. A single
                    // item stays itself; several become a `Compound`, which the
                    // representation layer draws one primitive at a time.
                    (proxy_geometry_compound(out.geometry), completeness)
                }
                Ok(out) => {
                    self.diagnostics.extend(out.diagnostics);
                    (
                        SemanticGeometry::Opaque {
                            type_key: class_name.to_string(),
                            version: 1,
                            payload: Vec::new(),
                        },
                        out.completeness,
                    )
                }
                Err(e) => {
                    self.diagnostics.push(Diagnostic {
                        object: None,
                        code: "import.proxy_error".into(),
                        message: format!("{class_name}: {e}"),
                    });
                    (
                        SemanticGeometry::Opaque {
                            type_key: class_name.to_string(),
                            version: 1,
                            payload: Vec::new(),
                        },
                        Completeness::Partial(vec!["proxy decode failed".into()]),
                    )
                }
            },
            None => {
                if let Some(raw) = raw {
                    let _ = self.proxy.inspect_raw_dwg(raw);
                }
                self.diagnostics.push(Diagnostic {
                    object: None,
                    code: "import.proxy_no_cache".into(),
                    message: format!(
                        "{class_name} has no proxy graphics cache; shown as a placeholder"
                    ),
                });
                (
                    SemanticGeometry::Opaque {
                        type_key: class_name.to_string(),
                        version: 1,
                        payload: Vec::new(),
                    },
                    Completeness::Missing(vec!["no proxy cache in the source drawing".into()]),
                )
            }
        }
    }

    pub(crate) fn style_id(&self, name: &str) -> StyleId {
        self.style_ids.get(name).copied().unwrap_or(StyleId(0))
    }

    /// Primary font file declared by a named text style, if any.
    pub(crate) fn style_font(&self, name: &str) -> Option<String> {
        self.style_fonts.get(&name.to_ascii_lowercase()).cloned()
    }
}
