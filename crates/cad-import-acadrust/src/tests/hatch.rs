//! Hatch/gradient fills and dimension placement geometry.

use super::*;
use acadrust::entities::BoundaryPath;

#[test]
fn dimension_placement_scales_rotates_then_translates() {
    // Scale (2,2,1), rotate +90 deg about Z, translate (1,2,3).
    let t = placement_transform(
        acadrust::types::Vector3::new(1.0, 2.0, 3.0),
        std::f64::consts::FRAC_PI_2,
        acadrust::types::Vector3::new(2.0, 2.0, 1.0),
    );
    // (1,0,0) -> scale (2,0,0) -> rotate (0,2,0) -> translate (1,4,3).
    let p = t.apply_point(Point3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    });
    assert!((p.x - 1.0).abs() < 1e-9, "{p:?}");
    assert!((p.y - 4.0).abs() < 1e-9, "{p:?}");
    assert!((p.z - 3.0).abs() < 1e-9, "{p:?}");
}

#[test]
fn hatch_sweeps_follow_their_winding() {
    let tau = std::f64::consts::TAU;
    let half = std::f64::consts::PI / 2.0;
    assert!((directed_sweep(half, 0.0, false) + half).abs() < 1e-9);
    assert!((directed_sweep(0.0, half, true) - half).abs() < 1e-9);
    // A full turn is kept, not collapsed to zero.
    assert!((directed_sweep(0.0, 0.0, true) - tau).abs() < 1e-9);
}

fn rect_path(x0: f64, y0: f64, x1: f64, y1: f64) -> BoundaryPath {
    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    let mut path = BoundaryPath::new();
    for i in 0..4 {
        let a = corners[i];
        let b = corners[(i + 1) % 4];
        path.add_edge(BoundaryEdge::Line(acadrust::entities::LineEdge {
            start: acadrust::types::Vector2::new(a.0, a.1),
            end: acadrust::types::Vector2::new(b.0, b.1),
        }));
    }
    path
}

fn hatch_of(geometry: &SemanticGeometry) -> &HatchGeometry {
    let SemanticGeometry::Hatch(hatch) = geometry else {
        panic!("expected a first-class hatch, got {geometry:?}");
    };
    hatch
}

fn mesh_area(geometry: &SemanticGeometry) -> Option<f64> {
    let hatch = hatch_of(geometry);
    hatch.display().iter().find_map(|child| match child {
        SemanticGeometry::Mesh(m) => Some(
            m.triangles
                .iter()
                .map(|t| {
                    let a = m.vertices[t[0] as usize];
                    let b = m.vertices[t[1] as usize];
                    let c = m.vertices[t[2] as usize];
                    let ab = Point3 {
                        x: b.x - a.x,
                        y: b.y - a.y,
                        z: b.z - a.z,
                    };
                    let ac = Point3 {
                        x: c.x - a.x,
                        y: c.y - a.y,
                        z: c.z - a.z,
                    };
                    let cx = ab.y * ac.z - ab.z * ac.y;
                    let cy = ab.z * ac.x - ab.x * ac.z;
                    let cz = ab.x * ac.y - ab.y * ac.x;
                    (cx * cx + cy * cy + cz * cz).sqrt() / 2.0
                })
                .sum(),
        ),
        _ => None,
    })
}

#[test]
fn solid_hatch_with_a_hole_fills_the_solid_band_only() {
    let mut hatch = acadrust::entities::Hatch::new();
    hatch.is_solid = true;
    hatch.paths = vec![
        rect_path(0.0, 0.0, 10.0, 10.0),
        rect_path(3.0, 3.0, 7.0, 7.0),
    ];
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    assert_eq!(completeness, Completeness::Complete, "{completeness:?}");
    let area = mesh_area(&geometry).expect("solid fill mesh");
    assert!((area - 84.0).abs() < 1e-6, "hole area not excluded: {area}");
}

fn mesh_of(geometry: &SemanticGeometry) -> Option<&Mesh> {
    hatch_of(geometry)
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Mesh(m) => Some(m),
            _ => None,
        })
}

