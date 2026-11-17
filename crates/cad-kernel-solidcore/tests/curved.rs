//! Curved bodies: the P2.1b part 2 chain — a cylinder, sphere, cone or torus (and
//! a boolean result over one) is lifted back into the pinned kernel with its
//! exact NURBS form and exact pcurves, then measured against its closed form.
//!
//! Every number here is integrated by the real pinned kernel from the body this
//! crate produced (no mock, no golden), so a wrong form or a wrong pcurve shows up
//! as a wrong volume, area or centroid. Evidence boundary: Linux x86-64, this
//! repository's test profile, the real kernel. No GPU, no host, no other platform
//! (ADR-0036).

mod common;

use cad_domain::BrepSurface;
use cad_kernel_adapter::{BooleanOp, PrimitiveSpec, SolidOperation};
use cad_kernel_solidcore::SolidCoreModeler;

use common::*;

/// Inverted from
/// `curved_primitives_map_analytic_surfaces_and_refuse_kernel_re_entry`: each
/// curved primitive is mapped with its analytic surface **and** re-enters the
/// kernel, so a boolean and mass properties are answered instead of refused.
#[test]
fn curved_primitives_map_analytic_surfaces_and_lift_again() {
    let modeler = SolidCoreModeler::new();
    let cases: [(PrimitiveSpec, &str, f64); 4] = [
        (
            PrimitiveSpec::Cylinder {
                origin: p(0.0, 0.0, 0.0),
                radius: 1.0,
                height: 2.0,
            },
            "cylinder",
            2.0 * std::f64::consts::PI,
        ),
        (
            PrimitiveSpec::Sphere {
                origin: p(0.0, 0.0, 0.0),
                radius: 1.0,
            },
            "sphere",
            4.0 / 3.0 * std::f64::consts::PI,
        ),
        (
            PrimitiveSpec::Cone {
                origin: p(0.0, 0.0, 0.0),
                radius_bottom: 1.0,
                radius_top: 0.5,
                height: 2.0,
            },
            "cone",
            std::f64::consts::PI * 2.0 * 1.75 / 3.0,
        ),
        (
            PrimitiveSpec::Torus {
                origin: p(0.0, 0.0, 0.0),
                major_radius: 2.0,
                minor_radius: 0.5,
            },
            "torus",
            std::f64::consts::PI * std::f64::consts::PI,
        ),
    ];
    for (spec, name, expected) in cases {
        let brep = body(execute(&modeler, SolidOperation::Primitive { spec }));
        // The curved face keeps its analytic kind: display and export get an exact
        // surface, not an unsupported placeholder.
        let analytic = brep
            .shells
            .iter()
            .flat_map(|shell| &shell.faces)
            .any(|face| {
                matches!(
                    (name, &face.surface),
                    ("cylinder", BrepSurface::Cylinder { .. })
                        | ("sphere", BrepSurface::Sphere { .. })
                        | ("cone", BrepSurface::Cone { .. })
                        | ("torus", BrepSurface::Torus { .. })
                )
            });
        assert!(analytic, "{name}: no analytic surface in the mapped body");

        // The lift carries the exact form, so a boolean over the body is answered.
        let other = body(execute(
            &modeler,
            SolidOperation::Primitive {
                spec: box_spec(p(-4.0, -4.0, -4.0), 1.0, 1.0, 1.0),
            },
        ));
        let outcome = execute(
            &modeler,
            SolidOperation::Boolean {
                op: BooleanOp::Union,
                left: &brep,
                right: &other,
            },
        );
        assert!(outcome.is_success(), "{name}: boolean refused: {outcome:?}");
        assert!(outcome.body().is_some(), "{name}: no body");

        // Mass properties take the same lift gate, and the volume is the closed
        // form, held to the kernel's **requested** relative accuracy (`1e-6`,
        // `ops::mass`) — the only accuracy the seam claims. Measured for these
        // bodies: cylinder 4.7e-10, sphere 3.8e-9, torus 7.1e-9, cone 5.0e-10.
        let measured = execute(
            &modeler,
            SolidOperation::MassProperties {
                brep: &brep,
                density: 1.0,
            },
        );
        let mass = measured
            .mass()
            .unwrap_or_else(|| panic!("{name}: mass refused: {measured:?}"));
        assert!(
            (mass.volume - expected).abs() <= 1e-6 * expected,
            "{name}: volume {} != {expected} (rel {})",
            mass.volume,
            (mass.volume - expected).abs() / expected
        );
    }
}

