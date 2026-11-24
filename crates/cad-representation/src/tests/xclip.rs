//! `XCLIP` display clipping through the representation layer.
//!
//! The boundary is stored on the model `INSERT` in the referenced block's local
//! space; expansion maps it through the composed transform and cuts the expanded
//! geometry to the world region it spans. These tests pin the exact paths
//! (polylines, conics, meshes, images, nested intersections), the concave and
//! rotated-boundary cases that the axis-aligned rectangle used to refuse, the
//! byte-identical rectangle output, and the honest refusals (degenerate or
//! self-intersecting ring, inverted clip, text/mask) rather than a visual claim.

use super::*;

use crate::layout::{clip_polyline_to_rect, WorldRectBoundary};

/// A block-local rectangle boundary.
fn clip_rect(minx: f64, miny: f64, maxx: f64, maxy: f64, visible: bool) -> BlockClip {
    BlockClip::new(vec![[minx, miny], [maxx, maxy]], true, visible)
}

/// A block-local polygon boundary.
fn clip_polygon(vertices: &[[f64; 2]]) -> BlockClip {
    BlockClip::new(vertices.to_vec(), true, true)
}

/// A concave six-point **L** in block space: a 10x10 square with the top-right
/// rectangle `x > 6, y > 4` removed (`4 x 6 = 24`), so its area is exactly 76.
fn l_shape() -> Vec<[f64; 2]> {
    vec![
        [0.0, 0.0],
        [10.0, 0.0],
        [10.0, 4.0],
        [6.0, 4.0],
        [6.0, 10.0],
        [0.0, 10.0],
    ]
}

fn rotation_z(degrees: f64) -> Transform3 {
    let theta = degrees.to_radians();
    let mut m = Transform3::identity().matrix;
    m[0][0] = theta.cos();
    m[0][1] = -theta.sin();
    m[1][0] = theta.sin();
    m[1][1] = theta.cos();
    Transform3 { matrix: m }
}

/// A database with block 1 (one member `member` in block space) and a model
/// INSERT (entity 1) of it at `transform`, carrying `clip`.
fn block_db(member: DbEntity, transform: Transform3, clip: Option<BlockClip>) -> DrawingDatabase {
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: vec![member.id],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(member).unwrap();
    let mut insert = insert_entity(1, SpaceId::Model, 1, 0.0);
    insert.geometry = SemanticGeometry::Insert {
        block: BlockId(1),
        transform,
        attributes: Vec::new(),
    };
    b.insert_entity(insert).unwrap();
    let attributes = EntityRenderAttributes {
        clip,
        ..EntityRenderAttributes::default()
    };
    b.set_entity_render_attributes(EntityId(1), attributes)
        .unwrap();
    b.finish().unwrap()
}

fn expand(db: &DrawingDatabase) -> DisplayRepresentation {
    expand_with(db, &context())
}

fn expand_with(db: &DrawingDatabase, ctx: &RepresentationContext) -> DisplayRepresentation {
    ProviderRegistry::with_default_provider()
        .build_expanded(db, db.entity(EntityId(1)).unwrap(), ctx)
        .unwrap()
}

fn circle_entity(id: u128, space: SpaceId, center: Point3, radius: f64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbCircle".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry: SemanticGeometry::Circle {
            center,
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius,
        },
        draw_order: id as i64,
    }
}

fn lines(rep: &DisplayRepresentation) -> Vec<Vec<Point3>> {
    rep.fragments
        .iter()
        .filter_map(|fragment| match &fragment.primitive {
            DisplayPrimitive::Lines(points) => Some(points.to_vec()),
            _ => None,
        })
        .collect()
}

fn triangle_mesh() -> Mesh {
    Mesh {
        uvs: Vec::new(),
        texture: None,
        pbr: None,
        vertices: vec![p(-2.0, -2.0), p(2.0, -2.0), p(0.0, 2.0)],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    }
}

/// A filled quad mesh covering `[0, 10]^2`: two triangles, area 100.
fn square_mesh() -> Mesh {
    Mesh {
        uvs: Vec::new(),
        texture: None,
        pbr: None,
        vertices: vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    }
}

