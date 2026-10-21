use super::*;

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
            wrap_width: None,
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
