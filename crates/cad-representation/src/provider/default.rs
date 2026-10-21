use crate::*;

use super::image::{
    image_basis_is_degenerate, image_clip_vertices, image_frame_corners, resolve_image_texture,
};
use super::registry::RepresentationProvider;

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

/// The chord-tessellation tolerance a build uses for curve geometry.
///
/// Shared by the provider's chord fallback and by the conic chord expansion so
/// the two can never drift apart.
pub(crate) fn chord_tessellation_params(context: &RepresentationContext) -> TessellationParams {
    TessellationParams {
        tolerance: context.tolerance.display_pixels.max(1e-12),
        ..TessellationParams::default()
    }
}

/// Build the analytic conic for a curve entity, or `None` when the source is
/// degenerate or non-finite (the caller then keeps the legacy chord path).
///
/// The semi-diameter vectors mirror the frame [`tessellate_geometry`] uses, so
/// expanding the conic back through that same tessellation reproduces the
/// pre-conic chord output.
fn conic_from_geometry(geometry: &SemanticGeometry) -> Option<ConicSegment> {
    match geometry {
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            let (ax, ay, _) = cad_geometry::arbitrary_axis(*normal);
            let conic = ConicSegment {
                center: *center,
                major: cad_geometry::scale(ax, *radius),
                minor: cad_geometry::scale(ay, *radius),
                start: 0.0,
                sweep: std::f64::consts::TAU,
                full: true,
            };
            conic.axes_are_non_degenerate().then_some(conic)
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let (ax, ay, _) = cad_geometry::arbitrary_axis(*normal);
            let conic = ConicSegment {
                center: *center,
                major: cad_geometry::scale(ax, *radius),
                minor: cad_geometry::scale(ay, *radius),
                start: *start,
                sweep: *sweep,
                full: sweep.abs() >= std::f64::consts::TAU - 1e-9,
            };
            conic.axes_are_non_degenerate().then_some(conic)
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            let major_len = cad_geometry::length(*major_axis);
            if !(major_len.is_finite() && major_len > 0.0) {
                return None;
            }
            let u = cad_geometry::scale(*major_axis, 1.0 / major_len);
            // The minor direction is the in-plane +90° rotation of the major
            // axis, matching `ellipse_minor_dir` in the tessellation frame.
            let minor_dir = {
                let c = cad_geometry::cross(*normal, u);
                let len = cad_geometry::length(c);
                if len >= 1e-12 {
                    cad_geometry::scale(c, 1.0 / len)
                } else {
                    return None;
                }
            };
            let conic = ConicSegment {
                center: *center,
                major: *major_axis,
                minor: cad_geometry::scale(minor_dir, major_len * ratio.abs()),
                start: *start,
                sweep: *sweep,
                full: sweep.abs() >= std::f64::consts::TAU - 1e-9,
            };
            conic.axes_are_non_degenerate().then_some(conic)
        }
        _ => None,
    }
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
                        ),
                        None => engine.shape_unstyled(
                            text,
                            eff_position,
                            eff_height,
                            eff_rotation,
                            *h_align,
                            *v_align,
                            eff_wrap_width,
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
            } => {
                // A hidden image draws nothing; never substitute a fake frame.
                if *visible {
                    let (resource, decoded) =
                        resolve_image_texture(file.as_deref(), context.images.as_deref());
                    // Map the entity-level clip once. A clip that cannot be
                    // expressed as a visible-region polygon is reported below,
                    // never silently ignored.
                    let clip_vertices = clip
                        .as_ref()
                        .and_then(|clip| image_clip_vertices(clip, *origin, *u, *v, *pixels));
                    let clip_representable = clip.is_none() || clip_vertices.is_some();
                    match (&resource, decoded) {
                        (Some(key), Some(_image))
                            if clip_representable
                                && !image_basis_is_degenerate(*u, *v, *pixels) =>
                        {
                            // The texture's unit square maps to the placed quad.
                            let transform = Transform3::from_basis(
                                cad_geometry::scale(*u, pixels[0]),
                                cad_geometry::scale(*v, pixels[1]),
                                *origin,
                            );
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
                                primitive: DisplayPrimitive::Image {
                                    resource: key.clone(),
                                    transform,
                                    clip: clip_vertices
                                        .map(|vertices| Arc::from(vertices.into_boxed_slice())),
                                },
                            });
                        }
                        (resource, decoded) => {
                            // No usable texture, or an unrepresentable clip: draw
                            // the frame only and report the honest reason instead
                            // of a placeholder image.
                            let (code, message) = match (resource, clip, decoded) {
                                (None, _, _) => (
                                    "image.unresolved",
                                    "image has no resolvable resource key; only the frame is drawn"
                                        .to_string(),
                                ),
                                // An outside/mask clip keeps the region *outside*
                                // its boundary, which the visible-region polygon
                                // cannot express.
                                (Some(_), Some(_), _) if !clip_representable => (
                                    "image.clip_unsupported",
                                    "image clip is not representable as a visible-region polygon; \
                                     only the frame is drawn"
                                        .to_string(),
                                ),
                                // A cache miss is a missing texture, not a
                                // host-confirmed decode failure.
                                (Some(key), _, None) => (
                                    "image.unresolved",
                                    format!(
                                        "no decoded texture for '{}' was available; only the frame is drawn",
                                        key.0
                                    ),
                                ),
                                (Some(_), _, Some(_)) => (
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
                        },
                    });
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

/// Tessellate a curve-like entity into a chord polyline and push it as a
/// `Lines` fragment (the pre-conic display path).
///
/// A shape that yields fewer than two points becomes an explicit `Missing`
/// result with a diagnostic, never an empty success.
fn push_chord_fragment(
    entity: &DbEntity,
    geometry: &SemanticGeometry,
    context: &RepresentationContext,
    source: SelectionRef,
    geometry_source: GeometrySource,
    representation: &mut DisplayRepresentation,
) {
    let points = tessellate_geometry(geometry, chord_tessellation_params(context));
    if points.len() >= 2 {
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
            primitive: DisplayPrimitive::Lines(Arc::from(points.into_boxed_slice())),
        });
    } else {
        representation.completeness =
            Completeness::Missing(vec!["no display representation in this build".into()]);
        representation.diagnostics.push(Diagnostic {
            object: Some(ObjectId(entity.id.0)),
            code: "representation.empty".into(),
            message: format!("{} produced no drawable points", entity.object.type_key),
        });
    }
}
