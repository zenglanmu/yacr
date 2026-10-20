//! Shared analytic case table for the DXF bulge -> arc expansions.
//!
//! The workspace has **six** independent implementations of this one piece of
//! math (`cad-geometry` display + EXPLODE, `cad-spatial` pick bounds,
//! `cad-measure` object snap, `cad-import-acadrust` hatch boundaries,
//! `cad-domain` hatch fill) plus a seventh, deliberately different one in the
//! SHX glyph decoder (`cad-representation`). They must all agree on *which side
//! of the chord the centre goes*, because a single flipped sign mirrors every
//! arc while leaving semicircles (`cos(theta/2) = 0`) untouched — exactly the
//! defect that shipped and passed every existing test.
//!
//! This file is included with `#[path]` by each copy's test module, so the
//! table and the assertions exist exactly **once** instead of seven times. It is
//! plain `std` + `f64` arrays: no crate under test is imported or called, and
//! nothing here can be satisfied by calling an implementation.
//!
//! ## The convention, derived here
//!
//! `bulge = tan(theta / 4)` with `theta` the signed included angle. An
//! **included angle** is the angle the arc subtends at its centre: positive
//! means the arc runs counter-clockwise (CCW) from `p0` to `p1`.
//!
//! With `c = |p1 - p0|`, `left = (-dy, dx) / c` (the left normal of `p0 -> p1`)
//! and `mid` the chord midpoint, the circle through both ends that subtends
//! `theta` has radius `r = c / (2 |sin(theta/2)|)`; using
//! `sin(theta/2) = 2b/(1+b^2)` and `cos(theta/2) = (1-b^2)/(1+b^2)`:
//!
//! ```text
//! r      = c (1 + b^2) / (4 |b|)
//! centre = mid + left * c (1 - b^2) / (4 b)
//! apex   = mid - left * (b c / 2)          <- arc point halfway in angle
//! ```
//!
//! The uniform invariant is the **apex**: `cross(chord, apex - p0) = -b c^2 / 2`,
//! i.e. the signed perpendicular offset of the apex is `-b c / 2` — the apex is
//! on the opposite side from a *minor* arc's centre. A CCW quarter arc from
//! `(0,0)` to `(4,0)` therefore bows *down*, with its centre at `(2, 2)`.
//!
//! The centre's side is only "opposite the apex" for `|b| < 1`: a **major** arc
//! (`|b| > 1`, `cos(theta/2) < 0`) puts the centre on the apex's side of the
//! chord (between chord and apex), still on the bulge side of the apex. A
//! **semicircle** (`|b| = 1`, `cos(theta/2) = 0`) puts the centre *on* the chord
//! and its arc is mirror-symmetric about the chord — which is exactly why the
//! flipped-centre defect survived every semicircle test.
//!
//! The expected numbers are fixed twice: by the algebra above *and* as literal
//! constants in [`assert_table_is_hand_computed`], so a typo in either is caught
//! before any implementation is compared against it.
#![allow(dead_code)]

use std::f64::consts::{PI, TAU};

/// One bulge segment and its analytic arc, computed from the chord + bulge.
#[derive(Debug, Clone)]
pub struct Case {
    pub name: String,
    pub p0: [f64; 2],
    pub p1: [f64; 2],
    pub bulge: f64,
    /// Analytic circle centre: on the perpendicular bisector, `c(1-b^2)/(4b)`
    /// from the chord midpoint along the left normal.
    pub center: [f64; 2],
    pub radius: f64,
    /// Signed included angle in radians: `4 atan(bulge)`.
    pub included_angle: f64,
    /// The arc point halfway in angle: the farthest point from the chord.
    pub apex: [f64; 2],
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn len(a: [f64; 2]) -> f64 {
    a[0].hypot(a[1])
}

fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn close(a: f64, b: f64, scale: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + scale + b.abs())
}

/// The analytic arc for the segment `p0 -> p1` with `bulge`.
///
/// Every value comes from the closed forms in the module docs; the result is
/// self-checked against those invariants before it is handed out. Panics only
/// for a degenerate (zero-length) chord, which callers filter out themselves.
pub fn analytic(name: &str, p0: [f64; 2], p1: [f64; 2], bulge: f64) -> Case {
    assert!(
        bulge.is_finite() && bulge != 0.0,
        "case {name}: the table only describes non-zero, finite bulges"
    );
    let chord = sub(p1, p0);
    let c = len(chord);
    assert!(c > 1e-9, "case {name}: degenerate chord (length {c})");
    let left = [-chord[1] / c, chord[0] / c];
    let mid = [(p0[0] + p1[0]) * 0.5, (p0[1] + p1[1]) * 0.5];
    let offset = c * (1.0 - bulge * bulge) / (4.0 * bulge);
    let center = [mid[0] + left[0] * offset, mid[1] + left[1] * offset];
    let radius = c * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
    let sagitta = bulge * c * 0.5;
    let apex = [mid[0] - left[0] * sagitta, mid[1] - left[1] * sagitta];
    let case = Case {
        name: name.to_string(),
        p0,
        p1,
        bulge,
        center,
        radius,
        included_angle: 4.0 * bulge.atan(),
        apex,
    };
    assert_self_consistent(&case);
    case
}

