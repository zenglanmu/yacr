use super::fill::cross2;
use super::*;

#[test]
fn simplify_drops_collinear_points_but_keeps_corners() {
    let square = vec![
        [0.0, 0.0],
        [1.0, 0.0],
        [2.0, 0.0],
        [2.0, 2.0],
        [0.0, 2.0],
        [0.0, 0.0],
    ];
    let simplified = simplify(&square, 1e-6);
    assert!(simplified.len() < square.len(), "{simplified:?}");
    assert!(simplified.contains(&[2.0, 2.0]), "{simplified:?}");
    assert!(simplified.contains(&[0.0, 2.0]), "{simplified:?}");
}

#[test]
fn triangulate_refuses_oversized_contours() {
    let ring: Vec<[f64; 2]> = (0..(MAX_FILL_POINTS + 1))
        .map(|i| {
            let t = i as f64 * 0.01;
            [t.cos(), t.sin()]
        })
        .collect();
    assert!(triangulate(&ring).is_empty());
}

#[test]
fn triangulates_a_square_into_two_triangles() {
    let square = vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
    let tris = triangulate(&square);
    assert_eq!(tris.len(), 2, "{tris:?}");
    // Indices must reference real vertices and cover the area.
    let mut area = 0.0;
    for t in &tris {
        for &i in t {
            assert!((i as usize) < square.len());
        }
        let a = square[t[0] as usize];
        let b = square[t[1] as usize];
        let c = square[t[2] as usize];
        area += cross2(a, b, c).abs() / 2.0;
    }
    assert!((area - 4.0).abs() < 1e-9, "area {area}");
}

#[test]
fn triangulates_clockwise_input_the_same() {
    let square = vec![[0.0, 0.0], [0.0, 2.0], [2.0, 2.0], [2.0, 0.0]];
    assert_eq!(triangulate(&square).len(), 2);
}

#[test]
fn degenerate_contours_produce_no_triangles() {
    assert!(triangulate(&[]).is_empty());
    assert!(triangulate(&[[0.0, 0.0], [1.0, 0.0]]).is_empty());
    // Collinear points have no area for ear clipping.
    assert!(triangulate(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]]).is_empty());
}

#[test]
fn pattern_lines_fill_a_square() {
    let square = vec![vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]];
    let family = PatternLine {
        angle: 0.0,
        base: [0.0, 0.5],
        offset: [0.0, 1.0],
        dashes: Vec::new(),
    };
    let lines = pattern_polylines(&square, &[family]);
    assert_eq!(lines.len(), 4, "{lines:?}");
    for line in &lines {
        assert_eq!(line.len(), 2);
        assert!((line[0][0] - 0.0).abs() < 1e-9);
        assert!((line[1][0] - 4.0).abs() < 1e-9);
    }
}

#[test]
fn pattern_even_odd_excludes_a_hole() {
    let outer = vec![[0.0, 0.0], [6.0, 0.0], [6.0, 6.0], [0.0, 6.0]];
    let hole = vec![[2.0, 2.0], [2.0, 4.0], [4.0, 4.0], [4.0, 2.0]];
    let family = PatternLine {
        angle: 0.0,
        base: [0.0, 0.5],
        offset: [0.0, 1.0],
        dashes: Vec::new(),
    };
    let lines = pattern_polylines(&[outer, hole], &[family]);
    // Rows y=0.5, 1.5, 4.5, 5.5 span the full width; rows y=2.5, 3.5 are
    // split by the hole into two 2-wide segments.
    let full = lines
        .iter()
        .filter(|l| (l[1][0] - l[0][0] - 6.0).abs() < 1e-9)
        .count();
    let split = lines
        .iter()
        .filter(|l| (l[1][0] - l[0][0] - 2.0).abs() < 1e-9)
        .count();
    assert_eq!(full, 4, "{lines:?}");
    assert_eq!(split, 4, "{lines:?}");
    assert_eq!(lines.len(), full + split);
}