/// Curved mass properties against their closed forms: volume, area and centroid.
#[test]
fn curved_mass_properties_are_their_closed_forms() {
    use std::f64::consts::PI;
    let modeler = SolidCoreModeler::new();
    let measure = |spec: PrimitiveSpec| {
        let brep = body(execute(&modeler, SolidOperation::Primitive { spec }));
        let outcome = execute(
            &modeler,
            SolidOperation::MassProperties {
                brep: &brep,
                density: 2.0,
            },
        );
        *outcome
            .mass()
            .unwrap_or_else(|| panic!("mass: {outcome:?}"))
    };

    // r = 1, h = 2: V = 2π, A = 2πr² + 2πrh = 6π, centroid at mid-height. Every
    // check here is at the kernel's requested `1e-6` (the seam claims no tighter
    // bound); the measured errors are ~1e-9 or better.
    let cylinder = measure(PrimitiveSpec::Cylinder {
        origin: p(0.0, 0.0, 0.0),
        radius: 1.0,
        height: 2.0,
    });
    assert!((cylinder.volume - 2.0 * PI).abs() <= 1e-6 * 2.0 * PI);
    assert!((cylinder.surface_area - 6.0 * PI).abs() <= 1e-6 * 6.0 * PI);
    assert!(
        (cylinder.centroid.z - 1.0).abs() <= 1e-6
            && cylinder.centroid.x.abs() <= 1e-6
            && cylinder.centroid.y.abs() <= 1e-6,
        "centroid {:?}",
        cylinder.centroid
    );
    assert_eq!(cylinder.density, 2.0);

    // r = 1: V = 4π/3, A = 4π, centroid at the centre.
    let sphere = measure(PrimitiveSpec::Sphere {
        origin: p(0.0, 0.0, 0.0),
        radius: 1.0,
    });
    let sphere_volume = 4.0 / 3.0 * PI;
    assert!((sphere.volume - sphere_volume).abs() <= 1e-6 * sphere_volume);
    assert!((sphere.surface_area - 4.0 * PI).abs() <= 1e-6 * 4.0 * PI);
    assert!(
        sphere.centroid.x.abs() <= 1e-6
            && sphere.centroid.y.abs() <= 1e-6
            && sphere.centroid.z.abs() <= 1e-6
    );

    // R = 2, r = 0.5: V = 2π²Rr² = π², A = 4π²Rr = 4π².
    let torus = measure(PrimitiveSpec::Torus {
        origin: p(0.0, 0.0, 0.0),
        major_radius: 2.0,
        minor_radius: 0.5,
    });
    assert!((torus.volume - PI * PI).abs() <= 1e-6 * PI * PI);
    assert!((torus.surface_area - 4.0 * PI * PI).abs() <= 1e-6 * 4.0 * PI * PI);

    // r1 = 1, r2 = 0.5, h = 2: V = πh(r1²+r1r2+r2²)/3 = 2π·1.75/3; the area is
    // π(r1+r2)·sqrt(h²+(r1−r2)²) + π(r1²+r2²); the centroid sits at
    // h(r1²+2r1r2+3r2²)/(4(r1²+r1r2+r2²)).
    let cone = measure(PrimitiveSpec::Cone {
        origin: p(0.0, 0.0, 0.0),
        radius_bottom: 1.0,
        radius_top: 0.5,
        height: 2.0,
    });
    let cone_volume = 2.0 * PI * 1.75 / 3.0;
    assert!((cone.volume - cone_volume).abs() <= 1e-6 * cone_volume);
    let slant = (4.0f64 + 0.25).sqrt();
    let cone_area = 1.5 * PI * slant + 1.25 * PI;
    assert!((cone.surface_area - cone_area).abs() <= 1e-6 * cone_area);
    assert!(
        (cone.centroid.z - 0.7857142857142857).abs() <= 1e-6,
        "centroid {:?}",
        cone.centroid
    );
}