/// The area of a triangle mesh in the XY plane.
fn mesh_area(mesh: &Mesh) -> f64 {
    let mut total = 0.0;
    for triangle in &mesh.triangles {
        let a = mesh.vertices[triangle[0] as usize];
        let b = mesh.vertices[triangle[1] as usize];
        let c = mesh.vertices[triangle[2] as usize];
        total += ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() * 0.5;
    }
    total
}

/// Every mesh fragment of `rep`, flattened into one area.
fn clipped_mesh_area(rep: &DisplayRepresentation) -> f64 {
    rep.fragments
        .iter()
        .filter_map(|fragment| match &fragment.primitive {
            DisplayPrimitive::Mesh(mesh) => Some(mesh_area(mesh)),
            _ => None,
        })
        .sum()
}

#[test]
fn a_crossing_line_is_clipped_into_the_inside_run() {
    let member = line_entity(11, SpaceId::Block(BlockId(1)), p(-5.0, 0.0), p(5.0, 0.0));
    let db = block_db(
        member,
        Transform3::identity(),
        Some(clip_rect(-1.0, -1.0, 1.0, 1.0, true)),
    );
    let rep = expand(&db);
    assert_eq!(rep.completeness, Completeness::Complete);
    let runs = lines(&rep);
    assert_eq!(runs.len(), 1, "one continuous inside run");
    assert_eq!(runs[0].len(), 2);
    assert!((runs[0][0].x + 1.0).abs() < 1e-9);
    assert!((runs[0][1].x - 1.0).abs() < 1e-9);
}

#[test]
fn a_fully_outside_line_emits_nothing() {
    let member = line_entity(11, SpaceId::Block(BlockId(1)), p(-5.0, 0.0), p(5.0, 0.0));
    let db = block_db(
        member,
        Transform3::identity(),
        Some(clip_rect(10.0, 10.0, 12.0, 12.0, true)),
    );
    let rep = expand(&db);
    assert_eq!(rep.completeness, Completeness::Complete);
    assert!(
        rep.fragments.is_empty(),
        "an exact omission, not a kept line"
    );
}

#[test]
fn a_conic_is_sliced_analytically() {
    let member = circle_entity(11, SpaceId::Block(BlockId(1)), p(0.0, 0.0), 2.0);
    let db = block_db(
        member,
        Transform3::identity(),
        Some(clip_rect(-3.0, -1.0, 3.0, 1.0, true)),
    );
    let rep = expand(&db);
    assert_eq!(rep.completeness, Completeness::Complete);
    assert!(!rep.fragments.is_empty(), "the circle crosses the window");
    for fragment in &rep.fragments {
        match &fragment.primitive {
            DisplayPrimitive::Conic(conic) => {
                // Still analytic: the center/semi-diameters are untouched and
                // only the parameter interval was cut.
                assert_eq!(conic.center, p(0.0, 0.0));
                assert!(!conic.full, "a clipped slice is not a full circle");
                assert!(conic.sweep.abs() < std::f64::consts::TAU);
                assert_eq!(fragment.precision, Precision::Analytic);
            }
            _ => panic!("expected an analytic conic slice"),
        }
    }
}

#[test]
fn a_mesh_triangle_is_clipped_exactly() {
    let mut member = line_entity(11, SpaceId::Block(BlockId(1)), p(0.0, 0.0), p(0.0, 0.0));
    member.geometry = SemanticGeometry::Mesh(triangle_mesh());
    let db = block_db(
        member,
        Transform3::identity(),
        Some(clip_rect(-1.0, -1.0, 1.0, 1.0, true)),
    );
    let rep = expand(&db);
    assert_eq!(rep.completeness, Completeness::Complete);
    let rect = WorldRect {
        min: [-1.0, -1.0],
        max: [1.0, 1.0],
    };
    let mut meshes = 0;
    for fragment in &rep.fragments {
        if let DisplayPrimitive::Mesh(mesh) = &fragment.primitive {
            meshes += 1;
            assert!(!mesh.triangles.is_empty());
            for vertex in &mesh.vertices {
                assert!(
                    rect.contains([vertex.x, vertex.y]),
                    "clipped vertex ({}, {}) outside the boundary",
                    vertex.x,
                    vertex.y
                );
            }
        }
    }
    assert_eq!(meshes, 1, "one clipped mesh fragment");
}

