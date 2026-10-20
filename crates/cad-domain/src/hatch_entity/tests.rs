//! HATCH entity contract tests (ADR 0018 G.1): style dispatch by loop-set
//! filtering, the pattern/gradient regions per style, and the transform rules.

use super::geometry::push_bulge_points;
use super::*;
use crate::hatch::GradientStop;

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> HatchLoop {
    HatchLoop::new(
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
        Vec::new(),
        HatchLoopFlags::default(),
    )
}

/// Three concentric rings: region 1 = 0..10 band, 2 = 2..8 band, 3 = centre.
fn three_level() -> Vec<HatchLoop> {
    vec![
        rect(0.0, 0.0, 10.0, 10.0),
        rect(2.0, 2.0, 8.0, 8.0),
        rect(4.0, 4.0, 6.0, 6.0),
    ]
}

/// Two disjoint outer loops.
fn two_disjoint() -> Vec<HatchLoop> {
    vec![rect(0.0, 0.0, 4.0, 4.0), rect(10.0, 10.0, 14.0, 14.0)]
}

fn pattern_fill(scale: f64) -> HatchFill {
    HatchFill::Pattern(HatchPattern {
        name: "USER".into(),
        angle: 0.0,
        scale,
        double: false,
        lines: vec![HatchPatternLine {
            angle: 0.0,
            base_x: 0.0,
            base_y: 1.0,
            offset_x: 0.0,
            offset_y: 1.0,
            dashes: Vec::new(),
        }],
    })
}

