//! `$PDMODE` surrounding shapes: `32` circle, `64` square, `96` both.
//!
//! AutoCAD's point display mode adds a surrounding shape to the base glyph with
//! bit `32` (circle) and bit `64` (square); `96` requests both. These are drawn
//! here as explicit line segments **in the same POINT marker batch** as the base
//! glyph — one batch, one colour, one draw order, one synthetic source id — so
//! the overlay stays `Lines`-only: the circle is a closed chord ring, not a
//! conic, and [`cad_scene::RenderTopology`] gains no new variant.
//!
//! ## Radius convention (chosen here, NOT verified against AutoCAD)
//!
//! The surround **circumscribes the marker arm span**: the circle radius and the
//! square half-side are exactly the marker half-extent derived from `$PDSIZE`
//! ([`super::marker_half_extent`]). A `$PDMODE 35` cross therefore touches its
//! circle at the diagonal ends and never pokes outside it, and the whole marker
//! still spans 2 × half-extent, i.e. the same 5 %-of-viewport-height footprint
//! the base glyphs have at `$PDSIZE = 0`.
//!
//! This proportion is **our choice**. AutoCAD's own surround-to-glyph size ratio
//! has **not** been verified against a real AutoCAD drawing, so no compatibility
//! claim is made for it (`UNVERIFIED`).
//!
//! ## Determinism
//!
//! The segment count per shape is a fixed constant, never derived from the zoom
//! level or the screen scale, so a given (point set, style, viewport) always
//! produces byte-identical geometry:
//!
//! * square: [`POINT_SURROUND_SQUARE_SEGMENTS`] = 4 segments at `±half`
//!   (the same corner-to-corner emission as the grip marker square);
//! * circle: a closed ring of [`POINT_SURROUND_CIRCLE_SEGMENTS`] = 16 chords,
//!   starting at angle 0 and ending exactly on its first vertex.
//!
//! The ring is a **polygon approximation** of the circle: its vertices lie on the
//! circumscribed radius but the chords cut inside it, `half * (1 - cos(π/16))`
//! ≈ 1.9 % of the radius at the mid-chord. That is the honest price of a
//! `Lines`-only topology; there is no conic path in the renderer to use instead.

use super::*;

/// Number of chords a `$PDMODE` `32` circle surround is drawn with.
///
/// Fixed, never zoom-derived (see the module docs). 16 chords keep the ring
/// visually round at the ~10 px marker size while staying far below the
/// per-marker budget of the base glyph; the sagitta is ≈ 1.9 % of the radius.
pub const POINT_SURROUND_CIRCLE_SEGMENTS: usize = 16;

/// Number of segments a `$PDMODE` `64` square surround is drawn with: the four
/// sides, emitted corner to corner.
pub const POINT_SURROUND_SQUARE_SEGMENTS: usize = 4;

/// The surrounding shapes one `$PDMODE` value asks for.
///
/// Derived from the mode **bits** only (`32` circle, `64` square); it never
/// depends on the size, the viewport or the base glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PointSurrounds {
    /// `$PDMODE` bit `32`: draw a circle around the base glyph.
    pub circle: bool,
    /// `$PDMODE` bit `64`: draw a square around the base glyph.
    pub square: bool,
}

impl PointSurrounds {
    /// No surrounding shape: the plain base glyph.
    pub const NONE: Self = Self {
        circle: false,
        square: false,
    };

    /// Whether any surrounding shape is requested.
    pub const fn is_empty(self) -> bool {
        !self.circle && !self.square
    }

    /// Segments this surround set emits per marker (square first, then circle).
    pub const fn segment_count(self) -> usize {
        let square = if self.square {
            POINT_SURROUND_SQUARE_SEGMENTS
        } else {
            0
        };
        let circle = if self.circle {
            POINT_SURROUND_CIRCLE_SEGMENTS
        } else {
            0
        };
        square + circle
    }
}

/// Append the surround segments for one marker centred on `center`.
///
/// Emission order is fixed — the square's four sides, then the circle's closed
/// chord ring — so the batch is deterministic. Every segment is an independent
/// `LineList` pair (no shared-vertex index buffer), exactly like the other
/// overlay line batches.
pub(super) fn push_point_surrounds(
    vertices: &mut Vec<Point3>,
    center: Point3,
    half: f64,
    surrounds: PointSurrounds,
) {
    if surrounds.square {
        let (left, right) = (center.x - half, center.x + half);
        let (bottom, top) = (center.y - half, center.y + half);
        let corner = |x: f64, y: f64| Point3 { x, y, z: center.z };
        vertices.extend_from_slice(&[
            corner(left, bottom),
            corner(right, bottom),
            corner(right, bottom),
            corner(right, top),
            corner(right, top),
            corner(left, top),
            corner(left, top),
            corner(left, bottom),
        ]);
    }
    if surrounds.circle {
        for i in 0..POINT_SURROUND_CIRCLE_SEGMENTS {
            // `(i + 1) % segments` closes the ring on the exact first vertex bits
            // instead of a recomputed `TAU` endpoint.
            vertices.push(ring_vertex(center, half, i));
            vertices.push(ring_vertex(
                center,
                half,
                (i + 1) % POINT_SURROUND_CIRCLE_SEGMENTS,
            ));
        }
    }
}

/// Vertex `index` of the surround ring: `half` from `center`, starting at angle 0
/// and running counter-clockwise.
fn ring_vertex(center: Point3, radius: f64, index: usize) -> Point3 {
    let theta = std::f64::consts::TAU * (index as f64) / (POINT_SURROUND_CIRCLE_SEGMENTS as f64);
    Point3 {
        x: center.x + theta.cos() * radius,
        y: center.y + theta.sin() * radius,
        z: center.z,
    }
}
