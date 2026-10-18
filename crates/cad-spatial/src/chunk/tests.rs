use super::*;

fn box_at(x: f64, y: f64, half: f64) -> Bounds3 {
    Bounds3 {
        min: Point3 {
            x: x - half,
            y: y - half,
            z: -half,
        },
        max: Point3 {
            x: x + half,
            y: y + half,
            z: half,
        },
    }
}

/// The brute-force reference: every entry the tolerance-expanded slab test
/// accepts. The index must return exactly this set.
fn brute_force(entries: &[(u32, Bounds3)], ray: &Ray3, tolerance: f64) -> Vec<u32> {
    let mut out: Vec<u32> = entries
        .iter()
        .filter(|(_, b)| ray_intersects_expanded_bounds(ray, b.min, b.max, tolerance))
        .map(|(p, _)| *p)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Deterministic xorshift so a failure is reproducible.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next()
    }
}

#[test]
fn empty_index_has_no_candidates() {
    let index = ChunkGridIndex::new();
    let ray = Ray3 {
        origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        direction: Point3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        },
    };
    assert!(index.ray_candidates(&ray, 0.1).unwrap().is_empty());
    let built = ChunkGridIndex::build(&[]).unwrap();
    assert_eq!(built.entry_count(), 0);
    assert_eq!(built.chunk_count(), 0);
    assert!(built.ray_candidates(&ray, 0.1).unwrap().is_empty());
}

#[test]
fn candidates_match_the_brute_force_reference() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    // Adversarial coordinates: exact quarter-unit multiples (which land on
    // chunk boundaries for a range of layouts) mixed with random points.
    let mut entries: Vec<(u32, Bounds3)> = Vec::new();
    for id in 0..400u32 {
        let (x, y, half) = if id % 2 == 0 {
            let k = (rng.range(0.0, 40.0)).floor();
            let m = (rng.range(0.0, 40.0)).floor();
            (k * 0.25, m * 0.25, rng.range(0.0, 0.5))
        } else {
            (
                rng.range(-10.0, 10.0),
                rng.range(-10.0, 10.0),
                rng.range(0.0, 1.5),
            )
        };
        entries.push((id, box_at(x, y, half)));
    }
    let index = ChunkGridIndex::build(&entries).unwrap();
    assert!(index.chunk_count() > 0);
    for _ in 0..600 {
        let origin = Point3 {
            x: rng.range(-12.0, 12.0),
            y: rng.range(-12.0, 12.0),
            z: rng.range(-3.0, 3.0),
        };
        let direction = Point3 {
            x: rng.range(-1.0, 1.0),
            y: rng.range(-1.0, 1.0),
            z: rng.range(-1.0, 1.0),
        };
        if direction.x.hypot(direction.y).hypot(direction.z) < MIN_RAY_DIRECTION {
            continue;
        }
        let tolerance = rng.range(0.01, 2.0);
        let ray = Ray3 { origin, direction };
        let expected = brute_force(&entries, &ray, tolerance);
        let got = index.ray_candidates(&ray, tolerance).unwrap();
        assert_eq!(got, expected, "ray={ray:?} tolerance={tolerance}");
    }
}

/// The two-level query is a conservative broad phase: it never drops a
/// candidate the brute-force scan reports for the same region.
#[test]
fn two_level_query_is_a_conservative_superset_of_brute_force() {
    let mut rng = Rng(0xC0FF_EE00_1234_5678);
    let entries: Vec<(u32, Bounds3)> = (0..300u32)
        .map(|id| {
            (
                id,
                box_at(
                    rng.range(-20.0, 20.0),
                    rng.range(-20.0, 20.0),
                    rng.range(0.02, 2.5),
                ),
            )
        })
        .collect();
    let index = ChunkGridIndex::build(&entries).unwrap();
    for _ in 0..400 {
        let origin = Point3 {
            x: rng.range(-22.0, 22.0),
            y: rng.range(-22.0, 22.0),
            z: rng.range(-5.0, 5.0),
        };
        let direction = Point3 {
            x: rng.range(-1.0, 1.0),
            y: rng.range(-1.0, 1.0),
            z: rng.range(-1.0, 1.0),
        };
        if direction.x.hypot(direction.y).hypot(direction.z) < MIN_RAY_DIRECTION {
            continue;
        }
        let tolerance = rng.range(0.01, 3.0);
        let ray = Ray3 { origin, direction };
        let reference = brute_force(&entries, &ray, tolerance);
        let got = index.ray_candidates(&ray, tolerance).unwrap();
        for candidate in &reference {
            assert!(
                got.contains(candidate),
                "broad phase dropped {candidate} for ray={ray:?} tolerance={tolerance}"
            );
        }
    }
}

