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
use crate::layers::LayerOverrideSet;
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
/// A pick is a screen-space gesture: the policy expresses the pick/selection
/// radius in logical pixels ([`TolerancePolicy::pickbox_logical_pixels`], the
/// AutoCAD `PICKBOX` analogue) and the projection converts it to world units.
/// The snap/candidate aperture ([`TolerancePolicy::interaction_logical_pixels`])
/// is deliberately **not** used here: pick and snap are distinct quantities.
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
    let tolerance = (policy.pickbox_logical_pixels * world_per_px)
        .max(policy.computation_world)
        .max(f64::MIN_POSITIVE);
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(CadError::InvalidInput(
            "pick tolerance is not finite and positive".into(),
        ));
    }
    Ok(tolerance)
}

/// Whether a ref is drawn under the temporary layer overrides.
///
/// This is the **exact** rule `select_all`/`select_box` apply: every step of the
/// ref's instance path and its target entity must be visible under
/// [`LayerOverrideSet::is_entity_visible`]. It is reused here — not duplicated as
/// a second predicate — so a screen/ray pick targets the same set the rendered
/// base contains. A ref with an unresolved step is refused (never silently
/// included): an entity that cannot be resolved is not visible.
pub fn ref_is_visible(
    database: &DrawingDatabase,
    overrides: &LayerOverrideSet,
    reference: &SelectionRef,
) -> bool {
    reference
        .instance
        .0
        .iter()
        .chain(std::iter::once(&reference.entity))
        .all(|id| {
            database
                .entity(*id)
                .is_some_and(|entity| overrides.is_entity_visible(database, entity))
        })
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
                "YACR_PROFILE_PICK pick_index_build items={} bounded={} unbounded={} chunks={} rstar_chunks={} oversized={} stored={} elapsed_ms={:.3}",
                items.len(),
                broad.entry_count(),
                unbounded.len(),
                broad.chunk_count(),
                broad.rstar_chunks(),
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
    pick_ray_filtered(database, document, ray, options, None)
}

/// The layer-visibility-aware counterpart of [`pick_ray`].
///
/// Identical candidate set and ranking to [`pick_ray`], except that every
/// candidate hidden under `overrides` (see [`ref_is_visible`]) is dropped
/// **before** the precise stage, so the next visible candidate wins exactly as
/// it would in the rendered base. Without this the screen/ray pick could target
/// an entity on a temporarily hidden layer — a ref that is not resident in the
/// scene — and its `FLAG_HOVERED`/`FLAG_SELECTED` would surface as
/// `flags.unresolved` instead of a real highlight.
pub fn pick_ray_with_visibility(
    database: &DrawingDatabase,
    document: DocumentId,
    ray: &Ray3,
    options: &PickOptions,
    overrides: &LayerOverrideSet,
) -> CadResult<PickReport> {
    pick_ray_filtered(database, document, ray, options, Some(overrides))
}

fn pick_ray_filtered(
    database: &DrawingDatabase,
    document: DocumentId,
    ray: &Ray3,
    options: &PickOptions,
    overrides: Option<&LayerOverrideSet>,
) -> CadResult<PickReport> {
    let index = cached_pick_index(database, document);
    pick_ray_with_index_filtered(
        database,
        document,
        ray,
        options,
        index.as_deref(),
        overrides,
    )
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
    pick_ray_exhaustive_filtered(database, document, ray, options, None)
}

fn pick_ray_exhaustive_filtered(
    database: &DrawingDatabase,
    document: DocumentId,
    ray: &Ray3,
    options: &PickOptions,
    overrides: Option<&LayerOverrideSet>,
) -> CadResult<PickReport> {
    let profile = pick_profile_enabled();
    let started = profile.then(Instant::now);
    let items = borrowed_pick_items(database, document);
    // Drop the hidden candidates before the precise stage (the same predicate
    // the indexed path applies), so the exhaustive fallback and the index agree
    // on the visible set. Without overrides the list is untouched.
    let items: Vec<_> = match overrides {
        Some(overrides) => items
            .into_iter()
            .filter(|(source, _, _)| ref_is_visible(database, overrides, source))
            .collect(),
        None => items,
    };
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
    pick_ray_with_index_filtered(database, document, ray, options, index, None)
}

fn pick_ray_with_index_filtered(
    database: &DrawingDatabase,
    document: DocumentId,
    ray: &Ray3,
    options: &PickOptions,
    index: Option<&PickIndex>,
    overrides: Option<&LayerOverrideSet>,
) -> CadResult<PickReport> {
    let Some(index) = index else {
        return pick_ray_exhaustive_filtered(database, document, ray, options, overrides);
    };
    let profile = pick_profile_enabled();
    let started = profile.then(Instant::now);
    let mut candidates = filter_by_index(index, ray, options.tolerance)?;
    // The index is a conservative AABB broad phase and is keyed by scene
    // identity, which a session-only layer override does not change; the
    // visibility rule therefore has to be applied here, at query time.
    if let Some(overrides) = overrides {
        candidates.retain(|source| ref_is_visible(database, overrides, source));
    }
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
    pick_at_screen_filtered(
        database,
        document,
        camera,
        logical,
        logical_size,
        policy,
        back_faces,
        None,
    )
}

/// The layer-visibility-aware counterpart of [`pick_at_screen`].
///
/// The screen→ray→pick funnel of [`pick_at_screen`], with the temporary layer
/// overrides applied to the candidate set (see [`pick_ray_with_visibility`]).
/// This is the entry point a host uses so hover and click can never target an
/// entity that a temporarily hidden layer keeps out of the rendered base.
#[allow(clippy::too_many_arguments)]
pub fn pick_at_screen_with_visibility(
    database: &DrawingDatabase,
    document: DocumentId,
    camera: &Camera,
    logical: [f64; 2],
    logical_size: [f64; 2],
    policy: &TolerancePolicy,
    back_faces: BackFacePolicy,
    overrides: &LayerOverrideSet,
) -> CadResult<PickReport> {
    pick_at_screen_filtered(
        database,
        document,
        camera,
        logical,
        logical_size,
        policy,
        back_faces,
        Some(overrides),
    )
}

#[allow(clippy::too_many_arguments)]
fn pick_at_screen_filtered(
    database: &DrawingDatabase,
    document: DocumentId,
    camera: &Camera,
    logical: [f64; 2],
    logical_size: [f64; 2],
    policy: &TolerancePolicy,
    back_faces: BackFacePolicy,
    overrides: Option<&LayerOverrideSet>,
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
    let result = pick_ray_filtered(database, document, &ray, &options, overrides);
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
mod tests;
