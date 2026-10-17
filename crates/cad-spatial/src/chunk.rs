//! Two-level spatial index for tolerance-expanded ray queries (ADR-0012 §7).
//!
//! Top level: a uniform, **loose** X/Y chunk grid over the drawing extent. An
//! entry is assigned to exactly one chunk — the chunk whose *loose* cell (the
//! cell expanded by half a cell on every side, i.e. a 2x loose factor) contains
//! the entry's AABB. The loose factor makes a primitive no larger than the cell
//! size fit in exactly one cell ([`ChunkGridIndex::chunk_placements`] is then
//! 1). A primitive that does not fit any single loose cell (a very long line, a
//! large hatch: its size exceeds the cell size) is **not** copied into every
//! cell it crosses; it goes to the separate `oversized` list, tested directly on
//! every query. That is the ADR-0012 §7 / spec §8.1 rule against a
//! city-block-sized fill being replicated across thousands of cells.
//!
//! The grid is sized **coarsely** on purpose: the chunk count is derived from
//! the entry count and a target entries-per-chunk
//! `TARGET_ENTRIES_PER_CHUNK` rather than the fixed 65 536-cell cap
//! [`crate::GridSpatialIndex`] uses. A fine grid gives each chunk only a handful
//! of entries and — because the loose single-cell rule cannot place an entry
//! that straddles a cell boundary — pushes a large fraction of the drawing into
//! the always-scanned oversized list. Coarsening the cells cuts the oversized
//! list by orders of magnitude on the measured fixture and makes the visited
//! chunk a meaningful unit (see the pick-index evidence note).
//!
//! Bottom level: each chunk holds a **flat payload list** (`ChunkEntry`) that
//! is scanned linearly when the chunk is visited. A query walks the top-level
//! grid, then each visited chunk's local list, and returns the candidate
//! payloads.
//!
//! # Per-chunk structure (measured deviation from ADR-0012 §7)
//!
//! ADR-0012 §7 names an `rstar` R-tree per chunk, built once with
//! `RTree::bulk_load` (a rebuild-style, immutable snapshot). That was
//! implemented and measured against the flat list on the committed
//! `complex-test.dwg` fixture, sweeping the cell size from ~256 to ~65536
//! entries per chunk. The per-chunk R-tree was a **net regression at every cell
//! size** — build time, warm pick median/max and RSS were all worse, with
//! candidate counts identical. The reason is that the pick tolerance is
//! comparable to the entry spacing, so a visited chunk's query envelope covers
//! most of that chunk and the R-tree has little to prune while still paying its
//! per-tree overhead; the flat scan is already close to optimal.
//!
//! The landed structure is therefore a flat per-chunk list. This is a
//! **measured, documented** deviation from the literal per-chunk-R-tree reading
//! of §7, permitted by the ADR's own measurement gate (§8: "串行先行，按实测再议
//! （§8.5 要求 profiling 先行）"; §10 S6 "按实测再评估"). The two-level shape the
//! ADR requires is unchanged: loose grid, single-cell placement (no duplication),
//! a separate oversized list, an immutable build-once snapshot, `Revision`-keyed
//! versioning and the conservative-superset / exhaustive-fallback semantics. The
//! cell-size sweep and before/after numbers live in the pick-index evidence note
//! (`docs/validation-pick-index-2026-10-17.md`).
//!
//! # Query (conservative broad phase)
//!
//! The query clips the ray's X/Y projection to the grid rectangle expanded by
//! the pick tolerance, expands that rectangle by the loose margin to get the
//! chunks to visit, and applies the **same** tolerance-expanded slab test the
//! precise stage uses ([`crate::pick::ray_intersects_expanded_bounds`]) to every
//! entry the local structure hands back. Oversized entries are tested directly.
//!
//! The result is conservative by construction: the test only rejects an entry
//! when the ray stays further than the tolerance from its AABB, and
//! non-finite arithmetic falls through to a candidate. A rejected entry can
//! therefore never have been hit, so the broad phase can never turn a hit into
//! a miss. `[BBox intersects query] ⟹ hit` is never assumed (ADR-0012 §2).

use cad_domain::*;

use crate::pick::{ray_intersects_expanded_bounds, validate_ray, MIN_RAY_DIRECTION};

