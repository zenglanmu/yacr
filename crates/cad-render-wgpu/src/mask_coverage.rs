use cad_domain::Point3;

/// An axis-aligned world-XY rectangle.
///
/// Used to describe both the visible world rect of a frame and the region an
/// inverted mask's complement fill covers. `min`/`max` are per-axis bounds in
/// world units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MaskCoverage {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl MaskCoverage {
    pub(crate) fn new(min: [f64; 2], max: [f64; 2]) -> Self {
        Self { min, max }
    }

    /// A square of half-extent `radius` centred on `center` (world XY).
    pub(crate) fn around(center: [f64; 2], radius: f64) -> Self {
        Self {
            min: [center[0] - radius, center[1] - radius],
            max: [center[0] + radius, center[1] + radius],
        }
    }

    /// Axis-aligned bounds of `boundary`; a square of half-extent 1 at the origin
    /// when the boundary has no finite extent (never used to clip, only to grow a
    /// ring, so the fallback is harmless and non-finite values cannot escape).
    pub(crate) fn boundary(boundary: &[Point3]) -> Self {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for point in boundary {
            if !(point.x.is_finite() && point.y.is_finite()) {
                return Self::around([0.0, 0.0], 1.0);
            }
            min[0] = min[0].min(point.x);
            min[1] = min[1].min(point.y);
            max[0] = max[0].max(point.x);
            max[1] = max[1].max(point.y);
        }
        if min[0] > max[0] || min[1] > max[1] {
            return Self::around([0.0, 0.0], 1.0);
        }
        Self { min, max }
    }

    pub(crate) fn union(self, other: Self) -> Self {
        Self {
            min: [self.min[0].min(other.min[0]), self.min[1].min(other.min[1])],
            max: [self.max[0].max(other.max[0]), self.max[1].max(other.max[1])],
        }
    }

    /// Whether `other` lies entirely inside `self` (inclusive bounds).
    pub(crate) fn contains(self, other: Self) -> bool {
        self.min[0] <= other.min[0]
            && self.min[1] <= other.min[1]
            && self.max[0] >= other.max[0]
            && self.max[1] >= other.max[1]
    }

    pub(crate) fn expanded(self, margin: f64) -> Self {
        Self {
            min: [self.min[0] - margin, self.min[1] - margin],
            max: [self.max[0] + margin, self.max[1] + margin],
        }
    }

    pub(crate) fn is_finite(self) -> bool {
        [self.min[0], self.min[1], self.max[0], self.max[1]]
            .iter()
            .all(|value| value.is_finite())
    }
}
