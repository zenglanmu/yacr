//! Unit tests.

use super::*;
use cad_representation::DisplayFragment;
use std::sync::Arc;

fn stamp() -> TaskStamp {
    TaskStamp::new(DocumentId(1), 0)
}

fn line_representation(entity: u128, points: Vec<Point3>) -> DisplayRepresentation {
    DisplayRepresentation {
        fragments: vec![DisplayFragment {
            source: SelectionRef {
                document: DocumentId(1),
                entity: EntityId(entity),
                instance: InstancePath::default(),
                sub_element: None,
            },
            geometry_source: GeometrySource::Analytic,
            precision: Precision::Analytic,
            alpha: 1.0,
            color: cad_representation::DEFAULT_RENDER_COLOR,
            color_unresolved: true,
            lineweight: cad_representation::DEFAULT_LINEWEIGHT_MM,
            lineweight_unresolved: true,
            linetype: cad_db::LinetypePattern::continuous(),
            linetype_unresolved: true,
            linetype_scale: 1.0,
            primitive: DisplayPrimitive::Lines(Arc::from(points.into_boxed_slice())),
        }],
        completeness: Completeness::Complete,
        diagnostics: Vec::new(),
    }
}

#[test]
fn build_produces_relative_vertices() {
    let mut cache = SceneCache::default();
    let rep = line_representation(
        1,
        vec![
            Point3 {
                x: 1_000_000.0,
                y: 2_000_000.0,
                z: 0.0,
            },
            Point3 {
                x: 1_000_010.0,
                y: 2_000_000.0,
                z: 0.0,
            },
        ],
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    assert_eq!(delta.added.len(), 1);
    let batch = &delta.added[0];
    assert_eq!(batch.local_origin.x, 1_000_000.0);
    // Relative coordinates stay small and precise.
    assert_eq!(batch.vertices[1][0], 10.0);
}

#[test]
fn publish_rejects_stale_stamp() {
    let mut cache = SceneCache::default();
    let rep = line_representation(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    );
    let delta = cache.build(&rep, TaskStamp::new(DocumentId(1), 5)).unwrap();
    let current = TaskStamp::new(DocumentId(1), 6);
    assert_eq!(cache.publish(delta, &current), Err(CadError::StaleResult));
    assert_eq!(cache.chunk_count(), 0);
}

#[test]
fn change_set_removes_affected_chunks() {
    let mut cache = SceneCache::default();
    let rep = line_representation(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.chunk_count(), 1);
    let changes = ChangeSet {
        database: DatabaseId(1),
        before: Revision(0),
        after: Revision(1),
        transaction: TransactionId(1),
        reason: "edit".into(),
        changes: vec![ObjectChange::Update(
            ObjectId(1),
            cad_db::ChangeMask::GEOMETRY,
        )],
    };
    cache.apply_changes(&changes).unwrap();
    assert_eq!(cache.chunk_count(), 0);
}

#[test]
fn eviction_respects_budget() {
    let mut cache = SceneCache::new(SceneBudget {
        cpu_bytes: 40,
        ..Default::default()
    });
    for i in 0..4 {
        let rep = line_representation(
            i,
            vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 2.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        );
        let delta = cache.build(&rep, stamp()).unwrap();
        cache.publish(delta, &stamp()).unwrap();
    }
    assert!(cache.used_bytes() <= 40);
    assert!(cache.chunk_count() < 4);
}

fn mesh_representation(entity: u128, mesh: Mesh) -> DisplayRepresentation {
    DisplayRepresentation {
        fragments: vec![DisplayFragment {
            source: SelectionRef {
                document: DocumentId(1),
                entity: EntityId(entity),
                instance: InstancePath::default(),
                sub_element: None,
            },
            geometry_source: GeometrySource::DirectMesh,
            precision: Precision::Analytic,
            alpha: 1.0,
            color: cad_representation::DEFAULT_RENDER_COLOR,
            color_unresolved: true,
            lineweight: cad_representation::DEFAULT_LINEWEIGHT_MM,
            lineweight_unresolved: true,
            linetype: cad_db::LinetypePattern::continuous(),
            linetype_unresolved: true,
            linetype_scale: 1.0,
            primitive: DisplayPrimitive::Mesh(std::sync::Arc::new(mesh)),
        }],
        completeness: Completeness::Complete,
        diagnostics: Vec::new(),
    }
}

fn quad() -> Mesh {
    Mesh {
        vertices: vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        normals: vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            };
            4
        ],
        face_sources: Vec::new(),
        colors: Vec::new(),
    }
}