/// Check the derivation against itself, without reusing its own centring
/// formula: the three points `p0`, `p1` and the apex must have the stated
/// centre as their **circumcentre**, and the apex must sit `|b|c/2` from the
/// chord.
///
/// The apex relation is the mirror invariant. Note it does *not* say "centre
/// opposite the apex" for every bulge: for `|b| < 1` the centre is on the
/// `+sign(b)` side of the chord and the apex on the other, while for a **major**
/// arc (`|b| > 1`, `cos(theta/2) < 0`) the centre moves to the apex's side but
/// still on the bulge side of the apex — the apex offset `-b c/2` is the
/// uniformity. A semicircle (`|b| = 1`) has its centre on the chord, which is
/// why the mirrored-centre defect was invisible to semicircle tests.
fn assert_self_consistent(case: &Case) {
    let chord = sub(case.p1, case.p0);
    let c = len(chord);
    let label = &case.name;
    let circum = circumcenter(case.p0, case.p1, case.apex)
        .unwrap_or_else(|| panic!("{label}: p0, p1 and the apex are collinear"));
    assert!(
        close(circum[0], case.center[0], case.radius)
            && close(circum[1], case.center[1], case.radius),
        "{label}: the circumcentre of p0, p1, apex is {circum:?}, derived centre {:?}",
        case.center
    );
    let radius = len(sub(case.p0, circum));
    assert!(
        close(radius, case.radius, case.radius),
        "{label}: circumradius {radius} != derived radius {}",
        case.radius
    );
    let apex_offset = cross(chord, sub(case.apex, case.p0)) / c;
    assert!(
        close(apex_offset, -case.bulge * c * 0.5, c),
        "{label}: apex offset {apex_offset} is not -b*c/2"
    );
}

