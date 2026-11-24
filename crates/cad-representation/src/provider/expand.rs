use crate::layout::{clip_conic, clip_polyline, WorldRectBoundary};
use crate::*;

use cad_db::LayerScope;

use super::clip::{
    block_clip_boundary, block_clip_depth, rejection_code, rejection_message, ClipDepth,
};
use super::clip_emit::emit_clipped;
use super::default::attribute_value_fragments;
use super::default::chord_tessellation_params;
use super::depth::{
    conic_depth_span, depth_straddle_message, filter_runs_by_depth, primitive_depth_kind,
    primitive_depth_span,
};
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
        // Where this entity sits in AutoCAD's layer-`0` chain: the drawing's
        // layer-`0` id (resolved once per expansion) plus the containing
        // INSERT's *effective* layer. A member that is itself on layer `0`
        // resolves its `ByLayer` styles against the INSERT's layer; the chain
        // continues through nested blocks because the recursion descends with
        // `child_scope`.
        layers: LayerScope,
        parent_color: Option<[f32; 3]>,
        parent_lineweight: Option<f32>,
        // The containing INSERT's resolved pattern **and** its resolved linetype
        // scale: a `ByBlock` child inherits both.
        parent_linetype: Option<(&LinetypePattern, f32)>,
        global_lt_scale: f64,
        // The world-space clip region inherited from a containing INSERT's
        // `XCLIP` boundary (`None` when nothing clips this branch). A block
        // member is clipped by its own INSERT's region, intersected with every
        // ancestor's, exactly like nested viewport cuts. The region is a
        // rectangle or a (possibly concave) simple polygon — several disjoint
        // polygons when two boundaries intersect — so every primitive is cut
        // piece by piece and the union of the pieces is exactly
        // `primitive ∩ clip`.
        inherited_clip: Option<&WorldRectBoundary>,
        // The world z band inherited from a containing INSERT's `XCLIP` **clip
        // depth** (front/back distances). `None` means the branch is not depth
        // clipped, which is what every clip without those distances keeps.
        inherited_depth: Option<ClipDepth>,
        stack: &mut Vec<BlockId>,
        out: &mut DisplayRepresentation,
    ) -> CadResult<()> {
        // Opacity, colour, lineweight and linetype **provenance** is resolved
        // **here**, because `ByLayer` means "the owning layer's value" and only
        // this layer has the layer table: a layer edit therefore changes its
        // `ByLayer` entities on the next expansion with no entity write and no
        // revision bump. `ByBlock` inherits the containing INSERT's resolved
        // value, threaded down through the `parent_*` arguments; at the model
        // root those fall back to the documented defaults.
        let attributes = database.entity_render_attributes(entity.id);
        // The provider sees the attributes too, so annotative text can be
        // scaled while it is shaped (the geometry is not post-processed).
        // Resolution rule for `ByLayer`: the row of the layer the entity is
        // **effectively** on. That is its own layer (`DbEntity::layer`), except
        // for a block member that is itself on layer `0`: AutoCAD draws such a
        // member on the containing INSERT's layer, and the rule chains through
        // nested blocks (`LayerScope` carries the INSERT's *effective* layer, so
        // one recursion level resolves one level of the chain). All four
        // `ByLayer` styles follow this one resolution, so colour, opacity,
        // lineweight and linetype can never disagree.
        //
        // `ByBlock` is a **different** rule and still walks up the parent chain
        // to the INSERT's already-resolved value; the layer swap does not touch
        // it, so a layer-`0` member cannot inherit twice.
        //
        // A layer with no stored attribute row uses
        // `DrawingDatabase::layer_attributes`'s documented
        // `LayerAttributes::default()` fallback (ACI 7 white, `Continuous`,
        // ByLayer weight, opaque) rather than a second, locally invented
        // default.
        let effective_layer = layers.effective_layer(entity.layer);
        let layer = database.layer_attributes(effective_layer);
        let own_alpha = resolve_alpha(attributes.transparency, parent_alpha, layer.opacity);
        let (own_color, color_unresolved) =
            resolve_color(attributes.color, parent_color, layer.color.rgb);
        let (own_lineweight, lineweight_unresolved) =
            resolve_lineweight(attributes.lineweight, parent_lineweight, layer.lineweight);
        // A layer stores a linetype **name**; its pattern comes from the
        // drawing's linetype table (`None` when the name is not defined there,
        // which stays unresolved rather than becoming invented dashes).
        let layer_linetype = layer_linetype_pattern(database, &layer);
        // The entity's own scale (group `48`) is stored for symbolic linetypes
        // too, so `ByLayer`/`ByBlock` keep the source's factor instead of
        // rendering at the identity.
        let (own_linetype, linetype_unresolved, linetype_scale) = resolve_linetype(
            attributes.linetype.clone(),
            attributes.linetype_scale,
            parent_linetype,
            layer_linetype.as_ref(),
        );
        let representation = self.build_with_attributes(entity, &attributes, context)?;
        out.completeness = weaker_completeness(&out.completeness, &representation.completeness);
        out.diagnostics.extend(representation.diagnostics);
        for fragment in representation.fragments {
            // The database orders a space by the INSERT entity, so a block's
            // children take the INSERT's order rather than their own (which is
            // only meaningful inside the block definition) or a synthetic 0.
            let child_draw_order = fragment.draw_order;
            // A provider may resolve a **primitive's own** colour/lineweight (a
            // table cell's appearance is the case this build has): it then emits
            // a concrete value with the matching `*_unresolved` flag clear
            // instead of the `DEFAULT_*` placeholder. That value wins over the
            // entity-level `ByLayer`/`ByBlock` provenance resolved above, and an
            // unresolved (placeholder) one is replaced exactly as before, so
            // every entity that carries no per-primitive appearance is
            // unchanged.
            let (fragment_color, fragment_color_unresolved) = if fragment.color_unresolved {
                (own_color, color_unresolved)
            } else {
                (fragment.color, false)
            };
            let (fragment_lineweight, fragment_lineweight_unresolved) =
                if fragment.lineweight_unresolved {
                    (own_lineweight, lineweight_unresolved)
                } else {
                    (fragment.lineweight, false)
                };
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
                    // The clip is stored on the INSERT in **block-local** space,
                    // so it is mapped through the same composed transform as the
                    // children. A nested clip intersects with the inherited one;
                    // a boundary this build cannot render exactly is reported and
                    // leaves the branch clipped by its ancestors only.
                    let mut empty_clip = false;
                    // The depth band is resolved on its own: it is a z range, not
                    // a region, so a boundary whose depth cannot be mapped keeps
                    // its 2D cut and names the depth loss, while a boundary that
                    // has no renderable region is not depth-clipped either (the
                    // whole boundary was refused, reported below).
                    let mut child_depth = inherited_depth;
                    if let Some(stored) = attributes.clip.as_ref() {
                        if stored.visible {
                            match block_clip_depth(stored, &composed) {
                                Ok(Some(own)) => {
                                    child_depth = match inherited_depth {
                                        None => Some(own),
                                        Some(outer) => match outer.intersect(own) {
                                            Some(band) => Some(band),
                                            // Disjoint bands: nothing of this
                                            // instance can be inside both, so
                                            // the children are omitted exactly.
                                            None => {
                                                empty_clip = true;
                                                None
                                            }
                                        },
                                    };
                                }
                                Ok(None) => {}
                                Err(_) => report_xclip(
                                    out,
                                    entity,
                                    "representation.xclip_depth_unsupported",
                                    "the clip's front/back depth distances cannot be mapped to a \
                                     world z band (the reference transform is singular or not a \
                                     finite plan map), so the reference is clipped in XY only",
                                ),
                            }
                        }
                    }
                    let child_clip = match attributes.clip.as_ref() {
                        None => inherited_clip.cloned(),
                        Some(stored) => match block_clip_boundary(stored, &composed) {
                            Ok(Some(region)) => match inherited_clip {
                                None => Some(region),
                                // An empty intersection means nothing of this
                                // instance can be inside both boundaries: the
                                // children are omitted exactly (like a fully
                                // outside line), never drawn unclipped. A
                                // rectangle ∩ rectangle stays a rectangle; any
                                // other pair becomes the disjoint polygon
                                // pieces of the real intersection.
                                Some(outer) => match outer.intersect(&region) {
                                    Some(intersection) => Some(intersection),
                                    None => {
                                        empty_clip = true;
                                        None
                                    }
                                },
                            },
                            Ok(None) => inherited_clip.cloned(),
                            Err(rejection) => {
                                report_xclip(
                                    out,
                                    entity,
                                    rejection_code(rejection),
                                    rejection_message(rejection),
                                );
                                inherited_clip.cloned()
                            }
                        },
                    };
                    if !empty_clip {
                        let mut child_path = path.clone();
                        child_path.0.push(entity.id);
                        // The children inherit this INSERT's **effective** layer,
                        // so a layer-`0` INSERT passes the outer block's layer one
                        // level further down (the chained rule).
                        let child_layers = layers.child_scope(effective_layer);
                        // ADR-0055 M2: the reference's own attribute values draw
                        // as text in the same block-local frame as the members,
                        // under this reference's transform, in definition order.
                        // A block-definition `ATTDEF` is skipped by the child loop
                        // below; it is a template, not an extra drawable.
                        if let SemanticGeometry::Insert { attributes, .. } = &entity.geometry {
                            let definitions: Vec<(&str, u32)> = database
                                .block_entities(block)
                                .into_iter()
                                .filter_map(|member| match &member.geometry {
                                    SemanticGeometry::AttributeDefinition(definition) => {
                                        Some((definition.tag.as_str(), definition.order))
                                    }
                                    _ => None,
                                })
                                .collect();
                            let mut visible: Vec<&AttributeValue> = attributes
                                .iter()
                                .filter(|value| context.attdisp.shows(value.flags))
                                .collect();
                            // Stable: values with the same definition keep their
                            // stored relative order, so duplicate tags stay legal.
                            visible.sort_by_key(|value| {
                                definitions
                                    .iter()
                                    .find(|(tag, _)| *tag == value.tag.as_str())
                                    .map(|(_, order)| *order)
                                    .unwrap_or(u32::MAX)
                            });
                            for value in visible {
                                emit_attribute_value(
                                    database,
                                    entity,
                                    value,
                                    context,
                                    &composed,
                                    path,
                                    child_layers,
                                    own_alpha,
                                    Some(own_color),
                                    Some(own_lineweight),
                                    &own_linetype,
                                    linetype_scale,
                                    draw_order,
                                    child_clip.as_ref(),
                                    child_depth,
                                    out,
                                );
                            }
                        }
                        stack.push(block);
                        // The owning INSERT of the fragment is the entity whose
                        // `attributes` were read above; every child of this loop
                        // belongs to the same instance path.
                        for child in database.block_entities(block) {
                            if matches!(child.geometry, SemanticGeometry::AttributeDefinition(_)) {
                                continue;
                            }
                            self.expand_entity(
                                database,
                                child,
                                context,
                                &composed,
                                &child_path,
                                depth + 1,
                                own_alpha,
                                child_draw_order,
                                child_layers,
                                Some(own_color),
                                Some(own_lineweight),
                                Some((&own_linetype, linetype_scale)),
                                global_lt_scale,
                                child_clip.as_ref(),
                                child_depth,
                                stack,
                                out,
                            )?;
                        }
                        stack.pop();
                    }
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
                        fragment_color,
                        fragment_color_unresolved,
                        fragment_lineweight,
                        fragment_lineweight_unresolved,
                        &own_linetype,
                        linetype_unresolved,
                        linetype_scale,
                        draw_order,
                        inherited_clip,
                        inherited_depth,
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
                    // The depth band is a z filter on the transformed geometry, so
                    // it is decided once here and both draw paths below obey it.
                    if let Some(band) = inherited_depth {
                        match conic_depth_span(&transformed) {
                            Some((min, max)) if band.excludes(min, max) => continue,
                            Some((min, max)) if !band.contains(min, max) => report_xclip(
                                out,
                                entity,
                                "representation.xclip_depth_straddle",
                                &depth_straddle_message(band),
                            ),
                            Some(_) => {}
                            None => report_xclip(
                                out,
                                entity,
                                "representation.xclip_depth_unsupported",
                                "the conic's z extent is not finite, so the clip depth band is not \
                                 applied to it and it is kept",
                            ),
                        }
                    }
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
                        match inherited_clip {
                            None => out.fragments.push(DisplayFragment {
                                source,
                                geometry_source: attributes.geometry_source.clone(),
                                precision: Precision::Analytic,
                                alpha: own_alpha,
                                color: fragment_color,
                                color_unresolved: fragment_color_unresolved,
                                lineweight: fragment_lineweight,
                                lineweight_unresolved: fragment_lineweight_unresolved,
                                linetype: own_linetype.clone(),
                                linetype_unresolved,
                                linetype_scale,
                                draw_order,
                                primitive: DisplayPrimitive::Conic(transformed),
                            }),
                            Some(region) => {
                                // Analytic slicing: the visible parameter
                                // intervals stay circles/arcs, never chords. A
                                // multi-piece region (a nested intersection)
                                // yields several slices, one per piece.
                                match clip_conic_region(&transformed, region) {
                                    Ok(pieces) => {
                                        for piece in pieces {
                                            out.fragments.push(DisplayFragment {
                                                source: source.clone(),
                                                geometry_source: attributes.geometry_source.clone(),
                                                precision: Precision::Analytic,
                                                alpha: own_alpha,
                                                color: fragment_color,
                                                color_unresolved: fragment_color_unresolved,
                                                lineweight: fragment_lineweight,
                                                lineweight_unresolved:
                                                    fragment_lineweight_unresolved,
                                                linetype: own_linetype.clone(),
                                                linetype_unresolved,
                                                linetype_scale,
                                                draw_order,
                                                primitive: DisplayPrimitive::Conic(Arc::new(piece)),
                                            });
                                        }
                                    }
                                    Err(_) => {
                                        report_xclip(
                                            out,
                                            entity,
                                            "representation.xclip_conic_unclippable",
                                            "the conic parameters are not finite, so the analytic \
                                             conic is kept unclipped",
                                        );
                                        out.fragments.push(DisplayFragment {
                                            source,
                                            geometry_source: attributes.geometry_source.clone(),
                                            precision: Precision::Analytic,
                                            alpha: own_alpha,
                                            color: fragment_color,
                                            color_unresolved: fragment_color_unresolved,
                                            lineweight: fragment_lineweight,
                                            lineweight_unresolved: fragment_lineweight_unresolved,
                                            linetype: own_linetype.clone(),
                                            linetype_unresolved,
                                            linetype_scale,
                                            draw_order,
                                            primitive: DisplayPrimitive::Conic(transformed),
                                        });
                                    }
                                }
                            }
                        }
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
                            fragment_color,
                            fragment_color_unresolved,
                            fragment_lineweight,
                            fragment_lineweight_unresolved,
                            &own_linetype,
                            linetype_unresolved,
                            linetype_scale,
                            draw_order,
                            inherited_clip,
                            inherited_depth,
                            context.packed_line_segments,
                            global_lt_scale,
                            out,
                        );
                    }
                }
                primitive => {
                    // A wide polyline stroke is a filled triangle band, so the
                    // arc-length dash subdivision the `Lines` path performs does
                    // not apply. It is drawn solid and reported here instead of
                    // silently dropping the pattern.
                    if matches!(
                        &primitive,
                        DisplayPrimitive::Mesh(_) | DisplayPrimitive::MeshWithStoredEdges { .. }
                    ) && matches!(
                        &entity.geometry,
                        SemanticGeometry::Polyline { width, .. }
                            if width.is_finite() && *width > 0.0
                    ) && !own_linetype.is_continuous()
                    {
                        out.completeness = weaker_completeness(
                            &out.completeness,
                            &Completeness::Partial(vec![
                                "a wide polyline stroke is drawn solid: dash subdivision of a \
                                 filled band is not implemented"
                                    .into(),
                            ]),
                        );
                        out.diagnostics.push(Diagnostic {
                            object: Some(ObjectId(entity.id.0)),
                            code: "representation.polyline_width_dashes".into(),
                            message: "the wide polyline stroke ignores its dash pattern; it is \
                                      drawn solid"
                                .into(),
                        });
                    }
                    // Every primitive is transformed first, so the clip
                    // rectangle (already in world space) and the geometry share
                    // one coordinate system.
                    let primitive = primitive.transformed(transform);
                    // The depth band is a z filter, applied before the XY region
                    // clip: a primitive wholly outside the band is omitted exactly
                    // (the same answer a fully-outside line already gets), and one
                    // that straddles it is kept whole and reported — this build
                    // filters by z extent, it does not cut a primitive at a depth
                    // plane.
                    if let Some(band) = inherited_depth {
                        match primitive_depth_span(&primitive) {
                            Some((min, max)) if band.excludes(min, max) => continue,
                            Some((min, max)) if !band.contains(min, max) => report_xclip(
                                out,
                                entity,
                                "representation.xclip_depth_straddle",
                                &depth_straddle_message(band),
                            ),
                            Some(_) => {}
                            None => report_xclip(
                                out,
                                entity,
                                "representation.xclip_depth_unsupported",
                                &format!(
                                    "the {} geometry has no finite z extent, so the clip depth \
                                     band is not applied to it and it is kept",
                                    primitive_depth_kind(&primitive)
                                ),
                            ),
                        }
                    }
                    let emit = |primitive: DisplayPrimitive, out: &mut DisplayRepresentation| {
                        let mut source = fragment.source.clone();
                        source.instance = path.clone();
                        // A `Body3d` carries its own derived display cache, and
                        // its precision and producer live on that cache
                        // (`BodyDisplay`), not on the entity-level
                        // `geometry_source` attribute (whose default is
                        // `Analytic`). For this one class the provider-resolved
                        // values therefore win: an analytic body display still
                        // reports `Analytic`, and an approximate one is never
                        // upgraded to it.
                        let body_owns_precision =
                            matches!(&entity.geometry, SemanticGeometry::Body3d(_));
                        let (geometry_source, precision) = if body_owns_precision {
                            (fragment.geometry_source.clone(), fragment.precision.clone())
                        } else {
                            (
                                attributes.geometry_source.clone(),
                                precision_for_source(&attributes.geometry_source),
                            )
                        };
                        out.fragments.push(DisplayFragment {
                            source,
                            geometry_source,
                            precision,
                            alpha: own_alpha,
                            color: fragment_color,
                            color_unresolved: fragment_color_unresolved,
                            lineweight: fragment_lineweight,
                            lineweight_unresolved: fragment_lineweight_unresolved,
                            linetype: own_linetype.clone(),
                            linetype_unresolved,
                            linetype_scale,
                            draw_order,
                            primitive,
                        });
                    };
                    match inherited_clip {
                        None => emit(primitive, out),
                        Some(region) => {
                            emit_clipped(entity, primitive, region, &emit, out);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Clip one analytic conic against every piece of `region`.
///
/// A rectangle piece goes through `clip_conic_to_rect`, so an axis-aligned
/// block clip produces exactly the parameter slices it always did. `Err` is the
/// shared clipper's refusal (non-finite parameters): the caller keeps the whole
/// conic, never the slices it happened to get.
fn clip_conic_region(
    conic: &ConicSegment,
    region: &WorldRectBoundary,
) -> Result<Vec<ConicSegment>, ClipRefusal> {
    let mut pieces = Vec::new();
    for piece in region.pieces() {
        pieces.extend(clip_conic(conic, piece)?);
    }
    Ok(pieces)
}

/// Record one `XCLIP` degradation: a `Partial` completeness plus a diagnostic.
pub(super) fn report_xclip(
    out: &mut DisplayRepresentation,
    entity: &DbEntity,
    code: &str,
    message: &str,
) {
    out.completeness = weaker_completeness(
        &out.completeness,
        &Completeness::Partial(vec![message.to_string()]),
    );
    out.diagnostics.push(Diagnostic {
        object: Some(ObjectId(entity.id.0)),
        code: code.into(),
        message: message.to_string(),
    });
}

/// The dash pattern a layer's linetype **name** resolves to in this drawing.
///
/// [`cad_db::LayerAttributes::linetype`] is a name (DXF group `6`), not a
/// pattern, so the pattern has to come from the drawing's linetype table —
/// matched case-insensitively, the way the source table names linetypes.
///
/// `None` means the drawing defines no entry under that name (including the
/// empty name): the caller then draws the line continuous **and** flags it
/// unresolved, never a fabricated pattern.
fn layer_linetype_pattern(
    database: &DrawingDatabase,
    layer: &LayerAttributes,
) -> Option<LinetypePattern> {
    let name = layer.linetype.trim();
    if name.is_empty() {
        return None;
    }
    database
        .linetypes()
        .find(|linetype| linetype.name.trim().eq_ignore_ascii_case(name))
        .map(|linetype| linetype.pattern.clone())
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
    // World-space clip region of the containing INSERT's `XCLIP` boundary, if
    // any. The polyline is clipped **after** dash subdivision so a dash crossing
    // the boundary is cut exactly and a fully outside dash disappears; a
    // multi-piece region is clipped piece by piece.
    clip: Option<&WorldRectBoundary>,
    // World-space clip **depth** band of the containing INSERT's `XCLIP`, if any.
    // Applied per run (per dash in packed mode) before the XY cut: a run wholly
    // outside the band disappears, a run crossing it is kept whole and reported.
    depth: Option<ClipDepth>,
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
    let runs = match depth {
        Some(band) => filter_runs_by_depth(entity, runs, band, packed, out),
        None => runs,
    };
    let runs = match clip {
        Some(region) => clip_polyline_runs(runs, region, packed),
        None => runs,
    };
    let runs = if packed {
        // Preserve all segments (including corners) but avoid duplicating paths
        // and styles once per dash. Bound each packed fragment so downstream GPU
        // buffers stay small.
        let mut packed = Vec::new();
        let mut chunk: Vec<Point3> = Vec::new();
        for run in runs {
            for pair in run.as_chunks::<2>().0 {
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

/// Clip every run of a polyline bundle to `region`.
///
/// A packed bundle is a flat list of independent endpoint pairs (`LineSegments`
/// semantics), so each pair is clipped on its own; an unpacked run is one
/// continuous polyline and is clipped as a whole so the surviving pieces stay
/// joined. A multi-piece region (a nested intersection) is clipped piece by
/// piece: the union of the surviving runs is exactly the visible part.
///
/// A rectangle piece goes through `clip_polyline_to_rect` (Liang–Barsky), so a
/// rectangular block clip produces exactly the runs it produced before polygons
/// were supported.
fn clip_polyline_runs(
    runs: Vec<Vec<Point3>>,
    region: &WorldRectBoundary,
    packed: bool,
) -> Vec<Vec<Point3>> {
    let mut out = Vec::new();
    for piece in region.pieces() {
        for run in &runs {
            if packed {
                for pair in run.as_chunks::<2>().0 {
                    out.extend(clip_polyline(pair, piece));
                }
            } else {
                out.extend(clip_polyline(run, piece));
            }
        }
    }
    out
}

/// Draw one `INSERT` attribute value through the shared text path.
///
/// The value is stored in the reference's block-local frame, so it takes the
/// same composed transform, the same block-clip/depth treatment and the same
/// appearance resolution as a block member. The text itself is produced by
/// [`attribute_value_fragments`] (the one text renderer), never by a second
/// shaping path.
#[allow(clippy::too_many_arguments)]
fn emit_attribute_value(
    database: &DrawingDatabase,
    entity: &DbEntity,
    value: &AttributeValue,
    context: &RepresentationContext,
    transform: &Transform3,
    path: &InstancePath,
    layers: LayerScope,
    parent_alpha: f32,
    parent_color: Option<[f32; 3]>,
    parent_lineweight: Option<f32>,
    parent_linetype: &LinetypePattern,
    parent_linetype_scale: f32,
    draw_order: i64,
    clip: Option<&WorldRectBoundary>,
    depth: Option<ClipDepth>,
    out: &mut DisplayRepresentation,
) {
    let layer = database.layer_attributes(layers.effective_layer(value.layer));
    let alpha = resolve_alpha(value.appearance.transparency, parent_alpha, layer.opacity);
    let (color, color_unresolved) =
        resolve_color(value.appearance.color, parent_color, layer.color.rgb);
    let (lineweight, lineweight_unresolved) = resolve_lineweight(
        value.appearance.lineweight,
        parent_lineweight,
        layer.lineweight,
    );
    let layer_linetype = layer_linetype_pattern(database, &layer);
    let (linetype, linetype_unresolved, linetype_scale) = resolve_linetype(
        value.appearance.linetype.clone(),
        value.appearance.linetype_scale,
        Some((parent_linetype, parent_linetype_scale)),
        layer_linetype.as_ref(),
    );

    let representation = attribute_value_fragments(entity, context, value);
    out.completeness = weaker_completeness(&out.completeness, &representation.completeness);
    out.diagnostics.extend(representation.diagnostics);
    for fragment in representation.fragments {
        // Text primitives (unshaped) and glyph outlines (shaped) both transform
        // the same way every other primitive does.
        let primitive = fragment.primitive.transformed(transform);
        if let Some(band) = depth {
            match primitive_depth_span(&primitive) {
                Some((min, max)) if band.excludes(min, max) => continue,
                Some((min, max)) if !band.contains(min, max) => report_xclip(
                    out,
                    entity,
                    "representation.xclip_depth_straddle",
                    &depth_straddle_message(band),
                ),
                Some(_) => {}
                None => report_xclip(
                    out,
                    entity,
                    "representation.xclip_depth_unsupported",
                    &format!(
                        "the attribute text has no finite z extent, so the clip depth band is not \
                         applied to it and it is kept ({})",
                        primitive_depth_kind(&primitive)
                    ),
                ),
            }
        }
        let mut source = fragment.source.clone();
        source.instance = path.clone();
        let geometry_source = fragment.geometry_source;
        let precision = fragment.precision;
        let emit = |primitive: DisplayPrimitive, out: &mut DisplayRepresentation| {
            out.fragments.push(DisplayFragment {
                source: source.clone(),
                geometry_source: geometry_source.clone(),
                precision: precision.clone(),
                alpha,
                color,
                color_unresolved,
                lineweight,
                lineweight_unresolved,
                linetype: linetype.clone(),
                linetype_unresolved,
                linetype_scale,
                draw_order,
                primitive,
            });
        };
        match clip {
            None => emit(primitive, out),
            Some(region) => emit_clipped(entity, primitive, region, &emit, out),
        }
    }
}
