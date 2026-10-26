use crate::*;

use super::default::chord_tessellation_params;
use super::registry::ProviderRegistry;

impl ProviderRegistry {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn expand_entity(
        &self,
        database: &DrawingDatabase,
        entity: &DbEntity,
        context: &RepresentationContext,
        transform: &Transform3,
        path: &InstancePath,
        depth: usize,
        parent_alpha: f32,
        // Display order this entity inherits: its own at the root, and the
        // containing INSERT's when expanding a block's children (see the
        // `Instance` arm). It is carried onto every emitted fragment.
        draw_order: i64,
        parent_color: Option<[f32; 3]>,
        parent_lineweight: Option<f32>,
        parent_linetype: Option<&LinetypePattern>,
        global_lt_scale: f64,
        stack: &mut Vec<BlockId>,
        out: &mut DisplayRepresentation,
    ) -> CadResult<()> {
        // The importer resolved this entity's effective opacity, lineweight and
        // linetype into the database. Colour is resolved **here** instead of at
        // import time, because `ByLayer` means "the owning layer's colour" and
        // only this layer has the layer table: a layer colour edit therefore
        // recolours its `ByLayer` entities on the next expansion with no entity
        // write and no revision bump. `ByBlock` inherits the containing INSERT's
        // value, threaded down through the `parent_*` arguments; at the model
        // root those fall back to the documented defaults.
        let attributes = database.entity_render_attributes(entity.id);
        // The provider sees the attributes too, so annotative text can be
        // scaled while it is shaped (the geometry is not post-processed).
        let own_alpha = match attributes.transparency {
            EntityTransparency::Explicit(alpha) => alpha,
            EntityTransparency::ByBlock => parent_alpha,
        };
        // Resolution rule for `ByLayer`: the colour of the layer the entity is
        // **itself** on (`DbEntity::layer`), for model-space entities and for
        // entities inside a block definition alike. Only `ByBlock` walks up the
        // parent chain to the containing INSERT's already-resolved colour, so a
        // block child's `ByLayer` never inherits the INSERT's layer.
        //
        // A layer with no stored attribute row uses
        // `DrawingDatabase::layer_attributes`'s documented
        // `LayerAttributes::default()` fallback (ACI 7 white) rather than a
        // second, locally invented default.
        let layer_color = database.layer_attributes(entity.layer).color.rgb;
        let (own_color, color_unresolved) =
            resolve_color(attributes.color, parent_color, layer_color);
        let (own_lineweight, lineweight_unresolved) =
            resolve_lineweight(attributes.lineweight, parent_lineweight);
        let (own_linetype, linetype_unresolved, linetype_scale) =
            resolve_linetype(attributes.linetype.clone(), parent_linetype);
        let representation = self.build_with_attributes(entity, &attributes, context)?;
        out.completeness = weaker_completeness(&out.completeness, &representation.completeness);
        out.diagnostics.extend(representation.diagnostics);
        for fragment in representation.fragments {
            // The database orders a space by the INSERT entity, so a block's
            // children take the INSERT's order rather than their own (which is
            // only meaningful inside the block definition) or a synthetic 0.
            let child_draw_order = fragment.draw_order;
            match fragment.primitive {
                DisplayPrimitive::Instance {
                    block,
                    transform: insert,
                } => {
                    if depth >= MAX_INSTANCE_DEPTH || stack.contains(&block) {
                        out.completeness = weaker_completeness(
                            &out.completeness,
                            &Completeness::Partial(vec!["block nesting limit or cycle".into()]),
                        );
                        out.diagnostics.push(Diagnostic {
                            object: Some(ObjectId(entity.id.0)),
                            code: "representation.instance_cycle".into(),
                            message: format!(
                                "insert of block {:?} hit the nesting limit or a cycle",
                                block
                            ),
                        });
                        continue;
                    }
                    if database.block(block).is_none() {
                        out.completeness = weaker_completeness(
                            &out.completeness,
                            &Completeness::Missing(vec!["missing block definition".into()]),
                        );
                        out.diagnostics.push(Diagnostic {
                            object: Some(ObjectId(entity.id.0)),
                            code: "representation.missing_block".into(),
                            message: format!("insert references unknown block {:?}", block),
                        });
                        continue;
                    }
                    let composed = transform.matrix_mul(&insert);
                    let mut child_path = path.clone();
                    child_path.0.push(entity.id);
                    stack.push(block);
                    for child in database.block_entities(block) {
                        self.expand_entity(
                            database,
                            child,
                            context,
                            &composed,
                            &child_path,
                            depth + 1,
                            own_alpha,
                            child_draw_order,
                            Some(own_color),
                            Some(own_lineweight),
                            Some(&own_linetype),
                            global_lt_scale,
                            stack,
                            out,
                        )?;
                    }
                    stack.pop();
                }
                DisplayPrimitive::Lines(points) => {
                    emit_polyline(
                        entity,
                        &points,
                        transform,
                        path,
                        &fragment.source,
                        &attributes,
                        own_alpha,
                        own_color,
                        color_unresolved,
                        own_lineweight,
                        lineweight_unresolved,
                        &own_linetype,
                        linetype_unresolved,
                        linetype_scale,
                        draw_order,
                        context.packed_line_segments,
                        global_lt_scale,
                        out,
                    );
                }
                DisplayPrimitive::Conic(conic) => {
                    // Slice B flips the emission gate (see
                    // `docs/adr/0008-sdf-conic-rendering.md`): an **eligible**
                    // circle/arc travels as the analytic SDF instance the
                    // renderer draws, instead of being chord-expanded. Every
                    // other conic keeps the chord draw and is counted by
                    // `ConicStats`, so nothing is dropped silently.
                    //
                    // Eligibility is decided on the **transformed** conic: a
                    // non-uniform INSERT turns a circle into an ellipse, which is
                    // slice 2 and must stay chords. The transformed frame keeps
                    // `start`/`sweep` unchanged (an affine map preserves the
                    // parameter), so only `center`/`major`/`minor` move.
                    let transformed =
                        match DisplayPrimitive::Conic(conic.clone()).transformed(transform) {
                            DisplayPrimitive::Conic(transformed) => transformed,
                            _ => unreachable!("transforming a conic yields a conic"),
                        };
                    let geometric = transformed.is_sdf_circle_arc();
                    let eligible = own_alpha == 1.0
                        && own_linetype.is_continuous()
                        && !context.force_chord_conics
                        && geometric;
                    if let Some(stats) = context.conic_stats.as_deref() {
                        stats.record(geometric, eligible);
                    }
                    if eligible {
                        let mut source = fragment.source;
                        source.instance = path.clone();
                        out.fragments.push(DisplayFragment {
                            source,
                            geometry_source: attributes.geometry_source.clone(),
                            precision: Precision::Analytic,
                            alpha: own_alpha,
                            color: own_color,
                            color_unresolved,
                            lineweight: own_lineweight,
                            lineweight_unresolved,
                            linetype: own_linetype.clone(),
                            linetype_unresolved,
                            linetype_scale,
                            draw_order,
                            primitive: DisplayPrimitive::Conic(transformed),
                        });
                    } else {
                        let points = conic.tessellate(chord_tessellation_params(context));
                        emit_polyline(
                            entity,
                            &points,
                            transform,
                            path,
                            &fragment.source,
                            &attributes,
                            own_alpha,
                            own_color,
                            color_unresolved,
                            own_lineweight,
                            lineweight_unresolved,
                            &own_linetype,
                            linetype_unresolved,
                            linetype_scale,
                            draw_order,
                            context.packed_line_segments,
                            global_lt_scale,
                            out,
                        );
                    }
                }
                primitive => {
                    let mut source = fragment.source;
                    source.instance = path.clone();
                    out.fragments.push(DisplayFragment {
                        source,
                        geometry_source: attributes.geometry_source.clone(),
                        precision: precision_for_source(&attributes.geometry_source),
                        alpha: own_alpha,
                        color: own_color,
                        color_unresolved,
                        lineweight: own_lineweight,
                        lineweight_unresolved,
                        linetype: own_linetype.clone(),
                        linetype_unresolved,
                        linetype_scale,
                        draw_order,
                        primitive: primitive.transformed(transform),
                    });
                }
            }
        }
        Ok(())
    }
}

