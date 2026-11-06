//! Database-derived scene assembly, independent of the UI/GPU lifecycle.

use std::sync::Arc;

use crate::layers::LayerOverrideSet;
use crate::vendor::{VendorPrimitive, VendorRepresentationStore};
use cad_db::DrawingDatabase;
use cad_db::LinetypePattern;
use cad_domain::{
    CadResult, Completeness, EntityId, GeometrySource, InstancePath, Point3, Precision,
    SelectionRef, TaskStamp, TextAlignH, TextAlignV, TolerancePolicy,
};
use cad_representation::layout::{build_paper_space, enumerate_layouts};
use cad_representation::{
    ConicSegment, DisplayFragment, DisplayPrimitive, FontEngine, LayoutDescriptor,
    ProviderRegistry, RepresentationContext, SpaceSelection, DEFAULT_LINEWEIGHT_MM,
};
use cad_resources::DecodedImageCache;
use cad_scene::{RenderBatch, RenderTopology, SceneBudget, SceneCache, SceneDelta};

pub fn build_scene(database: &DrawingDatabase, stamp: TaskStamp) -> CadResult<SceneDelta> {
    build_scene_with_fonts(database, stamp, None)
}

/// Read-only counts describing how much the scene builder merged.
///
/// `fragments_in` is the display-fragment count the representation stage
/// produced (before merging); `batches_out` is the number of draw items the
/// scene emitted. Both come from the same build that produces the drawn
/// [`SceneDelta`], so the ratio is the real adjacency-merge yield, not an
/// estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SceneAssemblyStats {
    /// Display fragments the representation stage produced, before merging.
    pub fragments_in: usize,
    /// Draw items emitted: render batches plus image draw items (one GPU draw
    /// each).
    pub batches_out: usize,
    /// Adjacent emitted render batches that share every draw-relevant style
    /// attribute (alpha, colour, lineweight, linetype and their unresolved
    /// flags). Counted across the whole flat model-space stream, so it includes
    /// boundaries between entities that the per-entity merge cannot join; it is
    /// an upper bound on the extra adjacency a stronger (cross-representation)
    /// merge predicate could exploit.
    pub adjacent_same_style: usize,
    /// Adjacent style- and topology-class-equal render batches that differ only
    /// by paint order (`draw_order`). Zero until `draw_order` threading lands.
    pub adjacent_differing_by_draw_order: usize,
    /// Adjacent render batches that share style and paint order but are split by
    /// line-vs-mesh topology (one mesh-class, one line-class).
    pub adjacent_differing_by_topology: usize,
    /// Adjacent line batches that satisfy every gate the merge predicate applies
    /// (style, paint order, combined vertex cap, 8,192-unit local extent) yet
    /// stayed separate because they came from different representations — the
    /// realizable draw reduction a representation-spanning line merge would get.
    pub adjacent_mergeable_line_pairs: usize,
}

impl SceneAssemblyStats {
    /// Fragments per emitted draw item. `0.0` when nothing was emitted.
    pub fn merge_ratio(&self) -> f64 {
        if self.batches_out == 0 {
            0.0
        } else {
            self.fragments_in as f64 / self.batches_out as f64
        }
    }
}

/// Build scene batches, shaping text with `fonts` when supplied.
pub fn build_scene_with_fonts(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
) -> CadResult<SceneDelta> {
    build_scene_with_overrides(database, stamp, fonts, &LayerOverrideSet::new())
}

/// Temporary layer visibility changes batches without reparsing the database.
pub fn build_scene_with_overrides(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    overrides: &LayerOverrideSet,
) -> CadResult<SceneDelta> {
    build_scene_with_space(database, stamp, fonts, overrides, SpaceSelection::Model)
}

/// Build model geometry or a layout's paper geometry and clipped viewports.
/// Inspect [`layout_descriptors`] for per-layout support diagnostics.
pub fn build_scene_with_space(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    overrides: &LayerOverrideSet,
    space: SpaceSelection,
) -> CadResult<SceneDelta> {
    build_scene_with_space_and_images(database, stamp, fonts, None, overrides, space)
}

