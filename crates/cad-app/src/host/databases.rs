use super::*;
use std::time::Instant;

/// Synthetic drawing used before a real DWG is opened.
///
/// This is **not** a compatibility claim (the fixture manifest holds only
/// synthetic fixtures and one openly-distributed sample); it exercises the
/// shared UI/render pipeline with known geometry.
pub fn demo_database() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layer(Layer {
            id: LayerId(1),
            name: "WALLS".into(),
            visible: true,
        })
        .unwrap();

    let mut push = |id: u128, geometry: SemanticGeometry, layer: LayerId, order: i64| {
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(id),
                    type_key: class_name(&geometry).into(),
                    revision: Revision(0),
                    source_handle: Some(format!("{id:X}")),
                },
                id: EntityId(id),
                layer,
                space: SpaceId::Model,
                geometry,
                draw_order: order,
            })
            .unwrap();
    };

    // A simple room outline with a circle and a polyline-with-bulge.
    push(
        1,
        SemanticGeometry::Line {
            start: p(0.0, 0.0),
            end: p(4000.0, 0.0),
        },
        LayerId(1),
        0,
    );
    push(
        2,
        SemanticGeometry::Line {
            start: p(4000.0, 0.0),
            end: p(4000.0, 3000.0),
        },
        LayerId(1),
        1,
    );
    push(
        3,
        SemanticGeometry::Line {
            start: p(4000.0, 3000.0),
            end: p(0.0, 3000.0),
        },
        LayerId(1),
        2,
    );
    push(
        4,
        SemanticGeometry::Line {
            start: p(0.0, 3000.0),
            end: p(0.0, 0.0),
        },
        LayerId(1),
        3,
    );
    push(
        5,
        SemanticGeometry::Circle {
            center: p(2000.0, 1500.0),
            normal: pz(0.0, 0.0, 1.0),
            radius: 800.0,
        },
        LayerId(0),
        4,
    );
    push(
        6,
        SemanticGeometry::Polyline {
            points: vec![p(500.0, 500.0), p(1500.0, 500.0), p(1500.0, 1000.0)],
            bulges: vec![0.0, 1.0, 0.0],
            closed: false,
        },
        LayerId(0),
        5,
    );
    builder.finish().unwrap()
}

/// Empty drawing with only the required layer `"0"`.
///
/// This is the document a host installs for "New": no entities, no bounds, and
/// exactly the default layer every DWG must contain. It makes no compatibility
/// or content claim.
pub fn blank_database() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder.finish().unwrap()
}

pub(super) fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn pz(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn class_name(geometry: &SemanticGeometry) -> &'static str {
    crate::app_drawing::class_name(geometry)
}

/// Whether the snap profiler is enabled.
///
/// Gated by the same `YACR_PROFILE_PICK` flag as the pick profiler, so one run
/// produces the measured before/after numbers for both paths. The lines are
/// prefixed `YACR_PROFILE_SNAP` to keep them distinguishable from pick output.
fn snap_profile_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("YACR_PROFILE_PICK").is_some())
}

/// The identity a baked snap-target list was built for.
///
/// It is the same `SceneIdentity` + document pair [`crate::picking::PickIndex`]
/// caches on, so the snap targets share that single invalidation truth instead
/// of keeping a private revision of their own.
#[derive(Clone, Copy, PartialEq)]
struct SnapCacheKey {
    scene: SceneIdentity,
    document: DocumentId,
}

/// The transform-baked target list for one scene identity.
struct SnapTargetCache {
    key: SnapCacheKey,
    targets: Arc<Vec<SnapTarget>>,
}

/// The single candidate resolution of the last pointer move.
///
/// [`HostController::resolve_snap`] stores the snapped point in the tool
/// preview; [`HostController::snap_hints_near_cursor`] then reads that stored
/// point back. Keeping the resolved candidate set here lets the hint overlay
/// reuse the move's candidates instead of walking the drawing a second time,
/// without becoming a second source for the committed point.
struct SnapQueryCache {
    key: SnapQueryKey,
    /// The point the host stored for the query: the first candidate's point, or
    /// the raw cursor when nothing was found.
    stored: [u64; 3],
    candidates: Arc<Vec<SnapCandidate>>,
}

/// The exact inputs of one snap resolution.
#[derive(Clone, PartialEq)]
struct SnapQueryKey {
    scene: SceneIdentity,
    document: DocumentId,
    space: SpaceId,
    cursor: [u64; 3],
    world_per_px_bits: u64,
    /// The aperture the candidates were resolved with: a changed
    /// `view.aperture.sizePixels` must re-resolve, not reuse a candidate set
    /// computed under the previous tolerance.
    aperture_bits: u64,
}

