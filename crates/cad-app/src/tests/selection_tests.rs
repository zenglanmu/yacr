use super::*;
use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_domain::{
    BlockId, DatabaseId, DocumentId, EntityId, InstancePath, LayerId, ObjectId, Revision, StyleId,
    Transform3,
};

fn entity(id: u128, geometry: SemanticGeometry, layer: LayerId) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: Some(format!("{id:X}")),
        },
        id: EntityId(id),
        layer,
        space: SpaceId::Model,
        geometry,
        draw_order: id as i64,
    }
}

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn database() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(20)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(entity(
        20,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        LayerId(0),
    ))
    .unwrap();
    b.insert_entity(entity(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(3.0, 4.0),
        },
        LayerId(0),
    ))
    .unwrap();
    b.insert_entity(entity(
        2,
        SemanticGeometry::Circle {
            center: point(1.0, 2.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 2.5,
        },
        LayerId(0),
    ))
    .unwrap();
    b.insert_entity(entity(
        3,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(point(10.0, 0.0)),
        },
        LayerId(0),
    ))
    .unwrap();
    b.finish().unwrap()
}

fn ref_of(document: DocumentId, entity: EntityId) -> SelectionRef {
    model_ref(document, entity)
}

#[test]
fn selection_distinguishes_insert_instances_via_instance_path() {
    let doc = DocumentId(1);
    // Same block entity reached through two different placements.
    let a = SelectionRef {
        document: doc,
        entity: EntityId(20),
        instance: InstancePath(vec![EntityId(3)]),
        sub_element: None,
    };
    let b = SelectionRef {
        document: doc,
        entity: EntityId(20),
        instance: InstancePath(vec![EntityId(4)]),
        sub_element: None,
    };
    let mut set = SelectionSet::new();
    assert!(set.insert(a.clone()));
    assert!(set.insert(b.clone()));
    assert_eq!(set.len(), 2, "different instances are separate objects");

    // The identical instance is a duplicate.
    assert!(!set.insert(a.clone()));
    assert_eq!(set.len(), 2);

    // Identity labels differ.
    assert_ne!(
        SelectionSet::identity_label(&a),
        SelectionSet::identity_label(&b)
    );

    // A sub-element of the same instance is again distinct.
    let sub = SelectionRef {
        document: doc,
        entity: EntityId(20),
        instance: InstancePath(vec![EntityId(3)]),
        sub_element: Some(cad_domain::SubElementId {
            source_key: "face0".into(),
            topology_revision: Revision(0),
        }),
    };
    assert!(set.insert(sub));
    assert_eq!(set.len(), 3);
}

#[test]
fn toggle_and_remove_are_identity_exact() {
    let doc = DocumentId(1);
    let a = ref_of(doc, EntityId(1));
    let mut set = SelectionSet::new();
    set.toggle(a.clone());
    assert!(set.contains(&a));
    set.toggle(a.clone());
    assert!(!set.contains(&a));
    set.insert(a.clone());
    assert!(set.remove(&a));
    assert!(!set.remove(&a));
}

#[test]
fn empty_selection_is_explicitly_empty() {
    let db = database();
    let props = SelectionProperties::extract(&db, &SelectionSet::new());
    assert!(props.empty);
    assert_eq!(props.count, 0);
    assert!(props.rows.is_empty());
}

#[test]
fn single_line_selection_reports_identity_and_geometry() {
    let db = database();
    let set = SelectionSet::from_refs([ref_of(DocumentId(1), EntityId(1))]);
    let props = SelectionProperties::extract(&db, &set);
    assert!(!props.empty);
    assert_eq!(props.count, 1);
    let row = |key: &str| {
        props
            .rows
            .iter()
            .find(|r| r.key == key)
            .map(|r| r.value.clone())
    };
    assert_eq!(row("id").as_deref(), Some("1"));
    assert_eq!(row("handle").as_deref(), Some("1"));
    assert_eq!(row("type").as_deref(), Some("AcDbEntity"));
    assert_eq!(row("layer").as_deref(), Some("0 (0)"));
    assert_eq!(row("space").as_deref(), Some("model"));
    // Line length 3-4-5.
    assert_eq!(row("length").as_deref(), Some("5.000000"));
}

#[test]
fn circle_and_insert_report_geometry_specific_rows() {
    let db = database();
    let circle = SelectionProperties::extract(
        &db,
        &SelectionSet::from_refs([ref_of(DocumentId(1), EntityId(2))]),
    );
    let radius = circle
        .rows
        .iter()
        .find(|r| r.key == "radius")
        .unwrap()
        .value
        .clone();
    assert_eq!(radius, "2.500000");

    let insert = SelectionProperties::extract(
        &db,
        &SelectionSet::from_refs([ref_of(DocumentId(1), EntityId(3))]),
    );
    let block = insert
        .rows
        .iter()
        .find(|r| r.key == "block")
        .unwrap()
        .value
        .clone();
    assert_eq!(block, "0");
}

