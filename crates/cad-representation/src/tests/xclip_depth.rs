//! `XCLIP` **clip depth** through the representation layer.
//!
//! `BlockClip` stores the DXF front/back clip distances as a block-local z band,
//! mapped to world z by `block_clip_depth`; these tests pin what the display does
//! with it: a primitive wholly outside the band is omitted, one that straddles a
//! plane is kept whole and named, the band follows the reference's z scale, a
//! one-sided band leaves its absent side unbounded, and both the polyline and the
//! conic/mesh emission paths obey it. The 2D region tests live in `xclip.rs`.

use super::*;

/// A block-local rectangle boundary (the shape `xclip.rs` also uses).
fn clip_rect(minx: f64, miny: f64, maxx: f64, maxy: f64, visible: bool) -> BlockClip {
    BlockClip::new(vec![[minx, miny], [maxx, maxy]], true, visible)
}

/// A plan circle in `space` (the conic emission path the depth band must obey).
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

/// A plan triangle mesh in the block plane z = 0.
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

/// A database with block 1 (one member) and a model INSERT of it at `transform`,
/// carrying `clip` (the same fixture shape `xclip.rs` uses).
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
    b.set_entity_render_attributes(
        EntityId(1),
        EntityRenderAttributes {
            clip,
            ..EntityRenderAttributes::default()
        },
    )
    .unwrap();
    b.finish().unwrap()
}

