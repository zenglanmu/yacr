//! Line and segment batch construction, compaction, and merging.

use super::*;

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
fn compact_merge_requires_an_equal_draw_order() {
    // A fragment's order is carried onto its batch, not flattened to 0.
    let mut rep = line_representation(1, unit_points());
    rep.fragments[0].draw_order = 7;
    let mut batch = SceneCache::default()
        .build(&rep, stamp())
        .unwrap()
        .added
        .remove(0);
    assert_eq!(batch.draw_order, 7);
    let fragment = &rep.fragments[0];
    let points = match &fragment.primitive {
        DisplayPrimitive::Lines(points) => points.as_ref(),
        _ => panic!("expected a line fragment"),
    };
    // Two adjacent same-style fragments merge at the same order...
    assert!(crate::cache::line_fragment_merges_into(
        &batch,
        fragment,
        false,
        points,
        fragment.draw_order,
    ));
    // ...but a fragment emitted at a different paint order (e.g. a selection
    // highlight) must not be absorbed into the batch, so merging never draws
    // geometry at the wrong order.
    assert!(!crate::cache::line_fragment_merges_into(
        &batch,
        fragment,
        false,
        points,
        HIGHLIGHT_DRAW_ORDER,
    ));
    // Symmetrically, a batch already carrying a distinct order must not absorb a
    // normal scene fragment.
    batch.draw_order = HIGHLIGHT_DRAW_ORDER;
    assert!(!crate::cache::line_fragment_merges_into(
        &batch,
        fragment,
        false,
        points,
        fragment.draw_order,
    ));
}

#[test]
fn compact_merge_keeps_collinear_lines_with_different_orders_separate() {
    // Two collinear, identical-style segments that the merge would previously
    // have joined into one batch. With different display orders they must stay
    // separate, so each order is preserved for the renderer's sort.
    let p = |x| Point3 { x, y: 0.0, z: 0.0 };
    let mut rep = line_representation(1, vec![p(0.0), p(1.0)]);
    rep.fragments
        .extend(line_representation(2, vec![p(1.0), p(2.0)]).fragments);
    rep.fragments[1].draw_order = 3;
    let delta = SceneCache::default().build_compact(&rep, stamp()).unwrap();
    assert_eq!(
        delta.added.len(),
        2,
        "different display orders must not merge into one batch"
    );
    assert_eq!(delta.added[0].draw_order, 0);
    assert_eq!(delta.added[1].draw_order, 3);
    // The same two fragments at a shared order do merge: the split above is
    // caused by the order boundary, not by an unrelated style difference.
    rep.fragments[1].draw_order = 0;
    let merged = SceneCache::default().build_compact(&rep, stamp()).unwrap();
    assert_eq!(merged.added.len(), 1);
    assert_eq!(merged.added[0].draw_order, 0);
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