/// Smallest grid cell extent; keeps `cell` finite and positive for a degenerate
/// (single-point) drawing extent.
const MIN_CELL_EXTENT: f64 = 1e-9;
/// Loose factor: a cell is expanded by this fraction of the cell size on every
/// side when an entry is assigned to it (2x loose bounds). A primitive whose
/// size is at most the cell size always fits in exactly one loose cell.
const LOOSE_HALF_CELL: f64 = 0.5;
/// Target number of entries per chunk for the pick grid. The chunk count is
/// derived from the entry count and this target instead of the fixed 65 536-cell
/// cap `GridSpatialIndex` uses: a chunk must be a meaningful unit, and a fine
/// grid pushes too many cell-straddling entries into the always-scanned
/// oversized list. Swept in the pick-index evidence note.
const TARGET_ENTRIES_PER_CHUNK: usize = 512;
/// Floor on the derived chunk count, so a tiny scene still gets a grid with
/// empty chunks instead of a degenerate single cell.
const MIN_CHUNKS: usize = 16;
/// Upper bound on the derived chunk count, so a pathological extent cannot
/// allocate an unbounded grid.
const MAX_CHUNKS: usize = 65536;
/// Upper bound per grid dimension.
const MAX_GRID_DIM: usize = 1024;

/// One indexed entry: its caller payload and its conservative world AABB.
#[derive(Clone, Copy)]
struct ChunkEntry {
    payload: u32,
    min: Point3,
    max: Point3,
}

