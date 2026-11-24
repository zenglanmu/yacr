use crate::*;

use super::registry::RepresentationProvider;

mod attribute;
mod fragments;
mod raster;
mod table;
mod text;

pub(crate) use attribute::attribute_value_fragments;
pub(crate) use fragments::chord_tessellation_params;
use fragments::{body_fragment, conic_from_geometry, push_chord_fragment, push_mesh_fragment};
use raster::{image_fragment, ImageGeometry};
use text::{shape_fragment, text_fragment, ShapeGeometry, TextGeometry};

/// The built-in provider for the core entity set (spec §3.2).
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultRepresentationProvider;

/// The text/MTEXT annotation-scale adjustment for one build.
///
/// `factor` is the active scale's paper/drawing factor; `position`/`rotation`/
/// `height` are an optional per-scale override (applied as-is when present).
#[derive(Debug, Clone, Copy)]
pub(super) struct TextScale {
    pub(super) factor: f64,
    pub(super) position: Option<Point3>,
    pub(super) rotation: Option<f64>,
    pub(super) height: Option<f64>,
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
            // ADR-0055: attribute text rendering is a later lane; the block
            // instance primitive is unchanged here.
            SemanticGeometry::Insert {
                block, transform, ..
            } => {
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
            // ADR-0055: a standalone `ATTDEF` entity (model space) is drawn as
            // the text it templates, exactly like the pre-M2 TEXT fold. A block
            // definition's members are skipped during INSERT expansion instead
            // (their per-reference values are what draw there).
            SemanticGeometry::AttributeDefinition(definition) => {
                let text = if definition.default_value.is_empty() {
                    definition.tag.clone()
                } else {
                    definition.default_value.clone()
                };
                if definition.text.oblique.abs() > 1e-12 {
                    representation.completeness =
                        representation
                            .completeness
                            .clone()
                            .combine(Completeness::Partial(vec![
                                "attribute oblique angle is stored but drawn upright".into(),
                            ]));
                    representation.diagnostics.push(Diagnostic {
                        object: Some(ObjectId(entity.id.0)),
                        code: "attribute.oblique_not_drawn".into(),
                        message: format!(
                            "attribute {:?} has an oblique angle {} rad; the shared text path draws \
                             it upright",
                            definition.tag, definition.text.oblique
                        ),
                    });
                }
                text_fragment(
                    entity,
                    context,
                    annotative,
                    TextGeometry {
                        text: &text,
                        position: &definition.text.position,
                        style: &definition.text.style,
                        height: &definition.text.height,
                        rotation: &definition.text.rotation,
                        font: &definition.text.font,
                        h_align: &definition.text.h_align,
                        v_align: &definition.text.v_align,
                        wrap_width: &definition.text.wrap_width,
                        width_factor: &definition.text.width_factor,
                        line_spacing: &definition.text.line_spacing,
                        // Attribute definitions have no MIRRTEXT bit.
                        mirrored: &false,
                    },
                    source,
                    geometry_source,
                    &mut representation,
                )
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
                mirrored,
            } => text_fragment(
                entity,
                context,
                annotative,
                TextGeometry {
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
                    mirrored,
                },
                source,
                geometry_source,
                &mut representation,
            ),
            SemanticGeometry::Shape {
                shape_name,
                code,
                position,
                size,
                rotation,
                font,
            } => shape_fragment(
                entity,
                context,
                ShapeGeometry {
                    shape_name,
                    code,
                    position,
                    size,
                    rotation,
                    font,
                },
                source,
                geometry_source,
                &mut representation,
            ),
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
            } => image_fragment(
                entity,
                context,
                ImageGeometry {
                    origin,
                    u,
                    v,
                    pixels,
                    file,
                    clip,
                    visible,
                    adjust,
                },
                source,
                geometry_source,
                &mut representation,
            ),
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
            SemanticGeometry::Body3d(b) => {
                body_fragment(entity, b, context, source, &mut representation)
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
