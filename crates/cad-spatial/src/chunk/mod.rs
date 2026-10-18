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
//! Bottom level: each chunk holds a local **`rstar` R-tree** (ADR-0012 §7),
//! built once with [`rstar::RTree::bulk_load`] and immutable afterwards. A query
//! walks the top-level grid, then each visited chunk's local tree, and returns
//! the candidate payloads.
//!
//! # Per-chunk R-tree (landed, with its measured cost)
//!
//! ADR-0012 §7 specifies the per-chunk `rstar` R-tree. It was implemented and
//! measured against a flat per-chunk list on the committed `complex-test.dwg`
//! fixture and was a **net regression** there — build time, warm pick
//! median/max and RSS were all worse, with candidate counts identical. The
//! reason is structural: the pick tolerance is comparable to the entry spacing,
//! so a visited chunk's query envelope covers most of that chunk and the tree
//! has little to prune while still paying its per-tree overhead.
//!
//! The dependency was nevertheless landed on an explicit product decision
//! ("全面落地 rstar") because ADR conformance is required; the measured cost is
//! accepted and recorded, not hidden. The two-level shape and every other §7
//! requirement are unchanged: loose grid, single-cell placement (no
//! duplication), a separate oversized list, an immutable build-once snapshot,
//! `Revision`-keyed versioning and the conservative-superset /
//! exhaustive-fallback semantics. The cell-size sweep, the node-size tuning and
//! the before/after numbers live in the pick-index evidence note
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
use rstar::{RStarInsertionStrategy, RTree, RTreeObject, RTreeParams, AABB};

use crate::grid::{self, GridLayout, GridSizing, MIN_CELL_EXTENT};
use crate::pick::{ray_intersects_expanded_bounds, validate_ray, MIN_RAY_DIRECTION};

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

/// The X/Y envelope the per-chunk R-tree indexes. The pick grid is planar, so
/// the local tree is 2D; the exact test then runs on the full 3D box.
impl RTreeObject for ChunkEntry {
    type Envelope = AABB<[f64; 2]>;

    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners([self.min.x, self.min.y], [self.max.x, self.max.y])
    }
}

/// rstar node-size parameters for a chunk's local tree (ADR-0012 §7).
///
/// `bulk_load` (OMT) builds the tree with this fan-out; `MAX_SIZE` trades query
/// branching for cache locality. The library default (`MIN_SIZE = 3`,
/// `MAX_SIZE = 6`) pays per-node overhead a chunk-sized tree cannot earn back
/// here (the pick tolerance is comparable to the entry spacing, so the tree
/// barely prunes), and a **larger** node is measurably better on every axis:
/// build, RSS and warm pick all improve up to `MAX_SIZE ≈ 512`, then flatten.
/// `(256, 1024)` was the best balanced point in the sweep; it brings warm pick
/// to parity with the flat list while roughly halving the build/RSS regression
/// of the default parameters. `REINSERTION_COUNT` only affects incremental
/// `insert`, which the build-once snapshot never calls. Numbers live in the
/// pick-index evidence note (`docs/validation-pick-index-2026-10-17.md`).
struct PickTreeParams;

impl RTreeParams for PickTreeParams {
    const MIN_SIZE: usize = 256;
    const MAX_SIZE: usize = 1024;
    const REINSERTION_COUNT: usize = 256;
    type DefaultInsertionStrategy = RStarInsertionStrategy;
}

/// A chunk's local R-tree: built once with `bulk_load`, immutable afterwards.
type ChunkTree = RTree<ChunkEntry, PickTreeParams>;

/// Build one chunk's immutable local R-tree.
fn build_chunk_tree(entries: Vec<ChunkEntry>) -> ChunkTree {
    RTree::bulk_load_with_params(entries)
}