/// Booleans whose operand is curved: every result is checked against a closed
/// form, and all three are clean here because the cutter's faces are parallel to
/// the cylinder's axis, so the intersection curves are full circles.
#[test]
fn booleans_on_a_curved_body_match_their_closed_forms() {
    use std::f64::consts::PI;
    let modeler = SolidCoreModeler::new();
    let cylinder = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: PrimitiveSpec::Cylinder {
                origin: p(0.0, 0.0, 0.0),
                radius: 1.0,
                height: 2.0,
            },
        },
    ));
    // A 2 x 2 x 2 box around the axis, from z = 0.5 to 2.5: the part inside the
    // cylinder is a 1.5-high disc, πr²·1.5 = 1.5π.
    let cutter = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(-1.0, -1.0, 0.5), 2.0, 2.0, 2.0),
        },
    ));
    let cylinder_volume = 2.0 * PI;
    let shared = 1.5 * PI;

    for (op, expected) in [
        (BooleanOp::Union, cylinder_volume + 8.0 - shared),
        (BooleanOp::Subtract, cylinder_volume - shared),
        (BooleanOp::Intersect, shared),
    ] {
        let result = body(execute(
            &modeler,
            SolidOperation::Boolean {
                op,
                left: &cylinder,
                right: &cutter,
            },
        ));
        let outcome = execute(
            &modeler,
            SolidOperation::MassProperties {
                brep: &result,
                density: 1.0,
            },
        );
        let mass = outcome
            .mass()
            .unwrap_or_else(|| panic!("{op:?}: the result does not lift again: {outcome:?}"));
        assert!(
            (mass.volume - expected).abs() <= 1e-6 * expected,
            "{op:?}: volume {} != {expected}",
            mass.volume
        );
    }
}

/// A curved body survives **two** generations of lift → kernel → read.
///
/// This is the contract the written box has to satisfy: the reader carries the
/// producer's box verbatim, so a second lift cannot reuse those numbers. The
/// writer derives the box from the face's own boundary instead (`record::surfaces`),
/// which is why the volumes below are the same number twice rather than a body
/// that drifts or is refused on the second pass.
#[test]
fn a_lifted_curved_body_survives_a_second_generation() {
    let modeler = SolidCoreModeler::new();
    let cylinder = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: PrimitiveSpec::Cylinder {
                origin: p(0.0, 0.0, 0.0),
                radius: 1.0,
                height: 2.0,
            },
        },
    ));
    let first = volume(&modeler, &cylinder);
    // A boolean both re-lifts the operand and reads the result back.
    let cutter = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(-1.0, -1.0, 0.5), 2.0, 2.0, 2.0),
        },
    ));
    let cut = body(execute(
        &modeler,
        SolidOperation::Boolean {
            op: BooleanOp::Subtract,
            left: &cylinder,
            right: &cutter,
        },
    ));
    let second = volume(&modeler, &cut);
    assert!(
        (first - 2.0 * std::f64::consts::PI).abs() <= 1e-6 * 2.0 * std::f64::consts::PI,
        "first generation volume {first}"
    );
    assert!(
        (second - 0.5 * std::f64::consts::PI).abs() <= 1e-6,
        "the re-lifted body lost volume: {second}"
    );
}