#[test]
fn mesh_batch_is_indexed_and_carries_normals_and_edges() {
    let mut cache = SceneCache::default();
    let delta = cache
        .build(&mesh_representation(1, quad()), stamp())
        .unwrap();
    assert_eq!(delta.added.len(), 1);
    let batch = &delta.added[0];
    assert_eq!(batch.topology, RenderTopology::Mesh);
    // Indexed: one vertex per mesh vertex, not per triangle corner.
    assert_eq!(batch.vertices.len(), 4);
    assert_eq!(batch.normals.len(), 4);
    assert_eq!(batch.indices, vec![[0, 1, 2], [0, 2, 3]]);
    assert_eq!(batch.triangle_count(), 2);
    // Two triangles => six directed edges => twelve edge vertices.
    assert_eq!(batch.edges.len(), 12);
}

#[test]
fn lines_batch_has_no_triangles_or_normals() {
    let mut cache = SceneCache::default();
    let rep = line_representation(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert!(batch.normals.is_empty());
    assert!(batch.indices.is_empty());
    assert_eq!(batch.triangle_count(), 0);
}

#[test]
fn out_of_range_triangle_indices_are_dropped_not_uploaded() {
    let mut mesh = quad();
    mesh.triangles.push([0, 1, 9]); // index 9 does not exist
    let mut cache = SceneCache::default();
    let delta = cache.build(&mesh_representation(1, mesh), stamp()).unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.indices, vec![[0, 1, 2], [0, 2, 3]]);
}

#[test]
fn frame_budget_reports_over_vertex_limit() {
    let budget = FrameBudget {
        max_vertices: 10,
        max_triangles: 10,
        max_bytes: usize::MAX,
    };
    let mut usage = FrameUsage::default();
    budget.charge(&mut usage, 6, 2).unwrap();
    let exceeded = budget.charge(&mut usage, 6, 0).unwrap_err();
    assert_eq!(exceeded.category, "vertices");
    assert_eq!(exceeded.requested, 12);
    assert_eq!(exceeded.limit, 10);
    // The rejected charge did not apply to the running usage.
    assert_eq!(usage.vertices, 6);
}

#[test]
fn frame_budget_reports_over_triangle_limit() {
    let budget = FrameBudget {
        max_vertices: 1000,
        max_triangles: 3,
        max_bytes: usize::MAX,
    };
    let mut usage = FrameUsage::default();
    budget.charge(&mut usage, 3, 2).unwrap();
    let exceeded = budget.charge(&mut usage, 3, 2).unwrap_err();
    assert_eq!(exceeded.category, "triangles");
    assert_eq!(exceeded.requested, 4);
    assert_eq!(exceeded.limit, 3);
    assert_eq!(usage.triangles, 2);
}

#[test]
fn frame_budget_reports_over_byte_limit() {
    let budget = FrameBudget {
        max_vertices: usize::MAX,
        max_triangles: usize::MAX,
        max_bytes: 100,
    };
    let mut usage = FrameUsage::default();
    budget.charge_bytes(&mut usage, 60).unwrap();
    let exceeded = budget.charge_bytes(&mut usage, 60).unwrap_err();
    assert_eq!(exceeded.category, "bytes");
    assert_eq!(exceeded.requested, 120);
    assert_eq!(exceeded.limit, 100);
    // The rejected charge did not apply.
    assert_eq!(usage.bytes, 60);
}

#[test]
fn upload_bytes_of_counts_every_uploaded_buffer() {
    // A mesh batch uploads positions, normals, per-vertex colours, `[u32; 3]`
    // triangle indices and a `u32` edge index per de-duplicated edge.
    let mut cache = SceneCache::default();
    let delta = cache
        .build(&mesh_representation(1, quad()), stamp())
        .unwrap();
    let batch = &delta.added[0];
    // 4 positions + 4 normals = 8 vertex vectors * 12 = 96.
    // 4 per-vertex colours * 12 = 48 (a mesh always binds the colour attribute).
    // 2 triangles * 12 = 24 index bytes.
    // A quad triangulated as [0,1,2],[0,2,3] has 5 distinct undirected edges
    // (the diagonal is shared) => 10 `u32` edge indices = 40 bytes.
    // `edges` (the position list the scene carries) is separate and counted too.
    assert_eq!(batch.vertex_bytes(), (4 + 4 + 12) * 12);
    assert_eq!(edge_index_count(&batch.indices), 10);
    let expected = batch.vertex_bytes() + 4 * 12 + 2 * 12 + 10 * 4;
    assert_eq!(batch.upload_size_bytes(), expected);
}