#[test]
fn dashes_break_a_span() {
    let square = vec![vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]];
    let family = PatternLine {
        angle: 0.0,
        base: [0.0, 0.5],
        offset: [0.0, 2.0],
        dashes: vec![1.0, -1.0],
    };
    let lines = pattern_polylines(&square, &[family]);
    // Two rows, each split into dash/gap/dash -> 2 segments per row.
    assert_eq!(lines.len(), 4, "{lines:?}");
    for line in &lines {
        assert!((line[1][0] - line[0][0] - 1.0).abs() < 1e-9, "{line:?}");
    }
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Loop {
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

#[test]
fn fill_rings_single_ring_matches_its_area() {
    let mesh = fill_rings(&[rect(0.0, 0.0, 3.0, 2.0)]).expect("single ring fills");
    assert!(!mesh.triangles.is_empty());
    assert!((mesh.area() - 6.0).abs() < 1e-9, "area {}", mesh.area());
}

#[test]
fn fill_rings_donut_excludes_the_hole() {
    // 10x10 outer with a 4x4 hole -> 100 - 16 = 84.
    let outer = rect(0.0, 0.0, 10.0, 10.0);
    let hole = rect(3.0, 3.0, 7.0, 7.0);
    let mesh = fill_rings(&[outer, hole]).expect("donut fills");
    assert!(
        (mesh.area() - 84.0).abs() < 1e-6,
        "donut area {}",
        mesh.area()
    );
    // No triangle may sit entirely inside the hole.
    for t in &mesh.triangles {
        let cx = (mesh.vertices[t[0] as usize][0]
            + mesh.vertices[t[1] as usize][0]
            + mesh.vertices[t[2] as usize][0])
            / 3.0;
        let cy = (mesh.vertices[t[0] as usize][1]
            + mesh.vertices[t[1] as usize][1]
            + mesh.vertices[t[2] as usize][1])
            / 3.0;
        let in_hole = (3.0..7.0).contains(&cx) && (3.0..7.0).contains(&cy);
        assert!(!in_hole, "triangle centroid {cx},{cy} inside the hole");
    }
}

#[test]
fn fill_rings_tangential_hole_keeps_its_depth() {
    // An imported boundary set may keep a ring that touches another ring
    // (ADR B6: import never rejects). Here the inner ring's first vertex sits
    // exactly on the outer square's right edge, where a single-vertex
    // containment probe reads "outside" and would promote the hole to a
    // second outlying solid. The majority probe must still classify it as a
    // hole: 10x10 outer minus a triangle of area 12 = 88.
    let outer = rect(0.0, 0.0, 10.0, 10.0);
    let tangent_hole = vec![[10.0, 5.0], [6.0, 2.0], [6.0, 8.0]];
    let mesh = fill_rings(&[outer, tangent_hole]).expect("tangential hole still fills");
    assert!(
        (mesh.area() - 88.0).abs() < 1e-6,
        "tangential donut area {}",
        mesh.area()
    );
}

#[test]
fn fill_rings_nested_island_is_solid_again() {
    // Even-odd: 10x10 - 6x6 hole + 2x2 island = 100 - 36 + 4 = 68.
    let mesh = fill_rings(&[
        rect(0.0, 0.0, 10.0, 10.0),
        rect(2.0, 2.0, 8.0, 8.0),
        rect(4.0, 4.0, 6.0, 6.0),
    ])
    .expect("nested rings fill");
    assert!(
        (mesh.area() - 68.0).abs() < 1e-6,
        "nested area {}",
        mesh.area()
    );
}

#[test]
fn fill_rings_two_disjoint_outers_both_fill() {
    let mesh = fill_rings(&[rect(0.0, 0.0, 2.0, 2.0), rect(5.0, 0.0, 6.0, 1.0)])
        .expect("disjoint outers fill");
    assert!((mesh.area() - 5.0).abs() < 1e-9, "area {}", mesh.area());
}

#[test]
fn fill_rings_handles_a_polygon_with_apex_bands() {
    // A triangle tapers to an apex at the top and bottom of its band
    // decomposition; the fill must not drop those bands.
    let mesh = fill_rings(&[vec![[0.0, 0.0], [2.0, 0.0], [1.0, 2.0]]]).expect("triangle fills");
    assert!((mesh.area() - 2.0).abs() < 1e-9, "area {}", mesh.area());
    // A general (non-axis-aligned) pentagon must also fill.
    let pentagon = vec![
        [1.0, 0.0],
        [0.309, 0.951],
        [-0.809, 0.588],
        [-0.809, -0.588],
        [0.309, -0.951],
    ];
    let pent = fill_rings(&[pentagon]).expect("pentagon fills");
    assert!(pent.area() > 2.0, "pentagon area {}", pent.area());
}

#[test]
fn fill_rings_over_budget_is_an_error_not_a_partial_fill() {
    let ring: Loop = (0..=MAX_FILL_POINTS)
        .map(|i| {
            let t = i as f64 * 0.01;
            [t.cos(), t.sin()]
        })
        .collect();
    assert!(matches!(fill_rings(&[ring]), Err(FillError::Budget { .. })));
}

#[test]
fn fill_rings_rejects_non_finite_and_degenerate() {
    assert!(matches!(fill_rings(&[]), Err(FillError::Empty)));
    assert!(matches!(
        fill_rings(&[vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]]]),
        Err(FillError::Empty)
    ));
    assert!(matches!(
        fill_rings(&[vec![[0.0, 0.0], [f64::NAN, 0.0], [1.0, 1.0]]]),
        Err(FillError::Degenerate)
    ));
}

