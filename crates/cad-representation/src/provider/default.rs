use crate::*;

use super::image::{
    image_basis_is_degenerate, image_clip_pieces, image_clip_reason, image_frame_corners,
    resolve_image_texture,
};
use super::registry::RepresentationProvider;

mod fragments;

pub(crate) use fragments::chord_tessellation_params;
use fragments::{conic_from_geometry, push_chord_fragment, push_mesh_fragment};

/// The built-in provider for the core entity set (spec §3.2).
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultRepresentationProvider;

/// The text/MTEXT annotation-scale adjustment for one build.
///
/// `factor` is the active scale's paper/drawing factor; `position`/`rotation`/
/// `height` are an optional per-scale override (applied as-is when present).
#[derive(Debug, Clone, Copy)]
struct TextScale {
    factor: f64,
    position: Option<Point3>,
    rotation: Option<f64>,
    height: Option<f64>,
}

impl RepresentationProvider for DefaultRepresentationProvider {
    fn registration(&self) -> Registration {
        Registration {
            type_key: "yacr.representation.default".into(),
            version: 1,
            priority: 0,
            entity_types: vec![], // matches any database entity
            capabilities: vec![
                "lines".into(),
                "mesh".into(),
                "text".into(),
                "instance".into(),
            ],
        }
    }

    fn build(
        &self,
        entity: &DbEntity,
        context: &RepresentationContext,
    ) -> CadResult<DisplayRepresentation> {
        self.build_inner(entity, context, None)
    }

    /// Apply the imported annotative state: an annotative text/MTEXT entity is
    /// scaled by the active annotation scale factor about its anchor, and a
    /// per-scale placement override (when imported) replaces the base
    /// position/rotation. A non-annotative entity, or a context without an
    /// active annotation scale, builds exactly as before.
    fn build_with_attributes(
        &self,
        entity: &DbEntity,
        attributes: &EntityRenderAttributes,
        context: &RepresentationContext,
    ) -> CadResult<DisplayRepresentation> {
        let annotative = attributes.annotative.annotative;
        let Some(scale) = context.annotation_scale.as_ref().filter(|_| annotative) else {
            return self.build_inner(entity, context, None);
        };
        let factor = scale.factor;
        if !factor.is_finite() || factor <= 0.0 {
            // An unusable factor must not silently scale by 1.0; report it and
            // draw the base geometry so the entity is still visible.
            let mut representation = self.build_inner(entity, context, None)?;
            representation.completeness =
                representation
                    .completeness
                    .combine(Completeness::Partial(vec![format!(
                    "annotative scale '{}' has an invalid factor {factor}; drawn at its base size",
                    scale.name
                )]));
            representation.diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "annotative.invalid_scale".into(),
                message: format!("active annotation scale factor {factor} is not usable"),
            });
            return Ok(representation);
        }
        let over = attributes.annotative.override_for(&scale.name);
        let mut representation = self.build_inner(
            entity,
            context,
            Some(TextScale {
                factor,
                position: over.map(|o| o.position),
                rotation: over.map(|o| o.rotation),
                height: over.and_then(|o| o.height),
            }),
        )?;

        // Annotative scaling is implemented for text/MTEXT only. Any other
        // annotative geometry (dimension styles, multileader, hatch, blocks)
        // is left at its base placement and reported `Partial`, never silently
        // scaled as if it were exact.
        if !matches!(entity.geometry, SemanticGeometry::Text { .. }) {
            representation.completeness =
                representation
                    .completeness
                    .combine(Completeness::Partial(vec![format!(
                        "annotative scaling for {} is not implemented; drawn at its base size",
                        entity.object.type_key
                    )]));
            representation.diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "annotative.unsupported_entity".into(),
                message: format!(
                    "{} is annotative but only TEXT/MTEXT annotative scaling is supported",
                    entity.object.type_key
                ),
            });
        }
        Ok(representation)
    }
}

