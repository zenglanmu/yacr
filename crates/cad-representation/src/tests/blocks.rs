use super::*;

#[test]
fn insert_instances_expand_with_transform_and_instance_path() {
    let mut b = empty_db();
    // Block 2: one line (0,0)-(0,1).
    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(12)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(line_entity(
        12,
        SpaceId::Block(BlockId(2)),
        p(0.0, 0.0),
        p(0.0, 1.0),
    ))
    .unwrap();
    // Block 1: line (0,0)-(1,0) plus a nested insert of block 2 at y=5.
    b.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: vec![EntityId(11), EntityId(13)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(line_entity(
        11,
        SpaceId::Block(BlockId(1)),
        p(0.0, 0.0),
        p(1.0, 0.0),
    ))
    .unwrap();
    let mut nested = insert_entity(13, SpaceId::Block(BlockId(1)), 2, 0.0);
    nested.geometry = SemanticGeometry::Insert {
        block: BlockId(2),
        transform: Transform3::translation(p(0.0, 5.0)),
        attributes: Vec::new(),
    };
    b.insert_entity(nested).unwrap();
    // Model: a single insert of block 1 at x=10.
    b.insert_entity(insert_entity(1, SpaceId::Model, 1, 10.0))
        .unwrap();
    let db = b.finish().unwrap();

    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert_eq!(rep.completeness, Completeness::Complete);
    assert_eq!(rep.fragments.len(), 2);

    let mut direct = None;
    let mut deep = None;
    for fragment in &rep.fragments {
        match &fragment.primitive {
            DisplayPrimitive::Lines(pts) => match fragment.source.instance.0.len() {
                1 => direct = Some((pts[0], pts[1])),
                2 => deep = Some((pts[0], pts[1])),
                other => panic!("unexpected instance path depth {other}"),
            },
            _ => panic!("expected lines"),
        }
    }
    let (a, z) = direct.expect("direct block line");
    assert_eq!((a.x, a.y), (10.0, 0.0));
    assert_eq!((z.x, z.y), (11.0, 0.0));
    // Nested line shifted by block 1's insert (0,5) then model insert (10,0).
    let (a, z) = deep.expect("nested block line");
    assert_eq!((a.x, a.y), (10.0, 5.0));
    assert_eq!((z.x, z.y), (10.0, 6.0));
}

#[test]
fn block_children_inherit_the_insert_draw_order() {
    // The database orders a space by the INSERT entity, so expanding the block
    // must stamp the INSERT's order on the children, not the child's own order
    // (which only orders entities *inside* the block definition).
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(4),
        entities: vec![EntityId(12)],
        dynamic_visibility: None,
    })
    .unwrap();
    let mut child = line_entity(12, SpaceId::Block(BlockId(4)), p(0.0, 0.0), p(1.0, 0.0));
    child.draw_order = 3;
    b.insert_entity(child).unwrap();
    let mut insert = insert_entity(1, SpaceId::Model, 4, 10.0);
    insert.draw_order = 77;
    b.insert_entity(insert).unwrap();
    let db = b.finish().unwrap();

    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert_eq!(rep.fragments.len(), 1);
    assert_eq!(
        rep.fragments[0].draw_order, 77,
        "the child fragment takes the INSERT's order ({})",
        77
    );
}

#[test]
fn missing_block_definition_is_reported_missing() {
    let mut b = empty_db();
    b.insert_entity(insert_entity(1, SpaceId::Model, 99, 0.0))
        .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert!(matches!(rep.completeness, Completeness::Missing(_)));
    assert!(rep
        .diagnostics
        .iter()
        .any(|d| d.code == "representation.missing_block"));
    assert!(rep.fragments.is_empty());
}

#[test]
fn self_referencing_block_is_cut_with_partial_report() {
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(2)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(insert_entity(2, SpaceId::Block(BlockId(0)), 0, 1.0))
        .unwrap();
    b.insert_entity(insert_entity(1, SpaceId::Model, 0, 0.0))
        .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    assert!(rep
        .diagnostics
        .iter()
        .any(|d| d.code == "representation.instance_cycle"));
}

/// A MULTILEADER whose annotation content is a block draws the block's geometry:
/// the content kind reaches the provider as an ordinary block instance child, so
/// there is no second expansion path for it.
#[test]
fn a_multileader_block_content_expands_the_block_members() {
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(12)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(line_entity(
        12,
        SpaceId::Block(BlockId(2)),
        p(0.0, 0.0),
        p(2.0, 0.0),
    ))
    .unwrap();
    let leader = cad_domain::MultiLeaderGeometry::created_with_content(
        cad_domain::MLeaderStyleValues {
            path_type: cad_domain::MLeaderPathType::StraightLineSegments,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 1.0,
            landing_gap: 0.1,
            arrowhead_size: 0.5,
            text_height: 0.2,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 1.0,
            text_angle_type: cad_domain::MLeaderTextAngle::Horizontal,
        },
        "Standard".into(),
        2,
        p(0.0, 0.0),
        p(10.0, 5.0),
        cad_domain::MLeaderCreateContent::Block {
            block: BlockId(2),
            name: "Door".into(),
            // The definition's base point is what lands on the content point.
            base_point: Some(p(1.5, 0.0)),
        },
        Default::default(),
    )
    .expect("a valid block-content leader");
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbMultiLeader".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::MultiLeader(Box::new(leader)),
        draw_order: 1,
    })
    .unwrap();
    let db = b.finish().unwrap();

    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    // The block member line is drawn with the content placement composed in: the
    // leader's leader-line/arrowhead/dogleg fragments plus the block's line.
    let closes = |a: Point3, b: Point3| {
        (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9 && (a.z - b.z).abs() < 1e-9
    };
    // The member line is (0,0)-(2,0) in block space and the definition's base
    // point is (1.5, 0), so the base lands on the content point (10, 5) and the
    // member runs from (8.5, 5) to (10.5, 5).
    let block_line = rep
        .fragments
        .iter()
        .find_map(|fragment| match &fragment.primitive {
            DisplayPrimitive::Lines(points) if points.len() == 2 => {
                let (a, b) = (points[0], points[1]);
                closes(a, p(8.5, 5.0)).then_some((a, b))
            }
            _ => None,
        });
    let (_, end) = block_line.expect("the block's member line placed by its base point");
    assert!(closes(end, p(10.5, 5.0)), "{end:?}");
    assert!(
        !rep.fragments
            .iter()
            .any(|fragment| matches!(fragment.primitive, DisplayPrimitive::Instance { .. })),
        "the instance is expanded, not handed to the renderer"
    );
}