/// Transform a world-space polyline, subdivide it into dashes and push the
/// resulting line/segment fragments.
///
/// Shared by `Lines` fragments and by the chord expansion of a `Conic`
/// fragment, so the two paths cannot diverge.
#[allow(clippy::too_many_arguments)]
fn emit_polyline(
    entity: &DbEntity,
    points: &[Point3],
    transform: &Transform3,
    path: &InstancePath,
    source: &SelectionRef,
    attributes: &EntityRenderAttributes,
    own_alpha: f32,
    own_color: [f32; 3],
    color_unresolved: bool,
    own_lineweight: f32,
    lineweight_unresolved: bool,
    own_linetype: &LinetypePattern,
    linetype_unresolved: bool,
    linetype_scale: f32,
    // Inherited display order (see `expand_entity`): the INSERT's order for a
    // block's children, the entity's own at the root.
    draw_order: i64,
    packed_configured: bool,
    global_lt_scale: f64,
    out: &mut DisplayRepresentation,
) {
    // Dash subdivision happens on the transformed world-space polyline so arc
    // length is measured in final units.
    let world: Vec<Point3> = points.iter().map(|p| transform.apply_point(*p)).collect();
    let packed = packed_configured && own_alpha == 1.0;
    let (runs, fallback_reason) = if packed {
        let (segments, reason) = if own_linetype.is_continuous() {
            (world.windows(2).flatten().copied().collect(), None)
        } else {
            match cad_geometry::dash_polyline_segments(
                &world,
                &own_linetype.elements,
                linetype_scale as f64 * global_lt_scale,
            ) {
                Ok(segments) => (segments, None),
                Err(issue) => (
                    world.windows(2).flatten().copied().collect(),
                    Some(issue.reason().to_string()),
                ),
            }
        };
        (vec![segments], reason)
    } else {
        subdivide_dashes(&world, own_linetype, linetype_scale as f64, global_lt_scale)
    };
    if let Some(reason) = fallback_reason {
        out.completeness = weaker_completeness(
            &out.completeness,
            &Completeness::Partial(vec![format!(
                "linetype dashes could not be generated: {reason}"
            )]),
        );
        out.diagnostics.push(Diagnostic {
            object: Some(ObjectId(entity.id.0)),
            code: "representation.linetype_fallback".into(),
            message: format!("linetype dashes fell back to continuous: {reason}"),
        });
    }
    let runs = if packed {
        // Preserve all segments (including corners) but avoid duplicating paths
        // and styles once per dash. Bound each packed fragment so downstream GPU
        // buffers stay small.
        let mut packed = Vec::new();
        let mut chunk: Vec<Point3> = Vec::new();
        for run in runs {
            for pair in run.chunks_exact(2) {
                if let Some(origin) = chunk.first() {
                    if pair.iter().any(|p| {
                        [p.x - origin.x, p.y - origin.y, p.z - origin.z]
                            .iter()
                            .any(|v| !v.is_finite() || v.abs() > 8192.0)
                    }) {
                        packed.push(std::mem::take(&mut chunk));
                    }
                }
                chunk.extend_from_slice(pair);
                if chunk.len() == 65_536 {
                    packed.push(std::mem::take(&mut chunk));
                }
            }
        }
        if !chunk.is_empty() {
            packed.push(chunk);
        }
        packed
    } else {
        runs
    };
    for run in runs {
        if run.len() < 2 {
            continue;
        }
        let mut source = source.clone();
        source.instance = path.clone();
        out.fragments.push(DisplayFragment {
            source,
            geometry_source: attributes.geometry_source.clone(),
            precision: precision_for_source(&attributes.geometry_source),
            alpha: own_alpha,
            color: own_color,
            color_unresolved,
            lineweight: own_lineweight,
            lineweight_unresolved,
            linetype: own_linetype.clone(),
            linetype_unresolved,
            linetype_scale,
            draw_order,
            primitive: if packed {
                DisplayPrimitive::LineSegments(Arc::from(run.into_boxed_slice()))
            } else {
                DisplayPrimitive::Lines(Arc::from(run.into_boxed_slice()))
            },
        });
    }
}