/// Block 1 contains an INSERT (entity 13) of block 2, whose member is a line.
/// Both references carry a boundary, so the child is cut by the intersection.
fn nested_clip_db(outer: Option<BlockClip>, inner: Option<BlockClip>) -> DrawingDatabase {
    nested_clip_db_with_line(outer, inner, p(-5.0, 0.0), p(5.0, 0.0))
}

/// As [`nested_clip_db`], with a chosen block-2 member line.
fn nested_clip_db_with_line(
    outer: Option<BlockClip>,
    inner: Option<BlockClip>,
    start: Point3,
    end: Point3,
) -> DrawingDatabase {
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(21)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(line_entity(21, SpaceId::Block(BlockId(2)), start, end))
        .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: vec![EntityId(13)],
        dynamic_visibility: None,
    })
    .unwrap();
    let mut nested = insert_entity(13, SpaceId::Block(BlockId(1)), 2, 0.0);
    nested.geometry = SemanticGeometry::Insert {
        block: BlockId(2),
        transform: Transform3::identity(),
        attributes: Vec::new(),
    };
    b.insert_entity(nested).unwrap();
    b.insert_entity(insert_entity(1, SpaceId::Model, 1, 0.0))
        .unwrap();
    let inner_attributes = EntityRenderAttributes {
        clip: inner,
        ..EntityRenderAttributes::default()
    };
    b.set_entity_render_attributes(EntityId(13), inner_attributes)
        .unwrap();
    let outer_attributes = EntityRenderAttributes {
        clip: outer,
        ..EntityRenderAttributes::default()
    };
    b.set_entity_render_attributes(EntityId(1), outer_attributes)
        .unwrap();
    b.finish().unwrap()
}

#[test]
fn nested_clips_intersect() {
    let db = nested_clip_db(
        Some(clip_rect(-1.0, -1.0, 1.0, 1.0, true)),
        Some(clip_rect(-2.0, -1.0, 2.0, 1.0, true)),
    );
    let rep = expand(&db);
    let runs = lines(&rep);
    assert_eq!(runs.len(), 1);
    assert!((runs[0][0].x + 1.0).abs() < 1e-9, "outer boundary wins");
    assert!((runs[0][1].x - 1.0).abs() < 1e-9);
}

#[test]
fn disjoint_nested_clips_omit_the_instance_exactly() {
    let db = nested_clip_db(
        Some(clip_rect(-1.0, -1.0, 1.0, 1.0, true)),
        Some(clip_rect(10.0, 10.0, 12.0, 12.0, true)),
    );
    let rep = expand(&db);
    assert_eq!(rep.completeness, Completeness::Complete);
    assert!(
        rep.fragments.is_empty(),
        "an empty outer∩inner intersection draws nothing"
    );
}

#[test]
fn a_clip_on_the_inner_reference_alone_still_cuts() {
    let db = nested_clip_db(None, Some(clip_rect(-2.0, -1.0, 2.0, 1.0, true)));
    let rep = expand(&db);
    let runs = lines(&rep);
    assert_eq!(runs.len(), 1);
    assert!((runs[0][0].x + 2.0).abs() < 1e-9);
    assert!((runs[0][1].x - 2.0).abs() < 1e-9);
}

#[test]
fn an_off_clip_applies_nothing() {
    let member = line_entity(11, SpaceId::Block(BlockId(1)), p(-5.0, 0.0), p(5.0, 0.0));
    let db = block_db(
        member,
        Transform3::identity(),
        Some(clip_rect(-1.0, -1.0, 1.0, 1.0, false)),
    );
    let rep = expand(&db);
    let runs = lines(&rep);
    assert_eq!(runs.len(), 1);
    assert!(
        (runs[0][0].x + 5.0).abs() < 1e-9,
        "OFF keeps the full member"
    );
    assert!((runs[0][1].x - 5.0).abs() < 1e-9);
}

