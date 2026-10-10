//! Unit tests.

use super::*;
use cad_representation::DisplayFragment;
use std::sync::Arc;

fn stamp() -> TaskStamp {
    TaskStamp::new(DocumentId(1), 0)
}

#[test]
fn independent_segment_batches_never_connect_pairs_and_reject_odd_endpoints() {
    let points: Vec<_> = [0.0, 2.0, 5.0, 7.0]
        .into_iter()
        .map(|x| Point3 { x, y: 0.0, z: 0.0 })
        .collect();
    let mut rep = line_representation(1, points.clone());
    rep.fragments[0].primitive = DisplayPrimitive::LineSegments(Arc::from(points));
    let mut cache = SceneCache::default();
    for delta in [
        cache.build(&rep, stamp()).unwrap(),
        cache.build_compact(&rep, stamp()).unwrap(),
    ] {
        assert_eq!(
            delta.added[0].vertices,
            vec![
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [5.0, 0.0, 0.0],
                [7.0, 0.0, 0.0]
            ]
        );
        assert_eq!(
            delta.added[0].sources,
            vec![rep.fragments[0].source.clone()]
        );
    }
    rep.fragments[0].primitive = DisplayPrimitive::LineSegments(Arc::from(vec![Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    }]));
    assert!(cache.build_compact(&rep, stamp()).is_err());
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
fn packed_pairs_never_gain_connectors_and_reject_unmatched_endpoints() {
    let mut rep = line_representation(1, Vec::new());
    rep.fragments[0].primitive = DisplayPrimitive::LineSegments(Arc::from(
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
                x: 3.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 4.0,
                y: 0.0,
                z: 0.0,
            },
        ]
        .into_boxed_slice(),
    ));
    for compact in [false, true] {
        let mut cache = SceneCache::default();
        let delta = if compact {
            cache.build_compact(&rep, stamp())
        } else {
            cache.build(&rep, stamp())
        }
        .unwrap();
        assert_eq!(
            delta.added[0].vertices,
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [3.0, 0.0, 0.0],
                [4.0, 0.0, 0.0]
            ]
        );
        assert_eq!(
            delta.added[0].sources,
            vec![rep.fragments[0].source.clone()]
        );
    }
    rep.fragments[0].primitive = DisplayPrimitive::LineSegments(Arc::from(
        vec![Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }]
        .into_boxed_slice(),
    ));
    assert!(SceneCache::default().build_compact(&rep, stamp()).is_err());
}

