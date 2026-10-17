//! Screen → world ray → precise pick (spec F14 / F05).
//!
//! This module joins the two halves of picking that already exist elsewhere:
//!
//! * [`camera::Camera::screen_to_ray`] turns a logical screen pixel into a world
//!   [`Ray3`] for both projections (F13);
//! * [`cad_spatial::pick_closest`] runs the precise CPU hit test and returns the
//!   **closest** hit (F14).
//!
//! It adds the part that needs the database: walking model-space entities,
//! expanding `INSERT`s through their block definitions while accumulating the
//! [`InstancePath`], and deriving the world pick tolerance from the
//! [`TolerancePolicy`]. Two insert placements of the same block therefore
//! produce two distinct [`SelectionRef`]s and the hit carries the instance path
//! that separates them (audit F05).
//!
//! It never touches the GPU or a depth buffer: the pick is pure CPU geometry, so
//! it works on the WebGL2 path and in a host with no device (spec §8.5).
//!
//! What is deliberately *not* done here is recorded in `docs/picking-3d.md`:
//! sub-element (mesh-face) selection, GPU depth-test comparison and exact
//! analytic curve/line precision are open items; none of them is faked.

use crate::camera::{Camera, Projection};
use cad_db::{DrawingDatabase, MAX_INSTANCE_DEPTH};
use cad_domain::*;
use cad_spatial::{BackFacePolicy, ChunkGridIndex, PickItem, PickOptions, PickReport};
use std::sync::Arc;
use std::time::Instant;

fn pick_profile_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("YACR_PROFILE_PICK").is_some())
}

/// The process resident set size in kibibytes, read from `/proc` on Linux.
/// `None` elsewhere (or when the file cannot be read). A diagnostic for the
/// pick-index footprint evidence only; it never affects a pick.
#[cfg(target_os = "linux")]
fn process_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|kb| kb.parse().ok())
    })
}

#[cfg(not(target_os = "linux"))]
fn process_rss_kb() -> Option<u64> {
    None
}

/// Whether the pick path must fall back to the exhaustive scan.
///
/// A diagnostic escape hatch (`YACR_PICK_EXHAUSTIVE`) for evidence runs that
/// need the pre-index path, and the branch the fallback contract is tested
/// through. It never silently changes behaviour: the pick still returns the
/// same report, and the profile log states the mode.
fn exhaustive_forced() -> bool {
    static FORCED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| std::env::var_os("YACR_PICK_EXHAUSTIVE").is_some())
}

/// What the last pick's broad phase did. A diagnostic, never behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PickStats {
    /// `true` when the persisted spatial index narrowed the scan; `false` for
    /// the exhaustive fallback.
    pub indexed: bool,
    /// Expanded pick items in the drawing.
    pub items: usize,
    /// Items handed to the precise stage (equal to `items` on the fallback).
    pub candidates: usize,
}

thread_local! {
    static LAST_PICK_STATS: std::cell::Cell<Option<PickStats>> = const { std::cell::Cell::new(None) };
}

/// The broad phase of the last pick on this thread (indexed vs exhaustive, and
/// the item/candidate counts). `None` before any pick has run.
pub fn last_pick_stats() -> Option<PickStats> {
    LAST_PICK_STATS.with(|cell| cell.get())
}

fn record_pick_stats(stats: PickStats) {
    LAST_PICK_STATS.with(|cell| cell.set(Some(stats)));
}

/// Derive the world-space pick tolerance from the tolerance policy.
///
/// A pick is a screen-space gesture: the policy expresses the interaction radius
/// in logical pixels ([`TolerancePolicy::interaction_logical_pixels`]) and the
/// projection converts it to world units.
///
/// * **Orthographic** — the scale is already world units per logical pixel.
/// * **Perspective** — the world size of a pixel grows with depth; a pixel at
///   the camera target distance spans `2·d·tan(fov/2) / height` world units.
///
/// The result is floored at [`TolerancePolicy::computation_world`] so a pick can
/// never be tighter than the numeric predicate tolerance, and is rejected when
/// non-finite (a degenerate viewport or projection).
pub fn pick_tolerance(
    policy: &TolerancePolicy,
    camera: &Camera,
    logical_size: [f64; 2],
) -> CadResult<f64> {
    camera.validate()?;
    if !logical_size[0].is_finite() || logical_size[0] <= 0.0 || !logical_size[1].is_finite() {
        return Err(CadError::InvalidInput(
            "pick viewport size must be finite and positive".into(),
        ));
    }
    let world_per_px = match camera.projection {
        Projection::Orthographic { scale } => scale,
        Projection::Perspective {
            vertical_fov_radians,
        } => {
            let height = logical_size[1].max(1.0);
            let depth = camera.distance().max(1e-9);
            2.0 * depth * (vertical_fov_radians * 0.5).tan() / height
        }
    };
    let tolerance = (policy.interaction_logical_pixels * world_per_px)
        .max(policy.computation_world)
        .max(f64::MIN_POSITIVE);
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(CadError::InvalidInput(
            "pick tolerance is not finite and positive".into(),
        ));
    }
    Ok(tolerance)
}

/// Collect the model-space pick items of a drawing.
///
/// `INSERT`s are expanded through their block definitions: the composed
/// transform is accumulated and the insert chain is recorded as the
/// [`InstancePath`] of every child, so two placements of one block yield two
/// distinct refs. Expansion is bounded by [`MAX_INSTANCE_DEPTH`] and cuts
/// cycles, exactly like [`DrawingDatabase::bounds`]; a truncated or cyclic
/// branch is skipped rather than producing a wrong hit.
pub fn drawing_pick_items(database: &DrawingDatabase, document: DocumentId) -> Vec<PickItem> {
    borrowed_pick_items(database, document)
        .into_iter()
        .map(|(source, geometry, transform)| PickItem {
            source,
            geometry: geometry.clone(),
            transform,
            geometry_source: match geometry {
                SemanticGeometry::Mesh(_) => GeometrySource::DirectMesh,
                _ => GeometrySource::Analytic,
            },
        })
        .collect()
}

fn borrowed_pick_items(
    database: &DrawingDatabase,
    document: DocumentId,
) -> Vec<(SelectionRef, &SemanticGeometry, Transform3)> {
    let mut items = Vec::new();
    let mut stack: Vec<BlockId> = Vec::new();
    for entity in database.model_space() {
        collect_items(
            database,
            entity.id,
            &entity.geometry,
            document,
            &Transform3::identity(),
            &[],
            0,
            &mut stack,
            &mut items,
        );
    }
    items
}

#[allow(clippy::too_many_arguments)]
fn collect_items<'a>(
    database: &'a DrawingDatabase,
    entity: EntityId,
    geometry: &'a SemanticGeometry,
    document: DocumentId,
    transform: &Transform3,
    path: &[EntityId],
    depth: usize,
    stack: &mut Vec<BlockId>,
    out: &mut Vec<(SelectionRef, &'a SemanticGeometry, Transform3)>,
) {
    if let SemanticGeometry::Insert {
        block,
        transform: insert,
    } = geometry
    {
        if depth >= MAX_INSTANCE_DEPTH || stack.contains(block) {
            return;
        }
        let composed = transform.matrix_mul(insert);
        let mut child_path = path.to_vec();
        child_path.push(entity);
        stack.push(*block);
        for child in database.block_entities(*block) {
            collect_items(
                database,
                child.id,
                &child.geometry,
                document,
                &composed,
                &child_path,
                depth + 1,
                stack,
                out,
            );
        }
        stack.pop();
        return;
    }
    out.push((
        SelectionRef {
            document,
            entity,
            instance: InstancePath(path.to_vec()),
            sub_element: None,
        },
        geometry,
        *transform,
    ));
}

