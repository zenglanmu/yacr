//! HATCH entity contract tests (ADR 0018 G.1): style dispatch by loop-set
//! filtering, the pattern/gradient regions per style, and the transform rules.

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
    // Horizontal families at y = 1, 3, 5, 7, 9 (spacing 2). Rows inside the
    // 2..8 hole band keep only the outer stubs.
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