impl DefaultRepresentationProvider {
    /// Build a table's fragments, stamping each derived primitive's own
    /// appearance onto the fragments it produced.
    ///
    /// The derived display is one geometry per drawn primitive **plus** a
    /// parallel appearance list of the same length (the domain builds both
    /// together), so the zip cannot drift. A primitive with
    /// [`TableDisplayAppearance::NONE`] keeps the entity's resolved colour and
    /// lineweight, i.e. its fragments are byte-identical to the pre-appearance
    /// build.
    ///
    /// The one thing the display cannot honour is a border **lineweight**: every
    /// line in this build is drawn one pixel wide, so a stored weight reaches the
    /// batch (where the renderer reports `render.lineweight_not_drawn`) but no
    /// wider stroke. That gap is reported here, once per build, and only when a
    /// weight is actually modelled.
    fn build_table(
        &self,
        entity: &DbEntity,
        table: &cad_domain::TableGeometry,
        context: &RepresentationContext,
        annotative: Option<TextScale>,
    ) -> DisplayRepresentation {
        let mut representation = DisplayRepresentation {
            fragments: Vec::new(),
            completeness: Completeness::Complete,
            diagnostics: Vec::new(),
        };
        let appearances = table.display_appearances();
        let mut lineweight_modelled = false;
        for (index, child) in table.display().iter().enumerate() {
            let appearance = appearances
                .get(index)
                .copied()
                .unwrap_or(cad_domain::TableDisplayAppearance::NONE);
            let child_representation = match self.build_geometry(entity, child, context, annotative)
            {
                Ok(child) => child,
                Err(error) => {
                    // A child that cannot be built is reported, never silently
                    // dropped: the table's other cells still draw.
                    representation.completeness =
                        representation
                            .completeness
                            .combine(Completeness::Partial(vec![format!(
                                "a table display primitive could not be built: {error}"
                            )]));
                    representation.diagnostics.push(Diagnostic {
                        object: Some(ObjectId(entity.id.0)),
                        code: "representation.table_primitive_failed".into(),
                        message: format!("table display primitive {index} was not drawn: {error}"),
                    });
                    continue;
                }
            };
            if appearance.lineweight_hundredths.is_some() {
                lineweight_modelled = true;
            }
            for mut fragment in child_representation.fragments {
                if let Some(rgb) = appearance.color {
                    fragment.color = [
                        rgb[0] as f32 / 255.0,
                        rgb[1] as f32 / 255.0,
                        rgb[2] as f32 / 255.0,
                    ];
                    fragment.color_unresolved = false;
                }
                if let Some(hundredths) = appearance.lineweight_hundredths {
                    fragment.lineweight = hundredths as f32 / 100.0;
                    fragment.lineweight_unresolved = false;
                }
                representation.fragments.push(fragment);
            }
            representation
                .diagnostics
                .extend(child_representation.diagnostics);
            representation.completeness = representation
                .completeness
                .combine(child_representation.completeness);
        }
        if lineweight_modelled {
            representation.completeness = representation
                .completeness
                .combine(Completeness::Partial(vec![
                    "a table cell border lineweight is stored and exported but drawn one pixel                      wide: this build draws every line at one width"
                        .into(),
                ]));
            representation.diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "representation.table_lineweight_not_drawn".into(),
                message: "per-cell table border lineweights reach the batch but the renderer draws                           every line one pixel wide"
                    .into(),
            });
        }
        representation
    }

    fn build_inner(
        &self,
        entity: &DbEntity,
        context: &RepresentationContext,
        annotative: Option<TextScale>,
    ) -> CadResult<DisplayRepresentation> {
        self.build_geometry(entity, &entity.geometry, context, annotative)
    }

    fn build_geometry(
        &self,
        entity: &DbEntity,
        geometry: &SemanticGeometry,
        context: &RepresentationContext,
        annotative: Option<TextScale>,
    ) -> CadResult<DisplayRepresentation> {
        // A HATCH with `FILL OFF` draws its boundary loops only: the fill
        // (solid/gradient mesh or pattern lines) is the part `FILL` governs, and
        // dropping it is a display change, not a geometry change. The loops are
        // rebuilt from the hatch's own plane/loops (see `display_modes`), so the
        // outline keeps the filled draw's vertices exactly.
        if !context.fill {
            if let SemanticGeometry::Hatch(hatch) = geometry {
                let mut representation = DisplayRepresentation {
                    fragments: Vec::new(),
                    completeness: Completeness::Complete,
                    diagnostics: Vec::new(),
                };
                for child in super::display_modes::hatch_outline_geometry(hatch) {
                    let child_representation =
                        self.build_geometry(entity, &child, context, annotative)?;
                    representation
                        .fragments
                        .extend(child_representation.fragments);
                    representation
                        .diagnostics
                        .extend(child_representation.diagnostics);
                    representation.completeness = representation
                        .completeness
                        .combine(child_representation.completeness);
                }
                return Ok(representation);
            }
        }
        // A TABLE renders its derived display with the **per-primitive
        // appearance** each primitive carries (a cell's text colour, its
        // background fill and its border colour/lineweight). The provider is the
        // one layer that can state a concrete per-primitive value, so it stamps
        // the fragment's colour/lineweight and clears the matching `unresolved`
        // flag; `expand_entity` keeps a value the provider resolved instead of
        // applying the entity's own `ByLayer`/`ByBlock` provenance. A primitive
        // with no override (the default for every table that never set one) is
        // left untouched, so its fragment is exactly what it was before.
        if let SemanticGeometry::Table(table) = geometry {
            return Ok(self.build_table(entity, table, context, annotative));
        }
        // A compound entity (for example a HATCH) renders as a group; recurse
        // over borrowed children. Cloning the parent for each child copies the
        // entire compound repeatedly and makes large hatches quadratic. A
        // dimension renders the same way over its display children (ADR 0017
        // A.5/A.8) with no new primitive or overlay.
        let grouped: Option<&[SemanticGeometry]> = match geometry {
            SemanticGeometry::Compound(children) => Some(children),
            SemanticGeometry::Dimension(dimension) => Some(dimension.display()),
            SemanticGeometry::Hatch(hatch) => Some(hatch.display()),
            // A multileader renders its display children, exactly like the
            // compound it used to be (ADR 0019 decision G).
            SemanticGeometry::MultiLeader(multileader) => Some(multileader.display()),
            _ => None,
        };
        if let Some(children) = grouped {
            let mut representation = DisplayRepresentation {
                fragments: Vec::new(),
                completeness: Completeness::Complete,
                diagnostics: Vec::new(),
            };
            for child in children {
                let child_representation =
                    self.build_geometry(entity, child, context, annotative)?;
                representation
                    .fragments
                    .extend(child_representation.fragments);
                representation
                    .diagnostics
                    .extend(child_representation.diagnostics);
                representation.completeness = representation
                    .completeness
                    .combine(child_representation.completeness);
            }
            return Ok(representation);
        }
        let source = SelectionRef {
            document: context.document,
            entity: entity.id,
            instance: InstancePath::default(),
            sub_element: None,
        };
        let mut representation = DisplayRepresentation {
            fragments: Vec::new(),
            completeness: Completeness::Complete,
            diagnostics: Vec::new(),
        };
        let geometry_source = GeometrySource::Analytic;

        match geometry {
            SemanticGeometry::Mesh(mesh) => {
                // `FILL OFF` draws a 2D SOLID as its outline. The type key is
                // what tells a 2D SOLID apart from every other mesh (a kernel
                // display mesh, a dimension arrowhead, a vendor proxy): only
                // `AcDbSolid` is one of the filled areas AutoCAD's `FILL`
                // governs, and it is the only mesh this build stores with a
                // derivable area boundary. See `display_modes` for the rule and
                // for the meshes that stay filled by design.
                let outline = (!context.fill && entity.object.type_key == "AcDbSolid")
                    .then(|| super::display_modes::solid_outline_primitive(mesh))
                    .flatten();
                if let Some(primitive) = outline {
                    representation.fragments.push(DisplayFragment {
                        source,
                        geometry_source,
                        precision: Precision::Analytic,
                        alpha: 1.0,
                        draw_order: entity.draw_order,
                        color: DEFAULT_RENDER_COLOR,
                        color_unresolved: true,
                        lineweight: DEFAULT_LINEWEIGHT_MM,
                        lineweight_unresolved: true,
                        linetype: LinetypePattern::continuous(),
                        linetype_unresolved: true,
                        linetype_scale: 1.0,
                        primitive,
                    });
                } else {
                    representation.fragments.push(DisplayFragment {
                        source,
                        geometry_source,
                        precision: Precision::Analytic,
                        alpha: 1.0,
                        draw_order: entity.draw_order,
                        color: DEFAULT_RENDER_COLOR,
                        color_unresolved: true,
                        lineweight: DEFAULT_LINEWEIGHT_MM,
                        lineweight_unresolved: true,
                        linetype: LinetypePattern::continuous(),
                        linetype_unresolved: true,
                        linetype_scale: 1.0,
                        primitive: DisplayPrimitive::Mesh(Arc::new(mesh.clone())),
                    });
                }
                if mesh.triangles.is_empty() {
                    representation.completeness =
                        Completeness::Partial(vec!["mesh has no triangles".into()]);
                }
            }
            SemanticGeometry::Insert { block, transform } => {
                representation.fragments.push(DisplayFragment {
                    source,
                    geometry_source,
                    precision: Precision::Analytic,
                    alpha: 1.0,
                    draw_order: entity.draw_order,
                    color: DEFAULT_RENDER_COLOR,
                    color_unresolved: true,
                    lineweight: DEFAULT_LINEWEIGHT_MM,
                    lineweight_unresolved: true,
                    linetype: LinetypePattern::continuous(),
                    linetype_unresolved: true,
                    linetype_scale: 1.0,
                    primitive: DisplayPrimitive::Instance {
                        block: *block,
                        transform: *transform,
                    },
                });
            }
            SemanticGeometry::Text {
                text,
                position,
                style,
                height,
                rotation,
                font,
                h_align,
                v_align,
                wrap_width,
                width_factor,
                line_spacing,
            } => {
                let font_key = font.as_deref();
                // Annotative TEXT/MTEXT: the active annotation scale factor
                // scales the glyph size about the anchor, and a per-scale
                // placement override (when imported) replaces the base
                // position/rotation. Non-annotative entities use the base
                // geometry unchanged.
                let (eff_position, eff_height, eff_rotation) = match annotative {
                    Some(a) => (
                        a.position.unwrap_or(*position),
                        a.height.unwrap_or(height.abs() * a.factor),
                        a.rotation.unwrap_or(*rotation),
                    ),
                    None => (*position, height.abs(), *rotation),
                };
                // The wrap boundary is a world-space length like the glyph
                // height, so the annotation scale factor scales it too; leaving
                // it fixed would re-flow the text at every annotation scale.
                let eff_wrap_width = match annotative {
                    Some(a) => wrap_width.map(|width| width * a.factor),
                    None => *wrap_width,
                };
                let mut shaped = false;
                // A style that names no font still shapes through the engine's
                // fallback chain: dropping the text because the drawing has no
                // font-bearing STYLE entry would silently hide every created
                // dimension's measurement.
                if let Some(engine) = &context.fonts {
                    let font_label = font_key.unwrap_or("the text style's font");
                    let result = match font_key {
                        Some(key) => engine.shape(
                            key,
                            text,
                            eff_position,
                            eff_height,
                            eff_rotation,
                            *h_align,
                            *v_align,
                            eff_wrap_width,
                            *width_factor,
                            *line_spacing,
                        ),
                        None => engine.shape_unstyled(
                            text,
                            eff_position,
                            eff_height,
                            eff_rotation,
                            *h_align,
                            *v_align,
                            eff_wrap_width,
                            *width_factor,
                            *line_spacing,
                        ),
                    };
                    match result {
                        Ok(shaped_text) => {
                            let has_renderable_content = shaped_text.has_renderable_content;
                            for mesh in shaped_text.meshes {
                                representation.fragments.push(DisplayFragment {
                                    source: source.clone(),
                                    geometry_source: geometry_source.clone(),
                                    precision: Precision::Analytic,
                                    alpha: 1.0,
                                    draw_order: entity.draw_order,
                                    color: DEFAULT_RENDER_COLOR,
                                    color_unresolved: true,
                                    lineweight: DEFAULT_LINEWEIGHT_MM,
                                    lineweight_unresolved: true,
                                    linetype: LinetypePattern::continuous(),
                                    linetype_unresolved: true,
                                    linetype_scale: 1.0,
                                    primitive: DisplayPrimitive::MeshWithoutEdgeOverlay(Arc::new(
                                        mesh,
                                    )),
                                });
                            }
                            for polyline in shaped_text.polylines {
                                if polyline.len() >= 2 {
                                    representation.fragments.push(DisplayFragment {
                                        source: source.clone(),
                                        geometry_source: geometry_source.clone(),
                                        precision: Precision::Analytic,
                                        alpha: 1.0,
                                        draw_order: entity.draw_order,
                                        color: DEFAULT_RENDER_COLOR,
                                        color_unresolved: true,
                                        lineweight: DEFAULT_LINEWEIGHT_MM,
                                        lineweight_unresolved: true,
                                        linetype: LinetypePattern::continuous(),
                                        linetype_unresolved: true,
                                        linetype_scale: 1.0,
                                        primitive: DisplayPrimitive::Lines(Arc::from(
                                            polyline.into_boxed_slice(),
                                        )),
                                    });
                                }
                            }
                            shaped = true;
                            if representation.fragments.is_empty() && has_renderable_content {
                                representation.completeness = Completeness::Missing(vec![
                                    "text produced no drawable outlines".into(),
                                ]);
                                representation.diagnostics.push(Diagnostic {
                                    object: Some(ObjectId(entity.id.0)),
                                    code: "text.empty_outline".into(),
                                    message: format!(
                                        "font '{font_label}' produced no outline for the text"
                                    ),
                                });
                            }
                            // MTEXT formatting that shaped only approximately
                            // (color, decorations, stacked fractions, ...) is
                            // reported in the completeness verdict and
                            // diagnostics rather than silently ignored.
                            if !shaped_text.issues.is_empty() {
                                let reasons: Vec<String> = shaped_text
                                    .issues
                                    .iter()
                                    .map(|issue| issue.message.clone())
                                    .collect();
                                representation.completeness = representation
                                    .completeness
                                    .combine(Completeness::Partial(reasons));
                                for issue in &shaped_text.issues {
                                    representation.diagnostics.push(Diagnostic {
                                        object: Some(ObjectId(entity.id.0)),
                                        code: issue.code.into(),
                                        message: issue.message.clone(),
                                    });
                                }
                            }
                        }
                        Err(error) => {
                            representation.completeness = Completeness::Partial(vec![format!(
                                "text font '{font_label}' unavailable: {error}"
                            )]);
                            representation.diagnostics.push(Diagnostic {
                                object: Some(ObjectId(entity.id.0)),
                                code: "text.font_unavailable".into(),
                                message: error.to_string(),
                            });
                        }
                    }
                }
                if !shaped {
                    representation.fragments.push(DisplayFragment {
                        source,
                        geometry_source,
                        precision: Precision::Analytic,
                        alpha: 1.0,
                        draw_order: entity.draw_order,
                        color: DEFAULT_RENDER_COLOR,
                        color_unresolved: true,
                        lineweight: DEFAULT_LINEWEIGHT_MM,
                        lineweight_unresolved: true,
                        linetype: LinetypePattern::continuous(),
                        linetype_unresolved: true,
                        linetype_scale: 1.0,
                        primitive: DisplayPrimitive::Text {
                            text: text.clone(),
                            origin: eff_position,
                            font: ResourceKey(
                                font_key
                                    .map(str::to_string)
                                    .unwrap_or_else(|| format!("style:{}", style.0)),
                            ),
                            height: eff_height,
                        },
                    });
                }
                // `QTEXT`: replace the glyph geometry that was just built with
                // one placeholder rectangle derived from it. The text is shaped
                // either way — this build claims a display change, not a
                // redraw-speed improvement — and a text whose extent cannot be
                // derived keeps its own draw plus a `qtext.extent_unknown`
                // diagnostic (see `display_modes::apply_qtext`).
                if context.qtext {
                    super::display_modes::apply_qtext(&mut representation, entity.id.0);
                }
            }
            SemanticGeometry::Shape {
                shape_name,
                code,
                position,
                size,
                rotation,
                font,
            } => {
                if *code == 0 {
                    representation.completeness = Completeness::Partial(vec![format!(
                        "shape '{shape_name}' is referenced by name; only shape codes are resolved"
                    )]);
                } else if let (Some(engine), Some(key)) = (&context.fonts, font.as_deref()) {
                    match engine.shape_glyph(key, *code, *position, size.abs(), *rotation) {
                        Ok(polylines) => {
                            for polyline in polylines {
                                if polyline.len() >= 2 {
                                    representation.fragments.push(DisplayFragment {
                                        source: source.clone(),
                                        geometry_source: geometry_source.clone(),
                                        precision: Precision::Analytic,
                                        alpha: 1.0,
                                        draw_order: entity.draw_order,
                                        color: DEFAULT_RENDER_COLOR,
                                        color_unresolved: true,
                                        lineweight: DEFAULT_LINEWEIGHT_MM,
                                        lineweight_unresolved: true,
                                        linetype: LinetypePattern::continuous(),
                                        linetype_unresolved: true,
                                        linetype_scale: 1.0,
                                        primitive: DisplayPrimitive::Lines(Arc::from(
                                            polyline.into_boxed_slice(),
                                        )),
                                    });
                                }
                            }
                        }
                        Err(error) => {
                            representation.completeness = Completeness::Partial(vec![format!(
                                "shape font '{key}' unavailable: {error}"
                            )]);
                            representation.diagnostics.push(Diagnostic {
                                object: Some(ObjectId(entity.id.0)),
                                code: "shape.font_unavailable".into(),
                                message: error.to_string(),
                            });
                        }
                    }
                } else {
                    representation.completeness =
                        Completeness::Partial(
                            vec!["shape has no resolvable SHX shape font".into()],
                        );
                }
            }
            SemanticGeometry::Opaque { type_key, .. } => {
                representation.completeness =
                    Completeness::Missing(vec!["no display representation in this build".into()]);
                representation.diagnostics.push(Diagnostic {
                    object: Some(ObjectId(entity.id.0)),
                    code: "representation.opaque".into(),
                    message: format!("{type_key} has no display representation in this build"),
                });
            }
            SemanticGeometry::Image {
                origin,
                u,
                v,
                pixels,
                file,
                clip,
                visible,
                adjust,
            } => {
                // A hidden image draws nothing; never substitute a fake frame.
                if *visible {
                    let (resource, decoded) =
                        resolve_image_texture(file.as_deref(), context.images.as_deref());
                    // Map the entity-level clip once: a convex clip stays one
                    // piece (byte-identical to the behaviour before concave clips
                    // were supported), a concave clip becomes one convex piece per
                    // ear of its triangulation. A clip that is not a usable region
                    // is refused by name below, never silently widened.
                    let clip = clip
                        .as_ref()
                        .map(|clip| image_clip_pieces(clip, *origin, *u, *v, *pixels));
                    // The clip's own verdict, independent of texture resolution.
                    let clip_refusal: Option<(&'static str, String)> = match &clip {
                        None | Some(Ok(Some(_))) => None,
                        Some(Ok(None)) => Some((
                            image_clip_reason::UNSUPPORTED,
                            "the image clip has no area inside the image rectangle; only the \
                             frame is drawn"
                                .to_string(),
                        )),
                        Some(Err(refusal)) => Some((refusal.code, refusal.message.clone())),
                    };
                    match (&resource, decoded) {
                        (Some(key), Some(_image))
                            if clip_refusal.is_none()
                                && !image_basis_is_degenerate(*u, *v, *pixels) =>
                        {
                            // The texture's unit square maps to the placed quad.
                            let transform = Transform3::from_basis(
                                cad_geometry::scale(*u, pixels[0]),
                                cad_geometry::scale(*v, pixels[1]),
                                *origin,
                            );
                            // No clip draws the whole unit square; a clip draws one
                            // fragment per convex piece, so the union of the
                            // fragments is exactly the visible texture region (also
                            // for a concave clip). The refusal cases are already
                            // handled below, so they contribute nothing here.
                            let pieces = match clip {
                                None => vec![None],
                                Some(Ok(Some(pieces))) => pieces.into_iter().map(Some).collect(),
                                Some(Ok(None) | Err(_)) => Vec::new(),
                            };
                            for piece in pieces {
                                representation.fragments.push(DisplayFragment {
                                    source: source.clone(),
                                    geometry_source: geometry_source.clone(),
                                    precision: Precision::Analytic,
                                    alpha: 1.0,
                                    draw_order: entity.draw_order,
                                    color: DEFAULT_RENDER_COLOR,
                                    color_unresolved: true,
                                    lineweight: DEFAULT_LINEWEIGHT_MM,
                                    lineweight_unresolved: true,
                                    linetype: LinetypePattern::continuous(),
                                    linetype_unresolved: true,
                                    linetype_scale: 1.0,
                                    primitive: DisplayPrimitive::Image {
                                        resource: key.clone(),
                                        transform,
                                        clip: piece
                                            .map(|vertices| Arc::from(vertices.into_boxed_slice())),
                                        adjust: *adjust,
                                    },
                                });
                            }
                        }
                        (resource, decoded) => {
                            // No usable texture, or a clip that is not a usable
                            // region: draw the frame only and report the honest
                            // reason instead of a placeholder image.
                            let (code, message) = match (resource, &clip_refusal, decoded) {
                                (None, _, _) => (
                                    "image.unresolved",
                                    "image has no resolvable resource key; only the frame is drawn"
                                        .to_string(),
                                ),
                                // An outside/mask clip keeps the region *outside* its
                                // boundary, which a visible-region polygon cannot
                                // express; a self-intersecting ring or too few /
                                // degenerate points has no interior to draw. Each
                                // refusal names itself and draws no region at all.
                                (Some(_), Some((code, message)), _) => (*code, message.clone()),
                                // A cache miss is a missing texture, not a
                                // host-confirmed decode failure.
                                (Some(key), None, None) => (
                                    "image.unresolved",
                                    format!(
                                        "no decoded texture for '{}' was available; only the frame is drawn",
                                        key.0
                                    ),
                                ),
                                (Some(_), None, Some(_)) => (
                                    "image.degenerate_frame",
                                    "image frame has a degenerate basis; only the frame is drawn"
                                        .to_string(),
                                ),
                            };
                            representation.completeness = representation
                                .completeness
                                .combine(Completeness::Partial(vec![message.clone()]));
                            representation.diagnostics.push(Diagnostic {
                                object: Some(ObjectId(entity.id.0)),
                                code: code.into(),
                                message,
                            });
                            let corners = image_frame_corners(*origin, *u, *v, *pixels);
                            if corners.len() >= 2 {
                                representation.fragments.push(DisplayFragment {
                                    source,
                                    geometry_source,
                                    precision: Precision::Analytic,
                                    alpha: 1.0,
                                    draw_order: entity.draw_order,
                                    color: DEFAULT_RENDER_COLOR,
                                    color_unresolved: true,
                                    lineweight: DEFAULT_LINEWEIGHT_MM,
                                    lineweight_unresolved: true,
                                    linetype: LinetypePattern::continuous(),
                                    linetype_unresolved: true,
                                    linetype_scale: 1.0,
                                    primitive: DisplayPrimitive::Lines(Arc::from(
                                        corners.into_boxed_slice(),
                                    )),
                                });
                            }
                        }
                    }
                }
            }
            SemanticGeometry::Circle { .. }
            | SemanticGeometry::Arc { .. }
            | SemanticGeometry::Ellipse { .. } => {
                match conic_from_geometry(geometry) {
                    Some(conic) => {
                        representation.fragments.push(DisplayFragment {
                            source,
                            geometry_source,
                            precision: Precision::Analytic,
                            alpha: 1.0,
                            draw_order: entity.draw_order,
                            color: DEFAULT_RENDER_COLOR,
                            color_unresolved: true,
                            lineweight: DEFAULT_LINEWEIGHT_MM,
                            lineweight_unresolved: true,
                            linetype: LinetypePattern::continuous(),
                            linetype_unresolved: true,
                            linetype_scale: 1.0,
                            primitive: DisplayPrimitive::Conic(Arc::new(conic)),
                        });
                    }
                    None => {
                        // Degenerate or non-finite axes: keep the legacy chord
                        // path so the draw is unchanged. A non-finite source is
                        // reported rather than silently tessellated to NaN.
                        if !cad_geometry::geometry_is_finite(geometry) {
                            representation.completeness =
                                representation
                                    .completeness
                                    .combine(Completeness::Partial(vec![
                                        "conic has non-finite axes; drawn as chords".into(),
                                    ]));
                            representation.diagnostics.push(Diagnostic {
                                object: Some(ObjectId(entity.id.0)),
                                code: "representation.conic_degenerate".into(),
                                message: "conic has non-finite axes; falling back to the chord \
                                          tessellation"
                                    .into(),
                            });
                        }
                        push_chord_fragment(
                            entity,
                            geometry,
                            context,
                            source,
                            geometry_source,
                            &mut representation,
                        );
                    }
                }
            }
            // A POINT has no base-scene geometry. The chord tessellator would
            // degenerate it to a zero-length `[p, p]` segment (invisible), so
            // skip it here and record honestly that the overlay marker draws it.
            // `cad_geometry::tessellate_geometry` is deliberately unchanged: its
            // `[p, p]` output is what picking, bounds and snapping consume.
            SemanticGeometry::Point(_) => {
                // Compose like every sibling branch (rather than assigning a
                // fresh `Partial`): harmless today because a fresh
                // representation starts `Complete`, but it keeps the arm correct
                // if it ever composes with another verdict.
                representation.completeness =
                    representation
                        .completeness
                        .combine(Completeness::Partial(vec![
                            "point is drawn by the overlay marker, not by base-scene geometry"
                                .into(),
                        ]));
                representation.diagnostics.push(Diagnostic {
                    object: Some(ObjectId(entity.id.0)),
                    code: "representation.point_overlay".into(),
                    message: "POINT has no base-scene geometry; it is drawn by the overlay marker"
                        .into(),
                });
            }
            // A WIPEOUT renders as a colourless mask polygon (ADR 0022): the
            // renderer fills it with its own clear colour in a final
            // `depth_compare: Always` pass, so it really hides what is beneath.
            // Both clip modes are rendered: a non-`inverted` mask fills the
            // boundary, an `inverted` one (DXF clip mode Inside) fills the
            // complement, which the renderer expresses by adding a
            // viewport-covering ring to the same even-odd fill (no stencil).
            SemanticGeometry::Mask { boundary, inverted } => {
                if boundary.len() < 3 {
                    // A boundary with fewer than three vertices has no area, so
                    // there is nothing to fill: keep the polyline and say why.
                    representation.completeness = representation
                        .completeness
                        .combine(Completeness::Partial(vec![MASK_BOUNDARY_DEGENERATE.into()]));
                    representation.diagnostics.push(Diagnostic {
                        object: Some(ObjectId(entity.id.0)),
                        code: "representation.mask_degenerate".into(),
                        message: MASK_BOUNDARY_DEGENERATE.into(),
                    });
                    push_chord_fragment(
                        entity,
                        geometry,
                        context,
                        source,
                        geometry_source,
                        &mut representation,
                    );
                } else {
                    representation.fragments.push(DisplayFragment {
                        source,
                        geometry_source,
                        precision: Precision::Analytic,
                        alpha: 1.0,
                        draw_order: entity.draw_order,
                        color: DEFAULT_RENDER_COLOR,
                        color_unresolved: true,
                        lineweight: DEFAULT_LINEWEIGHT_MM,
                        lineweight_unresolved: true,
                        linetype: LinetypePattern::continuous(),
                        linetype_unresolved: true,
                        linetype_scale: 1.0,
                        primitive: DisplayPrimitive::Mask {
                            boundary: Arc::from(boundary.clone().into_boxed_slice()),
                            inverted: *inverted,
                            frame: context.wipeout_frames,
                        },
                    });
                }
            }
            // A polyline with a real uniform width is drawn as a filled stroke
            // band (see `super::stroke`). Width `0.0` keeps the byte-identical
            // centreline `Lines` path below; a degenerate band falls back to it
            // too, so a wide polyline can never disappear.
            SemanticGeometry::Polyline { width, .. } if width.is_finite() && *width > 0.0 => {
                // `FILL OFF` (and only then) draws the band's outline: the same
                // offsets the filled band is built from, joined into one closed
                // loop. A hundred-percent shared rule, so the outline can never
                // disagree with the fill about where the stroke is.
                if !context.fill {
                    if let Some(points) = super::display_modes::wide_polyline_outline(
                        geometry,
                        chord_tessellation_params(context),
                        *width,
                    ) {
                        representation.fragments.push(DisplayFragment {
                            source,
                            geometry_source,
                            precision: Precision::Analytic,
                            alpha: 1.0,
                            draw_order: entity.draw_order,
                            color: DEFAULT_RENDER_COLOR,
                            color_unresolved: true,
                            lineweight: DEFAULT_LINEWEIGHT_MM,
                            lineweight_unresolved: true,
                            linetype: LinetypePattern::continuous(),
                            linetype_unresolved: true,
                            linetype_scale: 1.0,
                            primitive: DisplayPrimitive::Lines(points.into_boxed_slice().into()),
                        });
                        return Ok(representation);
                    }
                }
                match super::stroke::wide_polyline_mesh(
                    geometry,
                    chord_tessellation_params(context),
                    *width,
                ) {
                    Some(mesh) => push_mesh_fragment(
                        entity,
                        mesh,
                        source,
                        geometry_source,
                        &mut representation,
                    ),
                    None => {
                        representation.completeness = weaker_completeness(
                            &representation.completeness,
                            &Completeness::Partial(vec![
                                "polyline width stroke is degenerate; drawn as its centreline"
                                    .into(),
                            ]),
                        );
                        representation.diagnostics.push(Diagnostic {
                            object: Some(ObjectId(entity.id.0)),
                            code: "representation.polyline_stroke_degenerate".into(),
                            message: "the wide polyline centreline has fewer than two distinct \
                                      finite vertices; the centreline is kept"
                                .into(),
                        });
                        push_chord_fragment(
                            entity,
                            geometry,
                            context,
                            source,
                            geometry_source,
                            &mut representation,
                        );
                    }
                }
            }
            other => push_chord_fragment(
                entity,
                other,
                context,
                source,
                geometry_source,
                &mut representation,
            ),
        }
        Ok(representation)
    }
}
