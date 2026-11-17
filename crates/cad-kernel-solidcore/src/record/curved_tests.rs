//! Unit tests for the curved writer's seam arithmetic (`record::curved`).
//!
//! Split out of `curved.rs` because the writer plus its tests crossed the
//! workspace's per-file line ceiling.

#![cfg(test)]

use cad_domain::Point3;

use crate::record::{add, is_axis, mul, normalize};

/// A deterministic LCG: the measurement below must not depend on a test-order
/// seed or on `rand`.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        // The top 53 bits, in [0, 1).
        ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
    }

    fn offset(&mut self, span: f64) -> f64 {
        (self.next() - 0.5) * span
    }
}

fn origin(rng: &mut Lcg) -> Point3 {
    Point3 {
        x: rng.offset(100.0),
        y: rng.offset(100.0),
        z: rng.offset(100.0),
    }
}

/// The fraction of frames on which `(o + x·R) + x·r` and `o + x·(R+r)` are
/// **different** floating-point points.
///
/// A torus's tube circle and its outer equator must share one record vertex,
/// and vertices dedup on exact bits, so `torus_seam` computes the point once
/// and hands it to both. This keeps that single computation load-bearing by
/// measuring how often the two independent expressions would disagree.
///
/// The axis-aligned case is measured too, because it explains why every older
/// torus test was green: with the primitive's own radii (`R = 2`, `r = 0.5`)
/// and `x = +X`, both expressions are exact for any origin, so an origin far
/// from `(0, 0, 0)` alone would **not** have caught this. A general unit frame
/// (a rotated torus) does.
#[test]
fn the_two_torus_seam_expressions_disagree_on_a_general_frame() {
    const MAJOR: f64 = 2.0;
    const MINOR: f64 = 0.5;
    const FRAMES: usize = 4096;
    let mut rng = Lcg(0x5eed_1234_abcd_0001);

    let mut axis_aligned = 0usize;
    let mut general = 0usize;
    for _ in 0..FRAMES {
        let origin = origin(&mut rng);
        let x = Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        };
        if add(add(origin, mul(x, MAJOR)), mul(x, MINOR)) != add(origin, mul(x, MAJOR + MINOR)) {
            axis_aligned += 1;
        }
        let x = normalize(Point3 {
            x: rng.offset(1.0),
            y: rng.offset(1.0),
            z: rng.offset(1.0),
        });
        if is_axis(x)
            && add(add(origin, mul(x, MAJOR)), mul(x, MINOR)) != add(origin, mul(x, MAJOR + MINOR))
        {
            general += 1;
        }
    }

    assert_eq!(
        axis_aligned, 0,
        "with R = 2 and r = 0.5 an axis-aligned frame is exact at every origin, which is \
             exactly why the older torus tests could not see this"
    );
    let general = general as f64 / FRAMES as f64;
    // Measured with this generator: 0.499. The floor is well below that (and
    // well above zero), so the test pins "the two expressions are not the same
    // computation" without depending on the exact stream.
    assert!(
        general > 0.4,
        "general-frame disagreement rate {general} (measured ~0.50)"
    );
}
