//! PRESSPULL / FOLLOWME kernel spike (2026-11-25).
//!
//! Question this answers with the **pinned** kernel (no mock): can
//! `cad-kernel-solidcore` express
//!
//! 1. "move a planar face of a solid along the face normal" (SketchUp Push/Pull)
//!    as a small sequence of public `solidcore-ops` calls — read the face's exact
//!    region from a plane section, extrude it, then union (pull out) or subtract
//!    (push in); and
//! 2. "sweep a closed planar profile along a sequence of placements"
//!    (`sweep_stations`, the Follow-Me-shaped call), with the profile written as
//!    plain `sweep::Curve` lines?
//!
//! This file is **evidence for the plan, not a shipped capability**. No seam
//! operation, command, session parameter or UI is wired by it, and `PRESSPULL` /
//! `FOLLOWME` remain named refusals. The volumes below are integrated by the
//! kernel from the produced bodies, so a wrong winding or a failed boolean shows
//! up as a wrong number, not as an empty success.
//!
//! Evidence boundary: Linux x86-64, this repository's test profile, real pinned
//! `solidcore` revision. No GPU, no host, no other platform (ADR-0036).

use solidcore_ops::{
    boolean::{boolean, BooleanOp},
    check::CheckLevel,
    make_box,
    prism::{extrude_planar_regions, prism_section, region_area},
    sweep::{sweep_stations, Affine, Curve, Ends},
    Body,
};
use solidcore_record::{Plane, Vec3};

/// The relative accuracy the volume is integrated at.
const VOLUME_ACCURACY: f64 = 1e-9;

fn volume(body: &Body) -> f64 {
    let value = body.volume(VOLUME_ACCURACY).expect("the kernel integrates");
    assert!(value.is_finite() && value > 0.0, "volume {value}");
    value
}

fn assert_valid(body: &Body) {
    let report = body
        .check(CheckLevel::Geometry)
        .expect("the geometry check runs on a made body");
    assert!(report.is_valid(), "kernel check: {report}");
}

fn assert_near(actual: f64, expected: f64) {
    let tolerance = 1e-6 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, got {actual} (tolerance {tolerance})"
    );
}

/// A 10 × 10 × 10 box at the origin, with its top face on `z = 10`.
fn box_10() -> Body {
    let body = make_box([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]).expect("the kernel makes a box");
    assert_near(volume(&body), 1000.0);
    assert_valid(&body);
    body
}

/// The top-face plane, in the record's sketch-plane form: origin on the face,
/// `+X`/`+Y` the in-plane axes and therefore `+Z` as the normal.
fn top_plane() -> Plane {
    Plane {
        origin: Vec3::new(0.0, 0.0, 10.0),
        x: Vec3::X,
        y: Vec3::Y,
    }
}

#[test]
fn a_planar_face_can_be_pushed_out_and_in_through_the_public_ops() {
    let body = box_10();

    // 1. The face's exact region: the body's section on the face plane. For a
    // box this is the 10 × 10 square, and the body spans -10..0 above the plane.
    let section = prism_section(&body, &top_plane())
        .expect("the section is read")
        .expect("a box is a uniform prism along its face normal");
    assert_near(section.lo, -10.0);
    assert_near(section.hi, 0.0);
    assert_near(
        region_area(&section.region)
            .expect("the region is planar")
            .value,
        100.0,
    );

    // 2. Pull out: extrude the region 3 above the face and union it with the
    // body. 1000 + 3 × 100 = 1300.
    let prism = extrude_planar_regions(
        &top_plane(),
        std::slice::from_ref(&section.region),
        0.0,
        3.0,
    )
    .expect("the face region extrudes");
    assert_near(volume(&prism), 300.0);
    let pulled = boolean(body, prism, BooleanOp::Union)
        .expect("the union runs")
        .expect("the union leaves a body");
    assert_near(volume(&pulled), 1300.0);
    assert_valid(&pulled);

    // 3. Push in: extrude the same region 3 below the face and subtract it.
    // 1000 − 3 × 100 = 700.
    let body = box_10();
    let section = prism_section(&body, &top_plane())
        .expect("the section is read")
        .expect("a box is a uniform prism");
    let cut = extrude_planar_regions(&top_plane(), &[section.region], -3.0, 0.0)
        .expect("the face region extrudes");
    let pushed = boolean(body, cut, BooleanOp::Subtract)
        .expect("the subtraction runs")
        .expect("the subtraction leaves a body");
    assert_near(volume(&pushed), 700.0);
    assert_valid(&pushed);
}

/// The square profile `[0,2] × [0,2]` in the plane `y = 0`, counter-clockwise
/// seen from `-Y`, as four `sweep::Curve` lines.
fn square_profile() -> Vec<Curve> {
    let corner = |x: f64, z: f64| [x, 0.0, z];
    vec![
        Curve::Line {
            start: corner(0.0, 0.0),
            end: corner(2.0, 0.0),
        },
        Curve::Line {
            start: corner(2.0, 0.0),
            end: corner(2.0, 2.0),
        },
        Curve::Line {
            start: corner(2.0, 2.0),
            end: corner(0.0, 2.0),
        },
        Curve::Line {
            start: corner(0.0, 2.0),
            end: corner(0.0, 0.0),
        },
    ]
}

#[test]
fn a_closed_profile_can_be_swept_through_placements() {
    let identity = Affine {
        m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        t: [0.0, 0.0, 0.0],
    };
    let stations: Vec<Affine> = [0.0, 2.0, 4.0]
        .iter()
        .map(|y| Affine {
            t: [0.0, *y, 0.0],
            ..identity
        })
        .collect();
    let swept = sweep_stations(&square_profile(), &stations, Ends::Capped)
        .expect("the profile sweeps along the stations");
    // 2 × 2 over a 4-long run: 16.
    assert_near(volume(&swept), 16.0);
    assert_valid(&swept);
}