/// Like [`build_scene_with_space`] but also feeds decoded raster images.
///
/// When `images` is `Some`, an `Image` whose key is present in the cache becomes
/// a texture primitive (`DisplayPrimitive::Image`) instead of an outline-only
/// frame; a key absent from the cache still draws the frame plus the provider's
/// honest `image.*` diagnostic. Passing `None` is exactly the previous
/// behaviour, so existing callers are unaffected.
#[allow(clippy::too_many_arguments)]
pub fn build_scene_with_space_and_images(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    images: Option<Arc<DecodedImageCache>>,
    overrides: &LayerOverrideSet,
    space: SpaceSelection,
) -> CadResult<SceneDelta> {
    build_scene_stats(database, stamp, fonts, images, overrides, space).map(|(delta, _stats)| delta)
}

/// Like [`build_scene_with_space_and_images`] but also merges vendor primitives.
#[allow(clippy::too_many_arguments)]
pub fn build_scene_with_space_and_images_and_vendor(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    images: Option<Arc<DecodedImageCache>>,
    overrides: &LayerOverrideSet,
    space: SpaceSelection,
    vendor: Option<&VendorRepresentationStore>,
) -> CadResult<SceneDelta> {
    build_scene_stats_with_vendor(database, stamp, fonts, images, overrides, space, vendor)
        .map(|(delta, _stats)| delta)
}

/// Like [`build_scene_with_space_and_images`] but also returns read-only
/// [`SceneAssemblyStats`] (fragments in vs. draw items out). Purely additive
/// reporting: the returned [`SceneDelta`] is byte-for-byte the same as the
/// reporting-free call.
#[allow(clippy::too_many_arguments)]
pub fn build_scene_stats(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    images: Option<Arc<DecodedImageCache>>,
    overrides: &LayerOverrideSet,
    space: SpaceSelection,
) -> CadResult<(SceneDelta, SceneAssemblyStats)> {
    build_scene_stats_with_vendor(database, stamp, fonts, images, overrides, space, None)
}

/// Like [`build_scene_stats`] but also merges the vendor derived representation.
///
/// The store is display-only: a fragment it contributes still carries the source
/// entity's id, so picking/selection keeps targeting the original `TCH_*` entity.
#[allow(clippy::too_many_arguments)]
pub fn build_scene_stats_with_vendor(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    images: Option<Arc<DecodedImageCache>>,
    overrides: &LayerOverrideSet,
    space: SpaceSelection,
    vendor: Option<&VendorRepresentationStore>,
) -> CadResult<(SceneDelta, SceneAssemblyStats)> {
    let registry = ProviderRegistry::with_default_provider();
    let mut context =
        RepresentationContext::new(stamp.document, TolerancePolicy::default(), stamp.clone())
            .with_packed_line_segments();
    if let Some(fonts) = &fonts {
        context = context.with_fonts(Arc::clone(fonts));
    }
    if let Some(images) = images {
        context = context.with_images(images);
    }
    let mut cache = SceneCache::new(SceneBudget::default());
    let mut combined = SceneDelta {
        stamp: stamp.clone(),
        added: Vec::new(),
        removed_chunks: Vec::new(),
        images: Vec::new(),
        masks: Vec::new(),
    };
    let mut stats = SceneAssemblyStats::default();
    match space {
        SpaceSelection::Model => {
            for entity in crate::layers::visible_model_entities(database, overrides) {
                let representation = model_entity_representation(
                    database,
                    entity,
                    &registry,
                    &context,
                    &stamp,
                    vendor,
                    fonts.as_deref(),
                )?;
                stats.fragments_in += representation.fragments.len();
                let delta = cache.build_compact(&representation, stamp.clone())?;
                stats.batches_out += delta.added.len() + delta.images.len() + delta.masks.len();
                combined.added.extend(delta.added);
                combined.images.extend(delta.images);
                combined.masks.extend(delta.masks);
            }
        }
        SpaceSelection::Paper(layout) => {
            let visible = |entity: &cad_db::DbEntity| overrides.is_entity_visible(database, entity);
            let representation =
                build_paper_space(&registry, database, layout, &context, &visible)?;
            stats.fragments_in += representation.fragments.len();
            let delta = cache.build_compact(&representation, stamp.clone())?;
            stats.batches_out += delta.added.len() + delta.images.len() + delta.masks.len();
            combined.added.extend(delta.added);
            combined.images.extend(delta.images);
            combined.masks.extend(delta.masks);
        }
    }
    tally_adjacent_merge_candidates(&combined.added, &mut stats);
    Ok((combined, stats))
}

