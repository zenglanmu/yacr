//! `XCLIP` display clipping through the representation layer.
//!
//! The boundary is stored on the model `INSERT` in the referenced block's local
//! space; expansion maps it through the composed transform and cuts the
//! expanded geometry. These tests pin the exact paths (lines, conics, meshes,
//! nested intersections) and the honest refusals (rotated boundary, polygon,
//! text/mask) rather than a visual claim.

use super::*;

/// A block-local rectangle boundary.
fn clip_rect(minx: f64, miny: f64, maxx: f64, maxy: f64, visible: bool) -> BlockClip {
    BlockClip {
        vertices: vec![[minx, miny], [maxx, maxy]],
        inside: true,
        visible,
    }
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
    ProviderRegistry::with_default_provider()
        .build_expanded(db, db.entity(EntityId(1)).unwrap(), &context())
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
        vertices: vec![p(-2.0, -2.0), p(2.0, -2.0), p(0.0, 2.0)],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    }
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
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(21)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(line_entity(
        21,
        SpaceId::Block(BlockId(2)),
        p(-5.0, 0.0),
        p(5.0, 0.0),
    ))
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
fn a_rotated_insert_is_reported_and_drawn_unclipped() {
    let member = line_entity(11, SpaceId::Block(BlockId(1)), p(-5.0, 0.0), p(5.0, 0.0));
    let db = block_db(
        member,
        rotation_z(45.0),
        Some(clip_rect(-1.0, -1.0, 1.0, 1.0, true)),
    );
    let rep = expand(&db);
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    assert!(
        rep.diagnostics
            .iter()
            .any(|d| d.code == "representation.xclip_rotation_unsupported"),
        "got {:?}",
        rep.diagnostics
    );
    let runs = lines(&rep);
    assert_eq!(runs.len(), 1);
    let (a, b) = (runs[0][0], runs[0][1]);
    let length = (b.x - a.x).hypot(b.y - a.y);
    assert!((length - 10.0).abs() < 1e-9, "the full member stays drawn");
}

#[test]
fn a_polygon_boundary_is_reported_and_drawn_unclipped() {
    let member = line_entity(11, SpaceId::Block(BlockId(1)), p(-5.0, 0.0), p(5.0, 0.0));
    let polygon = BlockClip {
        vertices: vec![[-1.0, -1.0], [1.0, -1.0], [0.0, 1.0]],
        inside: true,
        visible: true,
    };
    let db = block_db(member, Transform3::identity(), Some(polygon));
    let rep = expand(&db);
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    assert!(
        rep.diagnostics
            .iter()
            .any(|d| d.code == "representation.xclip_polygon_not_renderable"),
        "got {:?}",
        rep.diagnostics
    );
    assert_eq!(lines(&rep).len(), 1);
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
