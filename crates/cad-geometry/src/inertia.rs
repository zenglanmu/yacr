//! Exact planar second moments and the centroidal inertia tensor they define.
//!
//! A closed planar region has area moments `∫x² dA`, `∫y² dA` and `∫xy dA`
//! about any axis in its plane. They are the 2D analogue of a body's moments of
//! inertia, and `MASSPROP` reports them alongside the area and centroid. Every
//! value here is **closed form**:
//!
//! * straight edges use Green's theorem on the polygon (the same closed ring
//!   [`crate::measure_bulged_ring_area`] measures), so the accumulation is exact
//!   to floating point and involves **no sampling or quadrature**;
//! * every nonzero bulge adds one circular segment (`sector − isosceles
//!   triangle`, [`bulged_ring_moments`]), whose area, first moment and second
//!   moments are exact as well.
//!
//! A caller that samples a curve before calling this module (a closed B-spline,
//! an imported hatch boundary the importer flattened) is integrating the **sample
//! polygon**; such a result must be labelled approximate by that caller. Nothing
//! in this module samples anything.
//!
//! ## Frame and sign conventions
//!
//! A region is integrated in a 2D frame `(x, y)` with `x` along the frame's `u`
//! axis and `y` along its `v` axis. [`PlanarMoments`] is **signed**: a clockwise
//! ring yields a negative [`PlanarMoments::area`] and negative moments (that is
//! what makes a hatch's hole subtract instead of add). Call
//! [`PlanarMoments::with_positive_area`] to get the geometric convention, then
//! [`PlanarMoments::centroidal`] applies the parallel-axis theorem to move the
//! moments from the frame origin to the region's own centroid.
//!
//! [`crate::PlanarInertia::principal`] diagonalises the centroidal tensor with
//! the crate's single 2×2 symmetric eigen-solver
//! ([`crate::vector::symmetric_eigen_2x2`], the one the affine ellipse
//! decomposition uses), so no second eigen implementation exists.

use super::*;

/// The signed planar moments of one region about the origin of one 2D frame.
///
/// See the module docs for the sign convention. The second moments are about
/// the **frame origin**, not the centroid; [`Self::centroidal`] shifts them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanarMoments {
    /// `∬ dA` (signed by the ring's winding).
    pub area: f64,
    /// `∬ x dA`.
    pub first_x: f64,
    /// `∬ y dA`.
    pub first_y: f64,
    /// `∬ x² dA`.
    pub second_x: f64,
    /// `∬ y² dA`.
    pub second_y: f64,
    /// `∬ xy dA`.
    pub second_xy: f64,
}

impl PlanarMoments {
    /// Every moment zero (the additive identity of [`Self::add`]).
    pub const ZERO: Self = Self {
        area: 0.0,
        first_x: 0.0,
        first_y: 0.0,
        second_x: 0.0,
        second_y: 0.0,
        second_xy: 0.0,
    };

    /// Every moment multiplied by `factor` (a uniform scale, or `-1.0` to flip
    /// the sign convention).
    pub fn scaled(self, factor: f64) -> Self {
        Self {
            area: self.area * factor,
            first_x: self.first_x * factor,
            first_y: self.first_y * factor,
            second_x: self.second_x * factor,
            second_y: self.second_y * factor,
            second_xy: self.second_xy * factor,
        }
    }

    /// The geometric (positive-area) convention: flip every moment when the
    /// signed area came out negative.
    ///
    /// A second moment is a physical magnitude, so a caller that only wants the
    /// geometry (not a hatch's hole subtraction) normalises the winding away
    /// here instead of taking an absolute value per term.
    pub fn with_positive_area(self) -> Self {
        if self.area < 0.0 {
            self.scaled(-1.0)
        } else {
            self
        }
    }

    /// Whether every moment is finite.
    pub fn is_finite(self) -> bool {
        self.area.is_finite()
            && self.first_x.is_finite()
            && self.first_y.is_finite()
            && self.second_x.is_finite()
            && self.second_y.is_finite()
            && self.second_xy.is_finite()
    }

    /// The region's centroid in the frame, or `None` when the area is zero (or
    /// underflowed to nothing, the `AREA OBJECT` zero-area rule).
    pub fn centroid(self) -> Option<[f64; 2]> {
        if !self.area.is_finite() || self.area.abs() < 1e-15 {
            return None;
        }
        let centroid = [self.first_x / self.area, self.first_y / self.area];
        if !centroid[0].is_finite() || !centroid[1].is_finite() {
            return None;
        }
        Some(centroid)
    }

    /// The centroidal inertia tensor, by the parallel-axis theorem:
    /// `Ix = ∬y² dA − A·yc²`, `Iy = ∬x² dA − A·xc²`,
    /// `Ixy = ∬xy dA − A·xc·yc`.
    ///
    /// `None` when the area is zero (see [`Self::centroid`]).
    pub fn centroidal(self) -> Option<PlanarInertia> {
        let centroid = self.centroid()?;
        let area = self.area;
        let [cx, cy] = centroid;
        Some(PlanarInertia {
            area,
            centroid,
            inertia_x: self.second_y - area * cy * cy,
            inertia_y: self.second_x - area * cx * cx,
            product_xy: self.second_xy - area * cx * cy,
        })
    }
}

impl std::ops::Add for PlanarMoments {
    type Output = Self;