#[allow(clippy::too_many_arguments)]
fn model_entity_representation(
    database: &DrawingDatabase,
    entity: &cad_db::DbEntity,
    registry: &ProviderRegistry,
    context: &RepresentationContext,
    stamp: &TaskStamp,
    vendor: Option<&VendorRepresentationStore>,
    fonts: Option<&FontEngine>,
) -> CadResult<cad_representation::DisplayRepresentation> {
    let mut representation = registry.build_expanded(database, entity, context)?;
    append_vendor_fragments(
        &mut representation,
        database,
        registry,
        context,
        entity,
        stamp.document,
        vendor,
        fonts,
    );
    let block_vendor = BlockVendorContext {
        database,
        registry,
        context,
        document: stamp.document,
        vendor,
        fonts,
    };
    let placement = InstancePlacement {
        transform: cad_domain::Transform3::identity(),
        path: InstancePath::default(),
    };
    append_block_vendor_fragments(
        &mut representation,
        &block_vendor,
        &entity.geometry,
        entity.id,
        &placement,
        0,
        &mut Vec::new(),
    );
    Ok(representation)
}

/// Use precisely the full-build provider/style/compaction path for one entity.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_model_entity_scene(
    database: &DrawingDatabase,
    entity: &cad_db::DbEntity,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    images: Option<Arc<DecodedImageCache>>,
    overrides: &LayerOverrideSet,
    vendor: Option<&VendorRepresentationStore>,
) -> CadResult<SceneDelta> {
    let mut cache = SceneCache::new(SceneBudget::default());
    if !overrides.is_entity_visible(database, entity) {
        return Ok(SceneDelta {
            stamp,
            added: vec![],
            removed_chunks: vec![],
            images: vec![],
            masks: vec![],
        });
    }
    let registry = ProviderRegistry::with_default_provider();
    let mut context =
        RepresentationContext::new(stamp.document, TolerancePolicy::default(), stamp.clone())
            .with_packed_line_segments();
    if let Some(fonts) = &fonts {
        context = context.with_fonts(fonts.clone());
    }
    if let Some(images) = images {
        context = context.with_images(images);
    }
    let representation = model_entity_representation(
        database,
        entity,
        &registry,
        &context,
        &stamp,
        vendor,
        fonts.as_deref(),
    )?;
    cache.build_compact(&representation, stamp)
}

/// Read-only merge-ratio measurement for the model-space scene of `database`.
///
/// Builds the model-space batches with a throwaway cache (no fonts, no layer
/// overrides, no decoded images) — the same compaction path the host open draws
/// — and returns the fragment/batch counters. It never touches live scene state
/// and does not change what the host draws.
pub fn model_scene_merge_stats(
    database: &DrawingDatabase,
    stamp: TaskStamp,
) -> CadResult<SceneAssemblyStats> {
    let (_delta, stats) = build_scene_stats(
        database,
        stamp,
        None,
        None,
        &LayerOverrideSet::new(),
        SpaceSelection::Model,
    )?;
    Ok(stats)
}