impl SnapQueryKey {
    fn new(
        drawing: &DrawingDatabase,
        document: DocumentId,
        cursor: Point3,
        world_per_px: f64,
        space: &SpaceId,
        policy: &TolerancePolicy,
    ) -> Self {
        SnapQueryKey {
            scene: drawing.scene_identity(),
            document,
            space: space.clone(),
            cursor: point_bits(cursor),
            world_per_px_bits: world_per_px.to_bits(),
            aperture_bits: policy.interaction_logical_pixels.to_bits(),
        }
    }
}

fn point_bits(point: Point3) -> [u64; 3] {
    [point.x.to_bits(), point.y.to_bits(), point.z.to_bits()]
}

thread_local! {
    static SNAP_TARGET_CACHE: std::cell::RefCell<Option<SnapTargetCache>> =
        const { std::cell::RefCell::new(None) };
    static SNAP_QUERY_CACHE: std::cell::RefCell<Option<SnapQueryCache>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    static SNAP_TARGET_BUILD_COUNT: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };
}

/// How many baked snap-target lists this thread has built.
///
/// Test-only: it makes the cache contract (one build for repeated queries on
/// the same scene) directly observable without parsing a log.
#[cfg(test)]
pub(super) fn snap_target_build_count() -> u64 {
    SNAP_TARGET_BUILD_COUNT.with(std::cell::Cell::get)
}

/// Build the local snap targets for a drawing's model-space geometry.
///
/// [`crate::drawing_pick_items`] expands `INSERT`s and returns each item's
/// geometry in its **own local** coordinates plus the accumulated world
/// transform; the snap engine has no transform parameter, so the transform is
/// baked into the geometry with [`transform_geometry`]. An item whose kind this
/// build cannot transform exactly (text, spline, shape, opaque, compound) is
/// **skipped**, never snapped at a wrong location; a top-level entity carries
/// the identity transform and is passed through unchanged (so a model-space
/// spline still snaps to its sampled points).
///
/// Every expanded item is presented in **world model space** (block children
/// are inlined by their composed transform), so all targets are tagged
/// [`SpaceId::Model`]. The precision mirrors the source: a spline's snap points
/// come from a fixed sample set ([`Precision::Approximate`]); analytic kinds are
/// [`Precision::Analytic`].
fn snap_targets_for(drawing: &DrawingDatabase, document: DocumentId) -> Vec<SnapTarget> {
    let identity = Transform3::identity();
    crate::drawing_pick_items(drawing, document)
        .into_iter()
        .filter_map(|item| {
            let geometry = if item.transform == identity {
                item.geometry
            } else {
                match transform_geometry(&item.geometry, &item.transform) {
                    Ok(world) => world,
                    // A kind this build cannot bake exactly contributes nothing
                    // rather than a snap at an un-transformed location.
                    Err(_) => return None,
                }
            };
            let precision = match &geometry {
                SemanticGeometry::Spline { .. } => Precision::Approximate { error_bound: None },
                _ => Precision::Analytic,
            };
            Some(SnapTarget::new(
                geometry,
                item.source,
                SpaceId::Model,
                precision,
            ))
        })
        .collect()
}