#[test]
fn a_rotated_insert_clips_to_the_quad_its_corners_span_not_its_envelope() {
    // The block-local square (-1,-1)..(1,1) rotated 45 degrees is a diamond with
    // vertices (0, ±√2) and (±√2, 0). Its axis-aligned envelope is
    // x, y ∈ [-√2, √2], so a line along the rotated block x-axis would survive
    // for length 4 if the envelope were used — the diamond gives length 2.
    let member = line_entity(11, SpaceId::Block(BlockId(1)), p(-5.0, 0.0), p(5.0, 0.0));
    let db = block_db(
        member,
        rotation_z(45.0),
        Some(clip_rect(-1.0, -1.0, 1.0, 1.0, true)),
    );
    let rep = expand(&db);
    assert_eq!(
        rep.completeness,
        Completeness::Complete,
        "a rotated rectangle is a renderable region now: {:?}",
        rep.diagnostics
    );
    assert!(rep.diagnostics.is_empty(), "{:?}", rep.diagnostics);
    let runs = lines(&rep);
    assert_eq!(runs.len(), 1, "one inside run along the diagonal");
    let (a, b) = (runs[0][0], runs[0][1]);
    let half = std::f64::consts::FRAC_1_SQRT_2;
    assert!(
        (a.x + half).abs() < 1e-9 && (a.y + half).abs() < 1e-9,
        "{a:?}"
    );
    assert!(
        (b.x - half).abs() < 1e-9 && (b.y - half).abs() < 1e-9,
        "{b:?}"
    );
    let length = (b.x - a.x).hypot(b.y - a.y);
    assert!(
        (length - 2.0).abs() < 1e-9,
        "the quad's own diagonal is 2, the envelope's would be 4; got {length}"
    );
}

#[test]
fn a_sheared_insert_clips_to_the_parallelogram() {
    // A shear (x, y) -> (x, y + x) turns the block-local square into the
    // parallelogram |x| <= 1, |y - x| <= 1. A vertical world line at x = 0.5 is
    // inside for y ∈ [-0.5, 1.5] (length 2); the envelope y ∈ [-2, 2] would keep
    // length 4.
    let mut shear = Transform3::identity().matrix;
    shear[1][0] = 1.0;
    let shear = Transform3 { matrix: shear };
    let member = line_entity(11, SpaceId::Block(BlockId(1)), p(0.5, -3.0), p(0.5, 3.0));
    let db = block_db(member, shear, Some(clip_rect(-1.0, -1.0, 1.0, 1.0, true)));
    let rep = expand(&db);
    assert_eq!(rep.completeness, Completeness::Complete);
    let runs = lines(&rep);
    assert_eq!(runs.len(), 1);
    let (a, b) = (runs[0][0], runs[0][1]);
    assert!(
        (a.x - 0.5).abs() < 1e-9 && (a.y + 0.5).abs() < 1e-9,
        "{a:?}"
    );
    assert!(
        (b.x - 0.5).abs() < 1e-9 && (b.y - 1.5).abs() < 1e-9,
        "{b:?}"
    );
}

#[test]
fn a_concave_polygon_boundary_cuts_the_notch_out_of_a_line() {
    // At y = 8 the L only spans x ∈ [0, 6]; its envelope would keep x ∈ [0, 10].
    let member = line_entity(11, SpaceId::Block(BlockId(1)), p(-2.0, 8.0), p(12.0, 8.0));
    let db = block_db(
        member,
        Transform3::identity(),
        Some(clip_polygon(&l_shape())),
    );
    let rep = expand(&db);
    assert_eq!(
        rep.completeness,
        Completeness::Complete,
        "{:?}",
        rep.completeness
    );
    assert!(rep.diagnostics.is_empty(), "{:?}", rep.diagnostics);
    let runs = lines(&rep);
    assert_eq!(
        runs.len(),
        1,
        "one inside span, not two and not the envelope"
    );
    let (a, b) = (runs[0][0], runs[0][1]);
    assert!((a.x).abs() < 1e-9 && (a.y - 8.0).abs() < 1e-9, "{a:?}");
    assert!(
        (b.x - 6.0).abs() < 1e-9 && (b.y - 8.0).abs() < 1e-9,
        "{b:?}"
    );
}