/// A two-level chunk-grid broad phase keyed by opaque `u32` payloads.
///
/// The payload is whatever the caller uses to identify an entry (here: the
/// item's expansion ordinal). Payloads are returned in ascending order so the
/// caller's stable tie handling is preserved.
pub struct ChunkGridIndex {
    /// Shared X/Y grid arithmetic (see [`crate::grid`]); empty when the index
    /// has no entries.
    layout: GridLayout,
    /// Chunk ordinal (`row * cols + col`) -> its local R-tree. An empty chunk is
    /// `None` so it allocates nothing (rstar's empty `RTree` still allocates a
    /// root node, and a coarse grid leaves most chunks empty).
    chunks: Vec<Option<ChunkTree>>,
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
            layout: GridLayout::empty(),
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
        self.layout.cell_count()
    }

    /// Number of chunks that hold a non-empty local R-tree. A diagnostic for
    /// tests and evidence.
    pub fn rstar_chunks(&self) -> usize {
        self.chunks
            .iter()
            .filter(|tree| tree.as_ref().is_some_and(|t| t.size() > 0))
            .count()
    }

    /// Entries stored in one chunk's local index (0 for an empty chunk). A
    /// diagnostic for tests and evidence.
    pub fn chunk_entry_count(&self, chunk: usize) -> usize {
        self.chunks
            .get(chunk)
            .and_then(|tree| tree.as_ref())
            .map_or(0, |tree| tree.size())
    }

    /// Total entries stored across the two-level structure (chunk locals plus
    /// the oversized list). With the loose-bound rule this equals
    /// [`Self::entry_count`]: no entry is duplicated across cells. A diagnostic
    /// for tests and evidence.
    pub fn stored_entry_count(&self) -> usize {
        self.chunks
            .iter()
            .flatten()
            .map(|tree| tree.size())
            .sum::<usize>()
            + self.oversized.len()
    }

    /// How many chunks' local trees hold `payload` (0, 1, or — only if the
    /// loose-bound rule were broken — more). A diagnostic for tests.
    pub fn chunk_placements(&self, payload: u32) -> usize {
        self.chunks
            .iter()
            .flatten()
            .filter(|tree| tree.iter().any(|e| e.payload == payload))
            .count()
    }

    /// Grid origin (X/Y); a diagnostic for tests and evidence.
    pub fn grid_origin(&self) -> Point3 {
        self.layout.origin()
    }

    /// Grid cell size (X/Y); a diagnostic for tests and evidence.
    pub fn cell_size(&self) -> f64 {
        self.layout.cell
    }

    /// Grid dimensions `(cols, rows)`; a diagnostic for tests and evidence.
    pub fn grid_dimensions(&self) -> (usize, usize) {
        (self.layout.cols, self.layout.rows)
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
            self.layout = GridLayout::empty();
            self.chunks.clear();
            self.built = true;
            return;
        }
        // Coarse, pick-specific sizing: the chunk count follows the entry count
        // and a target entries-per-chunk (with a floor for tiny scenes), not the
        // fixed 65 536-cell cap `GridSpatialIndex` uses. The grid arithmetic
        // itself is shared (`crate::grid`).
        let target =
            (entries.len() / TARGET_ENTRIES_PER_CHUNK).clamp(MIN_CHUNKS, MAX_CHUNKS) as f64;
        self.layout = grid::layout(
            min,
            max,
            target,
            GridSizing {
                extent_eps: MIN_CELL_EXTENT,
                cell_eps: MIN_CELL_EXTENT,
                max_dim: MAX_GRID_DIM,
            },
        );
        let cols = self.layout.cols;
        let cell = self.layout.cell;

        // Assign every entry to exactly one loose cell, or to the oversized
        // list when it does not fit one.
        let half = cell * LOOSE_HALF_CELL;
        let mut acc: Vec<Vec<ChunkEntry>> = vec![Vec::new(); self.layout.cell_count()];
        for (payload, bounds) in entries {
            let cx = 0.5 * (bounds.min.x + bounds.max.x);
            let cy = 0.5 * (bounds.min.y + bounds.max.y);
            let (c, r) = self.layout.cell_of(cx, cy);
            let (cell_lo_x, cell_lo_y) = self.layout.cell_origin(c, r);
            let fits = bounds.min.x >= cell_lo_x - half
                && bounds.max.x <= cell_lo_x + cell + half
                && bounds.min.y >= cell_lo_y - half
                && bounds.max.y <= cell_lo_y + cell + half;
            let entry = ChunkEntry::new(*payload, bounds);
            if fits {
                acc[r * cols + c].push(entry);
            } else {
                self.oversized.push(entry);
            }
        }
        // Build-once, immutable snapshot (ADR-0012 §7): each chunk becomes an
        // `rstar` R-tree built with `bulk_load`, immutable afterwards.
        self.chunks = acc
            .into_iter()
            .map(|entries| (!entries.is_empty()).then(|| build_chunk_tree(entries)))
            .collect();
        self.built = true;
    }

    /// The chunks whose *loose* cells overlap a query rectangle, as an
    /// inclusive cell range. The rectangle is expanded by the loose margin
    /// (half a cell) because an entry is found only in its own cell, and its
    /// loose cell may reach half a cell beyond that cell's edge.
    fn loose_cell_range(&self, lo: Point3, hi: Point3) -> (usize, usize, usize, usize) {
        let half = self.layout.cell * LOOSE_HALF_CELL;
        let (c0, r0) = self.layout.cell_of(lo.x - half, lo.y - half);
        let (c1, r1) = self.layout.cell_of(hi.x + half, hi.y + half);
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
            // Each visited chunk's local R-tree is queried with the X/Y envelope
            // of the query region. An entry whose 2D envelope does not meet that
            // region cannot pass the 3D tolerance-expanded test, so this only
            // removes work; the exact predicate still decides every candidate.
            let envelope = AABB::from_corners([lo.x, lo.y], [hi.x, hi.y]);
            let (c0, r0, c1, r1) = self.loose_cell_range(lo, hi);
            let cols = self.layout.cols;
            for r in r0..=r1 {
                for c in c0..=c1 {
                    let Some(tree) = &self.chunks[r * cols + c] else {
                        continue;
                    };
                    for entry in tree.locate_in_envelope_intersecting(envelope) {
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
            x: self.layout.origin_x - tolerance,
            y: self.layout.origin_y - tolerance,
            z: 0.0,
        };
        let hi = Point3 {
            x: self.layout.origin_x + self.layout.cols as f64 * self.layout.cell + tolerance,
            y: self.layout.origin_y + self.layout.rows as f64 * self.layout.cell + tolerance,
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
mod tests;
