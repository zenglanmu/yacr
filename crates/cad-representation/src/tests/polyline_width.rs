//! Uniform polyline width: the wide-stroke display path (v1).
//!
//! Width `0.0` must keep the exact centreline `Lines` draw; a positive width
//! becomes a filled triangle band centred on the vertices, and an `XCLIP`
//! rectangle clips that band through the existing mesh clip. These are display
//! geometry assertions (vertices/triangles), not pixel or GPU claims.

use super::*;

fn polyline(points: Vec<Point3>, bulges: Vec<f64>, closed: bool, width: f64) -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points,
        bulges,
        closed,
        width,
    }
}

fn only_mesh(representation: &DisplayRepresentation) -> &Mesh {
    let mut meshes =
        representation
            .fragments
            .iter()
            .filter_map(|fragment| match &fragment.primitive {
                DisplayPrimitive::Mesh(mesh) => Some(mesh.as_ref()),
                _ => None,
            });
    let mesh = meshes.next().expect("one wide-stroke mesh fragment");
    assert!(meshes.next().is_none(), "exactly one mesh fragment");
    mesh
}

fn bounds(mesh: &Mesh) -> ([f64; 2], [f64; 2]) {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for vertex in &mesh.vertices {
        min[0] = min[0].min(vertex.x);
        min[1] = min[1].min(vertex.y);
        max[0] = max[0].max(vertex.x);
        max[1] = max[1].max(vertex.y);
    }
    (min, max)
}

#[test]
fn width_zero_keeps_the_exact_centreline_lines_path() {
    // The pre-width draw: a 3-point open polyline tessellates to its vertices.
    let geometry = polyline(
        vec![p(0.0, 0.0), p(4.0, 0.0), p(4.0, 4.0)],
        vec![0.0; 3],
        false,
        0.0,
    );
    let registry = ProviderRegistry::with_default_provider();
    let representation = registry
        .build(&entity(1, geometry.clone()), &context())
        .unwrap();
    assert_eq!(representation.fragments.len(), 1);
    assert_eq!(representation.completeness, Completeness::Complete);
    match &representation.fragments[0].primitive {
        DisplayPrimitive::Lines(points) => {
            assert_eq!(points.as_ref(), geometry_tessellation(&geometry).as_slice());
        }
        _ => panic!("width 0 must stay on the Lines path"),
    }
}

fn geometry_tessellation(geometry: &SemanticGeometry) -> Vec<Point3> {
    cad_geometry::tessellate_geometry(geometry, cad_geometry::TessellationParams::default())
}

#[test]
fn a_positive_width_emits_a_band_covering_the_centreline_plus_minus_half_width() {
    // Two straight segments meeting at a right angle; width 2 means the band
    // reaches 1 unit to either side of every centreline segment.
    let geometry = polyline(
        vec![p(0.0, 0.0), p(4.0, 0.0), p(4.0, 4.0)],
        vec![0.0; 3],
        false,
        2.0,
    );
    let registry = ProviderRegistry::with_default_provider();
    let representation = registry.build(&entity(1, geometry), &context()).unwrap();
    assert_eq!(representation.completeness, Completeness::Complete);
    assert!(
        representation
            .fragments
            .iter()
            .all(|fragment| !matches!(fragment.primitive, DisplayPrimitive::Lines(_))),
        "a wide stroke is not drawn as its centreline"
    );
    let mesh = only_mesh(&representation);
    // Two spans × two triangles.
    assert_eq!(mesh.triangles.len(), 4);
    assert_eq!(mesh.vertices.len(), 6, "one left/right pair per vertex");
    assert_eq!(
        mesh.normals.len(),
        mesh.vertices.len(),
        "every band vertex carries the plane normal"
    );
    let (min, max) = bounds(mesh);
    assert!((min[0] - 0.0).abs() < 1e-9, "the start cap stays at x=0");
    assert!((max[0] - 5.0).abs() < 1e-9, "the corner mitre reaches x=5");
    assert!((min[1] - (-1.0)).abs() < 1e-9, "the band reaches y=-1");
    assert!((max[1] - 4.0).abs() < 1e-9, "the end cap stays at y=4");

    // The first vertex's ±width/2 offsets are real band corners, not a
    // centreline pair: (0, -1) and (0, 1).
    let has = |x: f64, y: f64| {
        mesh.vertices
            .iter()
            .any(|v| (v.x - x).abs() < 1e-9 && (v.y - y).abs() < 1e-9)
    };
    assert!(has(0.0, -1.0), "left/right offsets at the first vertex");
    assert!(has(0.0, 1.0), "left/right offsets at the first vertex");
    // The right-angle miter corner is at (3, 1)/(5, -1).
    assert!(has(3.0, 1.0), "mitre join touches the inner corner");
    assert!(has(5.0, -1.0), "mitre join touches the outer corner");
    // Every triangle is a real area (no degenerate duplicates).
    for triangle in &mesh.triangles {
        let a = mesh.vertices[triangle[0] as usize];
        let b = mesh.vertices[triangle[1] as usize];
        let c = mesh.vertices[triangle[2] as usize];
        let area = ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs();
        assert!(area > 1e-9, "triangle {triangle:?} has no area");
    }
}