#[test]
fn a_concave_polygon_boundary_keeps_exactly_the_polygon_area_of_a_filled_mesh() {
    let mut member = line_entity(11, SpaceId::Block(BlockId(1)), p(0.0, 0.0), p(0.0, 0.0));
    member.geometry = SemanticGeometry::Mesh(square_mesh());
    let db = block_db(
        member,
        Transform3::identity(),
        Some(clip_polygon(&l_shape())),
    );
    let rep = expand(&db);
    assert_eq!(
        rep.completeness,
        Completeness::Complete,
        "{:?}",
        rep.diagnostics
    );
    let area = clipped_mesh_area(&rep);
    assert!(
        (area - 76.0).abs() < 1e-9,
        "the L keeps 76 of the quad's 100; got {area}"
    );
    // Every surviving vertex is inside the ring (boundary inclusive).
    let region = WorldRectBoundary::ring(&l_shape()).expect("the L is a simple ring");
    for fragment in &rep.fragments {
        if let DisplayPrimitive::Mesh(mesh) = &fragment.primitive {
            for vertex in &mesh.vertices {
                assert!(
                    region.contains([vertex.x, vertex.y]),
                    "clipped vertex ({}, {}) leaked out of the ring",
                    vertex.x,
                    vertex.y
                );
            }
        }
    }
}

#[test]
fn a_conic_is_sliced_analytically_to_a_concave_polygon_boundary() {
    // A circle of radius 4 at (5,5) crosses the L's notch; the visible arcs stay
    // analytic slices (no chords, no full circle) whose midpoints are inside.
    let member = circle_entity(11, SpaceId::Block(BlockId(1)), p(5.0, 5.0), 4.0);
    let db = block_db(
        member,
        Transform3::identity(),
        Some(clip_polygon(&l_shape())),
    );
    let rep = expand(&db);
    assert_eq!(
        rep.completeness,
        Completeness::Complete,
        "{:?}",
        rep.diagnostics
    );
    let region = WorldRectBoundary::ring(&l_shape()).expect("the L is a simple ring");
    let mut slices = 0;
    let mut sweep = 0.0;
    for fragment in &rep.fragments {
        if let DisplayPrimitive::Conic(conic) = &fragment.primitive {
            slices += 1;
            assert!(!conic.full, "a clipped circle is not a full circle");
            assert_eq!(fragment.precision, Precision::Analytic);
            sweep += conic.sweep.abs();
            let t = conic.start + conic.sweep * 0.5;
            let midpoint = [
                conic.center.x + conic.major.x * t.cos() + conic.minor.x * t.sin(),
                conic.center.y + conic.major.y * t.cos() + conic.minor.y * t.sin(),
            ];
            assert!(
                region.contains(midpoint),
                "a conic slice midpoint ({}, {}) left the ring",
                midpoint[0],
                midpoint[1]
            );
        }
    }
    assert!(slices >= 1, "the circle does cross the ring");
    assert!(
        sweep < std::f64::consts::TAU - 1e-9,
        "the outside arc is dropped, not kept"
    );
}

#[test]
fn an_axis_aligned_rectangle_keeps_exactly_the_pre_polygon_output() {
    // The old path: the axis-aligned oracle plus the half-plane polyline clipper.
    let member = line_entity(11, SpaceId::Block(BlockId(1)), p(-5.0, -3.0), p(7.0, 6.0));
    let clip = clip_rect(-1.3, -0.7, 2.9, 1.1, true);
    let transform = Transform3::translation(p(0.4, -0.2));
    let db = block_db(member, transform, Some(clip.clone()));
    let rep = expand(&db);

    let old = cad_representation_old_rect(
        &clip,
        &transform,
        &[
            transform.apply_point(p(-5.0, -3.0)),
            transform.apply_point(p(7.0, 6.0)),
        ],
    );
    let runs = lines(&rep);
    assert_eq!(runs.len(), old.len());
    for (got, expected) in runs.iter().zip(&old) {
        assert_eq!(got, expected, "byte-identical rectangle output");
    }
}