/// Append the derived fragments for one entity's source handle.
///
/// The fragments use the entity's own id and draw order, so selection, layer
/// visibility and paint order keep pointing at the authoritative `DbEntity`.
#[allow(clippy::too_many_arguments)]
fn append_vendor_fragments(
    representation: &mut cad_representation::DisplayRepresentation,
    database: &DrawingDatabase,
    registry: &ProviderRegistry,
    context: &RepresentationContext,
    entity: &cad_db::DbEntity,
    document: cad_domain::DocumentId,
    vendor: Option<&VendorRepresentationStore>,
    fonts: Option<&cad_representation::FontEngine>,
) {
    let Some(store) = vendor else {
        return;
    };
    let Some(handle) = entity.object.source_handle.as_deref() else {
        return;
    };
    let Some(primitives) = store.for_source_handle(handle) else {
        return;
    };
    let source = SelectionRef {
        document,
        entity: entity.id,
        instance: InstancePath::default(),
        sub_element: None,
    };
    // Derived fragments inherit the source entity's layer appearance. The
    // entity's own explicit style is not copied here; a TCH entity is opaque and
    // the reference representation (Tianzheng's own display) uses the layer
    // appearance for these bodies.
    let layer = database.layer_attributes(entity.layer);
    let color = [
        layer.color.rgb[0] as f32 / 255.0,
        layer.color.rgb[1] as f32 / 255.0,
        layer.color.rgb[2] as f32 / 255.0,
    ];
    let (lineweight, lineweight_unresolved) = match layer.lineweight {
        cad_db::LayerLineWeight::Hundredths(value) => (value as f32 / 100.0, false),
        cad_db::LayerLineWeight::Default | cad_db::LayerLineWeight::ByLayer => {
            (DEFAULT_LINEWEIGHT_MM, true)
        }
    };
    let mut vendor_issue: Option<String> = None;
    for primitive in primitives {
        if let VendorPrimitive::Text(text) = primitive {
            let Some(engine) = fonts else {
                vendor_issue = Some("vendor text has no font engine".to_string());
                continue;
            };
            let (h_align, v_align) = text_align(text.attachment);
            let shaped = engine.shape_unstyled(
                &text.text,
                text.origin,
                text.height,
                text.rotation,
                h_align,
                v_align,
                text.wrap_width,
            );
            match shaped {
                Ok(shaped) => {
                    for mesh in shaped.meshes {
                        representation.fragments.push(DisplayFragment {
                            source: source.clone(),
                            geometry_source: GeometrySource::ProxyCache,
                            precision: Precision::Approximate { error_bound: None },
                            alpha: layer.opacity,
                            color,
                            color_unresolved: false,
                            lineweight,
                            lineweight_unresolved,
                            linetype: LinetypePattern::continuous(),
                            linetype_unresolved: true,
                            linetype_scale: 1.0,
                            draw_order: entity.draw_order,
                            primitive: DisplayPrimitive::MeshWithoutEdgeOverlay(Arc::new(mesh)),
                        });
                    }
                    for polyline in shaped.polylines {
                        if polyline.len() >= 2 {
                            representation.fragments.push(DisplayFragment {
                                source: source.clone(),
                                geometry_source: GeometrySource::ProxyCache,
                                precision: Precision::Approximate { error_bound: None },
                                alpha: layer.opacity,
                                color,
                                color_unresolved: false,
                                lineweight,
                                lineweight_unresolved,
                                linetype: LinetypePattern::continuous(),
                                linetype_unresolved: true,
                                linetype_scale: 1.0,
                                draw_order: entity.draw_order,
                                primitive: DisplayPrimitive::Lines(Arc::from(
                                    polyline.into_boxed_slice(),
                                )),
                            });
                        }
                    }
                    if !shaped.has_renderable_content {
                        vendor_issue = Some("vendor text has no renderable glyphs".to_string());
                    }
                }
                Err(error) => {
                    vendor_issue = Some(format!("vendor text shape: {error}"));
                }
            }
            continue;
        }
        if let VendorPrimitive::Block {
            block,
            origin,
            scale,
            rotation,
        } = primitive
        {
            let Some(block_id) = database.block_by_source_handle(*block) else {
                vendor_issue = Some(format!(
                    "vendor block handle {:X} is not an imported block",
                    block
                ));
                continue;
            };
            let base = database.block_base_point(block_id).unwrap_or_default();
            let (sin, cos) = rotation.sin_cos();
            // Rotation and per-axis scale in one linear map, then the block's
            // insertion base point is removed before the insertion point is
            // applied — the same convention an imported INSERT uses.
            let transform = cad_domain::Transform3 {
                matrix: [
                    [cos * scale.x, -sin * scale.y, 0.0, origin.x],
                    [sin * scale.x, cos * scale.y, 0.0, origin.y],
                    [0.0, 0.0, scale.z, origin.z],
                    [0.0, 0.0, 0.0, 1.0],
                ],
            }
            .matrix_mul(&cad_domain::Transform3::translation(Point3 {
                x: -base.x,
                y: -base.y,
                z: -base.z,
            }));
            // A throwaway INSERT row: it is never written to `cad-db`, it only
            // lets the existing block expansion produce the child geometry.
            let insert = cad_db::DbEntity {
                object: cad_db::DbObject {
                    id: entity.object.id,
                    type_key: "INSERT".to_string(),
                    revision: entity.object.revision,
                    source_handle: entity.object.source_handle.clone(),
                },
                id: entity.id,
                layer: entity.layer,
                space: entity.space.clone(),
                geometry: cad_domain::SemanticGeometry::Insert {
                    block: block_id,
                    transform,
                },
                draw_order: entity.draw_order,
            };
            match registry.build_expanded(database, &insert, context) {
                Ok(expanded) => {
                    if matches!(expanded.completeness, Completeness::Missing(_)) {
                        vendor_issue =
                            Some("vendor block reference resolves to no geometry".to_string());
                    }
                    for mut fragment in expanded.fragments {
                        // The derived display belongs to the TCH source entity,
                        // not to a real INSERT row, so the instance path stays
                        // empty and picking never resolves a non-existent chain.
                        fragment.source = source.clone();
                        fragment.geometry_source = GeometrySource::ProxyCache;
                        fragment.precision = Precision::Approximate { error_bound: None };
                        representation.fragments.push(fragment);
                    }
                }
                Err(error) => {
                    vendor_issue = Some(format!("vendor block expansion: {error}"));
                }
            }
            continue;
        }
        let primitive = match primitive {
            VendorPrimitive::Lines(points) => {
                DisplayPrimitive::Lines(Arc::from(points.clone().into_boxed_slice()))
            }
            VendorPrimitive::Segments(points) => {
                DisplayPrimitive::LineSegments(Arc::from(points.clone().into_boxed_slice()))
            }
            VendorPrimitive::Arc {
                center,
                radius,
                start,
                sweep,
            } => DisplayPrimitive::Conic(Arc::new(ConicSegment {
                center: *center,
                major: Point3 {
                    x: *radius,
                    y: 0.0,
                    z: 0.0,
                },
                minor: Point3 {
                    x: 0.0,
                    y: *radius,
                    z: 0.0,
                },
                start: *start,
                sweep: *sweep,
                full: false,
            })),
            VendorPrimitive::Circle { center, radius } => {
                DisplayPrimitive::Conic(Arc::new(ConicSegment {
                    center: *center,
                    major: Point3 {
                        x: *radius,
                        y: 0.0,
                        z: 0.0,
                    },
                    minor: Point3 {
                        x: 0.0,
                        y: *radius,
                        z: 0.0,
                    },
                    start: 0.0,
                    sweep: std::f64::consts::TAU,
                    full: true,
                }))
            }
            VendorPrimitive::Text(_) => unreachable!("text handled above"),
            VendorPrimitive::Block { .. } => unreachable!("block handled above"),
        };
        representation.fragments.push(DisplayFragment {
            source: source.clone(),
            geometry_source: GeometrySource::ProxyCache,
            precision: Precision::Approximate { error_bound: None },
            alpha: layer.opacity,
            color,
            color_unresolved: false,
            lineweight,
            lineweight_unresolved,
            linetype: LinetypePattern::continuous(),
            linetype_unresolved: true,
            linetype_scale: 1.0,
            draw_order: entity.draw_order,
            primitive,
        });
    }
    if !primitives.is_empty() {
        // The semantic entity is still opaque; only its display is derived. Keep
        // the honest `Partial` note rather than claiming a complete semantic
        // representation.
        let note = vendor_issue.unwrap_or_else(|| "vendor derived display (display-only)".into());
        representation.completeness = Completeness::Partial(vec![note]);
    }
}