#[test]
fn upload_bytes_of_line_batch_has_no_index_bytes() {
    let mut cache = SceneCache::default();
    let rep = line_representation(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.upload_size_bytes(), 2 * 12);
}

#[test]
fn task_queue_rejects_over_capacity_without_dropping() {
    let mut queue = TaskQueue::new(2);
    queue.submit().unwrap();
    queue.submit().unwrap();
    let exceeded = queue.submit().unwrap_err();
    assert_eq!(exceeded.category, "queued_tasks");
    assert_eq!(exceeded.requested, 3);
    assert_eq!(exceeded.limit, 2);
    // The rejected submission did not consume a slot.
    assert_eq!(queue.in_flight(), 2);
    // Completing a task frees exactly one slot.
    queue.complete();
    queue.submit().unwrap();
    assert_eq!(queue.in_flight(), 2);
}

#[test]
fn scene_cache_accounts_cpu_bytes_and_evicts_over_budget() {
    // Each 3-point line batch is 36 vertex bytes. A 40-byte budget keeps one.
    let mut cache = SceneCache::new(SceneBudget {
        cpu_bytes: 40,
        ..Default::default()
    });
    let rep = line_representation(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 2.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.total_cpu_bytes(), 36);
    assert_eq!(cache.chunk_count(), 1);
    // Publishing another 36-byte batch would exceed 40, so the oldest is evicted
    // first: the cache is never above budget, and bytes are accounted for real.
    let delta = cache.build(&rep, stamp()).unwrap();
    cache.publish(delta, &stamp()).unwrap();
    assert!(cache.total_cpu_bytes() <= 40);
    assert_eq!(cache.chunk_count(), 1);
}

#[test]
fn scene_cpu_bytes_is_charged_on_publish_not_faked() {
    // Two small batches fit; the sum is exactly their counted bytes.
    let mut cache = SceneCache::default();
    let rep = line_representation(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.used_bytes(), 24);
    let delta = cache.build(&rep, stamp()).unwrap();
    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.used_bytes(), 48);
}

#[test]
fn centroid_is_origin_plus_mean_vertex() {
    let mut cache = SceneCache::default();
    let delta = cache
        .build(&mesh_representation(1, quad()), stamp())
        .unwrap();
    let batch = &delta.added[0];
    // Quad spans (0,0)-(1,1) with origin (0,0,0): centroid is (0.5, 0.5, 0).
    let c = batch.centroid();
    assert!((c[0] - 0.5).abs() < 1e-6, "got {c:?}");
    assert!((c[1] - 0.5).abs() < 1e-6, "got {c:?}");
    assert!(c[2].abs() < 1e-6, "got {c:?}");
}