    /// Sum two regions' moments measured in the **same** frame. This is how a
    /// hatch's loops (signed by nesting parity) and a multi-region selection are
    /// combined; the frames must already agree (see [`reexpress`]).
    fn add(self, other: Self) -> Self {
        Self {
            area: self.area + other.area,
            first_x: self.first_x + other.first_x,
            first_y: self.first_y + other.first_y,
            second_x: self.second_x + other.second_x,
            second_y: self.second_y + other.second_y,
            second_xy: self.second_xy + other.second_xy,
        }
    }
}

/// A region's centroidal second moments, i.e. its 2D inertia tensor.
///
/// `inertia_x` is the second moment **about** the centroidal axis parallel to
/// the frame's `u` direction (`∬(y−yc)² dA`), `inertia_y` about the axis
/// parallel to `v` (`∬(x−xc)² dA`), and `product_xy` is `∬(x−xc)(y−yc) dA`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanarInertia {
    /// The region's signed area (positive after
    /// [`PlanarMoments::with_positive_area`], negative for a hatch hole sum).
    pub area: f64,
    /// The region's centroid, in the frame.
    pub centroid: [f64; 2],
    /// `∬(y−yc)² dA`.
    pub inertia_x: f64,
    /// `∬(x−xc)² dA`.
    pub inertia_y: f64,
    /// `∬(x−xc)(y−yc) dA`.
    pub product_xy: f64,
}

impl PlanarInertia {
    /// The polar second moment `J = Ix + Iy` (the moment about the axis normal
    /// to the plane through the centroid).
    pub fn polar(&self) -> f64 {
        self.inertia_x + self.inertia_y
    }

    /// The radius of gyration about the centroidal `u`-parallel axis:
    /// `kx = sqrt(Ix / A)`.
    pub fn gyration_x(&self) -> Option<f64> {
        self.gyration(self.inertia_x)
    }

    /// The radius of gyration about the centroidal `v`-parallel axis:
    /// `ky = sqrt(Iy / A)`.
    pub fn gyration_y(&self) -> Option<f64> {
        self.gyration(self.inertia_y)
    }

    /// The radius of gyration about the polar axis: `kz = sqrt(J / A)`.
    pub fn gyration_polar(&self) -> Option<f64> {
        self.gyration(self.polar())
    }

    fn gyration(&self, second_moment: f64) -> Option<f64> {
        if !self.area.is_finite() || self.area <= 0.0 || !second_moment.is_finite() {
            return None;
        }
        let value = (second_moment / self.area).sqrt();
        (value.is_finite() && value >= 0.0).then_some(value)
    }

    /// Whether the tensor is a real region's (finite, positive semi-definite).
    ///
    /// A region's second moments always satisfy `Ix >= 0`, `Iy >= 0` and
    /// `Ix·Iy − Ixy² >= 0`; a violation means the accumulated arithmetic went
    /// wrong and the caller must refuse rather than print the numbers.
    pub fn is_finite_and_valid(&self) -> bool {
        let finite = self.area.is_finite()
            && self.centroid[0].is_finite()
            && self.centroid[1].is_finite()
            && self.inertia_x.is_finite()
            && self.inertia_y.is_finite()
            && self.product_xy.is_finite();
        if !finite || self.area < 0.0 || self.inertia_x < 0.0 || self.inertia_y < 0.0 {
            return false;
        }
        let determinant = self.inertia_x * self.inertia_y - self.product_xy * self.product_xy;
        determinant >= -1e-9 * (self.inertia_x * self.inertia_y).abs()
    }

    /// The principal second moments and the axes they are measured about.
    ///
    /// The eigen-decomposition is of the second-moment matrix
    /// `[[Ix, −Ixy], [−Ixy, Iy]]`, whose quadratic form `nᵀMn` is the second
    /// moment about the direction `n`; its eigenvalues are therefore the
    /// extremal second moments and its eigenvectors the axes they belong to.
    /// `major` is the larger of the two and is paired with `major_axis`.
    ///
    /// `None` when a principal value is not finite (see
    /// [`Self::is_finite_and_valid`] for the full validity rule).
    pub fn principal(&self) -> Option<PrincipalAxes> {
        let eigen =
            crate::vector::symmetric_eigen_2x2(self.inertia_x, -self.product_xy, self.inertia_y);
        if !eigen.major.is_finite() || !eigen.minor.is_finite() {
            return None;
        }
        let (ax, ay) = eigen.axis;
        if !ax.is_finite() || !ay.is_finite() {
            return None;
        }
        Some(PrincipalAxes {
            major: eigen.major,
            minor: eigen.minor,
            major_axis: [ax, ay],
            angle: ay.atan2(ax),
        })
    }
}

/// The principal second moments of a [`PlanarInertia`] and their axes.
///
/// Both axes are directions, not orientations: `n` and `−n` describe the same
/// axis. `major >= minor` always holds, and `major + minor == Ix + Iy` (the
/// polar moment is invariant).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrincipalAxes {
    /// The larger principal second moment.
    pub major: f64,
    /// The smaller principal second moment.
    pub minor: f64,
    /// Unit direction, in the frame's `(x, y)` coordinates, of the axis
    /// `major` is measured about.
    pub major_axis: [f64; 2],
    /// `atan2(major_axis[1], major_axis[0])`, in radians.
    pub angle: f64,
}

impl PrincipalAxes {
    /// The other principal axis (a quarter turn from [`Self::major_axis`]).
    pub fn minor_axis(&self) -> [f64; 2] {
        [-self.major_axis[1], self.major_axis[0]]
    }
}

