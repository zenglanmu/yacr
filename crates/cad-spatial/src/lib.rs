//! CPU candidate lookup remains available to the WebGL2 path.
//!
//! Spec v2.0 §8.5: basic picking must not depend on GPU readback. This uniform
//! grid answers viewport-bounds and ray candidate queries without scanning the
//! whole drawing.

use cad_domain::*;

pub mod pick;
pub use pick::{
    hit_geometry, pick_closest, ray_segment_closest, ray_triangle, validate_ray, BackFacePolicy,
    GeometryHit, PickItem, PickOptions, PickOutcome, PickReport, SkippedGeometry,
    MAX_PICK_SEGMENTS, MIN_RAY_DIRECTION,
};

#[derive(Debug, Clone)]
pub struct SpatialEntry {
    pub source: SelectionRef,
    pub bounds: Bounds3,
}

/// A precise pick: the selected object, the world point, and the depth.
#[derive(Debug, Clone, PartialEq)]
pub struct PickHit {
    pub source: SelectionRef,
    pub point: Point3,
    /// Distance along the unit pick ray; the primary hit ordering.
    pub distance: f64,
    /// Perpendicular distance from the ray; the tie-break at equal depth.
    pub offset: f64,
    pub precision: Precision,
    pub geometry_source: GeometrySource,
}

pub trait SpatialIndex {
    fn rebuild(&mut self, entries: &[SpatialEntry]) -> CadResult<()>;
    fn update(&mut self, inserted: &[SpatialEntry], removed: &[SelectionRef]) -> CadResult<()>;
    fn query_bounds(&self, bounds: &Bounds3) -> CadResult<Vec<SelectionRef>>;
    fn ray_candidates(&self, ray: &Ray3) -> CadResult<Vec<SelectionRef>>;
}

/// Grid-based spatial index.
pub struct GridSpatialIndex {
    entries: Vec<SpatialEntry>,
    min: Point3,
    cell: f64,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
    built: bool,
}

impl Default for GridSpatialIndex {
    fn default() -> Self {
        GridSpatialIndex {
            entries: Vec::new(),
            min: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            cell: 1.0,
            cols: 0,
            rows: 0,
            cells: Vec::new(),
            built: false,
        }
    }
}

impl GridSpatialIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    fn layout(&mut self, target_cells: usize) {
        let mut min = Point3 {
            x: f64::INFINITY,
            y: f64::INFINITY,
            z: f64::INFINITY,
        };
        let mut max = Point3 {
            x: f64::NEG_INFINITY,
            y: f64::NEG_INFINITY,
            z: f64::NEG_INFINITY,
        };
        for e in &self.entries {
            min = Point3 {
                x: min.x.min(e.bounds.min.x),
                y: min.y.min(e.bounds.min.y),
                z: min.z.min(e.bounds.min.z),
            };
            max = Point3 {
                x: max.x.max(e.bounds.max.x),
                y: max.y.max(e.bounds.max.y),
                z: max.z.max(e.bounds.max.z),
            };
        }
        if !min.x.is_finite() || !max.x.is_finite() {
            self.cols = 0;
            self.rows = 0;
            self.cells.clear();
            self.built = true;
            return;
        }
        let ex = (max.x - min.x).max(1e-6);
        let ey = (max.y - min.y).max(1e-6);
        let target = target_cells.max(1) as f64;
        let aspect = (ex / ey).max(1e-6);
        self.cols = ((target * aspect).sqrt().ceil() as usize).clamp(1, 1024);
        self.rows = ((target / self.cols as f64).ceil() as usize).clamp(1, 1024);
        self.cell = (ex / self.cols as f64).max(ey / self.rows as f64).max(1e-9);
        self.min = min;
        self.cells = vec![Vec::new(); self.cols * self.rows];
        for (i, e) in self.entries.iter().enumerate() {
            let (c0, r0) = self.cell_of(e.bounds.min.x, e.bounds.min.y);
            let (c1, r1) = self.cell_of(e.bounds.max.x, e.bounds.max.y);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    self.cells[r * self.cols + c].push(i as u32);
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

    fn query_cells(&self, b: &Bounds3) -> Vec<SelectionRef> {
        if !self.built || self.cells.is_empty() {
            return Vec::new();
        }
        let (c0, r0) = self.cell_of(b.min.x, b.min.y);
        let (c1, r1) = self.cell_of(b.max.x, b.max.y);
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for r in r0..=r1 {
            for c in c0..=c1 {
                for &idx in &self.cells[r * self.cols + c] {
                    if seen.insert(idx) {
                        let e = &self.entries[idx as usize];
                        if intersects(&e.bounds, b) {
                            out.push(e.source.clone());
                        }
                    }
                }
            }
        }
        out
    }
}

impl SpatialIndex for GridSpatialIndex {
    fn rebuild(&mut self, entries: &[SpatialEntry]) -> CadResult<()> {
        self.entries = entries.to_vec();
        let target = (entries.len() / 4).clamp(16, 65536);
        self.layout(target);
        Ok(())
    }