/// The cached, transform-baked targets for `drawing`/`document`.
///
/// On a key change the whole list is rebuilt; there is no partial/stale reuse.
/// The rebuild reason is always stated in the profile log (`cold`,
/// `scene_changed` or `document_changed`), so a miss is never mistaken for a
/// cache hit. Construction itself is total (untransformable items are skipped
/// by [`snap_targets_for`]), so the cache has no "unavailable" state to fall
/// back from: it either returns targets built for this exact identity or
/// rebuilds first.
fn cached_snap_targets(drawing: &DrawingDatabase, document: DocumentId) -> Arc<Vec<SnapTarget>> {
    let key = SnapCacheKey {
        scene: drawing.scene_identity(),
        document,
    };
    let profile = snap_profile_enabled();
    let lookup_started = profile.then(Instant::now);
    let cached = SNAP_TARGET_CACHE.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|cached| (cached.key, cached.targets.clone()))
    });
    if let Some((cached_key, targets)) = &cached {
        if *cached_key == key {
            if let Some(started) = lookup_started {
                eprintln!(
                    "YACR_PROFILE_SNAP snap_targets_cache mode=hit targets={} elapsed_ms={:.3}",
                    targets.len(),
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
            return targets.clone();
        }
    }
    let reason = match &cached {
        None => "cold",
        Some((cached_key, _)) if cached_key.document != document => "document_changed",
        Some(_) => "scene_changed",
    };
    let build_started = profile.then(Instant::now);
    let targets = Arc::new(snap_targets_for(drawing, document));
    #[cfg(test)]
    SNAP_TARGET_BUILD_COUNT.with(|count| count.set(count.get() + 1));
    if let Some(started) = build_started {
        eprintln!(
            "YACR_PROFILE_SNAP snap_targets_build reason={reason} targets={} elapsed_ms={:.3}",
            targets.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    SNAP_TARGET_CACHE.with(|cell| {
        *cell.borrow_mut() = Some(SnapTargetCache {
            key,
            targets: targets.clone(),
        })
    });
    targets
}

/// Validate the inputs every snap entry point shares.
fn validate_snap_inputs(cursor: Point3, world_per_px: f64) -> CadResult<()> {
    if !cursor.x.is_finite() || !cursor.y.is_finite() || !cursor.z.is_finite() {
        return Err(CadError::InvalidInput(
            "snap cursor must be finite".to_string(),
        ));
    }
    if !world_per_px.is_finite() || world_per_px <= 0.0 {
        return Err(CadError::InvalidInput(
            "snap world_per_px must be finite and positive".to_string(),
        ));
    }
    Ok(())
}

/// Resolve candidates from an already-built target list.
///
/// `mode` only labels the profile line (`cached`, `uncached`); the geometry and
/// engine inputs are identical on every path, so results cannot differ by mode.
fn resolve_candidates(
    targets: &[SnapTarget],
    cursor: Point3,
    world_per_px: f64,
    space: &SpaceId,
    mode: &'static str,
    policy: &TolerancePolicy,
) -> CadResult<Vec<SnapCandidate>> {
    let plane = WorkPlane {
        origin: cursor,
        u: Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        v: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
    };
    let ray = Ray3 {
        origin: cursor,
        direction: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    };
    let profile = snap_profile_enabled();
    let started = profile.then(Instant::now);
    let result = MeasurementEngine::new(*policy).snap_targets(
        targets,
        &ray,
        Some(&plane),
        world_per_px,
        Some(space.clone()),
    );
    if let Some(started) = started {
        eprintln!(
            "YACR_PROFILE_SNAP snap_candidates_resolve mode={mode} targets={} candidates={} elapsed_ms={:.3}",
            targets.len(),
            result.as_ref().map(|candidates| candidates.len()).unwrap_or(0),
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    result
}

/// Remember the candidate set a pointer move resolved, for the hint overlay.
fn record_snap_query(key: SnapQueryKey, candidates: Arc<Vec<SnapCandidate>>) {
    let stored = candidates
        .first()
        .map(|candidate| point_bits(candidate.point))
        .unwrap_or(key.cursor);
    SNAP_QUERY_CACHE.with(|cell| {
        *cell.borrow_mut() = Some(SnapQueryCache {
            key,
            stored,
            candidates,
        })
    });
}

/// The candidate set of the last move if `key` names the same query.
///
/// The hint overlay reads the *stored* tool cursor back, which is the snapped
/// point; hence a key may match either the raw cursor the move resolved or the
/// stored point it produced.
fn cached_hint_candidates(key: &SnapQueryKey) -> Option<Arc<Vec<SnapCandidate>>> {
    SNAP_QUERY_CACHE.with(|cell| {
        let cached = cell.borrow();
        let cached = cached.as_ref()?;
        let same_cursor = key.cursor == cached.key.cursor || key.cursor == cached.stored;
        (same_cursor
            && key.scene == cached.key.scene
            && key.document == cached.key.document
            && key.space == cached.key.space
            && key.world_per_px_bits == cached.key.world_per_px_bits)
            .then(|| cached.candidates.clone())
    })
}

/// Snap candidates near a world cursor on the model work plane.
///
/// This is the pure bridge between the database and the object-snap overlay:
/// the caller (a host) holds the live cursor and the viewport's
/// `world_per_px`, and gets back the real [`SnapCandidate`]s the measurement
/// snap engine resolves, ready to hand to `CadView::set_snap_hints`.
///
/// **Cursor/ray convention.** A measurement or draw cursor is a world point on
/// the planar work plane, not a screen ray. The pick ray is therefore built as
/// `origin = cursor`, `direction = +Z`, and the snap plane is the world-Z plane
/// through the cursor (`origin = cursor`, `u = +X`, `v = +Y`). With a plane the
/// engine uses the ray∩plane point as its proximity probe, so the probe is
/// exactly `cursor`; candidate points on that plane have ray parameter `t = 0`
/// and are accepted (`accept` only rejects `t < 0`). This mirrors the
/// engine's own test convention (`snap.rs` builds a downward ray onto `z = 0`)
/// but keeps the plane at the cursor's height so a non-zero-z planar drawing is
/// not silently shifted.
///
/// Only geometry exactly on the cursor's plane is in front of the ray; that is
/// the planar 2D contract, stated rather than faked. `space` is the session's
/// active space and is passed as the engine's space filter; because
/// [`crate::drawing_pick_items`] only walks **model** space, a non-model `space`
/// honestly yields no candidates (paper-space snapping is not built here).
///
/// The transform-baked targets are cached per scene identity
/// ([`cached_snap_targets`]), so repeated pointer moves over an unchanged
/// drawing do not re-expand or re-bake it. The resolved set is also recorded as
/// the last move's query, so the hint overlay can reuse it
/// ([`snap_candidates_for_hint`]).
///
/// Returns [`CadError::InvalidInput`] for a non-finite cursor or a
/// non-positive/non-finite `world_per_px`; it never fabricates a candidate.
pub fn snap_candidates_near(
    drawing: &DrawingDatabase,
    document: DocumentId,
    cursor: Point3,
    world_per_px: f64,
    space: SpaceId,
) -> CadResult<Vec<SnapCandidate>> {
    snap_candidates_near_with_policy(
        drawing,
        document,
        cursor,
        world_per_px,
        space,
        &TolerancePolicy::default(),
    )
}

/// [`snap_candidates_near`] with an explicit interaction tolerance policy.
///
/// This is the live path: the host passes the policy derived from its effective
/// configuration ([`TolerancePolicy::interaction_logical_pixels`] is the
/// configured object-snap aperture). The candidate set is cached per query, and
/// the aperture is part of that key, so changing `view.aperture.sizePixels` takes
/// effect for the very next query instead of reusing a stale set.
///
/// Returns [`CadError::InvalidInput`] for a non-finite cursor, a
/// non-positive/non-finite `world_per_px` or a non-positive/non-finite aperture;
/// it never fabricates a candidate.
pub fn snap_candidates_near_with_policy(
    drawing: &DrawingDatabase,
    document: DocumentId,
    cursor: Point3,
    world_per_px: f64,
    space: SpaceId,
    policy: &TolerancePolicy,
) -> CadResult<Vec<SnapCandidate>> {
    validate_snap_inputs(cursor, world_per_px)?;
    let targets = cached_snap_targets(drawing, document);
    let candidates = resolve_candidates(&targets, cursor, world_per_px, &space, "cached", policy)?;
    let key = SnapQueryKey::new(drawing, document, cursor, world_per_px, &space, policy);
    record_snap_query(key, Arc::new(candidates.clone()));
    Ok(candidates)
}

/// Snap candidates over the exhaustive target build — the pre-cache reference
/// path.
///
/// Kept as the differential-test reference for [`snap_candidates_near`]; it
/// must return exactly the same candidates. It deliberately does not touch the
/// target or query caches.
pub fn snap_candidates_near_uncached(
    drawing: &DrawingDatabase,
    document: DocumentId,
    cursor: Point3,
    world_per_px: f64,
    space: SpaceId,
) -> CadResult<Vec<SnapCandidate>> {
    validate_snap_inputs(cursor, world_per_px)?;
    let targets = snap_targets_for(drawing, document);
    resolve_candidates(
        &targets,
        cursor,
        world_per_px,
        &space,
        "uncached",
        &TolerancePolicy::default(),
    )
}

/// Snap candidates for the hint overlay at the tool's stored cursor.
///
/// When the active tool's stored point is the one the last move resolved (the
/// snapped point, or the raw point when nothing snapped), the move's candidate
/// set is returned instead of walking the drawing again. Otherwise the query is
/// resolved normally. Either way the point the tool will commit is untouched:
/// the stored tool point remains the single source for a commit.
///
/// `policy` must be the same policy the move resolved with: the cached query is
/// keyed by the aperture, so a stale set can never be reused after the user
/// changes `view.aperture.sizePixels`.
pub(super) fn snap_candidates_for_hint(
    drawing: &DrawingDatabase,
    document: DocumentId,
    cursor: Point3,
    world_per_px: f64,
    space: SpaceId,
    policy: &TolerancePolicy,
) -> CadResult<Vec<SnapCandidate>> {
    validate_snap_inputs(cursor, world_per_px)?;
    let key = SnapQueryKey::new(drawing, document, cursor, world_per_px, &space, policy);
    let profile = snap_profile_enabled();
    let started = profile.then(Instant::now);
    if let Some(candidates) = cached_hint_candidates(&key) {
        let reused = (*candidates).clone();
        if let Some(started) = started {
            eprintln!(
                "YACR_PROFILE_SNAP snap_candidates_resolve mode=reused candidates={} elapsed_ms={:.3}",
                reused.len(),
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        return Ok(reused);
    }
    snap_candidates_near_with_policy(drawing, document, cursor, world_per_px, space, policy)
}