/// The **exact** planar moments of a plain polygon ring (no bulges).
///
/// Closed-form Green's-theorem accumulation over the closed ring `flat` (which
/// must not repeat its first vertex). The second moments are about the frame
/// origin; `flat` may be any winding (see [`PlanarMoments::with_positive_area`]).
pub fn polygon_moments(flat: &[[f64; 2]]) -> PlanarMoments {
    let mut moments = PlanarMoments::ZERO;
    let count = flat.len();
    if count < 3 {
        return moments;
    }
    for index in 0..count {
        let a = flat[index];
        let b = flat[(index + 1) % count];
        let cross = a[0] * b[1] - b[0] * a[1];
        moments.area += 0.5 * cross;
        moments.first_x += (a[0] + b[0]) * cross / 6.0;
        moments.first_y += (a[1] + b[1]) * cross / 6.0;
        moments.second_x += (a[0] * a[0] + a[0] * b[0] + b[0] * b[0]) * cross / 12.0;
        moments.second_y += (a[1] * a[1] + a[1] * b[1] + b[1] * b[1]) * cross / 12.0;
        moments.second_xy +=
            cross * (2.0 * a[0] * a[1] + a[0] * b[1] + b[0] * a[1] + 2.0 * b[0] * b[1]) / 24.0;
    }
    moments
}

/// Below this **signed included angle** (|θ| < 1e-3 rad) the circular-segment
/// area and centroid are evaluated by their series, exactly like
/// [`crate::measure_bulged_ring_area`] does: `θ − sin θ` loses every significant
/// digit to cancellation there while the series stays exact to double precision.
const BULGE_SEGMENT_SERIES_RADIANS: f64 = 1e-3;

/// Below this **half-angle** (`α = |θ|/2 < 1e-2 rad`) the circular segment's
/// centroidal second moments are evaluated by their series.
///
/// The second moments are `O(R⁴α³)` while the closed form subtracts `O(R⁴α)`
/// values, so the cancellation (`~1/α²` in relative terms) becomes visible one
/// order earlier than it does for the area. The branch cuts over well before the
/// exact form's absolute error (`~ε·R⁴/α`) can reach the printing precision, and
/// its own truncation (`O(R⁴α⁹)`) is far below it.
const CAP_SECOND_MOMENT_SERIES_RADIANS: f64 = 1e-2;