/// Resolve one selection ref to its accumulated world transform, walking
/// **only** the ref's own `INSERT` chain instead of scanning the drawing.
///
/// This is the O(|instance path|) counterpart of [`drawing_pick_items`]: the
/// root of the instance path must be a model-space `INSERT`, every further step
/// must be an `INSERT` member of the previous step's block (visibility-filtered
/// exactly like [`DrawingDatabase::block_entities`]), and the target must be a
/// non-`INSERT` entity the full scan would have pushed as an item.
///
/// Returns `None` for every ref the full scan would **not** have produced — a
/// foreign document, a deleted entity, a broken/cyclic/over-deep path, a
/// truncated branch, or an `INSERT` root that only exists as an expansion — so
/// a caller can report the same "unresolved" outcome and never fabricate one.
pub fn resolve_pick_transform(
    database: &DrawingDatabase,
    document: DocumentId,
    reference: &SelectionRef,
) -> Option<Transform3> {
    if reference.document != document {
        return None;
    }
    let target = database.entity(reference.entity)?;
    let mut transform = Transform3::identity();
    let mut scope: Option<BlockId> = None;
    let mut stack: Vec<BlockId> = Vec::new();
    for (depth, insert_id) in reference.instance.0.iter().enumerate() {
        if depth >= MAX_INSTANCE_DEPTH {
            return None;
        }
        let insert = database.entity(*insert_id)?;
        match scope {
            None => {
                // The scan only starts from model space.
                if !matches!(insert.space, SpaceId::Model) {
                    return None;
                }
            }
            Some(block) => {
                // A nested step must be a visible member of the previous block.
                if !database
                    .block_entities(block)
                    .iter()
                    .any(|e| e.id == *insert_id)
                {
                    return None;
                }
            }
        }
        let SemanticGeometry::Insert {
            block,
            transform: local,
        } = &insert.geometry
        else {
            return None;
        };
        if stack.contains(block) {
            return None;
        }
        stack.push(*block);
        transform = transform.matrix_mul(local);
        scope = Some(*block);
    }
    match scope {
        None => {
            if !matches!(target.space, SpaceId::Model) {
                return None;
            }
        }
        Some(block) => {
            if !database
                .block_entities(block)
                .iter()
                .any(|e| e.id == reference.entity)
            {
                return None;
            }
        }
    }
    // An `INSERT` is always expanded, never pushed as an item.
    if matches!(target.geometry, SemanticGeometry::Insert { .. }) {
        return None;
    }
    Some(transform)
}

/// The persisted pick acceleration for one drawing identity.
///
/// It bundles the `INSERT`-expanded item list, a conservative-bounds
/// [`ChunkGridIndex`] (ADR-0012 §7), and the list of items whose bounds are
/// unknown. It is keyed by the drawing's own [`SceneIdentity`] (which embeds
/// the `Revision`), so it shares the single invalidation truth the rest of the
/// app uses (ADR-0012 §5) instead of inventing a second one.
///
/// Items are stored as [`SelectionRef`]s only; their geometry and accumulated
/// transform are resolved from the database for the (few) candidates a query
/// returns, so a large `INSERT` expansion is never deep-copied into the cache.
pub struct PickIndex {
    scene: SceneIdentity,
    document: DocumentId,
    /// Every expanded item's identity, in expansion order (`ordinal -> ref`).
    items: Vec<SelectionRef>,
    /// Bounded items: their ordinal and conservative world AABB.
    broad: ChunkGridIndex,
    /// Ordinals whose conservative bounds are unknown (opaque payloads, a
    /// compound with an unbounded child): always candidates, so their
    /// `skipped` entry is never lost.
    unbounded: Vec<u32>,
}