fn gradient_hatch(name: &str, stops: &[(f64, (u8, u8, u8))]) -> acadrust::entities::Hatch {
    let mut hatch = acadrust::entities::Hatch::new();
    // A real gradient HATCH is stored solid with gradient metadata, so the
    // importer must prefer the gradient over the solid flag.
    hatch.is_solid = true;
    hatch.paths = vec![rect_path(0.0, 0.0, 10.0, 10.0)];
    let g = &mut hatch.gradient_color;
    g.enabled = true;
    g.name = name.to_string();
    g.angle = 0.0;
    g.is_single_color = false;
    g.color_tint = 0.0;
    for (value, (r, gg, b)) in stops {
        g.colors.push(acadrust::entities::GradientColorEntry {
            value: *value,
            color: acadrust::Color::from_rgb(*r, *gg, *b),
            ..Default::default()
        });
    }
    hatch
}

#[test]
fn linear_gradient_hatch_is_complete_and_graded() {
    let hatch = gradient_hatch("LINEAR", &[(0.0, (255, 0, 0)), (1.0, (0, 0, 255))]);
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    assert_eq!(completeness, Completeness::Complete, "{completeness:?}");
    let mesh = mesh_of(&geometry).expect("gradient fill mesh");
    assert_eq!(mesh.colors.len(), mesh.vertices.len());
    let min_x = mesh
        .vertices
        .iter()
        .map(|p| p.x)
        .fold(f64::INFINITY, f64::min);
    let max_x = mesh
        .vertices
        .iter()
        .map(|p| p.x)
        .fold(f64::NEG_INFINITY, f64::max);
    let left = mesh
        .vertices
        .iter()
        .position(|p| (p.x - min_x).abs() < 1e-9)
        .expect("left vertex");
    let right = mesh
        .vertices
        .iter()
        .position(|p| (p.x - max_x).abs() < 1e-9)
        .expect("right vertex");
    assert_eq!(mesh.colors[left], [255, 0, 0], "left stop");
    assert_eq!(mesh.colors[right], [0, 0, 255], "right stop");
}

#[test]
fn spherical_gradient_hatch_is_complete() {
    // A triangle boundary has vertices at more than one radius from the bounds
    // midpoint, so the radial ramp is visible (a rectangle would collapse to a
    // constant, a documented tessellation gap).
    let mut hatch = gradient_hatch("SPHERICAL", &[(0.0, (0, 255, 0)), (1.0, (0, 0, 0))]);
    let corners = [(0.0f64, 0.0f64), (10.0, 0.0), (5.0, 10.0)];
    let mut path = BoundaryPath::new();
    for i in 0..3 {
        let a = corners[i];
        let b = corners[(i + 1) % 3];
        path.add_edge(BoundaryEdge::Line(acadrust::entities::LineEdge {
            start: acadrust::types::Vector2::new(a.0, a.1),
            end: acadrust::types::Vector2::new(b.0, b.1),
        }));
    }
    hatch.paths = vec![path];
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    assert_eq!(completeness, Completeness::Complete, "{completeness:?}");
    let mesh = mesh_of(&geometry).expect("gradient fill mesh");
    assert_eq!(mesh.colors.len(), mesh.vertices.len());
    assert!(mesh.colors.iter().any(|c| c[1] > 0), "green ramp expected");
}

#[test]
fn curved_gradient_kind_is_partial_with_a_stable_reason_code() {
    let hatch = gradient_hatch("HEMISPHERICAL", &[(0.0, (255, 0, 0)), (1.0, (0, 0, 255))]);
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    match completeness {
        Completeness::Partial(reasons) => {
            assert!(
                reasons
                    .iter()
                    .any(|r| r.contains("gradient_kind_not_supported")),
                "{reasons:?}"
            );
            assert!(reasons
                .iter()
                .any(|r| r.contains("gradient hatch not rendered")));
        }
        other => panic!("expected Partial, got {other:?}"),
    }
    assert!(mesh_of(&geometry).is_none(), "no gradient mesh expected");
}

#[test]
fn gradient_without_stops_is_partial_not_solid() {
    let hatch = gradient_hatch("LINEAR", &[]);
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    match completeness {
        Completeness::Partial(reasons) => {
            assert!(
                reasons
                    .iter()
                    .any(|r| r.contains("gradient_definition_unusable")),
                "{reasons:?}"
            );
        }
        other => panic!("expected Partial, got {other:?}"),
    }
    assert!(mesh_of(&geometry).is_none(), "no mesh expected");
}

