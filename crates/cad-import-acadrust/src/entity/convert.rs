//! Entity-to-semantic-geometry dispatch, proxy-graphics replay and the shared
//! style-id/linetype/font lookups used while converting.
//!
//! Submodule of [`crate::entity`].

use super::*;

impl<'a> ImporterBuilder<'a> {
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
                let bulges: Vec<f64> = pl.vertices.iter().map(|v| v.bulge).collect();
                let completeness = polyline_completeness(normal, pl.vertices.len(), &bulges);
                (
                    SemanticGeometry::Polyline {
                        points,
                        bulges,
                        closed: pl.is_closed,
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
                let bulges: Vec<f64> = pl.vertices.iter().map(|v| v.bulge).collect();
                let closed = pl.flags.bits() & 1 != 0;
                let completeness = polyline_completeness(normal, pl.vertices.len(), &bulges);
                (
                    SemanticGeometry::Polyline {
                        points,
                        bulges,
                        closed,
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
                (
                    SemanticGeometry::Text {
                        text: t.value.clone(),
                        position: p3(position),
                        style: self.style_id(&t.style),
                        height: t.height,
                        rotation: t.rotation,
                        font: self.style_font(&t.style),
                        h_align,
                        v_align,
                    },
                    Completeness::Complete,
                )
            }
            EntityType::MText(t) => {
                let (h_align, v_align) = attach_align(t.attachment_point);
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
                    },
                    Completeness::Complete,
                )
            }
            EntityType::Spline(s) => spline_semantics(s),
            EntityType::Solid(s) => solid_mesh_semantics(s),
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
            EntityType::Hatch(h) => Self::hatch_geometry(h),
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
            EntityType::AttributeDefinition(a) => {
                let text = if a.default_value.is_empty() {
                    a.tag.clone()
                } else {
                    a.default_value.clone()
                };
                (
                    self.attribute_text(
                        &a.text_style,
                        text,
                        p3(a.insertion_point),
                        a.height,
                        a.rotation,
                    ),
                    Completeness::Complete,
                )
            }
            EntityType::AttributeEntity(a) => (
                self.attribute_text(
                    &a.text_style,
                    a.value.clone(),
                    p3(a.insertion_point),
                    a.height,
                    a.rotation,
                ),
                Completeness::Complete,
            ),
            EntityType::Mesh(m) => (subd_mesh_semantics(m), Completeness::Complete),
            EntityType::PolyfaceMesh(m) => (polyface_mesh_semantics(m), Completeness::Complete),
            EntityType::PolygonMesh(m) => (polygon_mesh_semantics(m), Completeness::Complete),
            EntityType::Wipeout(w) => wipeout_semantics(w),
            EntityType::Helix(h) => spline_semantics(&h.spline),
            EntityType::Viewport(v) => (viewport_semantics(v), Completeness::Complete),
            EntityType::MLine(m) => self.mline_semantics(m),
            EntityType::Tolerance(t) => {
                let (frame, _width, _height) = tolerance_frame(t);
                let style_name = self.dim_text_style(&t.dimension_style_name);
                let text = self.attribute_text(
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
                ray_semantics(p3(r.base_point), p3(r.direction), true, self.model_bounds)
            }
            EntityType::XLine(x) => {
                ray_semantics(p3(x.base_point), p3(x.direction), false, self.model_bounds)
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
            EntityType::Solid3D(s) => acis_semantics(entity, &s.acis_data),
            EntityType::Region(r) => acis_semantics(entity, &r.acis_data),
            EntityType::Body(b) => acis_semantics(entity, &b.acis_data),
            EntityType::Surface(s) => acis_semantics(entity, &s.acis_data),
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

    /// Resolved dash pattern for a linetype name, or `None` when unknown.
    ///
    /// An empty or symbolic name has no concrete pattern; callers handle those
    /// cases before calling. The lookup is case-insensitive like the source
    /// table.
    pub(crate) fn linetype_pattern(&self, name: &str) -> Option<LinetypePattern> {
        let key = name.trim().to_ascii_lowercase();
        if key.is_empty() || key == "bylayer" || key == "byblock" {
            return None;
        }
        self.linetype_patterns.get(&key).cloned()
    }

    /// Primary font file declared by a named text style, if any.
    pub(crate) fn style_font(&self, name: &str) -> Option<String> {
        self.style_fonts.get(&name.to_ascii_lowercase()).cloned()
    }
}