#[test]
fn multi_select_reports_mixed_keys_instead_of_a_single_value() {
    let db = database();
    let set = SelectionSet::from_refs([
        ref_of(DocumentId(1), EntityId(1)), // Line
        ref_of(DocumentId(1), EntityId(2)), // Circle
    ]);
    let props = SelectionProperties::extract(&db, &set);
    assert_eq!(props.count, 2);
    assert!(props.rows.is_empty());
    // Geometry keys differ (line has `length`, circle has `radius`).
    assert!(props.mixed_keys.contains(&"length"));
    assert!(props.mixed_keys.contains(&"radius"));
    // Shared identity keys are not "mixed" when values agree; ids differ.
    assert!(props.mixed_keys.contains(&"id"));
}

#[test]
fn unresolved_entity_is_reported_honestly() {
    let db = database();
    let set = SelectionSet::from_refs([ref_of(DocumentId(1), EntityId(999))]);
    let props = SelectionProperties::extract(&db, &set);
    assert_eq!(props.count, 1);
    let status = props
        .rows
        .iter()
        .find(|r| r.key == "status")
        .map(|r| r.value.clone());
    assert_eq!(status.as_deref(), Some("unresolved"));
}

#[test]
fn text_without_font_is_explicitly_unknown() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(2));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(entity(
        7,
        SemanticGeometry::Text {
            text: "标注".into(),
            position: point(0.0, 0.0),
            style: StyleId(0),
            height: 2.5,
            rotation: 0.0,
            font: None,
            h_align: cad_domain::TextAlignH::Left,
            v_align: cad_domain::TextAlignV::Baseline,
        },
        LayerId(0),
    ))
    .unwrap();
    let db = b.finish().unwrap();
    let props = SelectionProperties::extract(
        &db,
        &SelectionSet::from_refs([ref_of(DocumentId(2), EntityId(7))]),
    );
    let font = props
        .rows
        .iter()
        .find(|r| r.key == "font")
        .map(|r| r.value.clone());
    assert_eq!(font.as_deref(), Some("unknown"));
}

#[test]
fn image_and_mask_report_only_stored_metadata() {
    let rows = geometry_property_rows(&SemanticGeometry::Image {
        origin: point(1.0, 2.0),
        u: point(0.5, 0.0),
        v: point(0.0, 0.25),
        pixels: [640.0, 480.0],
        file: Some("logo.png".into()),
        clip: None,
        visible: true,
    });
    let value = |key: &str| rows.iter().find(|r| r.key == key).map(|r| r.value.clone());
    assert_eq!(value("pixels").as_deref(), Some("640 x 480"));
    assert_eq!(value("file").as_deref(), Some("logo.png"));
    assert_eq!(value("visible").as_deref(), Some("true"));
    assert_eq!(value("length"), None);

    let mask_rows = geometry_property_rows(&SemanticGeometry::Mask {
        boundary: vec![point(0.0, 0.0), point(1.0, 0.0), point(1.0, 1.0)],
        inverted: true,
    });
    let value = |key: &str| {
        mask_rows
            .iter()
            .find(|r| r.key == key)
            .map(|r| r.value.clone())
    };
    assert_eq!(value("boundary").as_deref(), Some("3"));
    assert_eq!(value("inverted").as_deref(), Some("true"));
}