/// Shared inputs for the block-member vendor walk.
struct BlockVendorContext<'a> {
    database: &'a DrawingDatabase,
    registry: &'a ProviderRegistry,
    context: &'a RepresentationContext,
    document: cad_domain::DocumentId,
    vendor: Option<&'a VendorRepresentationStore>,
    fonts: Option<&'a cad_representation::FontEngine>,
}

/// One composed placement while walking nested INSERTs.
struct InstancePlacement {
    transform: cad_domain::Transform3,
    path: InstancePath,
}

/// Append derived fragments for vendor entities that live **inside** blocks.
///
/// `build_expanded` already recurses through block geometry, but the vendor
/// merge only runs for the top-level entity. This walk mirrors the INSERT
/// expansion: every member entity with a vendor source handle contributes its
/// derived fragments, transformed by the composed instance transform and
/// sourced to that member with the real instance path. Nested INSERTs recurse
/// with the same depth/cycle bounds as the representation provider. Paper-space
/// expansion (`build_paper_space`) does not call this yet; model space is the
/// path the TCH fixture uses.
#[allow(clippy::too_many_arguments)]
fn append_block_vendor_fragments(
    out: &mut cad_representation::DisplayRepresentation,
    vendor: &BlockVendorContext<'_>,
    geometry: &cad_domain::SemanticGeometry,
    owner: EntityId,
    placement: &InstancePlacement,
    depth: usize,
    stack: &mut Vec<cad_domain::BlockId>,
) {
    match geometry {
        cad_domain::SemanticGeometry::Insert { block, transform } => {
            if depth >= cad_representation::MAX_INSTANCE_DEPTH || stack.contains(block) {
                return;
            }
            let Some(definition) = vendor.database.block(*block) else {
                return;
            };
            let composed = placement.transform.matrix_mul(transform);
            let mut child = InstancePlacement {
                transform: composed,
                path: placement.path.clone(),
            };
            child.path.0.push(owner);
            stack.push(*block);
            for member_id in definition.entities.clone() {
                let Some(member) = vendor.database.entity(member_id) else {
                    continue;
                };
                if let Some(handle) = member.object.source_handle.as_deref() {
                    if vendor
                        .vendor
                        .is_some_and(|store| store.for_source_handle(handle).is_some())
                    {
                        let mut local = cad_representation::DisplayRepresentation {
                            fragments: Vec::new(),
                            completeness: Completeness::Complete,
                            diagnostics: Vec::new(),
                        };
                        append_vendor_fragments(
                            &mut local,
                            vendor.database,
                            vendor.registry,
                            vendor.context,
                            member,
                            vendor.document,
                            vendor.vendor,
                            vendor.fonts,
                        );
                        for mut fragment in local.fragments {
                            fragment.source = SelectionRef {
                                document: vendor.document,
                                entity: member.id,
                                instance: child.path.clone(),
                                sub_element: None,
                            };
                            fragment.primitive = fragment.primitive.transformed(&child.transform);
                            fragment.geometry_source = GeometrySource::ProxyCache;
                            fragment.precision = Precision::Approximate { error_bound: None };
                            out.fragments.push(fragment);
                        }
                    }
                }
                append_block_vendor_fragments(
                    out,
                    vendor,
                    &member.geometry,
                    member.id,
                    &child,
                    depth + 1,
                    stack,
                );
            }
            stack.pop();
        }
        cad_domain::SemanticGeometry::Compound(children) => {
            for child in children {
                append_block_vendor_fragments(out, vendor, child, owner, placement, depth, stack);
            }
        }
        _ => {}
    }
}