#[test]
fn single_color_gradient_hatch_is_complete_and_graded() {
    let mut hatch = acadrust::entities::Hatch::new();
    hatch.is_solid = true;
    hatch.paths = vec![rect_path(0.0, 0.0, 10.0, 10.0)];
    let g = &mut hatch.gradient_color;
    g.enabled = true;
    g.name = "LINEAR".to_string();
    g.angle = 0.0;
    g.is_single_color = true;
    g.color_tint = 1.0;
    g.colors.push(acadrust::entities::GradientColorEntry {
        value: 0.0,
        color: acadrust::Color::from_rgb(255, 0, 0),
        ..Default::default()
    });
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    assert_eq!(completeness, Completeness::Complete, "{completeness:?}");
    let mesh = mesh_of(&geometry).expect("gradient fill mesh");
    assert!(mesh.colors.iter().any(|c| c[0] == 255 && c[1] > 200));
}

#[test]
fn solid_hatch_without_gradient_still_has_no_vertex_colors() {
    let mut hatch = acadrust::entities::Hatch::new();
    hatch.is_solid = true;
    hatch.paths = vec![rect_path(0.0, 0.0, 4.0, 4.0)];
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    assert_eq!(completeness, Completeness::Complete);
    let mesh = mesh_of(&geometry).expect("solid mesh");
    assert!(
        mesh.colors.is_empty(),
        "solid fill has no per-vertex colours"
    );
}

