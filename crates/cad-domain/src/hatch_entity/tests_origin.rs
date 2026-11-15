//! Pattern-origin contract tests (the `HATCHEDIT ORIGIN` phase displacement).
//!
//! Split out of `hatch_entity/tests.rs` to stay inside the code-size gate. The
//! displacement is the only origin the model has, because neither the DXF nor the
//! DWG hatch record stores one separately: the phase lives in each definition
//! row's base point, so the exported rows carry it (see `cad-import-acadrust`'s
//! `export::hatch`).

use super::*;

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> HatchLoop {
    HatchLoop::new(
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
        Vec::new(),
        HatchLoopFlags::default(),
    )
}

/// A quarter turn about world +Z (the helper `tests.rs` keeps private).
fn rotation_z(angle: f64) -> Transform3 {
    let (sin, cos) = angle.sin_cos();
    Transform3::from_basis(
        Point3 {
            x: cos,
            y: sin,
            z: 0.0,
        },
        Point3 {
            x: -sin,
            y: cos,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    )
}

fn build(loops: Vec<HatchLoop>, style: HatchStyle, fill: HatchFill) -> HatchGeometry {
    HatchGeometry::new(
        loops,
        HatchPlane::default(),
        style,
        fill,
        Vec::new(),
        None,
        HatchPatternType::UserDefined,
    )
    .expect("valid hatch")
}

// ---------------------------------------------------------------------------
// Pattern origin (the pattern phase displacement HATCHEDIT ORIGIN writes).
// ---------------------------------------------------------------------------

/// The `y` of every visible pattern line, sorted, for a family whose lines are
/// horizontal and spaced by `offset_y`.
fn pattern_line_ys(hatch: &HatchGeometry) -> Vec<f64> {
    let boundaries = hatch.loops().len();
    let mut ys: Vec<f64> = hatch
        .display()
        .iter()
        .skip(boundaries)
        .map(|geometry| {
            let SemanticGeometry::Polyline { points, .. } = geometry else {
                panic!("pattern display must be polylines, got {geometry:?}");
            };
            points[0].y
        })
        .collect();
    ys.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    ys
}

/// A pattern whose one family draws horizontal lines at `y = 1 + 2k`.
fn spaced_pattern_fill() -> HatchFill {
    HatchFill::Pattern(HatchPattern {
        name: "USER".into(),
        angle: 0.0,
        scale: 1.0,
        double: false,
        lines: vec![HatchPatternLine {
            angle: 0.0,
            base_x: 0.0,
            base_y: 1.0,
            offset_x: 0.0,
            offset_y: 2.0,
            dashes: Vec::new(),
        }],
    })
}

#[test]
fn a_new_or_imported_hatch_starts_at_the_zero_pattern_origin() {
    // DXF/DWG have no pattern-origin group: the phase lives in the definition
    // rows, so both constructors must start at the zero displacement and leave
    // the rows untouched.
    let created = build(
        vec![rect(0.0, 0.0, 10.0, 10.0)],
        HatchStyle::Normal,
        spaced_pattern_fill(),
    );
    assert_eq!(created.pattern_origin(), [0.0, 0.0]);
    let (imported, reasons) = HatchGeometry::from_import(
        vec![rect(0.0, 0.0, 10.0, 10.0)],
        HatchPlane::default(),
        HatchStyle::Normal,
        spaced_pattern_fill(),
        Vec::new(),
        None,
        HatchPatternType::Predefined,
    );
    assert!(reasons.is_empty(), "{reasons:?}");
    assert_eq!(imported.pattern_origin(), [0.0, 0.0]);
    assert_eq!(
        imported.display(),
        created.display(),
        "the two constructors derive the same display at the same origin"
    );
    assert_eq!(
        pattern_line_ys(&created),
        vec![1.0, 3.0, 5.0, 7.0, 9.0],
        "the rows' own origin"
    );
}

#[test]
fn with_pattern_origin_translates_every_pattern_line_and_no_boundary() {
    let hatch = build(
        vec![rect(0.0, 0.0, 10.0, 10.0)],
        HatchStyle::Normal,
        spaced_pattern_fill(),
    );
    let moved = hatch.with_pattern_origin([0.0, 0.25]);
    assert_eq!(moved.pattern_origin(), [0.0, 0.25]);
    // Every composed family is translated by the displacement: the visible lines
    // move with their spacing and count intact.
    assert_eq!(
        pattern_line_ys(&moved),
        vec![1.25, 3.25, 5.25, 7.25, 9.25],
        "the pattern phase moved by the origin"
    );
    // Nothing else about the entity changed.
    assert_eq!(moved.loops(), hatch.loops());
    assert_eq!(moved.plane(), hatch.plane());
    assert_eq!(moved.style(), hatch.style());
    assert_eq!(moved.fill(), hatch.fill());
    assert_eq!(moved.seeds(), hatch.seeds());
    assert_eq!(moved.pixel_size(), hatch.pixel_size());
    assert_eq!(moved.pattern_type(), hatch.pattern_type());
    // The boundary polylines are the first `loops().len()` display entries and
    // are untouched by the pattern phase.
    assert_eq!(
        &moved.display()[..moved.loops().len()],
        &hatch.display()[..hatch.loops().len()]
    );
    // Zeroing it again is the exact inverse.
    assert_eq!(
        moved.with_pattern_origin([0.0, 0.0]).display(),
        hatch.display()
    );
}

#[test]
fn a_pattern_origin_follows_a_similarity_transform() {
    // The displacement is a vector in the hatch plane, so a uniform scale of 2
    // doubles it and the induced in-plane rotation turns it. With no rotation the
    // expectation is plain: (1, 2) -> (2, 4).
    let hatch = build(
        vec![rect(0.0, 0.0, 10.0, 10.0)],
        HatchStyle::Normal,
        spaced_pattern_fill(),
    )
    .with_pattern_origin([1.0, 2.0]);
    let moved = hatch
        .transformed(&Transform3::scale(2.0))
        .expect("uniform scale is representable");
    let origin = moved.pattern_origin();
    assert!((origin[0] - 2.0).abs() < 1e-9, "{origin:?}");
    assert!((origin[1] - 4.0).abs() < 1e-9, "{origin:?}");

    // A quarter turn about +Z turns the displacement with the loops: (1, 2) ->
    // (-2, 1).
    let turned = hatch
        .transformed(&rotation_z(std::f64::consts::FRAC_PI_2))
        .expect("a rotation is representable");
    let origin = turned.pattern_origin();
    assert!((origin[0] + 2.0).abs() < 1e-9, "{origin:?}");
    assert!((origin[1] - 1.0).abs() < 1e-9, "{origin:?}");

    // A mirrored transform is still refused for the whole entity.
    assert!(hatch.transformed(&Transform3::scale(-1.0)).is_err());
}