/// A torus **away from the origin and under a rotation** round-trips through the
/// kernel.
///
/// The seam loop its writer reconstructs (`record::curved::torus_seam`) has the
/// tube circle and the outer equator meeting at one vertex, and record vertices
/// dedup on **exact bits**: `(o + x·R) + x·r` and `o + x·(R + r)` must not be
/// computed separately. They agree for every origin only because the primitive's
/// own radii and an `x`-aligned frame make both sums exact; a rotated frame makes
/// them generic, and then roughly half of all frames disagree by a rounding step
/// (measured in `record::curved`'s unit test). The rotation is applied through the
/// public transform, so the placement is baked by the lift — the seam points, the
/// frame and the radii all move together.
#[test]
fn a_rotated_torus_away_from_the_origin_keeps_one_seam_vertex() {
    use std::f64::consts::PI;
    let modeler = SolidCoreModeler::new();
    let torus = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: PrimitiveSpec::Torus {
                origin: p(10.3, -4.7, 2.9),
                major_radius: 2.0,
                minor_radius: 0.5,
            },
        },
    ));
    let expected = 2.0 * PI * PI * 2.0 * 0.25;
    let first = volume(&modeler, &torus);
    assert!(
        (first - expected).abs() <= 1e-6 * expected,
        "torus volume {first} != {expected}"
    );

    // 0.37 rad about `z` (the torus's own axis) and a translation: a similarity,
    // so the volume is unchanged and the frame's x axis is no longer +X.
    let (s, c) = 0.37f64.sin_cos();
    let transform =
        cad_domain::Transform3::from_basis(p(c, s, 0.0), p(-s, c, 0.0), p(3.1, -8.2, 0.7));
    let rotated = body(execute(
        &modeler,
        SolidOperation::Transform {
            brep: &torus,
            transform: &transform,
        },
    ));
    let second = volume(&modeler, &rotated);
    assert!(
        (second - expected).abs() <= 1e-6 * expected,
        "the rotated torus does not lift: volume {second} != {expected}"
    );
}

/// A hand-authored torus whose frame's `x` has **generic** components, so the two
/// ways of reaching the seam point differ by a rounding step (verified for this
/// origin and angle: the tube side rounds one ulp off the equator side).
///
/// The vertex the seam loop chains through must therefore be computed **once**.
/// This body is the arithmetic witness; `a_rotated_torus_away_from_the_origin_…`
/// exercises the same seam through the placement bake.
#[test]
fn a_torus_on_a_generic_frame_keeps_one_seam_vertex() {
    use std::f64::consts::PI;
    let (s, c) = 0.37f64.sin_cos();
    let body = cad_domain::BrepData {
        shells: vec![cad_domain::BrepShell {
            id: 0,
            faces: vec![cad_domain::BrepFace {
                id: 0,
                surface: cad_domain::BrepSurface::Torus {
                    center: p(10.3, -4.7, 2.9),
                    normal: p(0.0, 0.0, 1.0),
                    major_radius: 2.0,
                    minor_radius: 0.5,
                    u_dir: p(c, s, 0.0),
                },
                reversed: false,
                loops: Vec::new(),
                domain: Some([[0.0, std::f64::consts::TAU], [0.0, std::f64::consts::TAU]]),
                tolerance: None,
            }],
        }],
        placement: None,
        tolerance: None,
    };
    let expected = 2.0 * PI * PI * 2.0 * 0.25;
    let measured = volume(&SolidCoreModeler::new(), &body);
    assert!(
        (measured - expected).abs() <= 1e-6 * expected,
        "the torus's seam loop does not chain: volume {measured} != {expected}"
    );
}