#[test]
fn over_budget_multi_ring_hatch_stays_partial_boundary_only() {
    // A zig-zag star defeats Douglas-Peucker, so the loop stays over
    // MAX_FILL_POINTS and the fill must be refused, never approximated.
    let points = 2200usize;
    let mut path = BoundaryPath::new();
    for i in 0..points {
        let t = i as f64 / points as f64 * std::f64::consts::TAU;
        let r = if i % 2 == 0 { 10.0 } else { 1.0 };
        let a = (r * t.cos(), r * t.sin());
        let t2 = (i + 1) as f64 / points as f64 * std::f64::consts::TAU;
        let r2 = if (i + 1) % 2 == 0 { 10.0 } else { 1.0 };
        let b = (r2 * t2.cos(), r2 * t2.sin());
        path.add_edge(BoundaryEdge::Line(acadrust::entities::LineEdge {
            start: acadrust::types::Vector2::new(a.0, a.1),
            end: acadrust::types::Vector2::new(b.0, b.1),
        }));
    }
    let mut hatch = acadrust::entities::Hatch::new();
    hatch.is_solid = true;
    hatch.paths = vec![path, rect_path(0.0, 0.0, 20.0, 20.0)];
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    match completeness {
        Completeness::Partial(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("budget")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
    // The boundary loops are still present; no fill mesh was fabricated.
    assert!(mesh_area(&geometry).is_none(), "{geometry:?}");
}

#[test]
fn degenerate_hatch_normal_reports_partial_boundary_only() {
    let mut hatch = acadrust::entities::Hatch::new();
    hatch.is_solid = true;
    hatch.normal = acadrust::types::Vector3::new(0.0, 0.0, 0.0);
    hatch.paths = vec![rect_path(0.0, 0.0, 4.0, 4.0)];
    let (_geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    match completeness {
        Completeness::Partial(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("normal")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}

#[test]
fn import_preserves_style_pattern_seeds_pixel_size_flags_and_source_edges() {
    let mut hatch = acadrust::entities::Hatch::new();
    hatch.is_solid = false;
    hatch.style = acadrust::entities::HatchStyleType::Outer;
    hatch.pattern_type = acadrust::entities::HatchPatternType::Custom;
    hatch.pattern.name = "ANSI31".into();
    hatch.pattern_angle = 0.25;
    hatch.pattern_scale = 2.0;
    hatch.is_double = true;
    hatch
        .pattern
        .add_line(acadrust::entities::HatchPatternLine {
            angle: std::f64::consts::FRAC_PI_4,
            base_point: acadrust::types::Vector2::new(0.5, 0.25),
            offset: acadrust::types::Vector2::new(0.0, 1.0),
            dash_lengths: vec![0.5, -0.25],
        });
    hatch.seed_points = vec![acadrust::types::Vector2::new(1.5, 2.5)];
    hatch.pixel_size = 0.125;
    let mut path = rect_path(0.0, 0.0, 10.0, 10.0);
    path.flags.set_external(true);
    hatch.paths = vec![path];
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    assert_eq!(completeness, Completeness::Complete, "{completeness:?}");
    let imported = hatch_of(&geometry);
    assert_eq!(imported.style(), HatchStyle::Outer);
    assert_eq!(imported.pattern_type(), HatchPatternType::Custom);
    assert_eq!(imported.seeds(), &[[1.5, 2.5]]);
    assert_eq!(imported.pixel_size(), Some(0.125));
    let loop_ = &imported.loops()[0];
    assert_eq!(loop_.flags.bits(), HatchLoopFlags::EXTERNAL);
    assert!(
        matches!(&imported.fill(), HatchFill::Pattern(_)),
        "{:?}",
        imported.fill()
    );
    let source_edges = loop_.source_edges.as_ref().expect("analytic line edges");
    assert_eq!(source_edges.len(), 4);
    assert!(matches!(
        source_edges[0],
        cad_domain::hatch_entity::BoundaryEdge::Line { .. }
    ));
}

#[test]
fn user_defined_pattern_without_families_is_synthesized() {
    // ADR U-1/N4: `76=0` with no definition rows (`78=0`) is synthesized from
    // angle/spacing/double instead of degrading to "no line families".
    let mut hatch = acadrust::entities::Hatch::new();
    hatch.is_solid = false;
    hatch.pattern_type = acadrust::entities::HatchPatternType::UserDefined;
    hatch.pattern.name = "USER".into();
    hatch.pattern_angle = 0.0;
    hatch.pattern_scale = 2.0;
    hatch.is_double = false;
    assert!(hatch.pattern.lines.is_empty());
    hatch.paths = vec![rect_path(0.0, 1.0, 10.0, 11.0)];
    let (geometry, completeness) = ImporterBuilder::hatch_geometry(&hatch);
    assert_eq!(completeness, Completeness::Complete, "{completeness:?}");
    let imported = hatch_of(&geometry);
    let looplets = imported.loops().len();
    let pattern_lines = imported
        .display()
        .iter()
        .skip(looplets)
        .filter(|geometry| matches!(geometry, SemanticGeometry::Polyline { .. }))
        .count();
    // Spacing 2 across a 10-unit high rectangle -> five horizontal lines.
    assert_eq!(pattern_lines, 5, "{:?}", imported.display());
}

#[test]
fn pattern_families_rotate_the_offset_with_the_pattern_angle() {
    // Values from the committed `isometric_grid.dxf`: pattern angle 75 degrees,
    // scale 0.5, a line at 120 degrees with a 1.5875 perpendicular offset. Before
    // P6a the offset was not rotated, so it stopped being perpendicular to the
    // final line direction (ADR U-7).
    let mut hatch = acadrust::entities::Hatch::new();
    hatch.pattern_angle = 75.0f64.to_radians();
    hatch.pattern_scale = 0.5;
    hatch
        .pattern
        .add_line(acadrust::entities::HatchPatternLine {
            angle: 120.0f64.to_radians(),
            base_point: acadrust::types::Vector2::new(0.5, 0.0),
            offset: acadrust::types::Vector2::new(-1.374_815_328_507_796, -0.793_75),
            dash_lengths: Vec::new(),
        });
    let pattern = import_pattern(&hatch);
    let families = pattern.families();
    let family = &families[0];
    let expected_angle = (120.0f64 + 75.0).to_radians();
    assert!((family.angle - expected_angle).abs() < 1e-12);
    let direction = [family.angle.cos(), family.angle.sin()];
    let perpendicular = family.offset[0] * direction[0] + family.offset[1] * direction[1];
    assert!(
        perpendicular.abs() < 1e-9,
        "offset must stay perpendicular: {perpendicular} ({:?})",
        family.offset
    );
    let magnitude =
        (family.offset[0] * family.offset[0] + family.offset[1] * family.offset[1]).sqrt();
    assert!((magnitude - 0.793_75).abs() < 1e-9, "{magnitude}");
    // The base point is scaled and rotated as well (pattern space -> hatch
    // space): scale 0.5 then +75 degrees.
    let base_x = 0.25 * 75.0f64.to_radians().cos();
    let base_y = 0.25 * 75.0f64.to_radians().sin();
    assert!((family.base[0] - base_x).abs() < 1e-9, "{:?}", family.base);
    assert!((family.base[1] - base_y).abs() < 1e-9, "{:?}", family.base);
}