/// The **exact** planar moments of a closed, bulged planar ring in its own 2D
/// frame.
///
/// `flat[i]` are the ring's vertices in the 2D frame (`x` along `u`, `y` along
/// `v`) and `bulges[i]` the bulge of the segment `i → i + 1` (the last one
/// closing the ring), the `SemanticGeometry::Polyline` convention: one bulge per
/// vertex, no duplicated closing vertex, `bulge = tan(θ/4)`.
///
/// The straight-edge part is [`polygon_moments`]. Each nonzero bulge adds the
/// circular segment between its chord and its arc, exactly:
///
/// * the segment is the circular **sector** (angle `|θ|`, radius `R`) minus the
///   **isosceles triangle** over the same chord, so its moments about the arc
///   centre are `R⁴/4·(|θ|/2 ± sin(|θ|/2)cos(|θ|/2))` minus the triangle's
///   `R⁴·s·c³/2` and `R⁴·s³·c/6`;
/// * its centroid lies on the arc's apex direction at
///   `2R·sin³(|θ|/2) / (3(|θ|/2 − sin(|θ|/2)cos(|θ|/2)))` from the centre (the
///   classic `4R/3π` for a semicircle), and the centroidal tensor is then
///   rotated into the ring frame and shifted onto that centroid;
/// * a negative bulge mirrors the segment (its apex direction flips) and enters
///   with a **negative** signed area/moment, exactly as it subtracts from the
///   ring's area.
///
/// Returns `None` (never a guessed value) for a bulge/vertex count mismatch,
/// fewer than three vertices, a non-finite coordinate, or a degenerate chord
/// (zero length, or a sagitta that does not define a circle).
pub fn bulged_ring_moments(flat: &[[f64; 2]], bulges: &[f64]) -> Option<PlanarMoments> {
    if flat.len() != bulges.len() || flat.len() < 3 {
        return None;
    }
    if flat
        .iter()
        .any(|point| !point[0].is_finite() || !point[1].is_finite())
        || bulges.iter().any(|bulge| !bulge.is_finite())
    {
        return None;
    }
    let mut moments = polygon_moments(flat);
    for (index, bulge) in bulges.iter().enumerate() {
        if *bulge == 0.0 {
            continue;
        }
        let a = flat[index];
        let b = flat[(index + 1) % flat.len()];
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let chord = dx.hypot(dy);
        if !chord.is_finite() || chord <= 0.0 {
            return None;
        }
        let theta = 4.0 * bulge.atan();
        let half = 0.5 * theta;
        let sagitta = bulge * chord * 0.5;
        let radius = (chord * chord * 0.25 + sagitta * sagitta) / (2.0 * sagitta.abs());
        if !radius.is_finite() || radius <= 0.0 {
            return None;
        }
        let direction = [dx / chord, dy / chord];
        // A positive bulge bows to the **right** of the chord (the frame
        // `bulge_arc_geometry` and the tessellator use); `apex` points from the
        // chord to the arc, so the arc centre is on the opposite side.
        let left = [-direction[1], direction[0]];
        let side = if *bulge > 0.0 { 1.0 } else { -1.0 };
        let apex = [-left[0] * side, -left[1] * side];
        let center = [
            (a[0] + b[0]) * 0.5 - apex[0] * radius * half.cos(),
            (a[1] + b[1]) * 0.5 - apex[1] * radius * half.cos(),
        ];

        // The geometric (strictly positive) segment: angle |θ| in [0, 2π).
        let theta_abs = theta.abs();
        let alpha = 0.5 * theta_abs;
        let sin_a = alpha.sin();
        let cos_a = alpha.cos();
        let segment_area = if theta_abs < BULGE_SEGMENT_SERIES_RADIANS {
            // (R²/2)(θ − sinθ) = R²θ³/12·(1 − θ²/20 + θ⁴/840)
            radius * radius * theta_abs.powi(3) / 12.0
                * (1.0 - theta_abs * theta_abs / 20.0 + theta_abs.powi(4) / 840.0)
        } else {
            0.5 * radius * radius * (theta_abs - theta_abs.sin())
        };
        let y_c = if theta_abs < BULGE_SEGMENT_SERIES_RADIANS {
            // 4R sin³(θ/2) / (3(θ − sinθ)) → R(1 − 3θ²/40 + …)
            radius * (1.0 - 3.0 * theta_abs * theta_abs / 40.0)
        } else {
            2.0 * radius * sin_a.powi(3) / (3.0 * (alpha - sin_a * cos_a))
        };
        if !segment_area.is_finite() || !y_c.is_finite() {
            return None;
        }

        // Second moments of the segment about the arc centre, in the cap frame
        // (`x` along the chord, `y` along `apex`): sector minus triangle.
        let radius4 = radius.powi(4);
        let (centroidal_along_chord, centroidal_along_apex) =
            if alpha < CAP_SECOND_MOMENT_SERIES_RADIANS {
                // The closed forms below subtract two `O(R⁴α)` values to leave an
                // `O(R⁴α⁵)` one, so a shallow cap loses every digit there. These
                // are the Taylor series of the same two quantities
                // (`R⁴(2α⁵/15 − 4α⁷/63 + …)` and `R⁴(2α⁷/175 + …)`), i.e. exact
                // to double precision on this branch.
                (
                    radius4 * alpha.powi(5) * (2.0 / 15.0 - 4.0 * alpha * alpha / 63.0),
                    radius4 * alpha.powi(7) * (2.0 / 175.0),
                )
            } else {
                let about_chord =
                    radius4 * (0.25 * (alpha - sin_a * cos_a) - sin_a.powi(3) * cos_a / 6.0);
                let about_apex =
                    radius4 * (0.25 * (alpha + sin_a * cos_a) - 0.5 * sin_a * cos_a.powi(3));
                // About the segment's own centroid: the chord coordinate has a zero
                // centroid offset, so only the apex moment is shifted.
                (about_chord, about_apex - segment_area * y_c * y_c)
            };
        let segment_centroid = [center[0] + apex[0] * y_c, center[1] + apex[1] * y_c];

        // Rotate the centroidal tensor from the cap frame (chord, apex) into the
        // ring frame, then shift it onto the segment centroid. `direction` and
        // `apex` are orthonormal, so this is a pure rotation/reflection.
        let rotated_x = centroidal_along_chord * direction[0] * direction[0]
            + centroidal_along_apex * apex[0] * apex[0];
        let rotated_y = centroidal_along_chord * direction[1] * direction[1]
            + centroidal_along_apex * apex[1] * apex[1];
        let rotated_xy = centroidal_along_chord * direction[0] * direction[1]
            + centroidal_along_apex * apex[0] * apex[1];
        let [cx, cy] = segment_centroid;
        let sign = side;
        moments.area += sign * segment_area;
        moments.first_x += sign * segment_area * cx;
        moments.first_y += sign * segment_area * cy;
        moments.second_x += sign * (segment_area * cx * cx + rotated_x);
        moments.second_y += sign * (segment_area * cy * cy + rotated_y);
        moments.second_xy += sign * (segment_area * cx * cy + rotated_xy);
    }
    Some(moments)
}