#[test]
fn queries_exactly_on_a_chunk_boundary_stay_correct() {
    let mut rng = Rng(0xDEAD_BEEF_1234_5678);
    let entries: Vec<(u32, Bounds3)> = (0..200u32)
        .map(|id| {
            (
                id,
                box_at(
                    rng.range(-8.0, 8.0),
                    rng.range(-8.0, 8.0),
                    rng.range(0.05, 0.8),
                ),
            )
        })
        .collect();
    let index = ChunkGridIndex::build(&entries).unwrap();
    let origin = index.grid_origin();
    let cell = index.cell_size();
    assert!(cell.is_finite() && cell > 0.0);
    // Rays whose origin sits exactly on a chunk edge (and the parallel /
    // perpendicular directions through it) must still match brute force.
    for k in 0..4 {
        let x = origin.x + cell * k as f64;
        let y = origin.y + cell * k as f64;
        for direction in [
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            },
            Point3 {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
        ] {
            for tolerance in [1e-9, 0.1, 1.0] {
                let ray = Ray3 {
                    origin: Point3 { x, y, z: 0.0 },
                    direction,
                };
                let expected = brute_force(&entries, &ray, tolerance);
                let got = index.ray_candidates(&ray, tolerance).unwrap();
                assert_eq!(got, expected, "ray={ray:?} tolerance={tolerance}");
            }
        }
    }
}

#[test]
fn oversized_entries_are_kept_and_still_candidates() {
    // A 1e6-long line spanning far more than one loose cell must be in the
    // oversized list and still be returned when the ray meets it.
    let entries = vec![
        (7u32, box_at(0.0, 0.0, 0.1)),
        (
            9u32,
            Bounds3 {
                min: Point3 {
                    x: -5e5,
                    y: -1.0,
                    z: -1.0,
                },
                max: Point3 {
                    x: 5e5,
                    y: 1.0,
                    z: 1.0,
                },
            },
        ),
    ];
    let index = ChunkGridIndex::build(&entries).unwrap();
    assert_eq!(index.oversized_count(), 1, "the long line is oversized");
    let ray = Ray3 {
        origin: Point3 {
            x: 400_000.0,
            y: 0.0,
            z: 0.0,
        },
        direction: Point3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        },
    };
    let got = index.ray_candidates(&ray, 0.1).unwrap();
    assert!(got.contains(&9), "the oversized line is still a candidate");
}

/// The loose-bound rule: a primitive no larger than the cell size lands in
/// exactly one chunk's local index (never duplicated across cells).
#[test]
fn a_primitive_within_the_cell_size_lands_in_exactly_one_chunk() {
    let mut rng = Rng(0x1122_3344_5566_7788);
    // A dense grid of tiny primitives so the cells are much larger than the
    // primitives themselves.
    let entries: Vec<(u32, Bounds3)> = (0..400u32)
        .map(|id| {
            (
                id,
                box_at(
                    rng.range(-50.0, 50.0),
                    rng.range(-50.0, 50.0),
                    rng.range(0.001, 0.02),
                ),
            )
        })
        .collect();
    let index = ChunkGridIndex::build(&entries).unwrap();
    let cell = index.cell_size();
    for (payload, bounds) in &entries {
        let size = (bounds.max.x - bounds.min.x).max(bounds.max.y - bounds.min.y);
        assert!(size <= cell, "test setup: primitive larger than the cell");
        assert_eq!(
            index.chunk_placements(*payload),
            1,
            "payload {payload} must be in exactly one chunk"
        );
        assert_eq!(index.oversized_count(), 0, "nothing here is oversized");
    }
    // No entry is duplicated anywhere in the two-level structure.
    assert_eq!(index.stored_entry_count(), index.entry_count());
}

/// A primitive much larger than the cell size is stored once (oversized),
/// never copied into every cell it crosses.
#[test]
fn an_oversized_primitive_is_not_duplicated_across_chunks() {
    let mut entries: Vec<(u32, Bounds3)> = (0..500u32)
        .map(|id| (id, box_at(id as f64 * 0.1, 0.0, 0.01)))
        .collect();
    // A single line spanning the whole (wide) drawing.
    entries.push((
        9999,
        Bounds3 {
            min: Point3 {
                x: -1000.0,
                y: -0.5,
                z: -0.5,
            },
            max: Point3 {
                x: 1000.0,
                y: 0.5,
                z: 0.5,
            },
        },
    ));
    let index = ChunkGridIndex::build(&entries).unwrap();
    assert_eq!(index.oversized_count(), 1, "the wide line is oversized");
    assert_eq!(index.chunk_placements(9999), 0, "it is not in any chunk");
    // No duplication: the structure stores exactly one entry per input.
    assert_eq!(index.stored_entry_count(), index.entry_count());
    assert_eq!(index.entry_count(), entries.len());
}

