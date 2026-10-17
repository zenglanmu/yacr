//! Two-level loose-chunk spatial index for tolerance-expanded ray queries.
//!
//! This is the broad-phase structure ADR-0012 §7 specifies for the CPU pick
//! path: a top-level uniform X/Y chunk grid over the drawing extent, and a
//! per-chunk local payload list. A pick only visits the few chunks its ray can
//! touch, so the precise stage no longer runs over the whole drawing.
//!
//! # Placement (the "oversized primitive" rule)
//!
//! Every bounded entry is placed in each chunk its AABB spans. An entry whose
//! AABB spans more than [`MAX_CHUNK_SPAN`] chunks (a very long line, a large
//! hatch) is **not** copied into every chunk it covers; it goes to an overflow
//! list that is tested directly on every query. That is the ADR-0012 §7 rule
//! against a city-block-sized fill being replicated across thousands of chunks.
//!
//! # Query (conservative broad phase)
//!
//! The query clips the ray's X/Y projection to the grid rectangle expanded by
//! the pick tolerance, visits the chunks that rectangle touches, and applies
//! the **same** tolerance-expanded slab test the precise stage uses
//! ([`crate::pick::ray_intersects_expanded_bounds`]) to every local entry.
//! Overflow entries are tested directly.
//!
//! The result is conservative by construction: the test only rejects an entry
//! when the ray stays further than the tolerance from its AABB, and
//! non-finite arithmetic falls through to a candidate. A rejected entry can
//! therefore never have been hit, so the broad phase can never turn a hit into
//! a miss. `[BBox intersects query] ⟹ hit` is never assumed (ADR-0012 §2).
//!
//! # What this is not
//!
//! The per-chunk local structure is a flat payload list, not the `rstar`
//! per-chunk tree ADR-0012 §7 names: this crate may not add the `rstar`
//! dependency in this change (the manifest is out of scope), so the local list
//! is the honest current structure. A point-like query (the 2D orthographic
//! pick ray is a single X/Y column) visits one chunk, so the local list is
//! small; a long diagonal ray degrades toward a linear scan of the chunks it
//! crosses. That degradation is reported, not hidden.

use cad_domain::*;

use crate::pick::{ray_intersects_expanded_bounds, validate_ray, MIN_RAY_DIRECTION};

/// Smallest grid cell extent; keeps `cell` finite and positive for a degenerate
/// (single-point) drawing extent.
const MIN_CELL_EXTENT: f64 = 1e-9;
/// An entry whose AABB spans more chunks than this is kept in the overflow list
/// instead of being copied into every chunk it covers.
const MAX_CHUNK_SPAN: usize = 16;

/// A two-level chunk-grid broad phase keyed by opaque `u32` payloads.
///
/// The payload is whatever the caller uses to identify an entry (here: the
/// item's expansion ordinal). Payloads are returned in ascending order so the
/// caller's stable tie handling is preserved.
pub struct ChunkGridIndex {
    min: Point3,
    cell: f64,
    cols: usize,
    rows: usize,
    /// Chunk ordinal (`row * cols + col`) -> payload ids.
    chunks: Vec<Vec<u32>>,
    /// Payload id -> conservative AABB.
    bounds: Vec<Bounds3>,
    /// Payload id -> caller payload.
    payloads: Vec<u32>,
    /// Payload ids of oversized entries, tested directly on every query.
    overflow: Vec<u32>,
    built: bool,
}

impl Default for ChunkGridIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkGridIndex {
    pub fn new() -> Self {
        ChunkGridIndex {
            min: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            cell: 1.0,
            cols: 0,
            rows: 0,
            chunks: Vec::new(),
            bounds: Vec::new(),
            payloads: Vec::new(),
            overflow: Vec::new(),
            built: false,
        }
    }

    /// Build from `(payload, bounds)` pairs.
    ///
    /// Rejects a non-finite or inverted box with [`CadError::InvalidInput`];
    /// a caller that cannot prove a box must not offer one (it belongs in an
    /// always-candidate list instead).
    pub fn build(entries: &[(u32, Bounds3)]) -> CadResult<Self> {
        for (_, bounds) in entries {
            validate_bounds(bounds)?;
        }
        let mut index = Self::new();
        index.bounds = entries.iter().map(|(_, b)| *b).collect();
        index.payloads = entries.iter().map(|(p, _)| *p).collect();
        index.layout();
        Ok(index)
    }