#[test]
fn fill_rings_rejects_self_intersecting_bowtie() {
    // A figure-eight ring has zero net even-odd area but a naive ear clip
    // would happily fill both lobes; the area cross-check refuses it.
    let bowtie = vec![[0.0, 0.0], [2.0, 2.0], [2.0, 0.0], [0.0, 2.0]];
    match fill_rings(&[bowtie]) {
        Err(FillError::Degenerate) | Err(FillError::Empty) => {}
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// --- gradient fills ---

fn linear_def(angle: f64, stops: Vec<GradientStop>) -> GradientDef {
    GradientDef {
        kind: GradientKind::Linear,
        angle,
        shift: 0.0,
        single_color: false,
        tint: 0.0,
        stops,
    }
}

fn red_blue() -> Vec<GradientStop> {
    vec![
        GradientStop {
            value: 0.0,
            rgb: [255, 0, 0],
        },
        GradientStop {
            value: 1.0,
            rgb: [0, 0, 255],
        },
    ]
}

#[test]
fn sample_gradient_interpolates_and_clamps_between_stops() {
    let def = linear_def(0.0, red_blue());
    assert_eq!(sample_gradient(&def, 0.0), [255, 0, 0]);
    assert_eq!(sample_gradient(&def, 1.0), [0, 0, 255]);
    assert_eq!(sample_gradient(&def, 0.5), [128, 0, 128]);
    // Out-of-range clamps to the end stops.
    assert_eq!(sample_gradient(&def, -3.0), [255, 0, 0]);
    assert_eq!(sample_gradient(&def, 9.0), [0, 0, 255]);
}

#[test]
fn sample_gradient_uses_a_single_stop_as_constant() {
    let def = linear_def(
        0.0,
        vec![GradientStop {
            value: 0.5,
            rgb: [10, 20, 30],
        }],
    );
    assert_eq!(sample_gradient(&def, 0.0), [10, 20, 30]);
    assert_eq!(sample_gradient(&def, 1.0), [10, 20, 30]);
}

#[test]
fn normalize_stops_sorts_and_drops_out_of_range() {
    let stops = normalize_stops(vec![
        GradientStop {
            value: 0.9,
            rgb: [1, 2, 3],
        },
        GradientStop {
            value: -0.5,
            rgb: [9, 9, 9],
        },
        GradientStop {
            value: 0.1,
            rgb: [4, 5, 6],
        },
    ]);
    assert_eq!(stops.len(), 2);
    assert_eq!(stops[0].rgb, [4, 5, 6]);
    assert_eq!(stops[1].rgb, [1, 2, 3]);
}

#[test]
fn linear_gradient_along_x_varies_left_to_right() {
    // Unit square fill; LINEAR angle 0 => red at x=0, blue at x=1.
    let mesh = fill_rings(&[rect(0.0, 0.0, 1.0, 1.0)]).expect("fills");
    let colors = gradient_vertex_colors(&mesh, &linear_def(0.0, red_blue()));
    assert_eq!(colors.len(), mesh.vertices.len());
    for (i, p) in mesh.vertices.iter().enumerate() {
        let expected_r = (255.0 * (1.0 - p[0])).round() as u8;
        let expected_b = (255.0 * p[0]).round() as u8;
        assert!(
            (colors[i][0] as i16 - expected_r as i16).abs() <= 1,
            "vertex {p:?} red {} vs {expected_r}",
            colors[i][0]
        );
        assert!(
            (colors[i][2] as i16 - expected_b as i16).abs() <= 1,
            "vertex {p:?} blue {} vs {expected_b}",
            colors[i][2]
        );
        assert_eq!(colors[i][1], 0, "green channel must stay 0");
    }
}

#[test]
fn linear_gradient_angle_rotates_the_ramp() {
    // Angle PI/2 => ramp along +Y: red at y=0, blue at y=1.
    let mesh = fill_rings(&[rect(0.0, 0.0, 1.0, 1.0)]).expect("fills");
    let colors =
        gradient_vertex_colors(&mesh, &linear_def(std::f64::consts::FRAC_PI_2, red_blue()));
    for (i, p) in mesh.vertices.iter().enumerate() {
        let expected_b = (255.0 * p[1]).round() as u8;
        assert!(
            (colors[i][2] as i16 - expected_b as i16).abs() <= 1,
            "vertex {p:?} blue {} vs {expected_b}",
            colors[i][2]
        );
    }
}

#[test]
fn single_color_gradient_blends_toward_white_or_black() {
    let mesh = fill_rings(&[rect(0.0, 0.0, 1.0, 1.0)]).expect("fills");
    let white_tint = GradientDef {
        kind: GradientKind::Linear,
        angle: 0.0,
        shift: 0.0,
        single_color: true,
        tint: 1.0,
        stops: vec![GradientStop {
            value: 0.0,
            rgb: [255, 0, 0],
        }],
    };
    let colors = gradient_vertex_colors(&mesh, &white_tint);
    // Leftmost vertex (x=0) is pure red; rightmost (x=1) is white.
    let left = mesh
        .vertices
        .iter()
        .position(|p| p[0] == 0.0)
        .expect("left vertex");
    let right = mesh
        .vertices
        .iter()
        .position(|p| p[0] == 1.0)
        .expect("right vertex");
    assert_eq!(colors[left], [255, 0, 0]);
    assert_eq!(colors[right], [255, 255, 255]);
}

#[test]
fn spherical_gradient_ramps_by_radius_across_the_boundary() {
    // A kite has boundary vertices at more than one radius once the y-band
    // decomposition adds edge crossings, so the radial ramp is visible. A
    // plain rectangle only yields its four corners and collapses to a
    // constant (a documented tessellation gap).
    let kite: Loop = vec![[1.0, 0.0], [0.0, 3.0], [-1.0, 0.0], [0.0, -1.0]];
    let mesh = fill_rings(&[kite]).expect("fills");
    let def = GradientDef {
        kind: GradientKind::Spherical,
        angle: 0.0,
        shift: 0.0,
        single_color: false,
        tint: 0.0,
        stops: red_blue(),
    };
    let colors = gradient_vertex_colors(&mesh, &def);
    // The gradient centre is the bounds midpoint (0, 1). The farthest
    // vertices from it (0, 3) and (0, -1) are at t = 1 (blue); the nearest,
    // (±1, 0), are at t = 1/sqrt(2) and must be redder.
    let color_at = |want: [f64; 2]| -> [u8; 3] {
        let i = mesh
            .vertices
            .iter()
            .position(|p| (p[0] - want[0]).abs() < 1e-9 && (p[1] - want[1]).abs() < 1e-9)
            .unwrap_or_else(|| panic!("vertex {want:?} present"));
        colors[i]
    };
    assert_eq!(color_at([0.0, 3.0]), [0, 0, 255]);
    assert_eq!(color_at([0.0, -1.0]), [0, 0, 255]);
    let near = color_at([1.0, 0.0]);
    assert!(near[2] < 255 && near[0] > 0, "near vertex {near:?}");
}

#[test]
fn unusable_gradient_produces_no_colors() {
    let mesh = fill_rings(&[rect(0.0, 0.0, 1.0, 1.0)]).expect("fills");
    let empty = linear_def(0.0, Vec::new());
    assert!(gradient_vertex_colors(&mesh, &empty).is_empty());
    let nan_angle = linear_def(f64::NAN, red_blue());
    assert!(gradient_vertex_colors(&mesh, &nan_angle).is_empty());
}

#[test]
fn shift_moves_the_ramp_start() {
    let mesh = fill_rings(&[rect(0.0, 0.0, 1.0, 1.0)]).expect("fills");
    let mut def = linear_def(0.0, red_blue());
    def.shift = 0.5;
    let colors = gradient_vertex_colors(&mesh, &def);
    // At shift 0.5 the left edge (t=0) maps to 0.5 => mid purple.
    let left = mesh
        .vertices
        .iter()
        .position(|p| p[0] == 0.0)
        .expect("left vertex");
    assert_eq!(colors[left], [128, 0, 128]);
}

#[test]
fn pattern_respects_a_nested_island() {
    // A hole inside the outer, an island inside the hole: even-odd means the
    // pattern must paint the island but not the hole ring.
    let family = PatternLine {
        angle: 0.0,
        base: [0.0, 0.5],
        offset: [0.0, 1.0],
        dashes: Vec::new(),
    };
    let lines = pattern_polylines(
        &[
            rect(0.0, 0.0, 10.0, 10.0),
            rect(2.0, 2.0, 8.0, 8.0),
            rect(4.0, 4.0, 6.0, 6.0),
        ],
        &[family],
    );
    // Row y=4.5 crosses the island (4..6) and must be painted; rows in the
    // hole but outside the island (e.g. y=2.5) must not span the hole.
    fn spans_y(lines: &[Loop], y: f64) -> Vec<(f64, f64)> {
        lines
            .iter()
            .filter(|l| (l[0][1] - y).abs() < 1e-9 && (l[1][1] - y).abs() < 1e-9)
            .map(|l| (l[0][0].min(l[1][0]), l[0][0].max(l[1][0])))
            .collect()
    }
    let island_row = spans_y(&lines, 4.5);
    assert_eq!(
        island_row,
        vec![(0.0, 2.0), (4.0, 6.0), (8.0, 10.0)],
        "island row {island_row:?}"
    );
    let hole_row = spans_y(&lines, 2.5);
    // y=2.5 is inside the hole but outside the island: only the two outer
    // bands (x=0..2 and 8..10) are painted.
    assert_eq!(hole_row, vec![(0.0, 2.0), (8.0, 10.0)], "{hole_row:?}");
}