    fn update(&mut self, inserted: &[SpatialEntry], removed: &[SelectionRef]) -> CadResult<()> {
        if !removed.is_empty() {
            let removed: std::collections::BTreeSet<_> = removed.iter().map(source_key).collect();
            self.entries
                .retain(|e| !removed.contains(&source_key(&e.source)));
        }
        self.entries.extend(inserted.iter().map(|e| SpatialEntry {
            source: e.source.clone(),
            bounds: e.bounds,
        }));
        let target = (self.entries.len() / 4).clamp(16, 65536);
        self.layout(target);
        Ok(())
    }

    fn query_bounds(&self, bounds: &Bounds3) -> CadResult<Vec<SelectionRef>> {
        Ok(self.query_cells(bounds))
    }

    fn ray_candidates(&self, ray: &Ray3) -> CadResult<Vec<SelectionRef>> {
        if !self.built || self.entries.is_empty() {
            return Ok(Vec::new());
        }
        // Broad phase: entries whose AABB the ray passes within a generous
        // radius. Precise hit tests happen against the actual geometry later.
        let mut out = Vec::new();
        for e in &self.entries {
            if ray_hits_aabb(ray, &e.bounds) {
                out.push(e.source.clone());
            }
        }
        Ok(out)
    }
}

fn source_key(s: &SelectionRef) -> (DocumentId, EntityId, Vec<(EntityId, String)>) {
    (
        s.document,
        s.entity,
        s.instance.0.iter().map(|id| (*id, String::new())).collect(),
    )
}

fn intersects(a: &Bounds3, b: &Bounds3) -> bool {
    a.min.x <= b.max.x
        && a.max.x >= b.min.x
        && a.min.y <= b.max.y
        && a.max.y >= b.min.y
        && a.min.z <= b.max.z
        && a.max.z >= b.min.z
}

/// Slab test for a ray against an axis-aligned box.
fn ray_hits_aabb(ray: &Ray3, b: &Bounds3) -> bool {
    let mut tmin = f64::NEG_INFINITY;
    let mut tmax = f64::INFINITY;
    for axis in 0..3 {
        let (o, d, lo, hi) = match axis {
            0 => (ray.origin.x, ray.direction.x, b.min.x, b.max.x),
            1 => (ray.origin.y, ray.direction.y, b.min.y, b.max.y),
            _ => (ray.origin.z, ray.direction.z, b.min.z, b.max.z),
        };
        if d.abs() < 1e-12 {
            if o < lo || o > hi {
                return false;
            }
        } else {
            let mut t1 = (lo - o) / d;
            let mut t2 = (hi - o) / d;
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }
            tmin = tmin.max(t1);
            tmax = tmax.min(t2);
            if tmin > tmax {
                return false;
            }
        }
    }
    tmax >= 0.0
}

/// Placeholder kept for hosts that have not installed a real index.
pub struct PendingSpatialIndex;

impl SpatialIndex for PendingSpatialIndex {
    fn rebuild(&mut self, _: &[SpatialEntry]) -> CadResult<()> {
        Err(CadError::NotImplemented("spatial.rebuild"))
    }
    fn update(&mut self, _: &[SpatialEntry], _: &[SelectionRef]) -> CadResult<()> {
        Err(CadError::NotImplemented("spatial.update"))
    }
    fn query_bounds(&self, _: &Bounds3) -> CadResult<Vec<SelectionRef>> {
        Err(CadError::NotImplemented("spatial.query_bounds"))
    }
    fn ray_candidates(&self, _: &Ray3) -> CadResult<Vec<SelectionRef>> {
        Err(CadError::NotImplemented("spatial.ray_candidates"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u128, x: f64, y: f64) -> SpatialEntry {
        SpatialEntry {
            source: SelectionRef {
                document: DocumentId(1),
                entity: EntityId(id),
                instance: InstancePath::default(),
                sub_element: None,
            },
            bounds: Bounds3 {
                min: Point3 {
                    x: x - 1.0,
                    y: y - 1.0,
                    z: -1.0,
                },
                max: Point3 {
                    x: x + 1.0,
                    y: y + 1.0,
                    z: 1.0,
                },
            },
        }
    }

    #[test]
    fn query_returns_only_intersecting_entries() {
        let mut index = GridSpatialIndex::new();
        index
            .rebuild(&[entry(1, 0.0, 0.0), entry(2, 100.0, 100.0)])
            .unwrap();
        let hits = index
            .query_bounds(&Bounds3 {
                min: Point3 {
                    x: -5.0,
                    y: -5.0,
                    z: -5.0,
                },
                max: Point3 {
                    x: 5.0,
                    y: 5.0,
                    z: 5.0,
                },
            })
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entity, EntityId(1));
    }

    #[test]
    fn ray_candidates_use_aabb_slab_test() {
        let mut index = GridSpatialIndex::new();
        index
            .rebuild(&[entry(1, 0.0, 0.0), entry(2, 50.0, 0.0)])
            .unwrap();
        let ray = Ray3 {
            origin: Point3 {
                x: -10.0,
                y: 0.0,
                z: 0.0,
            },
            direction: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        };
        let hits = index.ray_candidates(&ray).unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn update_removes_by_source() {
        let mut index = GridSpatialIndex::new();
        let e = entry(1, 0.0, 0.0);
        index.rebuild(std::slice::from_ref(&e)).unwrap();
        index.update(&[], std::slice::from_ref(&e.source)).unwrap();
        assert_eq!(index.entry_count(), 0);
    }
}
