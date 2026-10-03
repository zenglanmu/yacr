//! Entity, block and hatch conversion methods for [`ImporterBuilder`].

use super::*;

impl<'a> ImporterBuilder<'a> {
    pub(crate) fn push_entity(
        &mut self,
        entity: &EntityType,
        space: SpaceId,
        _layout: LayoutId,
    ) -> Option<EntityId> {
        if matches!(entity, EntityType::Block(_) | EntityType::BlockEnd(_)) {
            return None;
        }
        if self.entity_total >= self.request.limits.max_entities {
            self.dropped += 1;
            return None;
        }
        let common = entity.common();
        let class_name = entity_class_name(entity);
        let layer = self
            .layer_ids
            .get(&common.layer)
            .copied()
            .unwrap_or_else(|| self.layer_ids.get("0").copied().unwrap_or(LayerId(0)));

        let (geometry, completeness) = self.convert(entity, common);

        // Track the model-space extent as entities are imported so RAY/XLINE
        // construct lines can be clipped to it (a fixed viewport has no clip).
        if matches!(space, SpaceId::Model) {
            let mut accumulator = cad_geometry::BoundsAccumulator::new();
            if let Some((min, max)) = self.model_bounds {
                accumulator.union_point(min);
                accumulator.union_point(max);
            }
            accumulator.add_geometry(&geometry);
            if let Some(bounds) = accumulator.finish() {
                self.model_bounds = Some((bounds.min, bounds.max));
            }
        }

        // Render/pick are judged from the drawn result, not from parse success
        // (audit B20). An INSERT inherits the resolved status of its block; an
        // array INSERT is a Compound of Instances that all reference one block.
        let (mut render, mut pick) = display_support(&geometry);
        let insert_blocks = referenced_blocks(&geometry);
        for block in &insert_blocks {
            if let Some(status) = self.block_status.get(block) {
                render = weaker(render, *status);
                pick = weaker(pick, *status);
            } else {
                render = weaker(render, SupportStatus::Unverified);
                pick = weaker(pick, SupportStatus::Unverified);
            }
        }
        if let SpaceId::Block(block) = &space {
            self.block_members
                .entry(*block)
                .or_default()
                .push(BlockMember {
                    render,
                    insert_blocks,
                });
        }
        if matches!(&space, SpaceId::Model) {
            self.model_render = weaker(self.model_render, render);
            if render == SupportStatus::Verified {
                self.model_drawable = true;
            } else {
                self.model_render_types.insert(class_name.clone());
            }
        }

        let entity_id = EntityId(self.next_entity);
        let object_id = ObjectId(self.next_object);
        self.next_entity += 1;
        self.next_object += 1;
        self.entity_total += 1;

        self.note_capability(&class_name, &geometry, &completeness, render, pick);

        // Resolve the entity's effective display opacity, colour, lineweight
        // and linetype so the representation/scene layers can carry real style
        // instead of always drawing opaque grey (audit F14 / §2.1.3). ByObject
        // wins over ByLayer; ByBlock is kept symbolic for INSERT expansion.
        let layer_alpha = self.layer_transparency.get(&layer).copied().unwrap_or(1.0);
        let layer_color = self.layer_colors.get(&layer).copied();
        let layer_lineweight = self.layer_lineweights.get(&layer).copied();
        let layer_pattern = self
            .layer_linetypes
            .get(&layer)
            .and_then(|name| self.linetype_pattern(name));
        // An explicit entity linetype is looked up in the drawing's table; a
        // missing name is reported, never silently turned into dashes. R13/R14
        // entities may carry only a handle, so fall back to the table's handle
        // index when the name is absent.
        let handle_name = common
            .linetype_handle
            .filter(|h| !h.is_null())
            .and_then(|h| self.linetype_names_by_handle.get(&h.value()).cloned());
        let source_linetype: String = if is_bylayer_linetype(&common.linetype) {
            match handle_name {
                Some(name) => name,
                None => common.linetype.clone(),
            }
        } else {
            common.linetype.clone()
        };
        let named_key = source_linetype.trim().to_ascii_lowercase();
        let named_pattern =
            if is_bylayer_linetype(&source_linetype) || is_byblock_linetype(&source_linetype) {
                None
            } else {
                self.linetype_patterns.get(&named_key).cloned()
            };
        if !named_key.is_empty()
            && !is_bylayer_linetype(&source_linetype)
            && !is_byblock_linetype(&source_linetype)
            && named_pattern.is_none()
        {
            self.diagnostics.push(Diagnostic {
                object: Some(object_id),
                code: "import.linetype_unknown".into(),
                message: format!(
                    "linetype '{}' is not in the drawing's linetype table; line drawn continuous",
                    source_linetype
                ),
            });
        }
        let resolved_linetype = resolve_entity_linetype(
            &source_linetype,
            common.linetype_scale,
            named_pattern.clone(),
            layer_pattern.clone(),
        );
        // A `ByLayer` entity whose layer names a non-continuous linetype that is
        // missing from the table cannot resolve; report rather than silently
        // drawing the layer solid.
        if is_bylayer_linetype(&source_linetype) {
            if let Some(layer_name) = self.layer_linetypes.get(&layer) {
                let key = layer_name.trim().to_ascii_lowercase();
                if !key.is_empty()
                    && !is_continuous_linetype(layer_name)
                    && !self.linetype_patterns.contains_key(&key)
                {
                    self.diagnostics.push(Diagnostic {
                        object: Some(object_id),
                        code: "import.linetype_unknown".into(),
                        message: format!(
                            "layer linetype '{}' is not in the drawing's linetype table; line drawn continuous",
                            layer_name
                        ),
                    });
                }
            }
        }
        // A complex linetype still dashes by its segment lengths, but its
        // shape/text glyphs are not drawn; report the omission.
        let effective_key = if is_bylayer_linetype(&source_linetype) {
            self.layer_linetypes
                .get(&layer)
                .map(|n| n.trim().to_ascii_lowercase())
        } else if is_byblock_linetype(&source_linetype) {
            None
        } else {
            Some(named_key)
        };
        if let Some(key) = &effective_key {
            if self.complex_linetypes.contains(key) {
                self.diagnostics.push(Diagnostic {
                    object: Some(object_id),
                    code: "import.linetype_complex".into(),
                    message: format!(
                        "linetype '{key}' has shape/text elements; only its dash segments are drawn"
                    ),
                });
            }
        }
        let (annotative, annotative_reason) =
            annotative_attributes(self.acad, entity, &self.scale_names);
        if let Some(reason) = annotative_reason {
            self.diagnostics.push(Diagnostic {
                object: Some(object_id),
                code: "annotative.context_unsupported".into(),
                message: reason,
            });
        }
        let attributes = EntityRenderAttributes {
            transparency: resolve_entity_transparency(common.transparency, layer_alpha),
            color: resolve_entity_color(common.color, layer_color),
            lineweight: resolve_entity_lineweight(common.line_weight, layer_lineweight),
            linetype: resolved_linetype,
            geometry_source: if proxy_geometry_allowed(entity) {
                GeometrySource::ProxyCache
            } else {
                GeometrySource::Analytic
            },
            annotative,
        };

        let record = DbEntity {
            object: DbObject {
                id: object_id,
                type_key: class_name.clone(),
                revision: Revision(0),
                source_handle: Some(format!("{:X}", common.handle.value())),
            },
            id: entity_id,
            layer,
            space,
            geometry,
            draw_order: self.entity_total as i64,
        };
        if let Err(e) = self.builder.insert_entity(record) {
            self.diagnostics.push(Diagnostic {
                object: Some(object_id),
                code: "import.insert_failed".into(),
                message: e.to_string(),
            });
            return None;
        }
        if let Err(e) = self
            .builder
            .set_entity_render_attributes(entity_id, attributes)
        {
            // The entity was just inserted, so this cannot fail today; report
            // rather than pretend the transparency was recorded.
            self.diagnostics.push(Diagnostic {
                object: Some(object_id),
                code: "import.render_attributes_failed".into(),
                message: e.to_string(),
            });
        }
        Some(entity_id)
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
            EntityType::Hatch(h) => Self::hatch_geometry(h),
            EntityType::Dimension(d) => {
                // Prefer the pre-rendered anonymous block (`*D...`) when the
                // drawing stores it; expand it like an insert. Many producers
                // (including the QCAD flange sample) omit the block and keep
                // only the definition points, so synthesize the display
                // geometry from those points and the DIMSTYLE instead.
                let base = d.base();
                let has_block =
                    !base.block_name.is_empty() && self.block_ids.contains_key(&base.block_name);
                if has_block {
                    (
                        SemanticGeometry::Insert {
                            block: self.block_id(&base.block_name),
                            transform: dimension_transform(base),
                        },
                        Completeness::Complete,
                    )
                } else {
                    self.dimension_semantics(d)
                }
            }
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
            EntityType::MLine(m) => mline_semantics(m),
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

    /// Text geometry for an ATTRIB/ATTDEF value, at its insertion point and
    /// using its own text style.
    pub(crate) fn attribute_text(
        &self,
        style: &str,
        text: String,
        position: Point3,
        height: f64,
        rotation: f64,
    ) -> SemanticGeometry {
        SemanticGeometry::Text {
            text,
            position,
            style: self.style_id(style),
            height,
            rotation,
            font: self.style_font(style),
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
        }
    }

    pub(crate) fn block_id(&self, name: &str) -> BlockId {
        self.block_ids
            .get(name)
            .copied()
            .unwrap_or(BlockId(u128::MAX))
    }

    /// The block's insertion base point, defaulting to the origin.
    pub(crate) fn block_base(&self, name: &str) -> Point3 {
        self.block_base_points
            .get(name)
            .copied()
            .unwrap_or_default()
    }

    /// Convert an INSERT (including an array / MINSERT) into expandable
    /// geometry.
    ///
    /// One cell yields a single [`SemanticGeometry::Insert`]; `rows × columns`
    /// cells yield a [`SemanticGeometry::Compound`] of them, one per cell (audit
    /// B31). The block definition itself is never emitted directly, so it cannot
    /// be double-drawn. Unknown block names are reported `Missing` rather than
    /// expanded as an empty success.
    pub(crate) fn insert_semantics(
        &self,
        i: &acadrust::entities::Insert,
    ) -> (SemanticGeometry, Completeness) {
        let name = i.block_name.clone();
        let base = self.block_base(&name);
        let mut completeness = Completeness::Complete;
        if !self.block_ids.contains_key(&name) {
            completeness =
                Completeness::Missing(vec![format!("insert references unknown block '{name}'")]);
        }
        let columns = i.column_count.max(1) as usize;
        let rows = i.row_count.max(1) as usize;
        // A non-finite spacing cannot produce distinct cells; drawing them all on
        // top of each other would hide the state, so report it.
        let spacing_bad = |s: f64| !s.is_finite();
        if columns * rows > 1 && (spacing_bad(i.column_spacing) || spacing_bad(i.row_spacing)) {
            completeness = Completeness::Partial(vec![
                "insert array has a non-finite row/column spacing".into(),
            ]);
        }
        let mut instances = Vec::with_capacity(columns * rows);
        for row in 0..rows {
            for column in 0..columns {
                instances.push(SemanticGeometry::Insert {
                    block: self.block_id(&name),
                    transform: insert_array_transform(
                        i,
                        base,
                        column as f64 * i.column_spacing,
                        row as f64 * i.row_spacing,
                    ),
                });
            }
        }
        let geometry = match instances.len() {
            1 => instances.pop().expect("length checked"),
            _ => SemanticGeometry::Compound(instances),
        };
        (geometry, completeness)
    }

    /// Convert a HATCH into a compound of boundary loops plus a solid fill or
    /// pattern lines. Boundaries are always emitted; fills that cannot be
    /// generated are reported as Partial rather than faked.
    pub(crate) fn hatch_geometry(h: &Hatch) -> (SemanticGeometry, Completeness) {
        let normal = p3(h.normal);
        // A degenerate normal cannot define a hatch plane; the boundary is still
        // emitted, but any fill is honestly reported as boundary-only.
        let plane_ok = cad_geometry::is_finite(normal) && cad_geometry::length(normal) > 1e-12;
        let (ux, uy, un) = arbitrary_axis(normal);
        let origin = cad_geometry::scale(un, h.elevation);
        let to_world = |p: [f64; 2]| {
            cad_geometry::add(
                origin,
                cad_geometry::add(cad_geometry::scale(ux, p[0]), cad_geometry::scale(uy, p[1])),
            )
        };
        let params = TessellationParams {
            tolerance: hatch_tolerance(h),
            ..TessellationParams::default()
        };
        let mut loops: Vec<Vec<[f64; 2]>> = Vec::new();
        let mut children: Vec<SemanticGeometry> = Vec::new();
        for path in &h.paths {
            let mut points: Vec<[f64; 2]> = Vec::new();
            for edge in &path.edges {
                append_boundary_edge(edge, &mut points, &params);
            }
            dedup_loop(&mut points);
            if points.len() < 3 {
                continue;
            }
            children.push(SemanticGeometry::Polyline {
                points: points.iter().map(|p| to_world(*p)).collect(),
                bulges: Vec::new(),
                closed: true,
            });
            loops.push(points);
        }
        if loops.is_empty() {
            return (
                SemanticGeometry::Opaque {
                    type_key: "AcDbHatch".into(),
                    version: 1,
                    payload: Vec::new(),
                },
                Completeness::Partial(vec!["hatch has no usable boundary".into()]),
            );
        }
        let mut completeness = Completeness::Complete;
        let solid = h.is_solid || h.pattern.name.eq_ignore_ascii_case("SOLID");

        // Simplify every ring with a tolerance scaled to the hatch's own extent.
        // This keeps the even-odd fill bounded (spec §3.2) and shared by the
        // solid and gradient paths.
        let fill_tolerance = {
            let (min, max) = loops.iter().flatten().fold(
                ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
                |(mut lo, mut hi), p| {
                    lo[0] = lo[0].min(p[0]);
                    lo[1] = lo[1].min(p[1]);
                    hi[0] = hi[0].max(p[0]);
                    hi[1] = hi[1].max(p[1]);
                    (lo, hi)
                },
            );
            let diagonal = ((max[0] - min[0]).powi(2) + (max[1] - min[1]).powi(2)).sqrt();
            (diagonal * 1e-3).max(1e-9)
        };
        let simplified: Vec<Vec<[f64; 2]>> = loops
            .iter()
            .filter_map(|loop2| {
                let mut closed = loop2.clone();
                closed.push(closed[0]);
                let mut s = cad_geometry::simplify(&closed, fill_tolerance);
                if s.len() > 1 && s.last() == s.first() {
                    s.pop();
                }
                (s.len() >= 3).then_some(s)
            })
            .collect();

        // A gradient is checked before the solid flag: a gradient HATCH is
        // stored as a "solid" hatch with gradient metadata, so testing `solid`
        // first would silently draw it as a single flat colour.
        match translate_gradient(h) {
            GradientTranslation::Unsupported(code) => {
                // Never fake a gradient as solid or boundary-only-success.
                completeness = Completeness::Partial(vec![format!(
                    "gradient hatch not rendered ({code}); boundary only"
                )]);
            }
            GradientTranslation::Supported(def) => {
                if !plane_ok {
                    completeness = Completeness::Partial(vec![
                        "hatch plane normal is degenerate; boundary only".into(),
                    ]);
                } else {
                    match cad_geometry::fill_rings(&simplified) {
                        Ok(fill) => {
                            let colors = cad_geometry::gradient_vertex_colors(&fill, &def);
                            if colors.len() != fill.vertices.len() {
                                completeness = Completeness::Partial(vec![
                                    "gradient hatch could not be baked; boundary only".into(),
                                ]);
                            } else {
                                let vertices: Vec<Point3> =
                                    fill.vertices.iter().map(|p| to_world(*p)).collect();
                                let normals = vec![un; vertices.len()];
                                children.push(SemanticGeometry::Mesh(Mesh {
                                    vertices,
                                    triangles: fill.triangles,
                                    normals,
                                    face_sources: Vec::new(),
                                    colors,
                                }));
                            }
                        }
                        Err(error) => {
                            completeness = Completeness::Partial(vec![format!(
                                "gradient hatch could not be filled: {}",
                                error.reason()
                            )]);
                        }
                    }
                }
            }
            GradientTranslation::Disabled if solid => {
                if !plane_ok {
                    completeness = Completeness::Partial(vec![
                        "hatch plane normal is degenerate; boundary only".into(),
                    ]);
                } else {
                    // Multi-ring holes/islands go through the even-odd fill; a
                    // failure is reported as Partial (boundary only), never faked.
                    match cad_geometry::fill_rings(&simplified) {
                        Ok(fill) => {
                            let vertices: Vec<Point3> =
                                fill.vertices.iter().map(|p| to_world(*p)).collect();
                            let normals = vec![un; vertices.len()];
                            children.push(SemanticGeometry::Mesh(Mesh {
                                vertices,
                                triangles: fill.triangles,
                                normals,
                                face_sources: Vec::new(),
                                colors: Vec::new(),
                            }));
                        }
                        Err(error) => {
                            completeness = Completeness::Partial(vec![format!(
                                "solid hatch could not be filled: {}",
                                error.reason()
                            )]);
                        }
                    }
                }
            }
            GradientTranslation::Disabled => {
                let families = pattern_families(h);
                if families.is_empty() {
                    completeness =
                        Completeness::Partial(vec!["hatch pattern has no line families".into()]);
                } else {
                    for line in cad_geometry::pattern_polylines(&loops, &families) {
                        if line.len() >= 2 {
                            children.push(SemanticGeometry::Polyline {
                                points: line.iter().map(|p| to_world(*p)).collect(),
                                bulges: Vec::new(),
                                closed: false,
                            });
                        }
                    }
                }
            }
        }
        (SemanticGeometry::Compound(children), completeness)
    }

    pub(crate) fn note_capability(
        &mut self,
        class_name: &str,
        geometry: &SemanticGeometry,
        completeness: &Completeness,
        render: SupportStatus,
        pick: SupportStatus,
    ) {
        let semantic = match completeness {
            Completeness::Complete => SupportStatus::Verified,
            Completeness::Partial(_) => SupportStatus::Partial,
            Completeness::Missing(_) => SupportStatus::Unsupported,
            Completeness::Unverified => SupportStatus::Unverified,
        };
        let measure = match geometry {
            SemanticGeometry::Mesh(_)
            | SemanticGeometry::Opaque { .. }
            | SemanticGeometry::Text { .. } => SupportStatus::Unsupported,
            _ => semantic,
        };
        let entry = self
            .capabilities
            .entry(class_name.to_string())
            .or_insert(EntityCapability {
                type_key: class_name.to_string(),
                read: SupportStatus::Verified,
                semantic,
                render,
                pick,
                measure,
            });
        // Keep the weakest status observed for the class.
        entry.semantic = weaker(entry.semantic, semantic);
        entry.render = weaker(entry.render, render);
        entry.pick = weaker(entry.pick, pick);
        entry.measure = weaker(entry.measure, measure);
    }
}