/// A **full** cone (`radius_top = 0`) re-lifts and measures its apex band.
///
/// The reader maps the kernel's full cone as one base circle plus a degenerate
/// apex loop (a `BrepCurve::Unsupported` marker: the whole boundary is one point).
/// The writer turns that back into the generator the record can hold — two uses of
/// one ruling whose far end is the apex — so `MASSPROP` on a user-drawn `CONE`
/// answers with the closed form instead of a curve-family refusal.
#[test]
fn a_full_cone_measures_its_apex_band() {
    use std::f64::consts::PI;
    let modeler = SolidCoreModeler::new();
    let cone = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: PrimitiveSpec::Cone {
                origin: p(0.0, 0.0, 0.0),
                radius_bottom: 1.0,
                radius_top: 0.0,
                height: 2.0,
            },
        },
    ));
    let measured = execute(
        &modeler,
        SolidOperation::MassProperties {
            brep: &cone,
            density: 1.0,
        },
    );
    let mass = measured
        .mass()
        .unwrap_or_else(|| panic!("a full cone must lift and measure: {measured:?}"));
    // V = πr²h/3 = 2π/3; A = πr² + πr·slant, slant = √(r² + h²) = √5.
    let volume = 2.0 * PI / 3.0;
    assert!(
        (mass.volume - volume).abs() <= 1e-6 * volume,
        "full-cone volume {} != {volume}",
        mass.volume
    );
    let area = PI * (1.0 + 5.0f64.sqrt());
    assert!(
        (mass.surface_area - area).abs() <= 1e-6 * area,
        "full-cone area {} != {area}",
        mass.surface_area
    );
    // A solid cone's centroid is a quarter of the height above its base (3h/4 from
    // the apex): the base is at z = 0 and the apex at z = 2 here.
    assert!(
        (mass.centroid.z - 0.5).abs() <= 1e-6,
        "centroid {:?}",
        mass.centroid
    );
}

/// A trimmed side face whose boundary is **off** the quarter-turn grid has no
/// exact pcurve this build can write, and says so by name.
///
/// Every exact pcurve on a labelled curved face rests on the boundary sharing the
/// face's quarter grid: a rim circle always does, a generator at a quarter-line
/// angle does, and a generator anywhere else does not (`solidcore`'s closed forms
/// speak in radians there while the face's parameter is the shared grid). Cutting
/// a cylinder with a rotated box puts the arcs and generators off that grid, so
/// the lift refuses with `solid.pcurve_not_exact` rather than fitting a trim.
#[test]
fn a_trimmed_face_cut_off_the_quarter_grid_is_refused_by_name() {
    let modeler = SolidCoreModeler::new();
    let cylinder = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: PrimitiveSpec::Cylinder {
                origin: p(0.0, 0.0, 0.0),
                radius: 1.0,
                height: 2.0,
            },
        },
    ));
    let cutter = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(-2.0, -2.0, -1.0), 4.0, 2.0, 4.0),
        },
    ));
    // 0.3 rad about `z`: a quarter-line cut becomes an off-grid one.
    let (s, c) = 0.3f64.sin_cos();
    let rotated = body(execute(
        &modeler,
        SolidOperation::Transform {
            brep: &cutter,
            transform: &cad_domain::Transform3::from_basis(
                p(c, s, 0.0),
                p(-s, c, 0.0),
                p(0.0, 0.0, 0.0),
            ),
        },
    ));
    let cut = body(execute(
        &modeler,
        SolidOperation::Boolean {
            op: BooleanOp::Subtract,
            left: &cylinder,
            right: &rotated,
        },
    ));
    let measured = execute(
        &modeler,
        SolidOperation::MassProperties {
            brep: &cut,
            density: 1.0,
        },
    );
    assert!(
        measured.mass().is_none(),
        "an off-grid trim must not be measured with a fitted pcurve: {measured:?}"
    );
    let reason = measured
        .unsupported()
        .unwrap_or_else(|| panic!("the lift names the gap: {measured:?}"));
    assert_eq!(reason.code, "solid.pcurve_not_exact");
    assert!(
        reason.detail.contains("exact pcurve") && reason.detail.contains("quarter grid"),
        "the refusal names the exact-pcurve gap and its cause: {reason:?}"
    );
}