impl ChunkEntry {
    fn new(payload: u32, bounds: &Bounds3) -> Self {
        ChunkEntry {
            payload,
            min: bounds.min,
            max: bounds.max,
        }
    }
}

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
    /// Chunk ordinal (`row * cols + col`) -> its entries (an empty vec is an
    /// empty chunk).
    chunks: Vec<Vec<ChunkEntry>>,
    /// Entries too large for a single loose cell, tested directly on every
    /// query (the ADR-0012 §7 oversized-primitive rule).
    oversized: Vec<ChunkEntry>,
    /// Number of indexed (bounded) entries, whether placed or oversized.
    entry_count: usize,
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
            oversized: Vec::new(),
            entry_count: 0,
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
        index.layout(entries);
        Ok(index)
    }

    /// Number of indexed (bounded) entries.
    pub fn entry_count(&self) -> usize {
        self.entry_count
    }

    /// Number of entries kept in the oversized list.
    pub fn oversized_count(&self) -> usize {
        self.oversized.len()
    }

    /// Number of chunks in the top-level grid.
    pub fn chunk_count(&self) -> usize {
        self.cols * self.rows
    }

    /// Entries stored in one chunk's local index (0 for an empty chunk). A
    /// diagnostic for tests and evidence.
    pub fn chunk_entry_count(&self, chunk: usize) -> usize {
        self.chunks.get(chunk).map_or(0, |c| c.len())
    }

    /// Total entries stored across the two-level structure (chunk locals plus
    /// the oversized list). With the loose-bound rule this equals
    /// [`Self::entry_count`]: no entry is duplicated across cells. A diagnostic
    /// for tests and evidence.
    pub fn stored_entry_count(&self) -> usize {
        self.chunks.iter().map(|c| c.len()).sum::<usize>() + self.oversized.len()
    }

    /// How many chunks' local lists hold `payload` (0, 1, or — only if the
    /// loose-bound rule were broken — more). A diagnostic for tests.
    pub fn chunk_placements(&self, payload: u32) -> usize {
        self.chunks
            .iter()
            .filter(|chunk| chunk.iter().any(|e| e.payload == payload))
            .count()
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

    fn layout(&mut self, entries: &[(u32, Bounds3)]) {
        self.entry_count = entries.len();
        self.oversized.clear();
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
        for (_, b) in entries {
            min.x = min.x.min(b.min.x);
            min.y = min.y.min(b.min.y);
            max.x = max.x.max(b.max.x);
            max.y = max.y.max(b.max.y);
        }
        if entries.is_empty() || !min.x.is_finite() || !max.x.is_finite() {
            self.cols = 0;
            self.rows = 0;
            self.chunks.clear();
            self.built = true;
            return;
        }
        let ex = (max.x - min.x).max(MIN_CELL_EXTENT);
        let ey = (max.y - min.y).max(MIN_CELL_EXTENT);
        // Coarse, pick-specific sizing: the chunk count follows the entry count
        // and a target entries-per-chunk (with a floor for tiny scenes), not the
        // fixed 65 536-cell cap `GridSpatialIndex` uses.
        let target =
            (entries.len() / TARGET_ENTRIES_PER_CHUNK).clamp(MIN_CHUNKS, MAX_CHUNKS) as f64;
        let aspect = (ex / ey).max(MIN_CELL_EXTENT);
        self.cols = ((target * aspect).sqrt().ceil() as usize).clamp(1, MAX_GRID_DIM);
        self.rows = ((target / self.cols as f64).ceil() as usize).clamp(1, MAX_GRID_DIM);
        self.cell = (ex / self.cols as f64)
            .max(ey / self.rows as f64)
            .max(MIN_CELL_EXTENT);
        self.min = min;

        // Assign every entry to exactly one loose cell, or to the oversized
        // list when it does not fit one.
        let half = self.cell * LOOSE_HALF_CELL;
        let mut acc: Vec<Vec<ChunkEntry>> = vec![Vec::new(); self.cols * self.rows];
        for (payload, bounds) in entries {
            let cx = 0.5 * (bounds.min.x + bounds.max.x);
            let cy = 0.5 * (bounds.min.y + bounds.max.y);
            let (c, r) = self.cell_of(cx, cy);
            let cell_lo_x = self.min.x + c as f64 * self.cell;
            let cell_lo_y = self.min.y + r as f64 * self.cell;
            let fits = bounds.min.x >= cell_lo_x - half
                && bounds.max.x <= cell_lo_x + self.cell + half
                && bounds.min.y >= cell_lo_y - half
                && bounds.max.y <= cell_lo_y + self.cell + half;
            let entry = ChunkEntry::new(*payload, bounds);
            if fits {
                acc[r * self.cols + c].push(entry);
            } else {
                self.oversized.push(entry);
            }
        }
        // Build-once, immutable snapshot (ADR-0012 §7): each chunk's list is
        // scanned linearly on a visit (see the module docs for the measured
        // deviation from the per-chunk-R-tree reading of §7).
        self.chunks = acc;
        self.built = true;
    }

    fn cell_of(&self, x: f64, y: f64) -> (usize, usize) {
        let cx = (((x - self.min.x) / self.cell).floor() as isize).clamp(0, self.cols as isize - 1);
        let cy = (((y - self.min.y) / self.cell).floor() as isize).clamp(0, self.rows as isize - 1);
        (cx as usize, cy as usize)
    }

    /// The chunks whose *loose* cells overlap a query rectangle, as an
    /// inclusive cell range. The rectangle is expanded by the loose margin
    /// (half a cell) because an entry is found only in its own cell, and its
    /// loose cell may reach half a cell beyond that cell's edge.
    fn loose_cell_range(&self, lo: Point3, hi: Point3) -> (usize, usize, usize, usize) {
        let half = self.cell * LOOSE_HALF_CELL;
        let (c0, r0) = self.cell_of(lo.x - half, lo.y - half);
        let (c1, r1) = self.cell_of(hi.x + half, hi.y + half);
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
        if !self.built || self.entry_count == 0 {
            return Ok(Vec::new());
        }
        let mut out: Vec<u32> = Vec::new();
        if let Some((lo, hi)) = self.query_region(ray, tolerance) {
            let (c0, r0, c1, r1) = self.loose_cell_range(lo, hi);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    for entry in &self.chunks[r * self.cols + c] {
                        self.test(entry, ray, tolerance, &mut out);
                    }
                }
            }
        }
        for entry in &self.oversized {
            self.test(entry, ray, tolerance, &mut out);
        }
        out.sort_unstable();
        out.dedup();
        Ok(out)
    }

    fn test(&self, entry: &ChunkEntry, ray: &Ray3, tolerance: f64, out: &mut Vec<u32>) {
        if ray_intersects_expanded_bounds(ray, entry.min, entry.max, tolerance) {
            out.push(entry.payload);
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
}