/// The pre-polygon rectangle path, spelled out so byte identity is a real
/// comparison and not a restatement of the new code.
fn cad_representation_old_rect(
    clip: &BlockClip,
    transform: &Transform3,
    world_points: &[Point3],
) -> Vec<Vec<Point3>> {
    let rect = block_clip_rect(clip, transform)
        .expect("the oracle accepts an axis-aligned rectangle")
        .expect("an active clip");
    let center = [
        (rect.min[0] + rect.max[0]) * 0.5,
        (rect.min[1] + rect.max[1]) * 0.5,
    ];
    let half = [
        (rect.max[0] - rect.min[0]) * 0.5,
        (rect.max[1] - rect.min[1]) * 0.5,
    ];
    clip_polyline_to_rect(world_points, center, half)
}

#[test]
fn the_axis_aligned_boundary_equals_the_oracle_rectangle() {
    let clip = clip_rect(-1.3, -0.7, 2.9, 1.1, true);
    let transform = Transform3::translation(p(3.0, 4.0)).matrix_mul(&Transform3::scale(2.5));
    let oracle = block_clip_rect(&clip, &transform)
        .unwrap()
        .expect("an active clip");
    let boundary = block_clip_boundary(&clip, &transform)
        .unwrap()
        .expect("an active clip");
    let (min, max) = boundary.as_rect().expect("still the rectangle path");
    assert_eq!(min, oracle.min);
    assert_eq!(max, oracle.max);
    assert_eq!(boundary.pieces().len(), 1);
}

#[test]
fn text_and_mask_are_kept_and_reported() {
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: vec![EntityId(11), EntityId(12)],
        dynamic_visibility: None,
    })
    .unwrap();
    let mut text = line_entity(11, SpaceId::Block(BlockId(1)), p(0.0, 0.0), p(0.0, 0.0));
    text.object.type_key = "AcDbText".into();
    text.geometry = SemanticGeometry::Text {
        text: "O".into(),
        position: p(0.0, 0.0),
        style: StyleId(0),
        height: 1.0,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: None,
        width_factor: 1.0,
        line_spacing: cad_domain::TextLineSpacing::default(),
    };
    b.insert_entity(text).unwrap();
    let mut mask = line_entity(12, SpaceId::Block(BlockId(1)), p(0.0, 0.0), p(0.0, 0.0));
    mask.object.type_key = "AcDbWipeout".into();
    mask.geometry = SemanticGeometry::Mask {
        boundary: vec![p(-2.0, -2.0), p(2.0, -2.0), p(2.0, 2.0), p(-2.0, 2.0)],
        inverted: false,
    };
    b.insert_entity(mask).unwrap();
    b.insert_entity(insert_entity(1, SpaceId::Model, 1, 0.0))
        .unwrap();
    let attributes = EntityRenderAttributes {
        clip: Some(clip_rect(-1.0, -1.0, 1.0, 1.0, true)),
        ..EntityRenderAttributes::default()
    };
    b.set_entity_render_attributes(EntityId(1), attributes)
        .unwrap();
    let db = b.finish().unwrap();

    let rep = expand(&db);
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    let mut saw_text = false;
    let mut saw_mask = false;
    for fragment in &rep.fragments {
        match &fragment.primitive {
            DisplayPrimitive::Text { .. } => saw_text = true,
            DisplayPrimitive::Mask { .. } => saw_mask = true,
            _ => {}
        }
    }
    assert!(saw_text && saw_mask, "both primitives stay, unclipped");
    let reported = rep
        .diagnostics
        .iter()
        .filter(|d| d.code == "representation.xclip_primitive_unsupported")
        .count();
    assert_eq!(reported, 2, "one report per unclippable primitive");
}