fn expand(db: &DrawingDatabase) -> DisplayRepresentation {
    ProviderRegistry::with_default_provider()
        .build_expanded(db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap()
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

/// A block-local rectangle with a validated front/back z band.
fn clip_rect_with_depth(
    minx: f64,
    miny: f64,
    maxx: f64,
    maxy: f64,
    front: f64,
    back: f64,
) -> BlockClip {
    clip_rect(minx, miny, maxx, maxy, true)
        .with_depth(Some(front), Some(back))
        .unwrap()
}

#[test]
fn a_depth_band_drops_the_members_that_lie_outside_it() {
    // A member of the block at block-local z = 5: inside the band it is drawn,
    // outside it is omitted exactly (not drawn unclipped).
    let member = line_entity(
        5,
        SpaceId::Block(BlockId(1)),
        p3(0.0, 0.0, 5.0),
        p3(4.0, 0.0, 5.0),
    );
    let outside = expand(&block_db(
        member.clone(),
        Transform3::identity(),
        Some(clip_rect_with_depth(-10.0, -10.0, 10.0, 10.0, 0.0, 1.0)),
    ));
    assert!(
        lines(&outside).is_empty(),
        "a member at z=5 is behind the [0, 1] band: {:?}",
        lines(&outside)
    );
    assert!(
        outside
            .diagnostics
            .iter()
            .all(|d| d.code != "representation.xclip_depth_straddle"),
        "a wholly outside primitive is not a straddle: {:?}",
        outside.diagnostics
    );

    let inside = expand(&block_db(
        member,
        Transform3::identity(),
        Some(clip_rect_with_depth(-10.0, -10.0, 10.0, 10.0, 4.0, 6.0)),
    ));
    assert_eq!(
        lines(&inside),
        vec![vec![p3(0.0, 0.0, 5.0), p3(4.0, 0.0, 5.0)]],
        "the same member is kept inside the [4, 6] band"
    );
    assert!(
        inside.diagnostics.is_empty(),
        "no degradation: {:?}",
        inside.diagnostics
    );
}

#[test]
fn the_depth_band_follows_the_reference_z_scale() {
    // Block-local band [0, 1] on a reference scaled by 3 in z keeps world z
    // 0..3, so the member at block z 0.5 (world 1.5) stays and the one at 2
    // (world 6) goes.
    let mut m = Transform3::identity().matrix;
    m[2][2] = 3.0;
    let transform = Transform3 { matrix: m };
    let clip = Some(clip_rect_with_depth(-10.0, -10.0, 10.0, 10.0, 0.0, 1.0));

    let kept_member = line_entity(
        5,
        SpaceId::Block(BlockId(1)),
        p3(0.0, 0.0, 0.5),
        p3(1.0, 0.0, 0.5),
    );
    let kept = expand(&block_db(kept_member, transform, clip.clone()));
    assert_eq!(
        lines(&kept),
        vec![vec![p3(0.0, 0.0, 1.5), p3(1.0, 0.0, 1.5)]],
        "the band is mapped into world z, not compared with block z"
    );

    let dropped_member = line_entity(
        5,
        SpaceId::Block(BlockId(1)),
        p3(0.0, 0.0, 2.0),
        p3(1.0, 0.0, 2.0),
    );
    let dropped = expand(&block_db(dropped_member, transform, clip));
    assert!(lines(&dropped).is_empty(), "world z 6 is outside 0..3");
}

#[test]
fn a_member_straddling_a_depth_plane_is_kept_whole_and_reported() {
    // The documented approximation: a primitive that crosses a depth plane is
    // kept whole (no per-vertex z cut) and named; it is never silently kept.
    let member = line_entity(
        5,
        SpaceId::Block(BlockId(1)),
        p3(0.0, 0.0, -1.0),
        p3(4.0, 0.0, 1.0),
    );
    let rep = expand(&block_db(
        member,
        Transform3::identity(),
        Some(clip_rect_with_depth(-10.0, -10.0, 10.0, 10.0, 0.0, 0.5)),
    ));
    assert_eq!(
        lines(&rep),
        vec![vec![p3(0.0, 0.0, -1.0), p3(4.0, 0.0, 1.0)]],
        "the straddling line is kept with both endpoints unchanged (not cut at z)"
    );
    let report = rep
        .diagnostics
        .iter()
        .find(|d| d.code == "representation.xclip_depth_straddle")
        .expect("a straddling primitive is reported by name");
    assert!(
        report.message.contains("kept whole") && report.message.contains("world z 0..0.5"),
        "the report states the band and what happens: {}",
        report.message
    );
    assert!(
        matches!(rep.completeness, Completeness::Partial(_)),
        "a kept straddle is a declared degradation: {:?}",
        rep.completeness
    );
}

#[test]
fn a_one_sided_depth_band_leaves_the_absent_side_unbounded() {
    // Only a back plane: block-local z <= 0 is kept, everything above is dropped.
    let clip = clip_rect(-10.0, -10.0, 10.0, 10.0, true)
        .with_depth(None, Some(0.0))
        .unwrap();
    let behind = line_entity(
        5,
        SpaceId::Block(BlockId(1)),
        p3(0.0, 0.0, -3.0),
        p3(1.0, 0.0, -3.0),
    );
    let rep = expand(&block_db(
        behind,
        Transform3::identity(),
        Some(clip.clone()),
    ));
    assert_eq!(lines(&rep).len(), 1, "z=-3 is inside (-inf, 0]");

    let above = line_entity(
        5,
        SpaceId::Block(BlockId(1)),
        p3(0.0, 0.0, 3.0),
        p3(1.0, 0.0, 3.0),
    );
    let rep = expand(&block_db(above, Transform3::identity(), Some(clip)));
    assert!(lines(&rep).is_empty(), "z=3 is outside (-inf, 0]");
}

#[test]
fn a_mesh_and_a_conic_obey_the_depth_band() {
    // The mesh and the analytic conic go through different emission paths; both
    // must drop when their z extent is outside the band.
    let mesh_entity = DbEntity {
        space: SpaceId::Block(BlockId(1)),
        ..entity(5, SemanticGeometry::Mesh(triangle_mesh()))
    };
    let clip = Some(clip_rect_with_depth(-10.0, -10.0, 10.0, 10.0, 5.0, 6.0));
    let rep = expand(&block_db(mesh_entity, Transform3::identity(), clip));
    assert!(
        rep.fragments.is_empty(),
        "a plan mesh at z=0 is outside [5, 6]: {} fragment(s)",
        rep.fragments.len()
    );

    let conic_entity = circle_entity(6, SpaceId::Block(BlockId(1)), p(0.0, 0.0), 1.0);
    let clip = Some(clip_rect_with_depth(-10.0, -10.0, 10.0, 10.0, 5.0, 6.0));
    let rep = expand(&block_db(conic_entity, Transform3::identity(), clip));
    assert!(
        rep.fragments.is_empty(),
        "a plan circle at z=0 is outside [5, 6]: {} fragment(s)",
        rep.fragments.len()
    );
}