fn gradient_fill() -> HatchFill {
    HatchFill::Gradient(HatchGradient::TwoColor {
        name: "LINEAR".into(),
        reserved: 0,
        angle_raw: 0.0,
        shift: 0.0,
        stops: vec![
            GradientStop {
                value: 0.0,
                rgb: [255, 0, 0],
            },
            GradientStop {
                value: 1.0,
                rgb: [0, 0, 255],
            },
        ],
    })
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

fn mesh_area(geometry: &SemanticGeometry) -> f64 {
    let SemanticGeometry::Mesh(mesh) = geometry else {
        return 0.0;
    };
    mesh.triangles
        .iter()
        .map(|t| {
            let a = mesh.vertices[t[0] as usize];
            let b = mesh.vertices[t[1] as usize];
            let c = mesh.vertices[t[2] as usize];
            let ab = [b.x - a.x, b.y - a.y, b.z - a.z];
            let ac = [c.x - a.x, c.y - a.y, c.z - a.z];
            let cross = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt() / 2.0
        })
        .sum()
}

fn display_area(hatch: &HatchGeometry) -> f64 {
    hatch.display().iter().map(mesh_area).sum()
}

/// Total length of the pattern polylines (everything after the boundary loops).
fn pattern_length(hatch: &HatchGeometry) -> f64 {
    let boundaries = hatch.loops().len();
    let mut total = 0.0;
    for geometry in hatch.display().iter().skip(boundaries) {
        let SemanticGeometry::Polyline { points, .. } = geometry else {
            panic!("pattern display must be polylines, got {geometry:?}");
        };
        for pair in points.windows(2) {
            let dx = pair[1].x - pair[0].x;
            let dy = pair[1].y - pair[0].y;
            total += (dx * dx + dy * dy).sqrt();
        }
    }
    total
}

// ---------------------------------------------------------------------------
// G.1: style dispatch (solid area, pattern length, gradient region).
// ---------------------------------------------------------------------------

#[test]
fn solid_style_areas_on_three_concentric_loops() {
    // Regions: 1 = 64, 2 = 32, 3 = 4.
    let cases = [
        (HatchStyle::Normal, 68.0),  // {1,3}
        (HatchStyle::Outer, 64.0),   // {1}
        (HatchStyle::Ignore, 100.0), // {1,2,3}
    ];
    for (style, expected) in cases {
        let hatch = build(three_level(), style, HatchFill::Solid);
        let area = display_area(&hatch);
        assert!(
            (area - expected).abs() < 1e-6,
            "{style:?}: expected {expected}, got {area}"
        );
        // Every boundary loop is still in the display (snap/pick parity).
        assert_eq!(
            hatch.display().len(),
            hatch.loops().len() + 1,
            "{style:?}: boundary loops plus one fill mesh"
        );
    }
}

#[test]
fn solid_style_areas_on_two_disjoint_outer_loops() {
    // Two outer rings are depth 1, so every style fills both (16 + 16).
    for style in [HatchStyle::Normal, HatchStyle::Outer, HatchStyle::Ignore] {
        let hatch = build(two_disjoint(), style, HatchFill::Solid);
        let area = display_area(&hatch);
        assert!(
            (area - 32.0).abs() < 1e-6,
            "{style:?}: expected 32, got {area}"
        );
    }
}

#[test]
fn pattern_line_length_per_style() {
    // The raw base point (y = 1) is scaled with the pattern (scale 2), so the
    // horizontal families land at y = 0, 2, 4, 6, 8 (spacing 2). Rows inside
    // the 2..8 hole band keep only the outer stubs.
    let cases = [
        (HatchStyle::Normal, 34.0), // 10+4+6+4+10
        (HatchStyle::Outer, 32.0),  // 10+4+4+4+10
        (HatchStyle::Ignore, 50.0), // 10*5
    ];
    for (style, expected) in cases {
        let hatch = build(three_level(), style, pattern_fill(2.0));
        let length = pattern_length(&hatch);
        assert!(
            (length - expected).abs() < 1e-6,
            "{style:?}: expected {expected} line length, got {length}"
        );
    }
}

#[test]
fn gradient_bake_region_per_style() {
    let cases = [
        (HatchStyle::Normal, 68.0),
        (HatchStyle::Outer, 64.0),
        (HatchStyle::Ignore, 100.0),
    ];
    for (style, expected) in cases {
        let hatch = build(three_level(), style, gradient_fill());
        let area = display_area(&hatch);
        assert!(
            (area - expected).abs() < 1e-6,
            "{style:?}: expected gradient region {expected}, got {area}"
        );
        let mesh = hatch
            .display()
            .iter()
            .find_map(|g| match g {
                SemanticGeometry::Mesh(mesh) => Some(mesh),
                _ => None,
            })
            .expect("gradient mesh");
        assert_eq!(
            mesh.colors.len(),
            mesh.vertices.len(),
            "{style:?}: every vertex is baked"
        );
    }
}

/// A horizontal user pattern whose rows (`y = 0.5 + k`) never coincide with a
/// loop edge, so the expected lengths do not depend on the half-open crossing
/// rule at a boundary row.
fn interior_pattern_fill() -> HatchFill {
    HatchFill::Pattern(HatchPattern {
        name: "USER".into(),
        angle: 0.0,
        scale: 1.0,
        double: false,
        lines: vec![HatchPatternLine {
            angle: 0.0,
            base_x: 0.0,
            base_y: 0.5,
            offset_x: 0.0,
            offset_y: 1.0,
            dashes: Vec::new(),
        }],
    })
}

#[test]
fn pattern_and_gradient_styles_on_two_disjoint_outer_loops() {
    // The solid case is covered above; the style filter also feeds the pattern
    // and gradient regions, so the disjoint configuration is exercised for
    // those fills too. Both loops are depth 1 (two 4x4 squares), so every style
    // keeps both and no clipping difference is expected.
    for style in [HatchStyle::Normal, HatchStyle::Outer, HatchStyle::Ignore] {
        let hatch = build(two_disjoint(), style, interior_pattern_fill());
        // Four rows per square (y = 0.5/1.5/2.5/3.5 and 10.5/11.5/12.5/13.5),
        // each 4 long: 2 squares * 4 rows * 4 = 32.
        let length = pattern_length(&hatch);
        assert!(
            (length - 32.0).abs() < 1e-6,
            "{style:?}: expected 32 pattern length, got {length}"
        );

        let hatch = build(two_disjoint(), style, gradient_fill());
        let area = display_area(&hatch);
        assert!(
            (area - 32.0).abs() < 1e-6,
            "{style:?}: expected gradient region 32, got {area}"
        );
        let mesh = hatch
            .display()
            .iter()
            .find_map(|g| match g {
                SemanticGeometry::Mesh(mesh) => Some(mesh),
                _ => None,
            })
            .expect("gradient mesh");
        assert_eq!(
            mesh.colors.len(),
            mesh.vertices.len(),
            "{style:?}: every gradient vertex is baked"
        );
    }
}

// ---------------------------------------------------------------------------
// Gradient angle unit (DXF group 460) and the baked ramp direction.
// ---------------------------------------------------------------------------

/// A LINEAR two-colour gradient at `angle` radians: red at stop 0, blue at 1.
fn linear_two_color(angle: f64) -> HatchFill {
    HatchFill::Gradient(HatchGradient::TwoColor {
        name: "LINEAR".into(),
        reserved: 0,
        angle_raw: angle,
        shift: 0.0,
        stops: vec![
            GradientStop {
                value: 0.0,
                rgb: [255, 0, 0],
            },
            GradientStop {
                value: 1.0,
                rgb: [0, 0, 255],
            },
        ],
    })
}

/// The baked gradient mesh a hatch's display carries.
fn gradient_mesh(hatch: &HatchGeometry) -> &Mesh {
    hatch
        .display()
        .iter()
        .find_map(|geometry| match geometry {
            SemanticGeometry::Mesh(mesh) => Some(mesh),
            _ => None,
        })
        .expect("gradient mesh")
}

/// Recover the baked linear ramp direction (degrees in `0..180`) from the
/// per-vertex colours instead of comparing pixels.
///
/// Stop 0 is pure red and stop 1 pure blue, so the ramp parameter is
/// `t = 1 - red/255`, and a linear ramp makes it affine in the hatch plane:
/// `t = a + u*x + v*y`, with `atan2(v, u)` the ramp direction (the positive
/// projection scale cancels out of the angle). `mesh` must be expressed in the
/// hatch plane's own coordinates.
fn baked_ramp_direction(mesh: &Mesh) -> f64 {
    assert_eq!(
        mesh.colors.len(),
        mesh.vertices.len(),
        "every vertex must be baked"
    );
    let samples: Vec<([f64; 2], f64)> = mesh
        .vertices
        .iter()
        .zip(&mesh.colors)
        .map(|(p, color)| ([p.x, p.y], 1.0 - color[0] as f64 / 255.0))
        .collect();
    let count = samples.len() as f64;
    let mean_x = samples.iter().map(|(p, _)| p[0]).sum::<f64>() / count;
    let mean_y = samples.iter().map(|(p, _)| p[1]).sum::<f64>() / count;
    let mean_t = samples.iter().map(|(_, t)| *t).sum::<f64>() / count;
    let (mut sxx, mut syy, mut sxy, mut sxt, mut syt) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for ([x, y], t) in &samples {
        let (dx, dy, dt) = (x - mean_x, y - mean_y, t - mean_t);
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
        sxt += dx * dt;
        syt += dy * dt;
    }
    let determinant = sxx * syy - sxy * sxy;
    assert!(
        determinant > 1e-12,
        "the fill must span two dimensions to recover a direction"
    );
    let u = (sxt * syy - syt * sxy) / determinant;
    let v = (syt * sxx - sxt * sxy) / determinant;
    v.atan2(u).to_degrees().rem_euclid(180.0)
}

#[test]
fn gradient_angle_is_stored_in_radians_not_degrees() {
    // Group 460 is the gradient angle **in radians** (acadrust's field doc and
    // the DXF reference). A hatch storing 30 degrees as `FRAC_PI_6` must bake a
    // ramp whose direction is 30 degrees; reading the same number as degrees
    // would put the ramp at ~0.52 degrees instead, so this pins the unit.
    let hatch = build(
        vec![rect(0.0, 0.0, 1.0, 1.0)],
        HatchStyle::Normal,
        linear_two_color(std::f64::consts::FRAC_PI_6),
    );
    let direction = baked_ramp_direction(gradient_mesh(&hatch));
    assert!(
        (direction - 30.0).abs() < 1.0,
        "a stored FRAC_PI_6 must bake a 30 degree ramp, got {direction} degrees"
    );
}

// ---------------------------------------------------------------------------
// Pattern definition composite (ADR U-1/U-7 fix).
// ---------------------------------------------------------------------------

#[test]
fn pattern_composite_rotates_offset_and_scales_base() {
    let pattern = HatchPattern {
        name: "USER".into(),
        angle: std::f64::consts::FRAC_PI_2,
        scale: 2.0,
        double: false,
        lines: vec![HatchPatternLine {
            angle: 0.0,
            base_x: 1.0,
            base_y: 0.0,
            offset_x: 0.0,
            offset_y: 1.0,
            dashes: vec![0.5],
        }],
    };
    let families = pattern.families();
    assert_eq!(families.len(), 1);
    let family = &families[0];
    // Base: scale 2 then rotate +90 degrees: (1,0) -> (2,0) -> (0,2).
    assert!((family.base[0] - 0.0).abs() < 1e-12, "{:?}", family.base);
    assert!((family.base[1] - 2.0).abs() < 1e-12, "{:?}", family.base);
    // Offset: (0,1) -> (0,2) -> (-2,0); perpendicular to the line direction.
    assert!(
        (family.offset[0] + 2.0).abs() < 1e-12,
        "{:?}",
        family.offset
    );
    assert!(
        (family.offset[1] - 0.0).abs() < 1e-12,
        "{:?}",
        family.offset
    );
    assert!(
        (family.angle - std::f64::consts::FRAC_PI_2).abs() < 1e-12,
        "{}",
        family.angle
    );
    assert!((family.dashes[0] - 1.0).abs() < 1e-12);
}

/// The hatch-level similarity `scale * (x + iy) * exp(i * angle)`, written here
/// as plain complex multiplication so the expectation does not reuse
/// [`HatchPattern::families`]'s own expression.
fn hatch_similarity(scale: f64, angle: f64, x: f64, y: f64) -> [f64; 2] {
    let (sin, cos) = angle.sin_cos();
    [scale * (cos * x - sin * y), scale * (sin * x + cos * y)]
}

#[test]
fn pattern_families_are_the_similarity_transform_of_the_raw_rows() {
    // A non-axis-aligned hatch angle so a rotation-sign or rotation-direction
    // error cannot cancel, and rows that carry both a base and an offset so the
    // comparison is not fixed by a single degree of freedom.
    let hatch_angle = std::f64::consts::FRAC_PI_6; // 30 degrees
    let scale = 1.5;
    let rows = [
        HatchPatternLine {
            angle: std::f64::consts::FRAC_PI_4,
            base_x: 2.0,
            base_y: -1.0,
            offset_x: 1.0,
            offset_y: 3.0,
            dashes: vec![2.0, -1.0],
        },
        HatchPatternLine {
            angle: -1.0,
            base_x: 0.5,
            base_y: 4.0,
            offset_x: -2.0,
            offset_y: 0.5,
            dashes: Vec::new(),
        },
    ];
    let pattern = HatchPattern {
        name: "ARBITRARY".into(),
        angle: hatch_angle,
        scale,
        double: false,
        lines: rows.to_vec(),
    };
    let families = pattern.families();
    assert_eq!(families.len(), rows.len(), "one family per raw row");

    for (row, family) in rows.iter().zip(&families) {
        // The line direction is the raw row angle turned by the hatch angle.
        let want_angle = row.angle + hatch_angle;
        assert!(
            (family.angle - want_angle).abs() < 1e-12,
            "{:?}: angle {} != raw {} + hatch {}",
            row,
            family.angle,
            row.angle,
            hatch_angle
        );
        // Dashes are pattern-space lengths, so only the scale applies.
        assert_eq!(family.dashes.len(), row.dashes.len());
        for (got, dash) in family.dashes.iter().zip(&row.dashes) {
            assert!(
                (got - dash * scale).abs() < 1e-12,
                "{:?}: dash {got} != raw {dash} * {scale}",
                row
            );
        }
        // Line `k` of a family is `base + k * offset`. The raw row defines the
        // same lines in pattern space, so the similarity transform of
        // `raw_base + k * raw_offset` must be exactly `base + k * offset`:
        // `k = 0` pins the base, `k = 1` pins the offset, and the remaining
        // samples pin the direction. Comparing transformed *points* keeps the
        // expectation independent of how `families()` splits base and offset
        // (and would fail if it applied the similarity twice).
        for k in -2..=2 {
            let raw = [
                row.base_x + row.offset_x * k as f64,
                row.base_y + row.offset_y * k as f64,
            ];
            let want = hatch_similarity(scale, hatch_angle, raw[0], raw[1]);
            let got = [
                family.base[0] + family.offset[0] * k as f64,
                family.base[1] + family.offset[1] * k as f64,
            ];
            assert!(
                (got[0] - want[0]).abs() < 1e-12 && (got[1] - want[1]).abs() < 1e-12,
                "{:?}: line {k} point {got:?} != similarity transform {want:?}",
                row
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Transforms (ADR E).
// ---------------------------------------------------------------------------

fn first_boundary_point(hatch: &HatchGeometry) -> Point3 {
    match &hatch.display()[0] {
        SemanticGeometry::Polyline { points, .. } => points[0],
        other => panic!("expected boundary polyline, got {other:?}"),
    }
}

#[test]
fn transform_translates_loops_plane_seeds_and_rebuilds_display() {
    let mut hatch = build(three_level(), HatchStyle::Normal, HatchFill::Solid);
    hatch.seeds.push([1.0, 2.0]);
    let moved = hatch
        .transformed(&Transform3::translation(Point3 {
            x: 5.0,
            y: -2.0,
            z: 3.0,
        }))
        .expect("translate");
    let p = first_boundary_point(&moved);
    assert!((p.x - 5.0).abs() < 1e-9, "{p:?}");
    assert!((p.y + 2.0).abs() < 1e-9, "{p:?}");
    assert!((p.z - 3.0).abs() < 1e-9, "{p:?}");
    assert!((moved.plane().elevation - 3.0).abs() < 1e-9);
    assert_eq!(moved.seeds(), &[[6.0, 0.0]]);
    // The rebuilt display covers exactly the same area.
    assert!((display_area(&moved) - 68.0).abs() < 1e-6);
}

#[test]
fn transform_rotates_loops_and_pattern_direction() {
    let hatch = build(three_level(), HatchStyle::Normal, pattern_fill(2.0));
    let rotated = hatch
        .transformed(&rotation_z(std::f64::consts::FRAC_PI_2))
        .expect("rotate");
    match &rotated.display()[0] {
        SemanticGeometry::Polyline { points, .. } => {
            // (10,0) -> (0,10).
            assert!((points[1].x).abs() < 1e-9, "{:?}", points[1]);
            assert!((points[1].y - 10.0).abs() < 1e-9, "{:?}", points[1]);
        }
        other => panic!("expected boundary polyline, got {other:?}"),
    }
    let HatchFill::Pattern(pattern) = rotated.fill() else {
        panic!("pattern fill expected");
    };
    assert!(
        (pattern.angle - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
        "pattern angle must follow the in-plane rotation: {}",
        pattern.angle
    );
}

#[test]
fn transform_rotates_the_gradient_ramp_with_the_geometry() {
    // A gradient hatch starts at angle 0 (ramp along +X) and is rotated +30°
    // about Z. The ramp must follow the loops. Pre-fix this test recovered 0°
    // (the geometry turned, the ramp stayed behind); rotating by `-delta`
    // instead would recover -30° (i.e. 150° after `rem_euclid(180)`), and doing
    // nothing would recover 0°, so the 30° assertion fails both wrong signs.
    let hatch = build(
        vec![rect(0.0, 0.0, 1.0, 1.0)],
        HatchStyle::Normal,
        linear_two_color(0.0),
    );
    let before = baked_ramp_direction(gradient_mesh(&hatch));
    assert!((before - 0.0).abs() < 1.0, "unrotated ramp: {before}°");

    let rotated = hatch
        .transformed(&rotation_z(std::f64::consts::FRAC_PI_6))
        .expect("rotate");
    let HatchFill::Gradient(gradient) = rotated.fill() else {
        panic!("gradient fill expected");
    };
    let angle_raw = match gradient {
        HatchGradient::TwoColor { angle_raw, .. }
        | HatchGradient::SingleColor { angle_raw, .. } => *angle_raw,
    };
    assert!(
        (angle_raw - std::f64::consts::FRAC_PI_6).abs() < 1e-9,
        "the stored angle must follow the +30° in-plane rotation: {angle_raw}"
    );

    // ...and the bake agrees. A Z rotation keeps the hatch plane equal to world
    // XY, so the display vertices are already the plane coordinates.
    let after = baked_ramp_direction(gradient_mesh(&rotated));
    assert!(
        (after - 30.0).abs() < 1.0,
        "a +30° hatch rotation must move the baked ramp to 30°, got {after}°"
    );
}

#[test]
fn gradient_rotation_applies_to_both_colour_modes() {
    // `rotated` has one arm per DXF colour mode; a copy-paste slip in either
    // would leave that mode's ramp behind on a rotated hatch.
    let delta = std::f64::consts::FRAC_PI_3;
    let stops = vec![GradientStop {
        value: 0.0,
        rgb: [1, 2, 3],
    }];
    let two = HatchGradient::TwoColor {
        name: "LINEAR".into(),
        reserved: 7,
        angle_raw: 0.25,
        shift: 0.5,
        stops: stops.clone(),
    };
    let single = HatchGradient::SingleColor {
        name: "LINEAR".into(),
        reserved: 7,
        angle_raw: 0.25,
        shift: 0.5,
        tint: -0.5,
        stops,
    };

    let HatchGradient::TwoColor {
        angle_raw,
        shift,
        reserved,
        ..
    } = two.rotated(delta)
    else {
        panic!("two-colour gradient expected");
    };
    assert!((angle_raw - (0.25 + delta)).abs() < 1e-12, "{angle_raw}");
    assert_eq!(shift, 0.5);
    assert_eq!(reserved, 7);

    let HatchGradient::SingleColor {
        angle_raw,
        shift,
        reserved,
        tint,
        ..
    } = single.rotated(delta)
    else {
        panic!("single-colour gradient expected");
    };
    assert!((angle_raw - (0.25 + delta)).abs() < 1e-12, "{angle_raw}");
    assert_eq!(shift, 0.5);
    assert_eq!(reserved, 7);
    assert_eq!(tint, -0.5);
}

#[test]
fn transform_uniform_scale_scales_loops_and_pattern() {
    let hatch = build(three_level(), HatchStyle::Normal, pattern_fill(2.0));
    let scaled = hatch.transformed(&Transform3::scale(2.0)).expect("scale");
    match &scaled.display()[0] {
        SemanticGeometry::Polyline { points, .. } => {
            assert!((points[1].x - 20.0).abs() < 1e-9, "{:?}", points[1]);
            assert!((points[2].y - 20.0).abs() < 1e-9, "{:?}", points[2]);
        }
        other => panic!("expected boundary polyline, got {other:?}"),
    }
    let HatchFill::Pattern(pattern) = scaled.fill() else {
        panic!("pattern fill expected");
    };
    assert!((pattern.scale - 4.0).abs() < 1e-9, "{}", pattern.scale);
}

#[test]
fn transform_refuses_non_uniform_and_mirror() {
    let hatch = build(two_disjoint(), HatchStyle::Normal, HatchFill::Solid);
    let non_uniform = Transform3::from_basis(
        Point3 {
            x: 2.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    );
    assert!(matches!(
        hatch.transformed(&non_uniform),
        Err(CadError::Unsupported(_))
    ));
    let mirror = Transform3::scale(-1.0);
    assert!(matches!(
        hatch.transformed(&mirror),
        Err(CadError::Unsupported(_))
    ));
}

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

// ---------------------------------------------------------------------------
// Creation-path rejections (ADR A.3; the import path never rejects).
// ---------------------------------------------------------------------------

fn creation(loops: Vec<HatchLoop>) -> CadResult<HatchGeometry> {
    HatchGeometry::new(
        loops,
        HatchPlane::default(),
        HatchStyle::Normal,
        HatchFill::Solid,
        Vec::new(),
        None,
        HatchPatternType::UserDefined,
    )
}

#[test]
fn creation_rejects_self_intersecting_loop() {
    let bowtie = HatchLoop::new(
        vec![[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]],
        Vec::new(),
        HatchLoopFlags::default(),
    );
    assert!(matches!(
        creation(vec![bowtie]),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn creation_rejects_touching_and_duplicate_loops() {
    let touching = vec![rect(0.0, 0.0, 10.0, 10.0), rect(10.0, 0.0, 20.0, 10.0)];
    assert!(matches!(creation(touching), Err(CadError::InvalidInput(_))));
    let duplicate = vec![rect(0.0, 0.0, 10.0, 10.0), rect(0.0, 0.0, 10.0, 10.0)];
    assert!(matches!(
        creation(duplicate),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn transform_moves_source_edges_with_the_loops() {
    let mut loop_ = rect(0.0, 0.0, 10.0, 10.0);
    loop_.source_edges = Some(vec![BoundaryEdge::Line {
        start: [0.0, 0.0],
        end: [10.0, 0.0],
    }]);
    let hatch = HatchGeometry::new(
        vec![loop_],
        HatchPlane::default(),
        HatchStyle::Normal,
        HatchFill::Solid,
        Vec::new(),
        None,
        HatchPatternType::UserDefined,
    )
    .expect("valid hatch");
    let moved = hatch
        .transformed(&Transform3::translation(Point3 {
            x: 5.0,
            y: 0.0,
            z: 0.0,
        }))
        .expect("translate");
    let edges = moved.loops()[0]
        .source_edges
        .as_ref()
        .expect("source edges survive the transform");
    match &edges[0] {
        BoundaryEdge::Line { start, end } => {
            assert!((start[0] - 5.0).abs() < 1e-9, "{start:?}");
            assert!((end[0] - 15.0).abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line edge, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Import-path depth probes (ADR B6: the import path never rejects, so a loop
// may touch another loop's boundary).
// ---------------------------------------------------------------------------

#[test]
fn import_depth_probes_several_vertices_when_the_first_touches_a_boundary() {
    // The triangle's first vertex sits on the outer loop's right edge, where the
    // even-odd crossing test reports "outside"; its other two vertices are
    // strictly inside. The creation path would reject this set, but the import
    // path keeps it, so a first-vertex-only depth would call the triangle an
    // outer loop and `Ignore` would fill it instead of dropping it.
    let loops = vec![
        rect(0.0, 0.0, 10.0, 10.0),
        HatchLoop::new(
            vec![[10.0, 5.0], [2.0, 2.0], [2.0, 8.0]],
            Vec::new(),
            HatchLoopFlags::default(),
        ),
    ];
    let (hatch, reasons) = HatchGeometry::from_import(
        loops,
        HatchPlane::default(),
        HatchStyle::Ignore,
        HatchFill::Solid,
        Vec::new(),
        None,
        HatchPatternType::UserDefined,
    );
    assert!(reasons.is_empty(), "{reasons:?}");
    // `Ignore` keeps depth-0 loops only: the outer square (100), not the
    // triangle (24) that is nested inside it.
    assert!((display_area(&hatch) - 100.0).abs() < 1e-6);
}

/// Fill-level guard for `sample_loop`: a four-quadrant loop whose bulges match
/// the arc side samples onto the analytic circle, and one whose bulges do not
/// samples the inward scallop — in both vertex orders and both bulge signs.
///
/// The equivalent **display**-level assertions live in
/// `cad-geometry/tests/bulge_display.rs`; the two must never diverge again (the
/// display used to mirror every arc across its chord while the fill did not).
#[test]
fn bulged_hatch_loop_samples_follow_the_bulge_sign_not_the_winding() {
    let radius = 10.0;
    let bulge = std::f64::consts::FRAC_PI_8.tan();
    let mut ccw_points: Vec<[f64; 2]> = (0..4)
        .map(|index| {
            let angle = index as f64 * std::f64::consts::FRAC_PI_2;
            let (sin, cos) = angle.sin_cos();
            [radius * cos, radius * sin]
        })
        .collect();
    let scallop = radius * (std::f64::consts::SQRT_2 - 1.0);
    for ccw in [true, false] {
        if !ccw {
            ccw_points.reverse();
        }
        for sign in [1.0, -1.0] {
            let loop_ = HatchLoop::new(
                ccw_points.clone(),
                vec![sign * bulge; 4],
                HatchLoopFlags::default(),
            );
            let ring = sample_loop(&loop_);
            let (min, max) = ring.iter().fold((f64::INFINITY, 0.0f64), |acc, q| {
                let r = q[0].hypot(q[1]);
                (acc.0.min(r), acc.1.max(r))
            });
            assert!(
                max <= radius + 1e-9,
                "ccw={ccw} sign={sign}: a fill sample overshoots the circle (max {max})"
            );
            if ccw == (sign > 0.0) {
                assert!(
                    (min - radius).abs() < 1e-9,
                    "ccw={ccw} sign={sign}: expected the analytic circle, min {min}"
                );
            } else {
                assert!(
                    (min - scallop).abs() < 1e-9,
                    "ccw={ccw} sign={sign}: expected the scallop apex {scallop}, min {min}"
                );
            }
        }
    }
}

#[path = "../../../cad-domain/tests/support/bulge_cases.rs"]
mod bulge_cases;

/// Regression guard for the hatch **fill** sampler (`geometry::push_bulge_points`):
/// the loop samples must lie on the analytic circle and bow to the side the
/// signed bulge encodes. The mirrored centre this guards against mirrored every
/// fill edge, while the semicircle-only tests used before were invariant.
#[test]
fn hatch_fill_bulge_samples_match_the_analytic_arc() {
    bulge_cases::assert_table_is_hand_computed();
    for case in bulge_cases::table() {
        let mut out: Vec<[f64; 2]> = Vec::new();
        push_bulge_points(&mut out, case.p0, case.p1, case.bulge);
        bulge_cases::assert_samples(
            "cad-domain hatch fill (push_bulge_points)",
            &case,
            &out,
            1e-9 * (1.0 + case.radius),
        );
        // The sampler caps its angular step at 0.05 rad with at least 2 steps.
        let steps = ((case.included_angle.abs() / 0.05).ceil() as usize).clamp(2, 4096);
        let half_step = 0.5 * case.included_angle.abs() / steps as f64;
        bulge_cases::assert_apex(
            "cad-domain hatch fill (push_bulge_points)",
            &case,
            &out,
            2.0 * case.radius * (1.0 - half_step.cos()) + 1e-9,
        );
    }
}