#[test]
fn centroid_keeps_precision_at_large_coordinates() {
    let mut cache = SceneCache::default();
    let rep = line_representation(
        1,
        vec![
            Point3 {
                x: 1_000_000.0,
                y: 2_000_000.0,
                z: 0.0,
            },
            Point3 {
                x: 1_000_010.0,
                y: 2_000_000.0,
                z: 0.0,
            },
        ],
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    let c = delta.added[0].centroid();
    assert!((c[0] - 1_000_005.0).abs() < 1e-3, "got {c:?}");
    assert!((c[1] - 2_000_000.0).abs() < 1e-3, "got {c:?}");
}

#[test]
fn empty_batch_centroid_falls_back_to_local_origin() {
    let batch = RenderBatch {
        local_origin: Point3 {
            x: 3.0,
            y: 4.0,
            z: 5.0,
        },
        topology: RenderTopology::Lines,
        vertices: Vec::new(),
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: DEFAULT_BATCH_LINEWEIGHT_MM,
        lineweight_unresolved: true,
        linetype: cad_db::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: Vec::new(),
        draw_order: 0,
    };
    assert_eq!(batch.centroid(), [3.0, 4.0, 5.0]);
}

fn line_representation_alpha(
    entity: u128,
    points: Vec<Point3>,
    alpha: f32,
) -> DisplayRepresentation {
    let mut rep = line_representation(entity, points);
    rep.fragments[0].alpha = alpha;
    rep
}

#[test]
fn batch_alpha_comes_from_the_fragment() {
    let mut cache = SceneCache::default();
    let rep = line_representation_alpha(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
        0.25,
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    assert_eq!(delta.added.len(), 1);
    assert_eq!(delta.added[0].alpha, 0.25);
}

#[test]
fn sanitize_alpha_matches_the_renderer_policy() {
    assert_eq!(sanitize_alpha(0.5), 0.5);
    assert_eq!(sanitize_alpha(-1.0), 0.0);
    assert_eq!(sanitize_alpha(2.0), 1.0);
    assert_eq!(sanitize_alpha(0.0), 0.0);
    // An unreadable opacity must not delete geometry.
    assert_eq!(sanitize_alpha(f32::NAN), 1.0);
}

fn line_representation_styled(
    entity: u128,
    points: Vec<Point3>,
    color: [f32; 3],
    color_unresolved: bool,
    lineweight: f32,
    lineweight_unresolved: bool,
) -> DisplayRepresentation {
    let mut rep = line_representation(entity, points);
    let fragment = &mut rep.fragments[0];
    fragment.color = color;
    fragment.color_unresolved = color_unresolved;
    fragment.lineweight = lineweight;
    fragment.lineweight_unresolved = lineweight_unresolved;
    rep
}

fn unit_points() -> Vec<Point3> {
    vec![
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    ]
}

#[test]
fn batch_carries_the_linetype_pattern_for_diagnostics() {
    let mut cache = SceneCache::default();
    let mut rep = line_representation(1, unit_points());
    rep.fragments[0].linetype = cad_db::LinetypePattern::from_elements([0.5, -0.25]);
    rep.fragments[0].linetype_unresolved = false;
    let delta = cache.build(&rep, stamp()).unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.linetype.elements, vec![0.5, -0.25]);
    assert!(!batch.linetype_unresolved);
}

#[test]
fn batch_color_and_lineweight_come_from_the_fragment() {
    let mut cache = SceneCache::default();
    let rep = line_representation_styled(1, unit_points(), [0.25, 0.5, 0.75], false, 0.35, false);
    let delta = cache.build(&rep, stamp()).unwrap();
    assert_eq!(delta.added.len(), 1);
    let batch = &delta.added[0];
    assert_eq!(batch.color, [0.25, 0.5, 0.75]);
    assert!(!batch.color_unresolved);
    assert_eq!(batch.lineweight, 0.35);
    assert!(!batch.lineweight_unresolved);
}

#[test]
fn unresolved_color_and_lineweight_are_carried_as_explicit_defaults() {
    let mut cache = SceneCache::default();
    // ByLayer/ByBlock with no reachable value: the fragment already carries
    // the documented fallback and marks it unresolved.
    let rep = line_representation_styled(
        1,
        unit_points(),
        [1.0, 1.0, 1.0],
        true,
        cad_representation::DEFAULT_LINEWEIGHT_MM,
        true,
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.color, DEFAULT_BATCH_COLOR);
    assert!(batch.color_unresolved);
    assert_eq!(batch.lineweight, DEFAULT_BATCH_LINEWEIGHT_MM);
    assert!(batch.lineweight_unresolved);
}

#[test]
fn non_finite_color_and_lineweight_are_replaced_by_defaults() {
    // A hostile fragment must never inject NaN/inf into the GPU uniform.
    assert_eq!(
        sanitize_color([f32::NAN, f32::INFINITY, -1.0]),
        [1.0, 1.0, 0.0]
    );
    assert_eq!(
        sanitize_color([f32::NEG_INFINITY, 2.0, 0.5]),
        [1.0, 1.0, 0.5]
    );
    assert_eq!(sanitize_lineweight(f32::NAN), DEFAULT_BATCH_LINEWEIGHT_MM);
    assert_eq!(
        sanitize_lineweight(f32::NEG_INFINITY),
        DEFAULT_BATCH_LINEWEIGHT_MM
    );
    assert_eq!(sanitize_lineweight(-1.0), 0.0);

    let mut cache = SceneCache::default();
    let rep = line_representation_styled(
        1,
        unit_points(),
        [f32::NAN, f32::NAN, f32::NAN],
        false,
        f32::NAN,
        false,
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    assert_eq!(delta.added[0].color, DEFAULT_BATCH_COLOR);
    assert_eq!(delta.added[0].lineweight, DEFAULT_BATCH_LINEWEIGHT_MM);
}

#[test]
fn fully_transparent_fragment_is_carried_not_forced_opaque() {
    let mut cache = SceneCache::default();
    let rep = line_representation_alpha(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
        0.0,
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    // The batch keeps the real value; the renderer classifies it invisible.
    assert_eq!(delta.added[0].alpha, 0.0);
}

#[test]
fn out_of_range_fragment_alpha_is_clamped_for_the_batch() {
    let mut cache = SceneCache::default();
    let rep = line_representation_alpha(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
        4.0,
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    assert_eq!(delta.added[0].alpha, 1.0);
}