/// Re-express moments measured in the `from` plane frame in the `to` frame.
///
/// The map between two orthonormal frames of the **same** plane is a rotation
/// (or, when the two `u × v` normals are antiparallel, a reflection) plus a
/// translation; applying that map to the moments is exact algebra, not a second
/// integration: every term is transformed by the parallel-axis theorem and by
/// the 2×2 rotation together. The result keeps the signed convention, and its
/// translation by `−1` is folded in — i.e. the returned moments are about
/// `to.origin`.
///
/// `None` when the two frames are not both orthonormal bases of the same plane
/// (`|det| != 1` within `1e-9`), or when a transformed moment is not finite.
pub fn reexpress(moments: PlanarMoments, from: WorkPlane, to: WorkPlane) -> Option<PlanarMoments> {
    if !moments.is_finite() {
        return None;
    }
    let uu = dot(from.u, to.u);
    let uv = dot(from.v, to.u);
    let vu = dot(from.u, to.v);
    let vv = dot(from.v, to.v);
    let determinant = uu * vv - uv * vu;
    if !determinant.is_finite() || (determinant.abs() - 1.0).abs() > 1e-9 {
        return None;
    }
    let offset = sub(from.origin, to.origin);
    let tx = dot(offset, to.u);
    let ty = dot(offset, to.v);
    let sign = if determinant < 0.0 { -1.0 } else { 1.0 };

    // A point of the region has `(x, y) = (uu·X + uv·Y + tx, vu·X + vv·Y + ty)`
    // in the target frame, where `(X, Y)` are its source-frame coordinates.
    let area = moments.area;
    let first_x = uu * moments.first_x + uv * moments.first_y + tx * area;
    let first_y = vu * moments.first_x + vv * moments.first_y + ty * area;
    let second_x = uu * uu * moments.second_x
        + 2.0 * uu * uv * moments.second_xy
        + uv * uv * moments.second_y
        + 2.0 * tx * first_x
        - tx * tx * area;
    let second_y = vu * vu * moments.second_x
        + 2.0 * vu * vv * moments.second_xy
        + vv * vv * moments.second_y
        + 2.0 * ty * first_y
        - ty * ty * area;
    let second_xy = uu * vu * moments.second_x
        + (uu * vv + uv * vu) * moments.second_xy
        + uv * vv * moments.second_y
        + tx * first_y
        + ty * first_x
        - tx * ty * area;
    let transformed = PlanarMoments {
        area: sign * area,
        first_x: sign * first_x,
        first_y: sign * first_y,
        second_x: sign * second_x,
        second_y: sign * second_y,
        second_xy: sign * second_xy,
    };
    transformed.is_finite().then_some(transformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOLERANCE: f64 = 1e-12;

    fn assert_close(actual: f64, expected: f64, what: &str) {
        assert!(
            (actual - expected).abs() <= TOLERANCE * expected.abs().max(1.0),
            "{what}: {actual} vs {expected}"
        );
    }

    /// A rectangle `width × height` with its lower-left corner at `(x, y)`,
    /// counter-clockwise.
    fn rectangle(x: f64, y: f64, width: f64, height: f64) -> Vec<[f64; 2]> {
        vec![
            [x, y],
            [x + width, y],
            [x + width, y + height],
            [x, y + height],
        ]
    }

    #[test]
    fn a_rectangle_has_bh_cubed_over_twelve_about_its_centroid() {
        let ring = rectangle(0.0, 0.0, 3.0, 4.0);
        let raw = polygon_moments(&ring);
        assert_close(raw.area, 12.0, "area");
        // About the frame origin (the lower-left corner): ∫y² = b·h³/3.
        assert_close(raw.second_y, 3.0 * 64.0 / 3.0, "∫y² about the origin");
        assert_close(raw.second_x, 4.0 * 27.0 / 3.0, "∫x² about the origin");
        assert_close(raw.second_xy, 36.0, "∫xy about the origin");
        let inertia = raw.centroidal().expect("a real rectangle");
        assert_close(inertia.centroid[0], 1.5, "xc");
        assert_close(inertia.centroid[1], 2.0, "yc");
        // Parallel axis: 64 - 12·4 = 16 and 27 - 12·2.25 = 9.
        assert_close(inertia.inertia_x, 3.0 * 64.0 / 12.0, "Ix = bh³/12");
        assert_close(inertia.inertia_y, 4.0 * 27.0 / 12.0, "Iy = hb³/12");
        assert_close(inertia.product_xy, 0.0, "Ixy");
        assert_close(inertia.polar(), 25.0, "J = Ix + Iy");
        assert_close(inertia.gyration_x().unwrap(), (16.0f64 / 12.0).sqrt(), "kx");
        assert_close(inertia.gyration_y().unwrap(), (9.0f64 / 12.0).sqrt(), "ky");
        assert_close(
            inertia.gyration_polar().unwrap(),
            (25.0f64 / 12.0).sqrt(),
            "kz",
        );
    }

    #[test]
    fn a_right_triangle_has_its_closed_form_moments() {
        // Legs b along x and h along y from the origin; counter-clockwise.
        let (b, h) = (3.0, 4.0);
        let ring = vec![[0.0, 0.0], [b, 0.0], [0.0, h]];
        let raw = polygon_moments(&ring);
        assert_close(raw.area, b * h / 2.0, "area");
        assert_close(raw.first_x, b * b * h / 6.0, "∫x");
        assert_close(raw.first_y, b * h * h / 6.0, "∫y");
        assert_close(raw.second_x, h * b.powi(3) / 12.0, "∫x²");
        assert_close(raw.second_y, b * h.powi(3) / 12.0, "∫y²");
        assert_close(raw.second_xy, b * b * h * h / 24.0, "∫xy");
        let inertia = raw.centroidal().unwrap();
        assert_close(inertia.centroid[0], b / 3.0, "xc");
        assert_close(inertia.centroid[1], h / 3.0, "yc");
        assert_close(inertia.inertia_x, b * h.powi(3) / 36.0, "Ix = bh³/36");
        assert_close(inertia.inertia_y, h * b.powi(3) / 36.0, "Iy = hb³/36");
        // Ixy − A·xc·yc = b²h²/24 − (bh/2)(b/3)(h/3) = −b²h²/72.
        assert_close(inertia.product_xy, -b * b * h * h / 72.0, "Ixy = −b²h²/72");
    }

    #[test]
    fn a_translated_rectangle_shifts_only_the_origin_moments() {
        let base = polygon_moments(&rectangle(0.0, 0.0, 3.0, 4.0));
        let moved = polygon_moments(&rectangle(7.0, -3.0, 3.0, 4.0));
        let (dx, dy) = (7.0, -3.0);
        assert_close(moved.area, base.area, "area is translation invariant");
        assert_close(moved.first_x, base.first_x + dx * base.area, "∫x");
        assert_close(moved.first_y, base.first_y + dy * base.area, "∫y");
        // Parallel axis on every second moment: I(+d) = I + 2d·S + A·d².
        assert_close(
            moved.second_x,
            base.second_x + 2.0 * dx * base.first_x + dx * dx * base.area,
            "∫x²",
        );
        assert_close(
            moved.second_y,
            base.second_y + 2.0 * dy * base.first_y + dy * dy * base.area,
            "∫y²",
        );
        assert_close(
            moved.second_xy,
            base.second_xy + dx * base.first_y + dy * base.first_x + dx * dy * base.area,
            "∫xy",
        );
        let a = base.centroidal().unwrap();
        let b = moved.centroidal().unwrap();
        assert_close(b.centroid[0], a.centroid[0] + dx, "centroid x");
        assert_close(b.centroid[1], a.centroid[1] + dy, "centroid y");
        assert_close(b.inertia_x, a.inertia_x, "centroidal Ix");
        assert_close(b.inertia_y, a.inertia_y, "centroidal Iy");
        assert_close(b.product_xy, a.product_xy, "centroidal Ixy");
    }

    #[test]
    fn a_clockwise_ring_normalises_to_the_same_geometric_moments() {
        let mut reversed = rectangle(0.0, 0.0, 3.0, 4.0);
        reversed.reverse();
        let raw = polygon_moments(&reversed);
        assert!(raw.area < 0.0, "a clockwise ring has negative signed area");
        let normalised = raw.with_positive_area();
        let ccw = polygon_moments(&rectangle(0.0, 0.0, 3.0, 4.0));
        assert_close(normalised.area, ccw.area, "area");
        assert_close(normalised.second_y, ccw.second_y, "∫y²");
        assert_close(normalised.second_xy, ccw.second_xy, "∫xy");
        let a = normalised.centroidal().unwrap();
        assert_close(a.centroid[0], 1.5, "xc");
        assert_close(a.inertia_x, 16.0, "Ix");
    }

    #[test]
    fn a_circle_written_as_four_quarter_arcs_has_pi_r4_over_4() {
        // The four vertices of a radius-1 circle with a quarter-arc bulge per
        // edge: each segment is a quarter disc, so the exact answer is
        // A = π, ∫x² = ∫y² = π/4 and ∫xy = 0 about the centre.
        let r: f64 = 1.0;
        let ring = vec![[r, 0.0], [0.0, r], [-r, 0.0], [0.0, -r]];
        let bulges = vec![std::f64::consts::FRAC_PI_8.tan(); 4];
        let moments = bulged_ring_moments(&ring, &bulges).expect("four exact quarter arcs");
        let pi = std::f64::consts::PI;
        assert_close(moments.area, pi * r * r, "area");
        assert_close(moments.first_x, 0.0, "∫x");
        assert_close(moments.first_y, 0.0, "∫y");
        assert_close(moments.second_x, pi * r.powi(4) / 4.0, "∫x² = πr⁴/4");
        assert_close(moments.second_y, pi * r.powi(4) / 4.0, "∫y² = πr⁴/4");
        assert_close(moments.second_xy, 0.0, "∫xy");
        let inertia = moments.centroidal().unwrap();
        assert_close(inertia.inertia_x, pi / 4.0, "centroidal Ix");
        assert_close(inertia.gyration_x().unwrap(), 0.5, "kx = r/2");
    }

    #[test]
    fn a_circle_written_as_many_equal_arcs_is_exact_on_both_series_branches() {
        // A regular N-gon whose every edge carries the matching bulge is exactly
        // the circle, so its moments must be πr⁴/4 whatever N is. N = 180 puts
        // the half-angle (π/180 ≈ 1.7e-2) above the second-moment series cutover;
        // N = 360 puts it below (π/360 ≈ 8.7e-3), i.e. the shallow-cap branch.
        let pi = std::f64::consts::PI;
        for sides in [180_usize, 360] {
            let r: f64 = 2.0;
            let ring: Vec<[f64; 2]> = (0..sides)
                .map(|index| {
                    let angle = std::f64::consts::TAU * index as f64 / sides as f64;
                    [r * angle.cos(), r * angle.sin()]
                })
                .collect();
            let bulges = vec![(std::f64::consts::TAU / sides as f64 / 4.0).tan(); sides];
            let moments =
                bulged_ring_moments(&ring, &bulges).unwrap_or_else(|| panic!("{sides} exact arcs"));
            assert_close(moments.area, pi * r * r, &format!("{sides} area"));
            assert_close(
                moments.second_x,
                pi * r.powi(4) / 4.0,
                &format!("{sides} x"),
            );
            assert_close(
                moments.second_y,
                pi * r.powi(4) / 4.0,
                &format!("{sides} y"),
            );
            assert_close(moments.second_xy, 0.0, &format!("{sides} xy"));
            let inertia = moments.centroidal().expect("a disc");
            assert_close(
                inertia.inertia_x,
                pi * r.powi(4) / 4.0,
                &format!("{sides} Ix"),
            );
            assert_close(
                inertia.gyration_x().unwrap(),
                r / 2.0,
                &format!("{sides} kx"),
            );
        }
    }

    #[test]
    fn a_semicircular_cap_adds_the_exact_segment_moments() {
        // The 3x4 rectangle with its bottom edge replaced by a semicircular cap
        // bowing outwards (bulge +1), i.e. the exact `MASSPROP` bulged fixture.
        let (b, h, r) = (3.0, 4.0, 1.5);
        let ring = vec![[0.0, 0.0], [b, 0.0], [b, h], [0.0, h]];
        let moments =
            bulged_ring_moments(&ring, &[1.0, 0.0, 0.0, 0.0]).expect("one semicircular cap");
        let pi = std::f64::consts::PI;
        let cap = pi * r * r / 2.0;
        // ∫y²: the rectangle's own 3·4³/3 = 64 plus the half disc's πr⁴/8 about
        // its diameter y = 0 (y² is even, so the side does not matter).
        let expected_second_y = b * h.powi(3) / 3.0 + pi * r.powi(4) / 8.0;
        // ∫x²: the rectangle's 4·3³/3 = 36 plus the half disc's own πr⁴/8 about
        // its centroidal axis plus cap·(r)² for the half disc's centre at x = r.
        let expected_second_x = h * b.powi(3) / 3.0 + pi * r.powi(4) / 8.0 + cap * r * r;
        // ∫xy: the rectangle's 36 plus x·y over the half disc. The half disc is
        // symmetric about x = r, so only r·∫y survives, and its centroid is
        // y = −4r/3π below the diameter.
        let expected_second_xy = (b * b / 2.0) * (h * h / 2.0) + r * cap * (-4.0 * r / (3.0 * pi));
        assert_close(moments.area, b * h + cap, "area");
        assert_close(moments.second_y, expected_second_y, "∫y²");
        assert_close(moments.second_x, expected_second_x, "∫x²");
        assert_close(moments.second_xy, expected_second_xy, "∫xy");
        // The cap pulls the centroid below the rectangle's own y = 2.
        let inertia = moments.centroidal().unwrap();
        assert_close(inertia.centroid[0], r, "xc");
        let expected_y = (b * h * h / 2.0 + cap * (-4.0 * r / (3.0 * pi))) / (b * h + cap);
        assert_close(inertia.centroid[1], expected_y, "yc");
        assert_close(
            inertia.inertia_x,
            expected_second_y - (b * h + cap) * expected_y * expected_y,
            "Ix about the centroid",
        );
    }

    #[test]
    fn a_reexpressed_ring_matches_a_direct_integration() {
        // The 3x4 rectangle whose own plane frame starts at the world point
        // (1, 2), so `ring` holds its local coordinates.
        let ring = rectangle(0.0, 0.0, 3.0, 4.0);
        let from = WorkPlane {
            origin: Point3 {
                x: 1.0,
                y: 2.0,
                z: 0.0,
            },
            u: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            v: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        };
        // A quarter-turn about +Z with a different origin: x' = y, y' = −x.
        let to = WorkPlane {
            origin: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            u: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            v: Point3 {
                x: -1.0,
                y: 0.0,
                z: 0.0,
            },
        };
        let source = polygon_moments(&ring);
        let moved = reexpress(source, from, to).expect("orthonormal coplanar frames");
        // The same rectangle, integrated directly in the target frame: local
        // (X, Y) is the world point (1 + X, 2 + Y), whose target coordinates are
        // (y, −x).
        let direct: Vec<[f64; 2]> = ring
            .iter()
            .map(|point| [2.0 + point[1], -1.0 - point[0]])
            .collect();
        let expected = polygon_moments(&direct);
        assert_close(moved.area, expected.area, "area");
        assert_close(moved.first_x, expected.first_x, "∫x");
        assert_close(moved.first_y, expected.first_y, "∫y");
        assert_close(moved.second_x, expected.second_x, "∫x²");
        assert_close(moved.second_y, expected.second_y, "∫y²");
        assert_close(moved.second_xy, expected.second_xy, "∫xy");
        let a = moved.centroidal().unwrap();
        let b = expected.centroidal().unwrap();
        assert_close(a.centroid[0], b.centroid[0], "xc");
        assert_close(a.centroid[1], b.centroid[1], "yc");
        assert_close(a.inertia_x, b.inertia_x, "Ix");
        assert_close(a.inertia_y, b.inertia_y, "Iy");
        assert_close(a.product_xy, b.product_xy, "Ixy");
    }

    #[test]
    fn a_reexpression_across_a_flipped_normal_keeps_the_geometric_moments() {
        let ring = rectangle(0.0, 0.0, 3.0, 4.0);
        let from = WorkPlane {
            origin: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            u: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            v: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        };
        // The same basis with `v` mirrored: u × v is now −Z, so the map includes
        // a reflection and the signed moments change sign.
        let mirrored = WorkPlane {
            origin: from.origin,
            u: from.u,
            v: Point3 {
                x: 0.0,
                y: -1.0,
                z: 0.0,
            },
        };
        let source = polygon_moments(&ring);
        let flipped = reexpress(source, from, mirrored).expect("a reflection is still |det| = 1");
        assert!(flipped.area < 0.0, "the reflection flips the signed area");
        let inertia = flipped
            .with_positive_area()
            .centroidal()
            .expect("still a real region");
        assert_close(inertia.area, 12.0, "area");
        assert_close(inertia.inertia_x, 16.0, "Ix");
        assert_close(inertia.inertia_y, 9.0, "Iy");
        assert_close(inertia.product_xy, 0.0, "Ixy");
        // A non-coplanar/orthonormality violation is refused, never guessed.
        let skew = WorkPlane {
            origin: from.origin,
            u: Point3 {
                x: 0.6,
                y: 0.8,
                z: 0.0,
            },
            v: Point3 {
                x: 0.8,
                y: 0.6,
                z: 0.0,
            },
        };
        assert!(reexpress(source, from, skew).is_none(), "{skew:?}");
    }

    #[test]
    fn a_rotated_rectangle_has_the_expected_product_and_principal_angle() {
        // The 3x4 rectangle rotated by +45° about its own centre. Its body-frame
        // moments are ∫ξ² = 9 and ∫η² = 16, so the rotated ones are
        // Ix = Iy = (9+16)/2 and Ixy = (9−16)·sin(90°)/2 = −3.5.
        let theta = std::f64::consts::FRAC_PI_4;
        let (b, h) = (3.0, 4.0);
        let (cos, sin) = (theta.cos(), theta.sin());
        let corners = [
            [-b / 2.0, -h / 2.0],
            [b / 2.0, -h / 2.0],
            [b / 2.0, h / 2.0],
            [-b / 2.0, h / 2.0],
        ];
        let ring: Vec<[f64; 2]> = corners
            .iter()
            .map(|point| {
                [
                    point[0] * cos - point[1] * sin,
                    point[0] * sin + point[1] * cos,
                ]
            })
            .collect();
        let inertia = polygon_moments(&ring)
            .centroidal()
            .expect("the rotated rectangle");
        assert_close(inertia.centroid[0], 0.0, "xc");
        assert_close(inertia.centroid[1], 0.0, "yc");
        assert_close(inertia.inertia_x, 12.5, "Ix");
        assert_close(inertia.inertia_y, 12.5, "Iy");
        assert_close(inertia.product_xy, -3.5, "Ixy");
        let principal = inertia.principal().expect("finite tensor");
        assert_close(principal.major, 16.0, "the larger principal moment");
        assert_close(principal.minor, 9.0, "the smaller principal moment");
        assert_close(principal.major + principal.minor, 25.0, "J is invariant");
        // The principal axis of the larger moment is a quarter turn from the long
        // side, i.e. along +45° for a rectangle rotated by +45°.
        assert_close(principal.angle.to_degrees(), 45.0, "principal angle");
        assert_close(
            principal.major_axis[0],
            std::f64::consts::FRAC_1_SQRT_2,
            "axis x",
        );
        assert_close(
            principal.major_axis[1],
            std::f64::consts::FRAC_1_SQRT_2,
            "axis y",
        );
        let minor_axis = principal.minor_axis();
        assert_close(
            minor_axis[0],
            -std::f64::consts::FRAC_1_SQRT_2,
            "minor axis x",
        );
        assert_close(
            minor_axis[1],
            std::f64::consts::FRAC_1_SQRT_2,
            "minor axis y",
        );
    }

    #[test]
    fn the_axis_aligned_principal_axes_are_the_frame_axes() {
        let inertia = polygon_moments(&rectangle(0.0, 0.0, 3.0, 4.0))
            .centroidal()
            .unwrap();
        let principal = inertia.principal().unwrap();
        assert_close(principal.major, 16.0, "major = Ix");
        assert_close(principal.minor, 9.0, "minor = Iy");
        assert_close(principal.angle, 0.0, "the major axis is +x");
        // A square is isotropic: both principal moments coincide and the polar
        // moment is the invariant that still discriminates it.
        let square = polygon_moments(&rectangle(0.0, 0.0, 2.0, 2.0))
            .centroidal()
            .unwrap();
        let square_principal = square.principal().unwrap();
        assert_close(square_principal.major, square_principal.minor, "square");
        assert_close(square.polar(), 8.0 / 3.0, "square polar moment");
    }

    #[test]
    fn a_degenerate_ring_is_refused_instead_of_guessed() {
        // Fewer than three vertices, a bulge count mismatch, a zero-length chord
        // and a non-finite coordinate are all refusals.
        assert!(bulged_ring_moments(&[[0.0, 0.0], [1.0, 0.0]], &[0.0, 0.0]).is_none());
        assert!(bulged_ring_moments(&rectangle(0.0, 0.0, 1.0, 1.0), &[0.0; 3]).is_none());
        assert!(
            bulged_ring_moments(&[[0.0, 0.0], [0.0, 0.0], [1.0, 1.0]], &[1.0, 0.0, 0.0]).is_none()
        );
        assert!(
            bulged_ring_moments(&[[0.0, 0.0], [f64::INFINITY, 0.0], [1.0, 1.0]], &[0.0; 3])
                .is_none()
        );
        // A zero-area ring has no centroid, so no inertia can be shifted onto it.
        let collinear = polygon_moments(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]]);
        assert!(collinear.centroid().is_none());
        assert!(collinear.centroidal().is_none());
    }

    #[test]
    fn a_non_positive_semidefinite_tensor_is_reported_invalid() {
        let valid = polygon_moments(&rectangle(0.0, 0.0, 3.0, 4.0))
            .centroidal()
            .unwrap();
        assert!(valid.is_finite_and_valid());
        // A product of inertia larger than sqrt(Ix·Iy) cannot come from a region.
        let impossible = PlanarInertia {
            product_xy: 100.0,
            ..valid
        };
        assert!(!impossible.is_finite_and_valid());
        let nan = PlanarInertia {
            inertia_x: f64::NAN,
            ..valid
        };
        assert!(!nan.is_finite_and_valid());
    }
}