impl PickIndex {
    /// Expand the drawing and build its broad phase.
    ///
    /// Fails only if the caller hands the index a non-finite box, which
    /// [`cad_spatial::pick_world_bounds`] never produces; a family that cannot
    /// be bounded conservatively is stored as an always-candidate instead.
    pub fn build(database: &DrawingDatabase, document: DocumentId) -> CadResult<Self> {
        let profile = pick_profile_enabled();
        let rss_before = if profile { process_rss_kb() } else { None };
        let started = profile.then(Instant::now);
        let borrowed = borrowed_pick_items(database, document);
        if let Some(started) = started {
            eprintln!(
                "YACR_PROFILE_PICK borrowed_pick_items count={} elapsed_ms={:.3}",
                borrowed.len(),
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        let build_started = profile.then(Instant::now);
        let mut items = Vec::with_capacity(borrowed.len());
        let mut entries: Vec<(u32, Bounds3)> = Vec::with_capacity(borrowed.len());
        let mut unbounded = Vec::new();
        for (ordinal, (source, geometry, transform)) in borrowed.into_iter().enumerate() {
            let ordinal = u32::try_from(ordinal).map_err(|_| {
                CadError::InvalidInput("pick expansion exceeds the index ordinal space".into())
            })?;
            items.push(source);
            match cad_spatial::pick_world_bounds(geometry, &transform) {
                Some((min, max)) => entries.push((ordinal, Bounds3 { min, max })),
                None => unbounded.push(ordinal),
            }
        }
        let broad = ChunkGridIndex::build(&entries)?;
        // The `entries` scratch (payload + AABB pairs) is not part of the
        // retained index; drop it before reporting the footprint so the RSS
        // delta measures the index, not the build scratch.
        drop(entries);
        if let Some(started) = build_started {
            eprintln!(
                "YACR_PROFILE_PICK pick_index_build items={} bounded={} unbounded={} chunks={} oversized={} stored={} elapsed_ms={:.3}",
                items.len(),
                broad.entry_count(),
                unbounded.len(),
                broad.chunk_count(),
                broad.oversized_count(),
                broad.stored_entry_count(),
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        if profile {
            // Process RSS across the build: a true measurement of the retained
            // index footprint. Allocator retention of freed build scratch can
            // only inflate it, so it is an upper bound; the before/after runs
            // use the same method.
            let rss_after = process_rss_kb();
            let delta = match (rss_before, rss_after) {
                (Some(before), Some(after)) => Some(after as i64 - before as i64),
                _ => None,
            };
            eprintln!(
                "YACR_PROFILE_PICK pick_index_footprint rss_before_kb={:?} rss_after_kb={:?} rss_delta_kb={:?}",
                rss_before, rss_after, delta
            );
        }
        Ok(PickIndex {
            scene: database.scene_identity(),
            document,
            items,
            broad,
            unbounded,
        })
    }

    /// The drawing identity this index was built for.
    pub fn scene_identity(&self) -> SceneIdentity {
        self.scene
    }

    /// The document this index's refs name.
    pub fn document(&self) -> DocumentId {
        self.document
    }

    /// Expanded pick items in the drawing.
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Items with a conservative bound in the broad phase.
    pub fn bounded_count(&self) -> usize {
        self.broad.entry_count()
    }

    /// Items kept as always-candidates (no conservative bound).
    pub fn unbounded_count(&self) -> usize {
        self.unbounded.len()
    }

    /// The candidate refs for a pick ray, in expansion order.
    fn ray_candidates(&self, ray: &Ray3, tolerance: f64) -> CadResult<Vec<SelectionRef>> {
        let mut ordinals = self.broad.ray_candidates(ray, tolerance)?;
        ordinals.extend_from_slice(&self.unbounded);
        ordinals.sort_unstable();
        ordinals.dedup();
        Ok(ordinals
            .into_iter()
            .map(|ordinal| self.items[ordinal as usize].clone())
            .collect())
    }
}

thread_local! {
    static PICK_INDEX_CACHE: std::cell::RefCell<Option<Arc<PickIndex>>> =
        const { std::cell::RefCell::new(None) };
}

/// The cached index for `database`/`document`, building it on a miss.
///
/// `None` means the caller must use the exhaustive fallback: the index was
/// explicitly disabled (`YACR_PICK_EXHAUSTIVE`) or could not be built. The
/// reason is always logged, never silently swallowed.
fn cached_pick_index(database: &DrawingDatabase, document: DocumentId) -> Option<Arc<PickIndex>> {
    if exhaustive_forced() {
        return None;
    }
    let scene = database.scene_identity();
    let cached = PICK_INDEX_CACHE.with(|cell| cell.borrow().clone());
    if let Some(index) = cached {
        if index.scene == scene && index.document == document {
            return Some(index);
        }
    }
    match PickIndex::build(database, document) {
        Ok(index) => {
            let index = Arc::new(index);
            PICK_INDEX_CACHE.with(|cell| *cell.borrow_mut() = Some(index.clone()));
            Some(index)
        }
        Err(error) => {
            eprintln!("YACR_PROFILE_PICK pick_index_unavailable reason={error:?}");
            None
        }
    }
}

/// Precise pick of the closest model-space geometry under a world ray.
///
/// Uses the persisted spatial index to narrow the scan to the ray's candidates
/// (ADR-0012 §10 S4); falls back to the exhaustive scan when the index is
/// unavailable. Both paths produce an identical [`PickReport`].
pub fn pick_ray(
    database: &DrawingDatabase,
    document: DocumentId,
    ray: &Ray3,
    options: &PickOptions,
) -> CadResult<PickReport> {
    let index = cached_pick_index(database, document);
    pick_ray_with_index(database, document, ray, options, index.as_deref())
}

/// Precise pick over the exhaustive item scan — the pre-index reference path.
///
/// Kept as the differential-test reference and the fallback body; the
/// [`PickReport`] it returns is the contract the indexed path must match
/// exactly (hit, tie-break, `skipped` list and order).
pub fn pick_ray_exhaustive(
    database: &DrawingDatabase,
    document: DocumentId,
    ray: &Ray3,
    options: &PickOptions,
) -> CadResult<PickReport> {
    let profile = pick_profile_enabled();
    let started = profile.then(Instant::now);
    let items = borrowed_pick_items(database, document);
    let item_count = items.len();
    if let Some(started) = started {
        eprintln!(
            "YACR_PROFILE_PICK borrowed_pick_items count={item_count} elapsed_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    record_pick_stats(PickStats {
        indexed: false,
        items: item_count,
        candidates: item_count,
    });
    let started = profile.then(Instant::now);
    let result = cad_spatial::pick_closest_borrowed(
        ray,
        items
            .iter()
            .map(|(source, geometry, transform)| (source, *geometry, transform)),
        options,
    );
    if let Some(started) = started {
        eprintln!(
            "YACR_PROFILE_PICK pick_closest_borrowed elapsed_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    let started = profile.then(Instant::now);
    drop(items);
    if let Some(started) = started {
        eprintln!(
            "YACR_PROFILE_PICK borrowed_pick_items_drop elapsed_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    result
}

/// The accumulated world transform of an index item.
///
/// Walks the ref's own instance path and composes the inserts' local
/// transforms, exactly like the expansion did. It deliberately skips the
/// membership re-validation [`resolve_pick_transform`] performs: the index was
/// built from the same expansion, so the path is already known good, and the
/// re-validation's `block_entities` allocation per candidate is what made a
/// wide candidate set slow. Returns `None` for a ref whose path no longer
/// resolves (a defensive fallback; such a ref is skipped, matching the
/// exhaustive scan which would not have produced it either).
fn item_transform(database: &DrawingDatabase, reference: &SelectionRef) -> Option<Transform3> {
    let mut transform = Transform3::identity();
    for insert_id in &reference.instance.0 {
        let insert = database.entity(*insert_id)?;
        match &insert.geometry {
            SemanticGeometry::Insert {
                transform: local, ..
            } => {
                transform = transform.matrix_mul(local);
            }
            _ => return None,
        }
    }
    Some(transform)
}

/// Precise pick through an explicit index (or the exhaustive fallback when
/// `index` is `None`). The single funnel both [`pick_ray`] and the fallback
/// contract test go through.
pub fn pick_ray_with_index(
    database: &DrawingDatabase,
    document: DocumentId,
    ray: &Ray3,
    options: &PickOptions,
    index: Option<&PickIndex>,
) -> CadResult<PickReport> {
    let Some(index) = index else {
        return pick_ray_exhaustive(database, document, ray, options);
    };
    let profile = pick_profile_enabled();
    let started = profile.then(Instant::now);
    let candidates = filter_by_index(index, ray, options.tolerance)?;
    let candidate_count = candidates.len();
    if let Some(started) = started {
        eprintln!(
            "YACR_PROFILE_PICK pick_broad_phase mode=indexed items={} candidates={} elapsed_ms={:.3}",
            index.item_count(),
            candidate_count,
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    record_pick_stats(PickStats {
        indexed: true,
        items: index.item_count(),
        candidates: candidate_count,
    });
    // Resolve only the candidates' geometry and accumulated transform from the
    // database; the item's identity (entity + instance path) is preserved
    // exactly, so two insert instances stay separable.
    let mut resolved: Vec<(&SelectionRef, &SemanticGeometry, Transform3)> =
        Vec::with_capacity(candidate_count);
    for source in &candidates {
        let Some(entity) = database.entity(source.entity) else {
            continue;
        };
        let Some(transform) = item_transform(database, source) else {
            continue;
        };
        resolved.push((source, &entity.geometry, transform));
    }
    let started = profile.then(Instant::now);
    let result = cad_spatial::pick_closest_borrowed(
        ray,
        resolved
            .iter()
            .map(|(source, geometry, transform)| (*source, *geometry, transform)),
        options,
    );
    if let Some(started) = started {
        eprintln!(
            "YACR_PROFILE_PICK pick_closest_borrowed elapsed_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    result
}

/// Precise pick of the closest model-space geometry under a logical screen pixel.
///
/// Combines [`Camera::screen_to_ray`] with [`pick_ray`] and a tolerance derived
/// from `policy`. An invalid pixel or viewport (no ray) is [`CadError::InvalidInput`].
pub fn pick_at_screen(
    database: &DrawingDatabase,
    document: DocumentId,
    camera: &Camera,
    logical: [f64; 2],
    logical_size: [f64; 2],
    policy: &TolerancePolicy,
    back_faces: BackFacePolicy,
) -> CadResult<PickReport> {
    let profile = pick_profile_enabled();
    let total_started = profile.then(Instant::now);
    let started = profile.then(Instant::now);
    let ray = camera
        .screen_to_ray(logical, logical_size)
        .ok_or_else(|| CadError::InvalidInput("screen pixel does not produce a pick ray".into()))?;
    let tolerance = pick_tolerance(policy, camera, logical_size)?;
    let options = PickOptions::new(tolerance)?.with_back_faces(back_faces);
    if let Some(started) = started {
        eprintln!(
            "YACR_PROFILE_PICK pick_at_screen_setup elapsed_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    let result = pick_ray(database, document, &ray, &options);
    if let Some(started) = total_started {
        eprintln!(
            "YACR_PROFILE_PICK pick_at_screen_total elapsed_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    result
}

/// Narrow the index's items to the candidates a pick ray can reach.
///
/// The index is a conservative AABB broad phase only; the returned refs keep
/// their exact identity (document + entity + instance path), and their order is
/// the drawing's expansion order, so the precise stage's stable tie handling is
/// unchanged. A candidate's geometry is resolved by the caller.
pub fn filter_by_index(
    index: &PickIndex,
    ray: &Ray3,
    tolerance: f64,
) -> CadResult<Vec<SelectionRef>> {
    index.ray_candidates(ray, tolerance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::SelectionSet;
    use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    fn entity(id: u128, geometry: SemanticGeometry, draw_order: i64) -> DbEntity {
        model_entity(id, geometry, draw_order)
    }

    fn model_entity(id: u128, geometry: SemanticGeometry, draw_order: i64) -> DbEntity {
        space_entity(id, geometry, draw_order, SpaceId::Model)
    }

    fn space_entity(
        id: u128,
        geometry: SemanticGeometry,
        draw_order: i64,
        space: SpaceId,
    ) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbEntity".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer: LayerId(0),
            space,
            geometry,
            draw_order,
        }
    }

    /// A drawing with two INSERT placements of the same block (one line each)
    /// at x = 10 and x = 20, plus a top-level line at x = 30.
    fn drawing() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(100)],
            dynamic_visibility: None,
        })
        .unwrap();
        b.insert_entity(space_entity(
            100,
            SemanticGeometry::Line {
                start: p(0.0, -1.0, 0.0),
                end: p(0.0, 1.0, 0.0),
            },
            0,
            SpaceId::Block(BlockId(0)),
        ))
        .unwrap();
        b.insert_entity(entity(
            10,
            SemanticGeometry::Insert {
                block: BlockId(0),
                transform: Transform3::translation(p(10.0, 0.0, 0.0)),
            },
            1,
        ))
        .unwrap();
        b.insert_entity(entity(
            20,
            SemanticGeometry::Insert {
                block: BlockId(0),
                transform: Transform3::translation(p(20.0, 0.0, 0.0)),
            },
            2,
        ))
        .unwrap();
        b.insert_entity(entity(
            30,
            SemanticGeometry::Line {
                start: p(30.0, -1.0, 0.0),
                end: p(30.0, 1.0, 0.0),
            },
            3,
        ))
        .unwrap();
        b.finish().unwrap()
    }

    fn camera_from(eye: Point3, target: Point3, projection: Projection) -> Camera {
        Camera {
            eye,
            target,
            up: p(0.0, 0.0, 1.0),
            projection,
        }
    }

    #[test]
    fn insert_expansion_carries_distinct_instance_paths() {
        let db = drawing();
        let items = drawing_pick_items(&db, DocumentId(1));
        // Two block children (one per insert) plus the top-level line.
        assert_eq!(items.len(), 3);
        let mut paths: Vec<Vec<EntityId>> =
            items.iter().map(|i| i.source.instance.0.clone()).collect();
        paths.sort();
        assert!(paths.contains(&vec![EntityId(10)]));
        assert!(paths.contains(&vec![EntityId(20)]));
        assert!(paths.contains(&Vec::new()));
    }

    #[test]
    fn two_insert_instances_are_separable_by_the_hit() {
        let db = drawing();
        let options = PickOptions::new(0.05).unwrap();
        // A ray along +x through the first placement hits the block child at
        // x = 10 and reports instance [10].
        let ray = Ray3 {
            origin: p(0.0, 0.0, 0.0),
            direction: p(1.0, 0.0, 0.0),
        };
        let report = pick_ray(&db, DocumentId(1), &ray, &options).unwrap();
        let hit = report.hit.expect("a hit");
        assert_eq!(hit.source.entity, EntityId(100), "the block child entity");
        assert_eq!(hit.source.instance, InstancePath(vec![EntityId(10)]));
        // The other placeholder instance is at x = 20 — deeper than the hit at
        // x = 10, so the closest wins and the distance is honest.
        assert!((hit.distance - 10.0).abs() < 1e-9);
    }

    #[test]
    fn screen_pick_uses_camera_ray_and_returns_closest() {
        let db = drawing();
        // Orthographic top view: a pixel whose world column is x = 20 must hit
        // the second instance of the block child, not the first.
        let camera = Camera {
            eye: p(20.0, 0.0, 1000.0),
            target: p(20.0, 0.0, 0.0),
            up: p(0.0, 1.0, 0.0),
            projection: Projection::Orthographic { scale: 1.0 },
        };
        let size = [800.0, 600.0];
        let centre = [size[0] * 0.5, size[1] * 0.5];
        let report = pick_at_screen(
            &db,
            DocumentId(1),
            &camera,
            centre,
            size,
            &TolerancePolicy::default(),
            BackFacePolicy::Cull,
        )
        .unwrap();
        let hit = report.hit.expect("a hit");
        assert_eq!(hit.source.instance, InstancePath(vec![EntityId(20)]));
        assert_eq!(hit.source.entity, EntityId(100));
    }

    #[test]
    fn screen_pick_misses_empty_space_without_a_wrong_hit() {
        let db = drawing();
        let camera = Camera::top_view_2d();
        let size = [800.0, 600.0];
        // A corner far from every line.
        let report = pick_at_screen(
            &db,
            DocumentId(1),
            &camera,
            [2.0, 2.0],
            size,
            &TolerancePolicy::default(),
            BackFacePolicy::Cull,
        )
        .unwrap();
        assert!(report.hit.is_none());
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn perspective_pick_hits_3d_geometry_along_the_view_ray() {
        // A vertical line at (5, 0, 0..10) is picked from an isometric-ish eye.
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(7));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(model_entity(
            1,
            SemanticGeometry::Line {
                start: p(5.0, 0.0, 0.0),
                end: p(5.0, 0.0, 10.0),
            },
            0,
        ))
        .unwrap();
        let db = b.finish().unwrap();
        let camera = camera_from(
            p(0.0, -20.0, 5.0),
            p(5.0, 0.0, 5.0),
            Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
        );
        let size = [800.0, 600.0];
        // Project a point known to lie on the line, then pick that pixel.
        let on_line = p(5.0, 0.0, 5.0);
        let pixel = camera.world_to_screen(on_line, size).unwrap();
        let report = pick_at_screen(
            &db,
            DocumentId(7),
            &camera,
            pixel,
            size,
            &TolerancePolicy::default(),
            BackFacePolicy::Cull,
        )
        .unwrap();
        let hit = report
            .hit
            .expect("the line is picked through its own pixel");
        assert_eq!(hit.source.entity, EntityId(1));
        assert!(hit.distance > 0.0);
    }

    #[test]
    fn pick_tolerance_scales_with_orthographic_zoom() {
        let policy = TolerancePolicy::default();
        let mut camera = Camera::top_view_2d();
        camera.projection = Projection::Orthographic { scale: 1.0 };
        let t1 = pick_tolerance(&policy, &camera, [800.0, 600.0]).unwrap();
        camera.projection = Projection::Orthographic { scale: 2.0 };
        let t2 = pick_tolerance(&policy, &camera, [800.0, 600.0]).unwrap();
        // Doubling world-units-per-pixel doubles the world tolerance.
        assert!((t2 - 2.0 * t1).abs() < 1e-9);
        assert!((t1 - policy.interaction_logical_pixels).abs() < 1e-9);
    }

    #[test]
    fn pick_tolerance_scales_inversely_with_perspective_height() {
        let policy = TolerancePolicy::default();
        let camera = camera_from(
            p(0.0, 0.0, 100.0),
            p(0.0, 0.0, 0.0),
            Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
        );
        let tall = pick_tolerance(&policy, &camera, [800.0, 600.0]).unwrap();
        let taller = pick_tolerance(&policy, &camera, [800.0, 1200.0]).unwrap();
        // More pixels over the same vertical FOV => narrower world-per-pixel.
        assert!(taller < tall);
        assert!((tall - 2.0 * taller).abs() < 1e-9);
    }

    #[test]
    fn pick_tolerance_never_drops_below_the_predicate_tolerance() {
        // A tiny interaction radius with a big predicate tolerance.
        let policy = TolerancePolicy {
            interaction_logical_pixels: 0.0,
            computation_world: 0.5,
            ..TolerancePolicy::default()
        };
        let camera = Camera::top_view_2d();
        let t = pick_tolerance(&policy, &camera, [800.0, 600.0]).unwrap();
        assert!((t - 0.5).abs() < 1e-12);
    }

    #[test]
    fn degenerate_viewport_has_no_pick_tolerance() {
        let policy = TolerancePolicy::default();
        let camera = Camera::top_view_2d();
        assert!(pick_tolerance(&policy, &camera, [0.0, 600.0]).is_err());
        assert!(pick_tolerance(&policy, &camera, [800.0, f64::NAN]).is_err());
    }

    #[test]
    fn invalid_screen_pixel_is_rejected_not_empty() {
        let db = drawing();
        let camera = Camera::top_view_2d();
        let result = pick_at_screen(
            &db,
            DocumentId(1),
            &camera,
            [f64::NAN, 0.0],
            [800.0, 600.0],
            &TolerancePolicy::default(),
            BackFacePolicy::Cull,
        );
        assert!(matches!(result, Err(CadError::InvalidInput(_))));
    }

    fn face(key: &str) -> SubElementId {
        SubElementId {
            source_key: key.into(),
            topology_revision: Revision(0),
        }
    }

    /// Two coplanar triangles at z = 2 (centres x = -1 and x = 1) with distinct
    /// stable face sources.
    fn two_face_mesh() -> Mesh {
        Mesh {
            vertices: vec![
                p(-2.0, -1.0, 2.0),
                p(-1.0, 1.0, 2.0),
                p(0.0, -1.0, 2.0),
                p(0.0, -1.0, 2.0),
                p(1.0, 1.0, 2.0),
                p(2.0, -1.0, 2.0),
            ],
            triangles: vec![[0, 1, 2], [3, 4, 5]],
            normals: Vec::new(),
            face_sources: vec![Some(face("face-a")), Some(face("face-b"))],
            colors: Vec::new(),
        }
    }

    fn mesh_database() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(9));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(entity(50, SemanticGeometry::Mesh(two_face_mesh()), 0))
            .unwrap();
        b.finish().unwrap()
    }

    #[test]
    fn screen_pick_propagates_the_mesh_face_sub_element() {
        let db = mesh_database();
        let doc = DocumentId(9);
        let options = PickOptions::new(1e-6).unwrap();
        let a = pick_ray(
            &db,
            doc,
            &Ray3 {
                origin: p(-1.0, 0.0, 0.0),
                direction: p(0.0, 0.0, 1.0),
            },
            &options,
        )
        .unwrap()
        .hit
        .expect("face a is hit");
        let b = pick_ray(
            &db,
            doc,
            &Ray3 {
                origin: p(1.0, 0.0, 0.0),
                direction: p(0.0, 0.0, 1.0),
            },
            &options,
        )
        .unwrap()
        .hit
        .expect("face b is hit");
        assert_eq!(a.source.entity, EntityId(50));
        assert_eq!(b.source.entity, EntityId(50));
        assert_eq!(
            a.source.sub_element.as_ref().map(|s| s.source_key.as_str()),
            Some("face-a")
        );
        assert_eq!(
            b.source.sub_element.as_ref().map(|s| s.source_key.as_str()),
            Some("face-b")
        );
        assert_ne!(a.source, b.source, "two faces select differently");
        assert!(a.sub_element_reason.is_none());
    }

    #[test]
    fn mesh_without_a_face_source_is_unresolved_with_a_reason() {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(10));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        let mut mesh = two_face_mesh();
        mesh.face_sources = Vec::new();
        b.insert_entity(entity(51, SemanticGeometry::Mesh(mesh), 0))
            .unwrap();
        let db = b.finish().unwrap();
        let hit = pick_ray(
            &db,
            DocumentId(10),
            &Ray3 {
                origin: p(-1.0, 0.0, 0.0),
                direction: p(0.0, 0.0, 1.0),
            },
            &PickOptions::new(1e-6).unwrap(),
        )
        .unwrap()
        .hit
        .expect("the mesh is still hit");
        // The entity is resolved; only the sub-element is not, and it says why.
        assert_eq!(hit.source.entity, EntityId(51));
        assert!(hit.source.sub_element.is_none());
        assert_eq!(
            hit.sub_element_reason,
            Some(cad_spatial::REASON_FACE_SOURCES_ABSENT)
        );
    }

    /// A block whose only child is a two-face mesh; two INSERT placements of the
    /// same block must still resolve the *same* face id on each instance.
    fn mesh_insert_database() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(11));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(200)],
            dynamic_visibility: None,
        })
        .unwrap();
        b.insert_entity(space_entity(
            200,
            SemanticGeometry::Mesh(two_face_mesh()),
            0,
            SpaceId::Block(BlockId(0)),
        ))
        .unwrap();
        for (id, x) in [(10u128, 10.0), (20, 20.0)] {
            b.insert_entity(entity(
                id,
                SemanticGeometry::Insert {
                    block: BlockId(0),
                    transform: Transform3::translation(p(x, 0.0, 0.0)),
                },
                id as i64,
            ))
            .unwrap();
        }
        b.finish().unwrap()
    }

    #[test]
    fn sub_element_is_stable_across_two_insert_instances() {
        let db = mesh_insert_database();
        let doc = DocumentId(11);
        let options = PickOptions::new(1e-6).unwrap();
        // The block's face A centre is local x = -1; instance 1 places it at
        // world x = 9, instance 2 at world x = 19.
        let first = pick_ray(
            &db,
            doc,
            &Ray3 {
                origin: p(9.0, 0.0, 0.0),
                direction: p(0.0, 0.0, 1.0),
            },
            &options,
        )
        .unwrap()
        .hit
        .expect("first instance face is hit");
        let second = pick_ray(
            &db,
            doc,
            &Ray3 {
                origin: p(19.0, 0.0, 0.0),
                direction: p(0.0, 0.0, 1.0),
            },
            &options,
        )
        .unwrap()
        .hit
        .expect("second instance face is hit");

        assert_eq!(first.source.entity, EntityId(200));
        assert_eq!(first.source.instance, InstancePath(vec![EntityId(10)]));
        assert_eq!(second.source.entity, EntityId(200));
        assert_eq!(second.source.instance, InstancePath(vec![EntityId(20)]));
        // Same stable face id — the instance path is what separates them.
        assert_eq!(first.source.sub_element, second.source.sub_element);
        assert_eq!(
            first
                .source
                .sub_element
                .as_ref()
                .map(|s| s.source_key.as_str()),
            Some("face-a")
        );
        assert_ne!(first.source, second.source);
    }

    #[test]
    fn a_face_selection_differs_from_its_entity_and_from_an_edge() {
        let doc = DocumentId(9);
        let base = SelectionRef {
            document: doc,
            entity: EntityId(50),
            instance: InstancePath::default(),
            sub_element: None,
        };
        let face_ref = SelectionRef {
            sub_element: Some(face("face-a")),
            ..base.clone()
        };
        // A line/edge pick is whole-entity (sub_element stays None).
        let mut set = SelectionSet::new();
        assert!(set.insert(base.clone()));
        assert!(set.insert(face_ref.clone()));
        assert_eq!(set.len(), 2, "entity and one of its faces are distinct");
        assert_ne!(
            SelectionSet::identity_label(&base),
            SelectionSet::identity_label(&face_ref)
        );
    }

    #[test]
    fn filter_by_index_keeps_identity_exact() {
        let db = drawing();
        let items = drawing_pick_items(&db, DocumentId(1));
        let index = PickIndex::build(&db, DocumentId(1)).unwrap();
        assert_eq!(index.item_count(), items.len());
        assert_eq!(index.bounded_count(), items.len());
        assert_eq!(index.unbounded_count(), 0);
        // A ray along +x passes through every item's box (the three lines all
        // straddle y = 0), and the candidates are the exact item identities in
        // expansion order — including the two distinct insert instance paths.
        let ray = Ray3 {
            origin: p(0.0, 0.0, 0.0),
            direction: p(1.0, 0.0, 0.0),
        };
        let filtered = filter_by_index(&index, &ray, 1e-6).unwrap();
        let expected: Vec<SelectionRef> = items.iter().map(|item| item.source.clone()).collect();
        assert_eq!(filtered, expected);
    }

    #[test]
    fn filter_by_index_narrows_a_ray_that_misses_every_item() {
        let db = drawing();
        let index = PickIndex::build(&db, DocumentId(1)).unwrap();
        // A ray along +x far above the drawing (y = 100) reaches nothing: the
        // broad phase must return no candidate rather than the whole drawing.
        let ray = Ray3 {
            origin: p(0.0, 100.0, 0.0),
            direction: p(1.0, 0.0, 0.0),
        };
        assert!(filter_by_index(&index, &ray, 1e-6).unwrap().is_empty());
    }

    // --- ADR-0012 §10 S4: indexed vs exhaustive pick differential ---------

    /// Deterministic xorshift so a failing scene is reproducible.
    struct XorShift(u64);

    impl XorShift {
        fn new(seed: u64) -> Self {
            XorShift(seed | 1)
        }
        fn next_u64(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn f64(&mut self, lo: f64, hi: f64) -> f64 {
            let unit = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
            lo + (hi - lo) * unit
        }
        fn unit(&mut self) -> f64 {
            self.f64(0.0, 1.0)
        }
        fn index(&mut self, len: usize) -> usize {
            (self.next_u64() % len as u64) as usize
        }
    }

    fn rand_point(rng: &mut XorShift, span: f64) -> Point3 {
        p(
            rng.f64(-span, span),
            rng.f64(-span, span),
            rng.f64(-span, span),
        )
    }

    fn rand_mesh(rng: &mut XorShift, span: f64) -> Mesh {
        let base = rand_point(rng, span);
        let vertices = vec![
            base,
            p(base.x + rng.f64(0.5, 3.0), base.y, base.z),
            p(
                base.x,
                base.y + rng.f64(0.5, 3.0),
                base.z + rng.f64(-1.0, 1.0),
            ),
        ];
        // Half the meshes carry a stable face source (sub-element addressable),
        // half do not (the hit then reports an unresolved sub-element reason).
        let face_sources = if rng.unit() < 0.5 {
            vec![Some(SubElementId {
                source_key: "face-0".into(),
                topology_revision: Revision(0),
            })]
        } else {
            Vec::new()
        };
        Mesh {
            vertices,
            triangles: vec![[0, 1, 2]],
            normals: Vec::new(),
            face_sources,
            colors: Vec::new(),
        }
    }

    fn rand_polyline(rng: &mut XorShift, span: f64) -> SemanticGeometry {
        let n = 2 + rng.index(5);
        let points: Vec<Point3> = (0..n).map(|_| rand_point(rng, span)).collect();
        let bulges: Vec<f64> = (0..n)
            .map(|_| {
                if rng.unit() < 0.4 {
                    rng.f64(-1.0, 1.0)
                } else {
                    0.0
                }
            })
            .collect();
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed: rng.unit() < 0.3,
        }
    }

    fn rand_spline(rng: &mut XorShift, span: f64) -> SemanticGeometry {
        let n = 3 + rng.index(3);
        let control_points: Vec<Point3> = (0..n).map(|_| rand_point(rng, span)).collect();
        let degree = 2u32;
        let mut knots: Vec<f64> = (0..(n + degree as usize + 1)).map(|i| i as f64).collect();
        // Occasionally malformed weights/knots: the tessellator then falls back
        // to the control points, which must still be bounded conservatively.
        if rng.unit() < 0.3 {
            let k = rng.index(n);
            knots[k] = f64::NAN;
        }
        let weights: Vec<f64> = (0..n)
            .map(|_| {
                if rng.unit() < 0.25 {
                    0.0
                } else {
                    rng.f64(0.2, 3.0)
                }
            })
            .collect();
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        }
    }

    fn rand_geometry(rng: &mut XorShift, span: f64) -> SemanticGeometry {
        match rng.index(15) {
            0 => SemanticGeometry::Line {
                start: rand_point(rng, span),
                end: rand_point(rng, span),
            },
            1 => rand_polyline(rng, span),
            2 => SemanticGeometry::Circle {
                center: rand_point(rng, span),
                normal: p(0.0, 0.0, 1.0),
                radius: rng.f64(0.1, span * 0.3),
            },
            3 => SemanticGeometry::Arc {
                center: rand_point(rng, span),
                normal: p(0.0, 0.0, 1.0),
                radius: rng.f64(0.1, span * 0.3),
                start: rng.f64(-3.0, 3.0),
                sweep: rng.f64(-3.0, 3.0),
            },
            4 => SemanticGeometry::Ellipse {
                center: rand_point(rng, span),
                normal: p(0.0, 0.0, 1.0),
                major_axis: p(rng.f64(0.1, span * 0.3), 0.0, 0.0),
                ratio: rng.f64(0.1, 1.0),
                start: 0.0,
                sweep: std::f64::consts::TAU,
            },
            5 => rand_spline(rng, span),
            6 => SemanticGeometry::Point(rand_point(rng, span)),
            7 => SemanticGeometry::Mesh(rand_mesh(rng, span)),
            8 => SemanticGeometry::Text {
                text: "abc".into(),
                position: rand_point(rng, span),
                style: StyleId(0),
                height: rng.f64(0.5, 5.0),
                rotation: 0.0,
                font: None,
                h_align: TextAlignH::Left,
                v_align: TextAlignV::Baseline,
            },
            9 => SemanticGeometry::Shape {
                shape_name: "shape".into(),
                code: 1,
                position: rand_point(rng, span),
                size: rng.f64(0.2, 2.0),
                rotation: 0.0,
                font: None,
            },
            10 => SemanticGeometry::Mask {
                boundary: (0..4).map(|_| rand_point(rng, span)).collect(),
                inverted: false,
            },
            11 => SemanticGeometry::Image {
                origin: rand_point(rng, span),
                u: p(rng.f64(0.01, 0.2), 0.0, 0.0),
                v: p(0.0, rng.f64(0.01, 0.2), 0.0),
                pixels: [rng.f64(1.0, 20.0), rng.f64(1.0, 20.0)],
                file: None,
                clip: None,
                visible: true,
            },
            12 => SemanticGeometry::Opaque {
                type_key: "AcDbUnknown".into(),
                version: 1,
                payload: vec![1, 2, 3],
            },
            13 => {
                let children = (0..1 + rng.index(3))
                    .map(|_| rand_geometry(rng, span))
                    .collect();
                SemanticGeometry::Compound(children)
            }
            _ => SemanticGeometry::Line {
                start: rand_point(rng, span),
                end: rand_point(rng, span),
            },
        }
    }

    /// A random model space with two blocks (one nested), INSERT placements and
    /// a spread of every supported geometry family.
    fn random_scene(rng: &mut XorShift, entities: usize, span: f64) -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(0x5104));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(900)],
            dynamic_visibility: None,
        })
        .unwrap();
        b.insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(901), EntityId(902)],
            dynamic_visibility: None,
        })
        .unwrap();
        b.insert_entity(space_entity(
            900,
            rand_geometry(rng, span),
            0,
            SpaceId::Block(BlockId(0)),
        ))
        .unwrap();
        b.insert_entity(space_entity(
            901,
            rand_geometry(rng, span),
            0,
            SpaceId::Block(BlockId(1)),
        ))
        .unwrap();
        // A nested INSERT: block 1 places block 0.
        b.insert_entity(space_entity(
            902,
            SemanticGeometry::Insert {
                block: BlockId(0),
                transform: Transform3::translation(rand_point(rng, span * 0.2)),
            },
            1,
            SpaceId::Block(BlockId(1)),
        ))
        .unwrap();
        for i in 0..entities {
            let id = 1000 + i as u128;
            let geometry = if i % 4 == 0 {
                SemanticGeometry::Insert {
                    block: if i % 8 == 0 { BlockId(1) } else { BlockId(0) },
                    transform: Transform3::translation(rand_point(rng, span)),
                }
            } else {
                rand_geometry(rng, span)
            };
            b.insert_entity(model_entity(id, geometry, i as i64))
                .unwrap();
        }
        b.finish().unwrap()
    }

    fn nondegenerate_ray(rng: &mut XorShift, span: f64) -> Ray3 {
        loop {
            let ray = Ray3 {
                origin: rand_point(rng, span * 1.2),
                direction: rand_point(rng, 1.0),
            };
            let len = ray
                .direction
                .x
                .hypot(ray.direction.y)
                .hypot(ray.direction.z);
            if len > 1e-3 {
                return ray;
            }
        }
    }

    #[test]
    fn indexed_and_exhaustive_picks_agree_over_random_scenes() {
        for seed in 0..16u64 {
            let mut rng = XorShift::new(
                0x00C0_FFEE_0000_u64.wrapping_add(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)),
            );
            let span = [2.0, 50.0, 500.0][(seed % 3) as usize];
            let db = random_scene(&mut rng, 24 + (seed as usize % 5) * 16, span);
            let doc = DocumentId(0x5104);
            let index = PickIndex::build(&db, doc).unwrap();
            for _ in 0..30 {
                let ray = nondegenerate_ray(&mut rng, span);
                let tolerance = [1e-6, 1e-3, 0.5, span * 0.01, span * 0.2][rng.index(5)];
                let options = PickOptions::new(tolerance).unwrap();
                let exhaustive = pick_ray_exhaustive(&db, doc, &ray, &options).unwrap();
                let indexed = pick_ray_with_index(&db, doc, &ray, &options, Some(&index)).unwrap();
                assert_eq!(
                    indexed, exhaustive,
                    "seed={seed} span={span} ray={ray:?} tolerance={tolerance}"
                );
            }
        }
    }

    #[test]
    fn indexed_pick_agrees_at_the_predicate_tolerance_floor() {
        // The smallest legal pick tolerance (the policy's `computation_world`)
        // forces heavy curve tessellation; the index must still agree.
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(0x7003));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(model_entity(
            1,
            SemanticGeometry::Spline {
                degree: 2,
                knots: vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
                control_points: vec![p(0.0, 0.0, 0.0), p(4.0, 6.0, 1.0), p(8.0, -3.0, 2.0)],
                weights: vec![1.0, 1.0, 1.0],
            },
            0,
        ))
        .unwrap();
        b.insert_entity(model_entity(
            2,
            SemanticGeometry::Polyline {
                points: vec![p(1.0, 1.0, 0.0), p(5.0, 1.0, 0.0), p(9.0, 3.0, 0.0)],
                bulges: vec![0.5, -0.25, 0.0],
                closed: false,
            },
            1,
        ))
        .unwrap();
        b.insert_entity(model_entity(
            3,
            SemanticGeometry::Circle {
                center: p(4.0, 4.0, 0.0),
                normal: p(0.0, 0.0, 1.0),
                radius: 3.0,
            },
            2,
        ))
        .unwrap();
        b.insert_entity(model_entity(
            4,
            SemanticGeometry::Mesh(rand_mesh(&mut XorShift::new(5), 4.0)),
            3,
        ))
        .unwrap();
        let db = b.finish().unwrap();
        let doc = DocumentId(0x7003);
        let index = PickIndex::build(&db, doc).unwrap();
        let options = PickOptions::new(TolerancePolicy::default().computation_world).unwrap();
        let mut rng = XorShift::new(0x5A5A_5A5A);
        for _ in 0..40 {
            let ray = nondegenerate_ray(&mut rng, 6.0);
            let exhaustive = pick_ray_exhaustive(&db, doc, &ray, &options).unwrap();
            let indexed = pick_ray_with_index(&db, doc, &ray, &options, Some(&index)).unwrap();
            assert_eq!(indexed, exhaustive, "ray={ray:?}");
        }
    }

    #[test]
    fn indexed_pick_agrees_on_a_ray_that_hits_nothing() {
        let mut rng = XorShift::new(0x1234_5678);
        let db = random_scene(&mut rng, 60, 10.0);
        let doc = DocumentId(0x5104);
        let index = PickIndex::build(&db, doc).unwrap();
        // A vertical ray far above everything: no geometry can be within tol.
        let ray = Ray3 {
            origin: p(1e6, 1e6, 1e6),
            direction: p(0.0, 0.0, -1.0),
        };
        let options = PickOptions::new(1e-6).unwrap();
        let exhaustive = pick_ray_exhaustive(&db, doc, &ray, &options).unwrap();
        let indexed = pick_ray_with_index(&db, doc, &ray, &options, Some(&index)).unwrap();
        assert_eq!(indexed, exhaustive);
        assert!(indexed.hit.is_none());
    }

    #[test]
    fn geometry_exactly_at_the_tolerance_boundary_agrees() {
        // A line along +x at y = 0; a vertical ray at y = tolerance passes at
        // exactly the tolerance, which the precise stage accepts (`dist <= tol`).
        let tolerance = 0.25;
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(0x7001));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(model_entity(
            1,
            SemanticGeometry::Line {
                start: p(0.0, 0.0, 0.0),
                end: p(10.0, 0.0, 0.0),
            },
            0,
        ))
        .unwrap();
        let db = b.finish().unwrap();
        let doc = DocumentId(0x7001);
        let index = PickIndex::build(&db, doc).unwrap();
        let options = PickOptions::new(tolerance).unwrap();
        for (offset, expect_hit) in [(tolerance, true), (tolerance + 1e-6, false)] {
            let ray = Ray3 {
                origin: p(5.0, offset, 100.0),
                direction: p(0.0, 0.0, -1.0),
            };
            let exhaustive = pick_ray_exhaustive(&db, doc, &ray, &options).unwrap();
            let indexed = pick_ray_with_index(&db, doc, &ray, &options, Some(&index)).unwrap();
            assert_eq!(indexed, exhaustive, "offset={offset}");
            assert_eq!(exhaustive.hit.is_some(), expect_hit, "offset={offset}");
        }
    }

    #[test]
    fn candidates_exactly_on_a_chunk_boundary_agree() {
        // Geometry at exact quarter-unit multiples, picked by rays whose origin
        // is exactly on a grid boundary of the built index.
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(0x7002));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        for (i, (x, y)) in [
            (0.0, 0.0),
            (0.25, 0.25),
            (0.5, 0.0),
            (0.75, 0.5),
            (1.0, 1.0),
        ]
        .into_iter()
        .enumerate()
        {
            b.insert_entity(model_entity(
                10 + i as u128,
                SemanticGeometry::Point(p(x, y, 0.0)),
                i as i64,
            ))
            .unwrap();
        }
        let db = b.finish().unwrap();
        let doc = DocumentId(0x7002);
        let index = PickIndex::build(&db, doc).unwrap();
        let options = PickOptions::new(0.3).unwrap();
        for (x, y) in [(0.0, 0.0), (0.25, 0.25), (0.5, 0.5), (0.75, 0.25)] {
            let ray = Ray3 {
                origin: p(x, y, 50.0),
                direction: p(0.0, 0.0, -1.0),
            };
            let exhaustive = pick_ray_exhaustive(&db, doc, &ray, &options).unwrap();
            let indexed = pick_ray_with_index(&db, doc, &ray, &options, Some(&index)).unwrap();
            assert_eq!(indexed, exhaustive, "point=({x},{y})");
        }
    }

    #[test]
    fn the_index_examines_fewer_items_on_a_large_scene() {
        // 100 x 50 = 5000 lines on a regular grid.
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(0x9001));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        let spacing = 1.0;
        let mut id = 1u128;
        for row in 0..50 {
            for col in 0..100 {
                let x = col as f64 * spacing;
                let y = row as f64 * spacing;
                b.insert_entity(model_entity(
                    id,
                    SemanticGeometry::Line {
                        start: p(x, y, 0.0),
                        end: p(x + 0.9, y, 0.0),
                    },
                    id as i64,
                ))
                .unwrap();
                id += 1;
            }
        }
        let db = b.finish().unwrap();
        let doc = DocumentId(0x9001);
        let index = PickIndex::build(&db, doc).unwrap();
        assert_eq!(index.item_count(), 5000);
        let options = PickOptions::new(1e-3).unwrap();
        // A vertical (2D orthographic) ray at one grid line: the broad phase
        // must hand the precise stage a handful of candidates, not all 5000.
        let ray = Ray3 {
            origin: p(3.1, 4.0, 1000.0),
            direction: p(0.0, 0.0, -1.0),
        };
        let indexed = pick_ray_with_index(&db, doc, &ray, &options, Some(&index)).unwrap();
        let stats = last_pick_stats().expect("a pick ran");
        assert!(stats.indexed, "the index path is used");
        assert_eq!(stats.items, 5000);
        assert!(
            stats.candidates < stats.items / 10,
            "candidates={} items={}",
            stats.candidates,
            stats.items
        );
        assert_eq!(
            indexed,
            pick_ray_exhaustive(&db, doc, &ray, &options).unwrap()
        );
    }

    #[test]
    fn the_fallback_scan_is_used_and_correct_without_an_index() {
        let db = drawing();
        let doc = DocumentId(1);
        let options = PickOptions::new(0.05).unwrap();
        let ray = Ray3 {
            origin: p(0.0, 0.0, 0.0),
            direction: p(1.0, 0.0, 0.0),
        };
        let exhaustive = pick_ray_exhaustive(&db, doc, &ray, &options).unwrap();
        let fallback = pick_ray_with_index(&db, doc, &ray, &options, None).unwrap();
        assert_eq!(fallback, exhaustive);
        let stats = last_pick_stats().expect("a pick ran");
        assert!(!stats.indexed, "no index means the exhaustive fallback");
        assert_eq!(stats.candidates, stats.items);
    }

    #[test]
    fn the_cached_indexed_pick_matches_the_exhaustive_reference() {
        let db = drawing();
        let doc = DocumentId(1);
        let options = PickOptions::new(0.05).unwrap();
        let ray = Ray3 {
            origin: p(0.0, 0.0, 0.0),
            direction: p(1.0, 0.0, 0.0),
        };
        let cached = pick_ray(&db, doc, &ray, &options).unwrap();
        let stats = last_pick_stats().expect("a pick ran");
        let exhaustive = pick_ray_exhaustive(&db, doc, &ray, &options).unwrap();
        assert_eq!(cached, exhaustive);
        assert!(stats.indexed);
    }
}