/// A MULTILEADER, optionally with text content (`None` = content-less).
fn leader_geometry(text: Option<&str>) -> SemanticGeometry {
    use cad_domain::{
        MLeaderContent, MLeaderLine, MLeaderPathType, MLeaderReserved, MLeaderRoot,
        MLeaderStyleValues, MLeaderText, MultiLeaderGeometry, MultiLeaderParts, PlaneFrame,
        StyleId,
    };
    let (geometry, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: MLeaderStyleValues {
            path_type: MLeaderPathType::StraightLineSegments,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 2.0,
            landing_gap: 0.1,
            arrowhead_size: 0.5,
            text_height: 0.2,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 1.0,
        },
        style_name: "Standard".into(),
        override_mask: 0,
        content: if text.is_some() {
            MLeaderContent::Text
        } else {
            MLeaderContent::None
        },
        text: MLeaderText {
            string: text.unwrap_or_default().into(),
            location: point(10.0, 5.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
            attachment_point: 2,
            left_attachment: 2,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: point(10.0, 5.0),
            dogleg_vector: point(1.0, 0.0),
            dogleg_length: 2.0,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![point(0.0, 0.0), point(8.0, 0.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    SemanticGeometry::MultiLeader(Box::new(geometry))
}

#[test]
fn selection_text_carries_multileader_text_and_stays_none_for_the_rest() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(entity(
        1,
        leader_geometry(Some("LEADER\\PNOTE")),
        LayerId(0),
    ))
    .unwrap();
    b.insert_entity(entity(2, leader_geometry(None), LayerId(0)))
        .unwrap();
    b.insert_entity(entity(
        3,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        LayerId(0),
    ))
    .unwrap();
    let database = b.finish().unwrap();
    let doc = DocumentId(1);

    let text_for = |refs: Vec<SelectionRef>| {
        let set = SelectionSet::from_refs(refs);
        SelectionProperties::extract(&database, &set).text
    };

    // The leader's raw string is the editable text (never a `\P` mapping).
    assert_eq!(
        text_for(vec![ref_of(doc, EntityId(1))]),
        Some("LEADER\\PNOTE".to_string())
    );
    // Content-less leader and a non-text entity: no editable text.
    assert_eq!(text_for(vec![ref_of(doc, EntityId(2))]), None);
    assert_eq!(text_for(vec![ref_of(doc, EntityId(3))]), None);
    // Multi-selection: never one object's string.
    assert_eq!(
        text_for(vec![ref_of(doc, EntityId(1)), ref_of(doc, EntityId(3))]),
        None
    );
    // An instance path or sub-element is never reached into.
    assert_eq!(
        text_for(vec![SelectionRef {
            instance: InstancePath(vec![EntityId(3)]),
            ..ref_of(doc, EntityId(1))
        }]),
        None
    );
    assert_eq!(
        text_for(vec![SelectionRef {
            sub_element: Some(cad_domain::SubElementId {
                source_key: "face0".into(),
                topology_revision: Revision(0),
            }),
            ..ref_of(doc, EntityId(1))
        }]),
        None
    );
}

/// The same fixture entity with an explicit source class (`AcDbText` /
/// `AcDbMText`), which is what the multiline capability keys off.
fn entity_typed(id: u128, type_key: &str, geometry: SemanticGeometry, layer: LayerId) -> DbEntity {
    let mut entity = entity(id, geometry, layer);
    entity.object.type_key = type_key.into();
    entity
}

fn text_geometry(text: &str) -> SemanticGeometry {
    SemanticGeometry::Text {
        text: text.into(),
        position: point(0.0, 0.0),
        style: StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: cad_domain::TextAlignH::Left,
        v_align: cad_domain::TextAlignV::Baseline,
    }
}

#[test]
fn text_multiline_is_a_capability_of_the_single_text_target() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(3));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(entity_typed(
        1,
        "AcDbText",
        text_geometry("ONE"),
        LayerId(0),
    ))
    .unwrap();
    b.insert_entity(entity_typed(
        2,
        "AcDbMText",
        text_geometry("TWO\\PTHREE"),
        LayerId(0),
    ))
    .unwrap();
    b.insert_entity(entity_typed(
        3,
        "AcDbMLeader",
        leader_geometry(Some("LEADER\\PNOTE")),
        LayerId(0),
    ))
    .unwrap();
    b.insert_entity(entity_typed(
        4,
        "AcDbMLeader",
        leader_geometry(None),
        LayerId(0),
    ))
    .unwrap();
    let database = b.finish().unwrap();
    let doc = DocumentId(3);

    let props = |refs: Vec<SelectionRef>| {
        SelectionProperties::extract(&database, &SelectionSet::from_refs(refs))
    };
    let single = |entity: u128| props(vec![ref_of(doc, EntityId(entity))]);

    // AcDbText: editable, single-line.
    let text = single(1);
    assert_eq!(text.text.as_deref(), Some("ONE"));
    assert!(!text.text_multiline);

    // AcDbMText: editable, multiline (the raw `\P` is preserved).
    let mtext = single(2);
    assert_eq!(mtext.text.as_deref(), Some("TWO\\PTHREE"));
    assert!(mtext.text_multiline);

    // A MULTILEADER with text content is multiline.
    let leader = single(3);
    assert_eq!(leader.text.as_deref(), Some("LEADER\\PNOTE"));
    assert!(leader.text_multiline);

    // A content-less leader has nothing editable, so no capability.
    let empty_leader = single(4);
    assert_eq!(empty_leader.text, None);
    assert!(!empty_leader.text_multiline);

    // No selection: `text: null` and `text_multiline: false`.
    let none = props(Vec::new());
    assert_eq!(none.text, None);
    assert!(!none.text_multiline);

    // A multi-selection is never one object's text target.
    let multi = props(vec![ref_of(doc, EntityId(1)), ref_of(doc, EntityId(2))]);
    assert_eq!(multi.text, None);
    assert!(!multi.text_multiline);

    // An unresolved entity has no text capability to report.
    let unresolved = single(999);
    assert_eq!(unresolved.text, None);
    assert!(!unresolved.text_multiline);

    // An instance path never reaches into block internals.
    let instance = props(vec![SelectionRef {
        instance: InstancePath(vec![EntityId(1)]),
        ..ref_of(doc, EntityId(2))
    }]);
    assert_eq!(instance.text, None);
    assert!(!instance.text_multiline);
}