#[test]
fn compact_lines_preserve_every_segment_without_connecting_polylines() {
    let p = |x, y| Point3 { x, y, z: 0.0 };
    let mut rep = line_representation(
        1,
        vec![
            p(1_000_000.0, 0.0),
            p(1_000_010.0, 0.0),
            p(1_000_010.0, 10.0),
        ],
    );
    rep.fragments
        .extend(line_representation(2, vec![p(1_000_100.0, 20.0), p(1_000_110.0, 20.0)]).fragments);
    let delta = SceneCache::default().build_compact(&rep, stamp()).unwrap();
    assert_eq!(delta.added.len(), 1);
    let batch = &delta.added[0];
    assert_eq!(
        batch.vertices,
        vec![
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 10.0, 0.0],
            [100.0, 20.0, 0.0],
            [110.0, 20.0, 0.0],
        ]
    );
    assert_eq!(
        batch.sources,
        rep.fragments
            .iter()
            .map(|f| f.source.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert_eq!(batch.triangle_count(), 0);
}

#[test]
fn compact_lines_are_bounded_and_repeated_dash_sources_are_not_duplicated() {
    let mut rep = line_representation(
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
    rep.fragments = (0..40_000)
        .map(|_| {
            line_representation(
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
            )
            .fragments
            .remove(0)
        })
        .collect();
    let delta = SceneCache::default().build_compact(&rep, stamp()).unwrap();
    assert_eq!(delta.added.len(), 2);
    assert_eq!(delta.added[0].vertices.len(), 65_536);
    assert_eq!(delta.added[1].vertices.len(), 80_000 - 65_536);
    for batch in &delta.added {
        assert_eq!(batch.sources, vec![rep.fragments[0].source.clone()]);
    }
}

#[test]
fn compact_lines_keep_style_boundaries_and_transparent_depth_keys() {
    let mut rep = DisplayRepresentation {
        fragments: (0..7)
            .map(|_| {
                line_representation(
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
                )
                .fragments
                .remove(0)
            })
            .collect(),
        completeness: Completeness::Complete,
        diagnostics: Vec::new(),
    };
    rep.fragments[1].color = [0.0, 1.0, 0.0];
    rep.fragments[2].lineweight = 2.0;
    rep.fragments[3].linetype_unresolved = false;
    rep.fragments[4].linetype = cad_db::LinetypePattern {
        elements: vec![1.0, -1.0],
        cycle: 2.0,
    };
    rep.fragments[5].alpha = 0.5;
    rep.fragments[6].alpha = 0.5;
    let mut cache = SceneCache::default();
    assert_eq!(
        cache.build_compact(&rep, stamp()).unwrap(),
        cache.build(&rep, stamp()).unwrap()
    );
}

#[test]
fn compact_lines_do_not_rebase_distant_geometry_or_cross_primitive_boundaries() {
    let p = |x| Point3 { x, y: 0.0, z: 0.0 };
    let mut rep = line_representation(1, vec![p(0.0), p(1.0)]);
    rep.fragments
        .extend(line_representation(2, vec![p(1e12), p(1e12 + 1.0)]).fragments);
    let mut instance = line_representation(1, vec![p(0.0), p(1.0)])
        .fragments
        .remove(0);
    instance.primitive = DisplayPrimitive::Instance {
        block: BlockId(1),
        transform: Transform3::identity(),
    };
    rep.fragments.push(instance);
    rep.fragments
        .extend(line_representation(3, vec![p(1e12 + 2.0), p(1e12 + 3.0)]).fragments);
    let delta = SceneCache::default().build_compact(&rep, stamp()).unwrap();
    assert_eq!(delta.added.len(), 3);
    assert_eq!(delta.added[1].local_origin.x, 1e12);
    assert_eq!(
        delta.added[1].vertices,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]
    );
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
fn line_topologies_do_not_consume_the_triangle_budget() {
    let mut cache = SceneCache::default();
    let mut points = unit_points();
    points.extend(unit_points());
    points.extend(unit_points());
    let delta = cache
        .build(&line_representation(1, points), stamp())
        .unwrap();
    let mut batch = delta.added[0].clone();
    let budget = FrameBudget {
        max_vertices: 6,
        max_triangles: 0,
        max_bytes: usize::MAX,
    };
    for topology in [RenderTopology::Lines, RenderTopology::MeshEdges] {
        batch.topology = topology;
        assert_eq!(batch.triangle_count(), 0);
        let mut usage = FrameUsage::default();
        budget
            .charge(&mut usage, batch.vertices.len(), batch.triangle_count())
            .unwrap();
        assert_eq!(usage.vertices, 6);
        assert_eq!(usage.triangles, 0);
    }
    // Non-indexed mesh batches retain their triangle-list fallback.
    batch.topology = RenderTopology::Mesh;
    assert_eq!(batch.triangle_count(), 2);
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
fn frame_budget_rejects_counter_overflow_without_changing_usage() {
    let budget = FrameBudget {
        max_vertices: usize::MAX,
        max_triangles: usize::MAX,
        max_bytes: usize::MAX,
    };
    let mut usage = FrameUsage {
        vertices: usize::MAX,
        triangles: usize::MAX,
        bytes: usize::MAX,
    };
    let before = usage;
    assert_eq!(
        budget.charge(&mut usage, 1, 0),
        Err(BudgetExceeded {
            category: "vertices",
            requested: usize::MAX,
            limit: usize::MAX,
        })
    );
    assert_eq!(usage, before);
    assert_eq!(
        budget.charge(&mut usage, 0, 1),
        Err(BudgetExceeded {
            category: "triangles",
            requested: usize::MAX,
            limit: usize::MAX,
        })
    );
    assert_eq!(usage, before);
    assert_eq!(
        budget.charge_bytes(&mut usage, 1),
        Err(BudgetExceeded {
            category: "bytes",
            requested: usize::MAX,
            limit: usize::MAX,
        })
    );
    assert_eq!(usage, before);
    budget.charge(&mut usage, 0, 0).unwrap();
    budget.charge_bytes(&mut usage, 0).unwrap();
    assert_eq!(usage, before);
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
fn impossible_cache_reservation_preserves_live_chunks_and_accounting() {
    let mut cache = SceneCache::new(SceneBudget {
        cpu_bytes: 48,
        ..Default::default()
    });
    let delta = cache
        .build(&line_representation(1, unit_points()), stamp())
        .unwrap();
    cache.publish(delta, &stamp()).unwrap();
    let before: Vec<_> = cache.chunks().cloned().collect();
    for required_bytes in [49, usize::MAX] {
        assert!(matches!(
            cache.evict(required_bytes),
            Err(CadError::InvalidInput(_))
        ));
        assert_eq!(cache.chunks().cloned().collect::<Vec<_>>(), before);
        assert_eq!(cache.used_bytes(), 24);
    }
    assert_eq!(cache.evict(24), Ok(0));
    assert_eq!(cache.evict(48), Ok(1));
    assert_eq!(cache.chunk_count(), 0);
    assert_eq!(cache.used_bytes(), 0);
}

#[test]
fn cache_reservation_near_usize_max_does_not_overflow() {
    let mut cache = SceneCache::new(SceneBudget {
        cpu_bytes: usize::MAX,
        ..Default::default()
    });
    let delta = cache
        .build(&line_representation(1, unit_points()), stamp())
        .unwrap();
    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.evict(usize::MAX - 24), Ok(0));
    assert_eq!(cache.used_bytes(), 24);
    assert_eq!(cache.evict(usize::MAX), Ok(1));
    assert_eq!(cache.chunk_count(), 0);
    assert_eq!(cache.used_bytes(), 0);
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

// ---- Dynamic-block visibility delta (spec §3.2 / incremental update) ----

#[test]
fn visibility_switch_invalidates_only_the_delta_members() {
    // Three member chunks are on the CPU cache: 50, 51 and 52. A visibility
    // switch A -> B removes member 50's geometry and adds member 52's; 51 is
    // common to both states and must stay cached.
    let mut cache = SceneCache::default();
    for entity in [50u128, 51, 52] {
        let rep = line_representation(
            entity,
            vec![
                Point3 {
                    x: entity as f64,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: entity as f64,
                    y: 1.0,
                    z: 0.0,
                },
            ],
        );
        let delta = cache.build(&rep, stamp()).unwrap();
        cache.publish(delta, &stamp()).unwrap();
    }
    assert_eq!(cache.chunk_count(), 3);

    // The exact ChangeSet the database publishes for switching A -> B:
    // member 50 leaves, member 52 enters, member 51 is untouched.
    let changes = ChangeSet {
        database: DatabaseId(1),
        before: Revision(0),
        after: Revision(1),
        transaction: TransactionId(9),
        reason: "dynamic visibility A -> B".into(),
        changes: vec![
            ObjectChange::Update(ObjectId(50), cad_db::ChangeMask::GEOMETRY),
            ObjectChange::Update(ObjectId(52), cad_db::ChangeMask::GEOMETRY),
        ],
    };
    cache.apply_changes(&changes).unwrap();

    // Only member 51's chunk survives; the changed members were invalidated.
    assert_eq!(cache.chunk_count(), 1);
    let survivor = cache.chunks().next().unwrap();
    assert_eq!(survivor.sources[0].entity, EntityId(51));
}

// ---- Image draw items (spec §4.8 image primitives) ----

fn image_representation(
    entity: u128,
    resource: ResourceKey,
    transform: Transform3,
) -> DisplayRepresentation {
    let mut rep = line_representation(entity, Vec::new());
    rep.fragments[0].primitive = DisplayPrimitive::Image {
        resource,
        transform,
        clip: None,
    };
    rep
}

#[test]
fn image_fragment_becomes_an_image_batch_with_deferred_draw_order() {
    let mut cache = SceneCache::default();
    let resource = ResourceKey("img:logo".into());
    let transform = Transform3::identity();
    let rep = image_representation(7, resource.clone(), transform);
    let delta = cache.build(&rep, stamp()).unwrap();
    // The image is not smuggled into the line/mesh list as a fake batch.
    assert!(delta.added.is_empty());
    assert_eq!(delta.images.len(), 1);
    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.chunk_count(), 0);
    assert_eq!(cache.image_count(), 1);
    let image = &cache.image_batches()[0];
    assert_eq!(image.resource, resource);
    assert_eq!(image.transform, transform);
    assert_eq!(image.alpha, 1.0);
    // Codifies the CURRENT deferred state: `draw_order` is NOT yet plumbed from
    // `DbEntity::draw_order` through `DisplayFragment` (deferred, see
    // `docs/handoff.md`), so the scene emits `0` and paint order falls back to
    // upload order. This is not a desired behaviour, only a snapshot of the
    // deferral; the assertion will change when threading lands.
    assert_eq!(image.draw_order, 0);
    assert_eq!(image.sources, vec![rep.fragments[0].source.clone()]);
    assert!(cache.image_used_bytes() > 0);
}

#[test]
fn over_budget_image_batches_are_evicted_and_counted() {
    // Learn the CPU size of one image draw item, then size a cache to hold
    // exactly one so a second publish must evict the oldest.
    let mut probe = SceneCache::default();
    let probe_delta = probe
        .build(
            &image_representation(1, ResourceKey("img:1".into()), Transform3::identity()),
            stamp(),
        )
        .unwrap();
    let image_bytes = probe_delta.images[0].approx_bytes();
    probe.publish(probe_delta, &stamp()).unwrap();
    assert_eq!(probe.image_evictions(), 0);

    let budget = SceneBudget {
        image_bytes,
        ..SceneBudget::default()
    };
    let mut cache = SceneCache::new(budget);

    let first = cache
        .build(
            &image_representation(1, ResourceKey("img:1".into()), Transform3::identity()),
            stamp(),
        )
        .unwrap();
    cache.publish(first, &stamp()).unwrap();
    assert_eq!(cache.image_count(), 1);
    assert_eq!(cache.image_evictions(), 0);

    let second = cache
        .build(
            &image_representation(2, ResourceKey("img:2".into()), Transform3::identity()),
            stamp(),
        )
        .unwrap();
    cache.publish(second, &stamp()).unwrap();

    // The oldest image was evicted to stay under the cap and the drop is
    // counted, not silently lost; the most recent image survives.
    assert_eq!(cache.image_count(), 1);
    assert_eq!(cache.image_evictions(), 1);
    assert!(cache.image_used_bytes() <= image_bytes);
    assert_eq!(
        cache.image_batches()[0].resource,
        ResourceKey("img:2".into())
    );
}

#[test]
fn change_set_removes_affected_image_batches() {
    let mut cache = SceneCache::default();
    let rep = image_representation(9, ResourceKey("img:9".into()), Transform3::identity());
    let delta = cache.build(&rep, stamp()).unwrap();
    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.image_count(), 1);
    let changes = ChangeSet {
        database: DatabaseId(1),
        before: Revision(0),
        after: Revision(1),
        transaction: TransactionId(1),
        reason: "edit image".into(),
        changes: vec![ObjectChange::Update(
            ObjectId(9),
            cad_db::ChangeMask::GEOMETRY,
        )],
    };
    cache.apply_changes(&changes).unwrap();
    assert_eq!(cache.image_count(), 0);
    assert_eq!(cache.image_used_bytes(), 0);
}

#[test]
fn text_and_instance_fragments_still_produce_no_batches() {
    let mut cache = SceneCache::default();
    let mut rep = line_representation(1, Vec::new());
    rep.fragments[0].primitive = DisplayPrimitive::Text {
        text: "A".into(),
        origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        font: ResourceKey("style:Standard".into()),
        height: 2.5,
    };
    let delta = cache.build(&rep, stamp()).unwrap();
    assert!(delta.added.is_empty());
    assert!(delta.images.is_empty());

    rep.fragments[0].primitive = DisplayPrimitive::Instance {
        block: BlockId(1),
        transform: Transform3::identity(),
    };
    let delta = cache.build(&rep, stamp()).unwrap();
    assert!(delta.added.is_empty());
    assert!(delta.images.is_empty());
}