/// The circumcentre of three non-collinear 2D points.
fn circumcenter(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<[f64; 2]> {
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    if !d.is_finite() || d.abs() <= 1e-300 {
        return None;
    }
    let aa = a[0] * a[0] + a[1] * a[1];
    let bb = b[0] * b[0] + b[1] * b[1];
    let cc = c[0] * c[0] + c[1] * c[1];
    let ux = (aa * (b[1] - c[1]) + bb * (c[1] - a[1]) + cc * (a[1] - b[1])) / d;
    let uy = (aa * (c[0] - b[0]) + bb * (a[0] - c[0]) + cc * (b[0] - a[0])) / d;
    Some([ux, uy])
}

/// The one case table every copy must satisfy.
///
/// Required coverage: quarter arc, semicircle, **major** arc (`|b| > 1`),
/// negative bulge, a very small bulge, and a non-axis-aligned chord.
pub fn table() -> Vec<Case> {
    vec![
        analytic(
            "quarter",
            [0.0, 0.0],
            [4.0, 0.0],
            std::f64::consts::FRAC_PI_8.tan(),
        ),
        analytic("semicircle", [0.0, 0.0], [2.0, 0.0], 1.0),
        analytic("major", [0.0, 0.0], [2.0, 0.0], 2.0),
        analytic("negative", [0.0, 0.0], [3.0, 0.0], -0.5),
        analytic("tiny", [0.0, 0.0], [4.0, 0.0], 1e-3),
        analytic("oblique", [1.0, 2.0], [5.0, 5.0], 0.25),
    ]
}

/// Hand-computed expectations for [`table`], written out by algebra (not by
/// running any implementation).
pub struct HandComputed {
    pub name: &'static str,
    pub center: [f64; 2],
    pub radius: f64,
    pub included_angle: f64,
    pub apex: [f64; 2],
}

/// The hand-computed expectations for [`table`], as literal constants.
pub fn hand_computed() -> Vec<HandComputed> {
    vec![
        // Quarter, chord 4, b = tan(pi/8): r = 2*sqrt(2), centre (2, 2).
        HandComputed {
            name: "quarter",
            center: [2.0, 2.0],
            radius: 2.828_427_124_746_190_3,
            included_angle: std::f64::consts::FRAC_PI_2,
            apex: [2.0, -0.828_427_124_746_190_1],
        },
        // Semicircle, chord 2, b = 1: centre on the chord, r = 1.
        HandComputed {
            name: "semicircle",
            center: [1.0, 0.0],
            radius: 1.0,
            included_angle: PI,
            apex: [1.0, -1.0],
        },
        // Major arc, chord 2, b = 2: theta = 4 atan(2) = 253.74 deg,
        // r = 2*5/8 = 1.25, centre below the chord, apex 2 below.
        HandComputed {
            name: "major",
            center: [1.0, -0.75],
            radius: 1.25,
            included_angle: 4.428_594_871_176_362,
            apex: [1.0, -2.0],
        },
        // Negative, chord 3, b = -1/2: r = 3*1.25/2 = 1.875, centre below
        // (the apex is above).
        HandComputed {
            name: "negative",
            center: [1.5, -1.125],
            radius: 1.875,
            included_angle: -1.854_590_436_003_224_4,
            apex: [1.5, 0.75],
        },
        // Tiny, chord 4, b = 1e-3: r ~ 1000, centre 999.999 above the chord.
        HandComputed {
            name: "tiny",
            center: [2.0, 999.999],
            radius: 1000.001,
            included_angle: 0.003_999_998_666_667_467,
            apex: [2.0, -0.002],
        },
        // Oblique chord (1,2) -> (5,5), b = 1/4: r = 5*17/16 = 5.3125.
        HandComputed {
            name: "oblique",
            center: [0.1875, 7.25],
            radius: 5.3125,
            included_angle: 0.979_914_652_507_456_6,
            apex: [3.375, 3.0],
        },
    ]
}

/// Assert that [`table`] equals the hand-computed literals above.
///
/// Call this once per test so an error in the derivation (or a typo in the
/// literals) is reported before any implementation is blamed.
pub fn assert_table_is_hand_computed() {
    let table = table();
    let pinned = hand_computed();
    assert_eq!(table.len(), pinned.len());
    for (case, pinned) in table.iter().zip(pinned.iter()) {
        let name = pinned.name;
        assert_eq!(case.name, name);
        let scale = 1.0 + case.bulge.abs();
        assert!(
            close(case.center[0], pinned.center[0], scale * case.radius)
                && close(case.center[1], pinned.center[1], scale * case.radius),
            "{name}: centre {:?} != hand-computed {:?}",
            case.center,
            pinned.center
        );
        assert!(
            close(case.radius, pinned.radius, case.radius),
            "{name}: radius {} != hand-computed {}",
            case.radius,
            pinned.radius
        );
        assert!(
            (case.included_angle - pinned.included_angle).abs() <= 1e-12,
            "{name}: included angle {} != hand-computed {}",
            case.included_angle,
            pinned.included_angle
        );
        assert!(
            close(case.apex[0], pinned.apex[0], case.radius)
                && close(case.apex[1], pinned.apex[1], case.radius),
            "{name}: apex {:?} != hand-computed {:?}",
            case.apex,
            pinned.apex
        );
    }
}

/// Assert an **analytic** representation: `centre + r*(cos, sin)` swept from
/// `start` by `sweep` (the tessellator's `Arc`/`BulgeArc` form), in the world
/// `+Z` frame the tests use as the polyline plane.
pub fn assert_analytic_arc(
    copy: &str,
    case: &Case,
    center: [f64; 2],
    radius: f64,
    start: f64,
    sweep: f64,
) {
    let label = label(copy, case);
    assert!(
        close(center[0], case.center[0], case.radius)
            && close(center[1], case.center[1], case.radius),
        "{label}: centre {center:?} != analytic {:?} (centre moved to the apex side?)",
        case.center
    );
    assert!(
        close(radius, case.radius, case.radius),
        "{label}: radius {radius} != analytic {}",
        case.radius
    );
    // The gauge is free (start +/- 2 pi), the direction is not.
    let expected_start = (case.p0[1] - case.center[1]).atan2(case.p0[0] - case.center[0]);
    let mut delta = start - expected_start;
    while delta > PI {
        delta -= TAU;
    }
    while delta <= -PI {
        delta += TAU;
    }
    assert!(
        delta.abs() <= 1e-9,
        "{label}: start angle {start} is {delta} away from the analytic p0 angle {expected_start}"
    );
    assert!(
        close(sweep, case.included_angle, 1.0),
        "{label}: sweep {sweep} != signed included angle {}",
        case.included_angle
    );
}

/// Assert a **sampled** representation: endpoints exact, every sample on the
/// analytic circle, and the samples traversing the analytic angular span in the
/// analytic direction.
///
/// `radial_tol` is the absolute distance a sample may sit off the circle (a
/// chord tessellation is inside its sagitta tolerance; an exact sampler is
/// ~1e-12). A mirrored arc fails here: its samples lie on the *mirrored*
/// circle, which only shares the two endpoints with the analytic one.
pub fn assert_samples(copy: &str, case: &Case, samples: &[[f64; 2]], radial_tol: f64) {
    let label = label(copy, case);
    assert!(
        samples.len() >= 2,
        "{label}: only {} sample(s), need at least the two endpoints",
        samples.len()
    );
    let scale = 1.0 + case.radius + case.p0[0].abs() + case.p0[1].abs();
    let endpoint_tol = 1e-9 * scale;
    for (what, got, want) in [
        ("first", samples[0], case.p0),
        ("last", samples[samples.len() - 1], case.p1),
    ] {
        let d = len(sub(got, want));
        assert!(
            d <= endpoint_tol,
            "{label}: {what} sample {got:?} is {d} from {want:?}"
        );
    }
    for (index, sample) in samples.iter().enumerate() {
        let d = len(sub(*sample, case.center));
        assert!(
            (d - case.radius).abs() <= radial_tol,
            "{label}: sample {index} at {sample:?} is {d} from the analytic centre {:?}, \
             radius {} (off by {})",
            case.center,
            case.radius,
            (d - case.radius).abs()
        );
    }
    // Unwrap the sweep sample by sample: every step is well below pi for these
    // cases, so the accumulated angle is the signed included angle.
    let mut total = 0.0;
    let mut previous = (samples[0][1] - case.center[1]).atan2(samples[0][0] - case.center[0]);
    for sample in &samples[1..] {
        let angle = (sample[1] - case.center[1]).atan2(sample[0] - case.center[0]);
        let mut step = angle - previous;
        while step > PI {
            step -= TAU;
        }
        while step <= -PI {
            step += TAU;
        }
        total += step;
        previous = angle;
    }
    assert!(
        (total - case.included_angle).abs() <= 1e-6 * (1.0 + case.included_angle.abs()),
        "{label}: samples sweep {total} rad, the analytic arc sweeps {} rad (wrong direction?)",
        case.included_angle
    );
}

/// Assert the sampled arc reaches the analytic apex: the sample farthest from
/// the chord sits `|b|c/2` away **on the side opposite the centre**.
///
/// `tol` must cover the sampler's own chord error at the apex (the arc is
/// sampled in angle, so the extreme sample can be up to one half-step short of
/// the true apex). A sampler that emits only the two endpoints cannot satisfy
/// this and should not call it.
pub fn assert_apex(copy: &str, case: &Case, samples: &[[f64; 2]], tol: f64) {
    let label = label(copy, case);
    let chord = sub(case.p1, case.p0);
    let c = len(chord);
    let mut extreme = 0.0f64;
    for sample in samples {
        // Signed perpendicular offset, positive on the left of the chord.
        let offset = cross(chord, sub(*sample, case.p0)) / c;
        if offset.abs() > extreme.abs() {
            extreme = offset;
        }
    }
    let expected = -case.bulge * c * 0.5;
    assert!(
        (extreme - expected).abs() <= tol,
        "{label}: apex offset {extreme} (left-positive) != analytic {expected} \
         (bulge {:+}); the arc is {} the chord",
        case.bulge,
        if extreme * case.bulge > 0.0 {
            "on the wrong side of"
        } else {
            "mirrored across"
        }
    );
}

/// Assert a conservative AABB for one bulge arc: the union of both endpoints
/// and the ball around the analytic centre.
pub fn assert_bounds(copy: &str, case: &Case, lo: [f64; 2], hi: [f64; 2], tol: f64) {
    let label = label(copy, case);
    for (axis, index) in [("x", 0), ("y", 1)] {
        let expected_lo = case.p0[index]
            .min(case.p1[index])
            .min(case.center[index] - case.radius);
        let expected_hi = case.p0[index]
            .max(case.p1[index])
            .max(case.center[index] + case.radius);
        assert!(
            (lo[index] - expected_lo).abs() <= tol,
            "{label}: {axis} min {} != analytic {expected_lo} (centre {:?}, radius {})",
            lo[index],
            case.center,
            case.radius
        );
        assert!(
            (hi[index] - expected_hi).abs() <= tol,
            "{label}: {axis} max {} != analytic {expected_hi} (centre {:?}, radius {})",
            hi[index],
            case.center,
            case.radius
        );
    }
}

fn label(copy: &str, case: &Case) -> String {
    format!(
        "{copy} [{}] p0={:?} p1={:?} bulge={}",
        case.name, case.p0, case.p1, case.bulge
    )
}