#[test]
fn a_nested_rectangle_is_refined_against_a_concave_parent_boundary() {
    // Outer boundary = the L (block-local on INSERT 1), inner = a rectangle that
    // covers everything (block-local on INSERT 13). rect ∩ concave ring must keep
    // the L, not the rectangle and not the L's envelope.
    let db = nested_clip_db_with_line(
        Some(clip_polygon(&l_shape())),
        Some(clip_rect(-100.0, -100.0, 100.0, 100.0, true)),
        p(-5.0, 8.0),
        p(8.0, 8.0),
    );
    let rep = expand(&db);
    assert_eq!(
        rep.completeness,
        Completeness::Complete,
        "{:?}",
        rep.diagnostics
    );
    let runs = lines(&rep);
    assert_eq!(runs.len(), 1);
    let (a, b) = (runs[0][0], runs[0][1]);
    assert!((a.x).abs() < 1e-9 && (a.y - 8.0).abs() < 1e-9, "{a:?}");
    assert!(
        (b.x - 6.0).abs() < 1e-9 && (b.y - 8.0).abs() < 1e-9,
        "{b:?}"
    );
    // The instance path still names the INSERT chain the member came through.
    let fragment = rep
        .fragments
        .iter()
        .find(|fragment| matches!(&fragment.primitive, DisplayPrimitive::Lines(_)))
        .expect("the clipped line fragment");
    assert_eq!(
        fragment.source.instance.0,
        vec![EntityId(1), EntityId(13)],
        "the nested member keeps its two-INSERT instance path"
    );
}

#[test]
fn a_nested_rectangle_inside_a_concave_parent_is_refined_into_a_rectangle() {
    // The inner rectangle cuts the L down to its lower arm: rect ∩ polygon is one
    // rectangle, and the clippers keep the exact (non-envelope) answer.
    let db = nested_clip_db_with_line(
        Some(clip_polygon(&l_shape())),
        Some(clip_rect(-100.0, -100.0, 3.0, 100.0, true)),
        p(-5.0, 8.0),
        p(8.0, 8.0),
    );
    let rep = expand(&db);
    assert_eq!(
        rep.completeness,
        Completeness::Complete,
        "{:?}",
        rep.diagnostics
    );
    let runs = lines(&rep);
    assert_eq!(runs.len(), 1);
    let (a, b) = (runs[0][0], runs[0][1]);
    assert!((a.x).abs() < 1e-9, "{a:?}");
    assert!((b.x - 3.0).abs() < 1e-9, "{b:?}");
}

/// An `Image` member inside block 1; `u`/`v` are per-pixel vectors, so the
/// 2x4-pixel image is mapped to the quad `[5,9] x [2,18]` (area 64).
fn block_image_entity(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbRasterImage".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Block(BlockId(1)),
        geometry: SemanticGeometry::Image {
            origin: p(5.0, 2.0),
            u: Point3 {
                x: 2.0,
                y: 0.0,
                z: 0.0,
            },
            v: Point3 {
                x: 0.0,
                y: 4.0,
                z: 0.0,
            },
            pixels: [2.0, 4.0],
            file: Some("clip.png".into()),
            clip: None,
            visible: true,
            adjust: ImageAdjust::default(),
        },
        draw_order: id as i64,
    }
}

/// The area of one emitted image piece (positions in world space).
fn image_piece_area(vertices: &[ImageVertex]) -> f64 {
    let mut sum = 0.0;
    for index in 0..vertices.len() {
        let a = vertices[index].position;
        let b = vertices[(index + 1) % vertices.len()].position;
        sum += a.x * b.y - b.x * a.y;
    }
    (sum * 0.5).abs()
}