/// Map MTEXT attachment numbering (1..9) to the text engine's alignment pair.
fn text_align(attachment: u8) -> (TextAlignH, TextAlignV) {
    let h = match attachment {
        1 | 4 | 7 => TextAlignH::Left,
        2 | 5 | 8 => TextAlignH::Center,
        _ => TextAlignH::Right,
    };
    let v = match attachment {
        1..=3 => TextAlignV::Top,
        4..=6 => TextAlignV::Middle,
        _ => TextAlignV::Baseline,
    };
    (h, v)
}

/// Whether two render batches share every draw-relevant style attribute, so an
/// order- and topology-agnostic merge predicate could in principle join them.
fn same_render_style(a: &RenderBatch, b: &RenderBatch) -> bool {
    a.alpha == b.alpha
        && a.color == b.color
        && a.color_unresolved == b.color_unresolved
        && a.lineweight == b.lineweight
        && a.lineweight_unresolved == b.lineweight_unresolved
        && a.linetype == b.linetype
        && a.linetype_unresolved == b.linetype_unresolved
}

/// Whether the successor line batch `b` could be absorbed into the adjacent line
/// batch `a` by a representation-spanning merge: the combined vertex count stays
/// under the builder's 65,536 cap and every `b` vertex lies within the builder's
/// 8,192-unit local extent of `a`'s origin.
///
/// This is exactly the gate the within-representation predicate applies to a
/// successor fragment, so [`SceneAssemblyStats::adjacent_mergeable_line_pairs`]
/// is a realizable reduction rather than a loose style-only bound.
fn line_merge_extent_ok(a: &RenderBatch, b: &RenderBatch) -> bool {
    a.topology == RenderTopology::Lines
        && b.topology == RenderTopology::Lines
        // The predicate only merges opaque line batches (a transparent batch
        // keeps its own centroid-based depth key).
        && a.alpha == 1.0
        && a.vertices.len().saturating_add(b.vertices.len()) <= 65_536
        && b.vertices.iter().all(|v| {
            [
                b.local_origin.x + v[0] as f64 - a.local_origin.x,
                b.local_origin.y + v[1] as f64 - a.local_origin.y,
                b.local_origin.z + v[2] as f64 - a.local_origin.z,
            ]
            .iter()
            .all(|d| d.is_finite() && d.abs() <= 8192.0)
        })
}

/// Tally the adjacency hints in [`SceneAssemblyStats`] over adjacent emitted
/// batches. Each mergeable pair a representation-spanning merge joins removes
/// exactly one draw item, so the mergeable count is the draw reduction.
fn tally_adjacent_merge_candidates(batches: &[RenderBatch], stats: &mut SceneAssemblyStats) {
    for pair in batches.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if !same_render_style(a, b) {
            continue;
        }
        stats.adjacent_same_style += 1;
        if a.draw_order == b.draw_order {
            if a.topology != b.topology {
                stats.adjacent_differing_by_topology += 1;
            } else if line_merge_extent_ok(a, b) {
                stats.adjacent_mergeable_line_pairs += 1;
            }
        } else if a.topology == b.topology {
            stats.adjacent_differing_by_draw_order += 1;
        }
    }
}

/// Database layouts with drawability verdicts; model space is always available.
pub fn layout_descriptors(database: &DrawingDatabase) -> Vec<LayoutDescriptor> {
    enumerate_layouts(database)
}