#[test]
fn a_closed_polyline_wraps_the_band_back_to_its_first_vertex() {
    let geometry = polyline(
        vec![p(0.0, 0.0), p(4.0, 0.0), p(4.0, 4.0), p(0.0, 4.0)],
        vec![0.0; 4],
        true,
        1.0,
    );
    let registry = ProviderRegistry::with_default_provider();
    let representation = registry.build(&entity(1, geometry), &context()).unwrap();
    let mesh = only_mesh(&representation);
    // Four ring spans × two triangles, including the closing span.
    assert_eq!(mesh.triangles.len(), 8);
    let (min, max) = bounds(mesh);
    assert!((min[0] - (-0.5)).abs() < 1e-9);
    assert!((max[0] - 4.5).abs() < 1e-9);
    assert!((min[1] - (-0.5)).abs() < 1e-9);
    assert!((max[1] - 4.5).abs() < 1e-9);
}

#[test]
fn a_bulged_polyline_is_tessellated_then_offset_into_a_band() {
    // One semicircular bulge on a two-vertex polyline: the centreline is a
    // chord approximation of the arc, and the band offsets every chord pair.
    let geometry = polyline(vec![p(0.0, 0.0), p(4.0, 0.0)], vec![-1.0, 0.0], false, 0.5);
    let registry = ProviderRegistry::with_default_provider();
    let representation = registry.build(&entity(1, geometry), &context()).unwrap();
    let mesh = only_mesh(&representation);
    assert!(
        mesh.triangles.len() > 2,
        "the bulge arc is tessellated into several chord spans"
    );
    // The arc bulges toward +Y; the band's extents include the arc apex plus
    // the half width, never just the straight chord box.
    let (min, max) = bounds(mesh);
    assert!(
        max[1] > 2.0,
        "the arc apex is outside the chord, got {max:?}"
    );
    assert!(
        min[1] < 0.0,
        "the band reaches below the chord, got {min:?}"
    );
    // Every band vertex stays within half-width of the tessellated centreline.
    let centreline = cad_geometry::tessellate_geometry(
        &SemanticGeometry::Polyline {
            points: vec![p(0.0, 0.0), p(4.0, 0.0)],
            bulges: vec![-1.0, 0.0],
            closed: false,
            width: 0.5,
        },
        cad_geometry::TessellationParams::default(),
    );
    for vertex in &mesh.vertices {
        let nearest = distance_to_polyline(vertex, &centreline);
        // Perpendicular distance to a segment is at most half-width; a miter
        // corner at a turn sits further from the vertex (up to the documented
        // 4× half-width clamp), so that is the honest bound.
        assert!(
            nearest <= 4.0 * 0.25 + 1e-6,
            "band vertex ({}, {}) is {nearest} from the centreline",
            vertex.x,
            vertex.y
        );
    }
}

/// Shortest 2D distance from `point` to a polyline's segments.
fn distance_to_polyline(point: &Point3, polyline: &[Point3]) -> f64 {
    polyline
        .windows(2)
        .map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let length_sq = dx * dx + dy * dy;
            let t = if length_sq <= 0.0 {
                0.0
            } else {
                (((point.x - a.x) * dx + (point.y - a.y) * dy) / length_sq).clamp(0.0, 1.0)
            };
            let closest = (a.x + dx * t, a.y + dy * t);
            ((point.x - closest.0).powi(2) + (point.y - closest.1).powi(2)).sqrt()
        })
        .fold(f64::INFINITY, f64::min)
}

/// A block-local `XCLIP` rectangle (the xclip tests' minimal boundary).
fn clip_rect(minx: f64, miny: f64, maxx: f64, maxy: f64) -> BlockClip {
    BlockClip {
        vertices: vec![[minx, miny], [maxx, maxy]],
        inside: true,
        visible: true,
    }
}

#[test]
fn an_xclip_rectangle_clips_the_wide_stroke_mesh() {
    let member = entity(
        11,
        polyline(vec![p(-4.0, 0.0), p(4.0, 0.0)], vec![0.0; 2], false, 2.0),
    );
    let member = DbEntity {
        space: SpaceId::Block(BlockId(1)),
        ..member
    };
    let mut builder = empty_db();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![member.id],
            dynamic_visibility: None,
        })
        .unwrap();
    builder.insert_entity(member).unwrap();
    let mut insert = insert_entity(1, SpaceId::Model, 1, 0.0);
    insert.geometry = SemanticGeometry::Insert {
        block: BlockId(1),
        transform: Transform3::identity(),
    };
    builder.insert_entity(insert).unwrap();
    builder
        .set_entity_render_attributes(
            EntityId(1),
            EntityRenderAttributes {
                clip: Some(clip_rect(-1.0, -1.0, 1.0, 1.0)),
                ..EntityRenderAttributes::default()
            },
        )
        .unwrap();
    let db = builder.finish().unwrap();
    let representation = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    let mesh = only_mesh(&representation);
    let rect = WorldRect {
        min: [-1.0, -1.0],
        max: [1.0, 1.0],
    };
    for vertex in &mesh.vertices {
        assert!(
            rect.contains([vertex.x, vertex.y]),
            "clipped band vertex ({}, {}) is outside the XCLIP rectangle",
            vertex.x,
            vertex.y
        );
    }
    assert!(!mesh.triangles.is_empty(), "the visible band survives");
}