#[test]
fn an_image_is_cut_to_a_concave_polygon_boundary() {
    // The quad [5,9] x [2,18] meets the L: x in [5,6] keeps y in [2,10]
    // (area 8) and x in [6,9] keeps y in [2,4] (area 6), so exactly 14 of the
    // quad's 64 survive, in two disjoint strips.
    let db = block_db(
        block_image_entity(11),
        Transform3::identity(),
        Some(clip_polygon(&l_shape())),
    );
    let ctx = image_context(&[("clip.png", white_pixel())]);
    let rep = expand_with(&db, &ctx);
    assert_eq!(
        rep.completeness,
        Completeness::Complete,
        "{:?}",
        rep.diagnostics
    );
    let region = WorldRectBoundary::ring(&l_shape()).expect("the L is a simple ring");
    let mut pieces = 0;
    let mut area = 0.0;
    for fragment in &rep.fragments {
        if let DisplayPrimitive::Image {
            clip: Some(vertices),
            ..
        } = &fragment.primitive
        {
            pieces += 1;
            area += image_piece_area(vertices);
            for vertex in vertices.iter() {
                assert!(
                    region.contains([vertex.position.x, vertex.position.y]),
                    "image vertex ({}, {}) leaked out of the ring",
                    vertex.position.x,
                    vertex.position.y
                );
            }
        }
    }
    assert!(
        pieces >= 2,
        "the concave cut needs at least two convex pieces"
    );
    assert!(
        (area - 14.0).abs() < 1e-9,
        "visible area must be 14, got {area}"
    );
}

/// One refused boundary: the stored clip, the stable code, and whether the
/// geometry must stay fully drawn.
fn assert_refused(clip: BlockClip, code: &str) {
    let member = line_entity(11, SpaceId::Block(BlockId(1)), p(-5.0, 0.0), p(5.0, 0.0));
    let db = block_db(member, Transform3::identity(), Some(clip));
    let rep = expand(&db);
    assert!(
        matches!(rep.completeness, Completeness::Partial(_)),
        "a refusal is never Complete"
    );
    assert!(
        rep.diagnostics.iter().any(|d| d.code == code),
        "expected {code}, got {:?}",
        rep.diagnostics
    );
    let runs = lines(&rep);
    assert_eq!(runs.len(), 1, "the member stays drawn");
    let (a, b) = (runs[0][0], runs[0][1]);
    let length = (b.x - a.x).hypot(b.y - a.y);
    assert!(
        (length - 10.0).abs() < 1e-9,
        "a refusal keeps the whole member, never a partial cut; got {length}"
    );
}

#[test]
fn a_self_intersecting_boundary_is_refused_and_drawn_unclipped() {
    assert_refused(
        clip_polygon(&[[0.0, -2.0], [2.0, 2.0], [2.0, -2.0], [0.0, 2.0]]),
        "representation.xclip_self_intersecting",
    );
}

#[test]
fn a_zero_area_boundary_is_refused_and_drawn_unclipped() {
    assert_refused(
        clip_polygon(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]]),
        "representation.xclip_degenerate",
    );
}

#[test]
fn a_boundary_with_a_repeated_vertex_is_refused_and_drawn_unclipped() {
    assert_refused(
        clip_polygon(&[[0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
        "representation.xclip_degenerate",
    );
}

#[test]
fn a_boundary_with_fewer_than_three_points_is_refused_and_drawn_unclipped() {
    assert_refused(
        BlockClip::new(vec![[0.0, 0.0]], true, true),
        "representation.xclip_boundary_arity",
    );
    assert_refused(
        BlockClip::new(Vec::new(), true, true),
        "representation.xclip_boundary_arity",
    );
}

#[test]
fn an_inverted_boundary_is_refused_and_drawn_unclipped() {
    let mut clip = clip_rect(-1.0, -1.0, 1.0, 1.0, true);
    clip.inside = false;
    assert_refused(clip, "representation.xclip_invert_unsupported");
}

#[test]
fn an_off_plane_boundary_is_refused_and_drawn_unclipped() {
    // Y of the block plane maps to world +Z, so the ring leaves world XY.
    let mut m = Transform3::identity().matrix;
    m[1][1] = 0.0;
    m[2][1] = 1.0;
    m[1][2] = -1.0;
    m[2][2] = 0.0;
    let member = line_entity(11, SpaceId::Block(BlockId(1)), p(-5.0, 0.0), p(5.0, 0.0));
    let db = block_db(
        member,
        Transform3 { matrix: m },
        Some(clip_rect(-1.0, -1.0, 1.0, 1.0, true)),
    );
    let rep = expand(&db);
    assert!(
        rep.diagnostics
            .iter()
            .any(|d| d.code == "representation.xclip_plane_unsupported"),
        "got {:?}",
        rep.diagnostics
    );
}