/// An empty chunk contributes no candidates; the query still finds the
/// entries in the neighbouring non-empty chunks.
#[test]
fn an_empty_chunk_contributes_no_candidates() {
    // Two point-like entries far apart: the layout has many empty chunks
    // between them.
    let entries = vec![
        (1u32, box_at(0.0, 0.0, 0.01)),
        (2u32, box_at(100.0, 0.0, 0.01)),
    ];
    let index = ChunkGridIndex::build(&entries).unwrap();
    let (cols, rows) = index.grid_dimensions();
    assert!(cols * rows > 2, "the grid must have empty chunks");
    let empty = (0..cols * rows)
        .find(|c| index.chunk_entry_count(*c) == 0)
        .expect("an empty chunk exists");
    assert_eq!(index.chunk_entry_count(empty), 0);
    // A ray over the empty chunk between the two entries hits neither.
    let ray = Ray3 {
        origin: Point3 {
            x: 50.0,
            y: 1000.0,
            z: 0.0,
        },
        direction: Point3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        },
    };
    assert!(index.ray_candidates(&ray, 0.05).unwrap().is_empty());
    // A ray over the first entry still finds it.
    let hit_ray = Ray3 {
        origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 10.0,
        },
        direction: Point3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        },
    };
    assert_eq!(index.ray_candidates(&hit_ray, 0.05).unwrap(), vec![1]);
}

/// A scene whose entries are all oversized: the grid has no stored entries
/// but the oversized list still answers queries.
#[test]
fn an_oversized_only_scene_queries_correctly() {
    let entries = vec![
        (
            1u32,
            Bounds3 {
                min: Point3 {
                    x: -1000.0,
                    y: -1.0,
                    z: -1.0,
                },
                max: Point3 {
                    x: 1000.0,
                    y: 1.0,
                    z: 1.0,
                },
            },
        ),
        (
            2u32,
            Bounds3 {
                min: Point3 {
                    x: -1000.0,
                    y: 100.0,
                    z: -1.0,
                },
                max: Point3 {
                    x: 1000.0,
                    y: 102.0,
                    z: 1.0,
                },
            },
        ),
    ];
    let index = ChunkGridIndex::build(&entries).unwrap();
    assert_eq!(index.oversized_count(), 2);
    assert_eq!(index.stored_entry_count(), 2);
    // A ray at y = 0 meets only the first oversized entry.
    let ray = Ray3 {
        origin: Point3 {
            x: 500.0,
            y: 0.0,
            z: 10.0,
        },
        direction: Point3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        },
    };
    assert_eq!(index.ray_candidates(&ray, 0.1).unwrap(), vec![1]);
}

/// The chunk count follows the entry count and the coarse
/// [`TARGET_ENTRIES_PER_CHUNK`] target, not the fixed fine cap the shared
/// grid arithmetic (`GridSpatialIndex`) uses: a large scene gets a small
/// number of chunks, each holding many entries.
#[test]
fn chunk_count_follows_the_target_entries_per_chunk() {
    let mut rng = Rng(0x00AB_CDEF_1234_5678);
    let entries: Vec<(u32, Bounds3)> = (0..20_000u32)
        .map(|id| {
            (
                id,
                box_at(rng.range(-500.0, 500.0), rng.range(-500.0, 500.0), 0.01),
            )
        })
        .collect();
    let index = ChunkGridIndex::build(&entries).unwrap();
    assert!(
        index.chunk_count() >= MIN_CHUNKS,
        "a tiny floor keeps empty chunks"
    );
    assert!(
        index.chunk_count() * 4 < entries.len(),
        "the grid must be coarse: chunks={} entries={}",
        index.chunk_count(),
        entries.len()
    );
}

#[test]
fn invalid_input_is_an_error_not_an_empty_success() {
    let index = ChunkGridIndex::build(&[(1, box_at(0.0, 0.0, 1.0))]).unwrap();
    let ray = Ray3 {
        origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        direction: Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    };
    assert!(matches!(
        index.ray_candidates(&ray, 0.0),
        Err(CadError::InvalidInput(_))
    ));
    assert!(matches!(
        index.ray_candidates(&ray, f64::NAN),
        Err(CadError::InvalidInput(_))
    ));
    assert!(matches!(
        ChunkGridIndex::build(&[(
            1,
            Bounds3 {
                min: Point3 {
                    x: f64::NAN,
                    y: 0.0,
                    z: 0.0,
                },
                max: Point3 {
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                },
            },
        )]),
        Err(CadError::InvalidInput(_))
    ));
}