    /// Number of indexed (bounded) entries.
    pub fn entry_count(&self) -> usize {
        self.payloads.len()
    }

    /// Number of oversized entries kept in the overflow list.
    pub fn overflow_count(&self) -> usize {
        self.overflow.len()
    }

    /// Number of chunks in the top-level grid.
    pub fn chunk_count(&self) -> usize {
        self.cols * self.rows
    }

    /// Grid origin (X/Y); a diagnostic for tests and evidence.
    pub fn grid_origin(&self) -> Point3 {
        self.min
    }

    /// Grid cell size (X/Y); a diagnostic for tests and evidence.
    pub fn cell_size(&self) -> f64 {
        self.cell
    }

    /// Grid dimensions `(cols, rows)`; a diagnostic for tests and evidence.
    pub fn grid_dimensions(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    fn layout(&mut self) {
        let mut min = Point3 {
            x: f64::INFINITY,
            y: f64::INFINITY,
            z: 0.0,
        };
        let mut max = Point3 {
            x: f64::NEG_INFINITY,
            y: f64::NEG_INFINITY,
            z: 0.0,
        };
        for b in &self.bounds {
            min.x = min.x.min(b.min.x);
            min.y = min.y.min(b.min.y);
            max.x = max.x.max(b.max.x);
            max.y = max.y.max(b.max.y);
        }
        if self.bounds.is_empty() || !min.x.is_finite() || !max.x.is_finite() {
            self.cols = 0;
            self.rows = 0;
            self.chunks.clear();
            self.built = true;
            return;
        }
        let ex = (max.x - min.x).max(MIN_CELL_EXTENT);
        let ey = (max.y - min.y).max(MIN_CELL_EXTENT);
        let target = (self.bounds.len() / 4).clamp(16, 65536) as f64;
        let aspect = (ex / ey).max(MIN_CELL_EXTENT);
        self.cols = ((target * aspect).sqrt().ceil() as usize).clamp(1, 1024);
        self.rows = ((target / self.cols as f64).ceil() as usize).clamp(1, 1024);
        self.cell = (ex / self.cols as f64)
            .max(ey / self.rows as f64)
            .max(MIN_CELL_EXTENT);
        self.min = min;
        self.chunks = vec![Vec::new(); self.cols * self.rows];
        for (id, bounds) in self.bounds.iter().enumerate() {
            let (c0, r0) = self.cell_of(bounds.min.x, bounds.min.y);
            let (c1, r1) = self.cell_of(bounds.max.x, bounds.max.y);
            let span = (c1 - c0 + 1) * (r1 - r0 + 1);
            if span > MAX_CHUNK_SPAN {
                self.overflow.push(id as u32);
                continue;
            }
            for r in r0..=r1 {
                for c in c0..=c1 {
                    self.chunks[r * self.cols + c].push(id as u32);
                }
            }
        }
        self.built = true;
    }

    fn cell_of(&self, x: f64, y: f64) -> (usize, usize) {
        let cx = (((x - self.min.x) / self.cell).floor() as isize).clamp(0, self.cols as isize - 1);
        let cy = (((y - self.min.y) / self.cell).floor() as isize).clamp(0, self.rows as isize - 1);
        (cx as usize, cy as usize)
    }

    /// The chunks overlapping a query rectangle, as an inclusive cell range.
    fn cell_range(&self, lo: Point3, hi: Point3) -> (usize, usize, usize, usize) {
        let (c0, r0) = self.cell_of(lo.x, lo.y);
        let (c1, r1) = self.cell_of(hi.x, hi.y);
        (c0, r0, c1, r1)
    }

    /// Tolerance-expanded ray candidates as ascending payloads.
    pub fn ray_candidates(&self, ray: &Ray3, tolerance: f64) -> CadResult<Vec<u32>> {
        validate_ray(ray)?;
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err(CadError::InvalidInput(
                "spatial query tolerance must be finite and > 0".into(),
            ));
        }
        if !self.built || self.payloads.is_empty() {
            return Ok(Vec::new());
        }
        let mut out: Vec<u32> = Vec::new();
        if let Some((lo, hi)) = self.query_region(ray, tolerance) {
            let (c0, r0, c1, r1) = self.cell_range(lo, hi);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    for &id in &self.chunks[r * self.cols + c] {
                        self.test(id, ray, tolerance, &mut out);
                    }
                }
            }
        }
        for &id in &self.overflow {
            self.test(id, ray, tolerance, &mut out);
        }
        out.sort_unstable();
        out.dedup();
        Ok(out)
    }

    fn test(&self, id: u32, ray: &Ray3, tolerance: f64, out: &mut Vec<u32>) {
        let bounds = &self.bounds[id as usize];
        if ray_intersects_expanded_bounds(ray, bounds.min, bounds.max, tolerance) {
            out.push(self.payloads[id as usize]);
        }
    }

    /// The X/Y rectangle the ray's projection can touch, or `None` when it
    /// cannot reach the grid at all.
    ///
    /// A degenerate (X/Y-parallel) ray is a single point; otherwise the 2D ray
    /// is clipped to the grid rectangle expanded by `tolerance`, and the
    /// rectangle spanned by the clipped endpoints is returned. The rectangle is
    /// a conservative superset of the ray's swept region, so a hit is never
    /// outside it.
    fn query_region(&self, ray: &Ray3, tolerance: f64) -> Option<(Point3, Point3)> {
        let lo = Point3 {
            x: self.min.x - tolerance,
            y: self.min.y - tolerance,
            z: 0.0,
        };
        let hi = Point3 {
            x: self.min.x + self.cols as f64 * self.cell + tolerance,
            y: self.min.y + self.rows as f64 * self.cell + tolerance,
            z: 0.0,
        };
        let (ox, oy) = (ray.origin.x, ray.origin.y);
        let (dx, dy) = (ray.direction.x, ray.direction.y);
        let dxy = dx.hypot(dy);
        if dxy < MIN_RAY_DIRECTION {
            // An X/Y-parallel ray (the 2D orthographic pick ray) touches the
            // single X/Y point it passes through, expanded by the tolerance.
            if ox < lo.x || ox > hi.x || oy < lo.y || oy > hi.y {
                return None;
            }
            return Some((
                Point3 {
                    x: ox - tolerance,
                    y: oy - tolerance,
                    z: 0.0,
                },
                Point3 {
                    x: ox + tolerance,
                    y: oy + tolerance,
                    z: 0.0,
                },
            ));
        }
        let mut t0 = f64::NEG_INFINITY;
        let mut t1 = f64::INFINITY;
        for (o, d, l, h) in [(ox, dx, lo.x, hi.x), (oy, dy, lo.y, hi.y)] {
            if d == 0.0 {
                if o < l || o > h {
                    return None;
                }
                continue;
            }
            let a = (l - o) / d;
            let b = (h - o) / d;
            let near = a.min(b);
            let far = a.max(b);
            if near.is_finite() {
                t0 = t0.max(near);
            }
            if far.is_finite() {
                t1 = t1.min(far);
            }
            if t0 > t1 {
                return None;
            }
        }
        if !t0.is_finite() || !t1.is_finite() {
            // The projection is parallel to an axis and stays inside the band;
            // fall back to the whole grid so nothing is silently dropped.
            return Some((lo, hi));
        }
        let p0 = Point3 {
            x: ox + dx * t0,
            y: oy + dy * t0,
            z: 0.0,
        };
        let p1 = Point3 {
            x: ox + dx * t1,
            y: oy + dy * t1,
            z: 0.0,
        };
        // Expand the swept rectangle by the tolerance: a candidate may sit up
        // to `tolerance` off the ray's projection, which can fall in the
        // neighbouring chunk.
        Some((
            Point3 {
                x: p0.x.min(p1.x) - tolerance,
                y: p0.y.min(p1.y) - tolerance,
                z: 0.0,
            },
            Point3 {
                x: p0.x.max(p1.x) + tolerance,
                y: p0.y.max(p1.y) + tolerance,
                z: 0.0,
            },
        ))
    }
}

fn validate_bounds(bounds: &Bounds3) -> CadResult<()> {
    for (min, max) in [
        (bounds.min.x, bounds.max.x),
        (bounds.min.y, bounds.max.y),
        (bounds.min.z, bounds.max.z),
    ] {
        if !min.is_finite() || !max.is_finite() {
            return Err(CadError::InvalidInput(
                "spatial bounds are not finite".into(),
            ));
        }
        if min > max {
            return Err(CadError::InvalidInput("spatial bounds are inverted".into()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
        // A 1e6-long line spanning far more than MAX_CHUNK_SPAN chunks must be
        // in the overflow list and still be returned when the ray meets it.
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
        assert_eq!(index.overflow_count(), 1, "the long line overflows");
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
}
