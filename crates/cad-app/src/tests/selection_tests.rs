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
            .and_then(|r| r.value.clone())
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
    assert_eq!(radius.as_deref(), Some("2.500000"));

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
    assert_eq!(block.as_deref(), Some("0"));
}

#[test]
fn multi_select_lists_common_keys_with_mixed_markers() {
    let db = database();
    let set = SelectionSet::from_refs([
        ref_of(DocumentId(1), EntityId(1)), // Line
        ref_of(DocumentId(1), EntityId(2)), // Circle
    ]);
    let props = SelectionProperties::extract(&db, &set);
    assert_eq!(props.count, 2);

    // Only the keys common to both entities are listed (the intersection).
    let keys: Vec<&str> = props.rows.iter().map(|row| row.key).collect();
    assert_eq!(
        keys,
        vec!["id", "handle", "type", "layer", "space", "draw_order"]
    );
    assert!(props.rows.iter().all(|row| row.key != "length"));
    assert!(props.rows.iter().all(|row| row.key != "radius"));

    // Identity values differ → mixed, explicit `None`, never a blank string.
    let id = props.rows.iter().find(|row| row.key == "id").unwrap();
    assert!(id.mixed);
    assert_eq!(id.value, None);
    assert!(props.mixed_keys.contains(&"id"));

    // The shared layer agrees → one real value, not mixed.
    let layer = props.rows.iter().find(|row| row.key == "layer").unwrap();
    assert!(!layer.mixed);
    assert_eq!(layer.value.as_deref(), Some("0 (0)"));
    assert!(!props.mixed_keys.contains(&"layer"));

    // `mixed_keys` is exactly the keys of the mixed rows, in row order.
    let mixed: Vec<&str> = props
        .rows
        .iter()
        .filter(|row| row.mixed)
        .map(|row| row.key)
        .collect();
    assert_eq!(props.mixed_keys, mixed);
}

#[test]
fn single_selection_rows_order_and_values_are_pinned() {
    let db = database();
    let set = SelectionSet::from_refs([ref_of(DocumentId(1), EntityId(1))]);
    let props = SelectionProperties::extract(&db, &set);
    let rows: Vec<(&str, Option<&str>)> = props
        .rows
        .iter()
        .map(|row| (row.key, row.value.as_deref()))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("id", Some("1")),
            ("handle", Some("1")),
            ("type", Some("AcDbEntity")),
            ("layer", Some("0 (0)")),
            ("space", Some("model")),
            ("draw_order", Some("1")),
            ("start", Some("0.000000, 0.000000, 0.000000")),
            ("end", Some("3.000000, 4.000000, 0.000000")),
            ("length", Some("5.000000")),
        ]
    );
    assert!(props.rows.iter().all(|row| !row.mixed));
    assert!(props.mixed_keys.is_empty());
}

#[test]
fn property_rows_carry_a_stable_group_and_kind() {
    let db = database();
    let set = SelectionSet::from_refs([ref_of(DocumentId(1), EntityId(1))]);
    let props = SelectionProperties::extract(&db, &set);
    let row = |key: &str| props.rows.iter().find(|row| row.key == key).unwrap();
    assert_eq!(row("id").group.key(), "entity");
    assert_eq!(row("layer").group.key(), "entity");
    assert_eq!(row("length").group.key(), "geometry");
    assert_eq!(row("length").kind, PropertyKind::Number);
    assert_eq!(row("layer").kind, PropertyKind::Enum);
    // Identity/geometry rows have no write path yet.
    assert!(!row("id").editable && !row("length").editable);

    // Per-geometry overrides: the same `style` key is text for a MULTILEADER
    // and display for a hatch.
    let mleader = geometry_property_rows(&leader_geometry(Some("NOTE")));
    assert_eq!(
        mleader
            .iter()
            .find(|row| row.key == "style")
            .unwrap()
            .group
            .key(),
        "text"
    );
    let hatch_style = PropertyRow::in_group("style", "solid", PropertyGroup::Display);
    assert_eq!(hatch_style.group.key(), "display");
    assert_eq!(PropertyGroup::for_key("style").key(), "text");
    assert_eq!(PropertyGroup::for_key("visible").key(), "display");
}

#[test]
fn multi_selection_with_the_same_layer_keeps_one_shared_value() {
    let db = database();
    // Entities 1 and 20 are both lines on layer 0; their lengths differ.
    let set = SelectionSet::from_refs([
        ref_of(DocumentId(1), EntityId(1)),
        ref_of(DocumentId(1), EntityId(20)),
    ]);
    let props = SelectionProperties::extract(&db, &set);
    let row = |key: &str| props.rows.iter().find(|row| row.key == key).unwrap();

    let layer = row("layer");
    assert_eq!(layer.value.as_deref(), Some("0 (0)"));
    assert!(!layer.mixed);
    assert!(!props.mixed_keys.contains(&"layer"));

    // Start points agree; end and length differ.
    assert_eq!(
        row("start").value.as_deref(),
        Some("0.000000, 0.000000, 0.000000")
    );
    assert!(!row("start").mixed);
    assert!(row("end").mixed && row("end").value.is_none());
    assert!(row("length").mixed);
    assert!(props.mixed_keys.contains(&"end"));
    assert!(props.mixed_keys.contains(&"length"));
}

#[test]
fn multi_selection_with_different_layers_reports_a_mixed_layer() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(4));
    for (id, name) in [(0, "0"), (1, "WALLS")] {
        b.insert_layer(Layer {
            id: LayerId(id),
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_entity(entity(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        LayerId(0),
    ))
    .unwrap();
    b.insert_entity(entity(
        2,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        LayerId(1),
    ))
    .unwrap();
    let db = b.finish().unwrap();
    let props = SelectionProperties::extract(
        &db,
        &SelectionSet::from_refs([
            ref_of(DocumentId(4), EntityId(1)),
            ref_of(DocumentId(4), EntityId(2)),
        ]),
    );

    let layer = props.rows.iter().find(|row| row.key == "layer").unwrap();
    assert!(layer.mixed);
    assert_eq!(layer.value, None);
    assert!(props.mixed_keys.contains(&"layer"));
    // The option list is still the drawing's real layer table.
    assert_eq!(layer.options, vec!["0".to_string(), "WALLS".to_string()]);
}

#[test]
fn enum_options_come_from_the_real_tables_sorted() {
    use cad_db::{LineType, LinetypePattern};

    let mut b = DrawingDatabaseBuilder::new(DatabaseId(5));
    for (id, name) in [(0, "0"), (1, "WALLS"), (2, "DOORS")] {
        b.insert_layer(Layer {
            id: LayerId(id),
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    for (id, name) in [(0, "DASHDOT"), (1, "CONTINUOUS")] {
        b.insert_linetype(LineType {
            id: cad_domain::LinetypeId(id),
            name: name.into(),
            pattern: LinetypePattern::continuous(),
            complex: false,
        })
        .unwrap();
    }
    b.insert_entity(entity(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        LayerId(1),
    ))
    .unwrap();
    let db = b.finish().unwrap();

    assert_eq!(
        enum_options(&db, "layer"),
        vec!["0".to_string(), "DOORS".to_string(), "WALLS".to_string()]
    );
    assert_eq!(
        enum_options(&db, "linetype"),
        vec!["CONTINUOUS".to_string(), "DASHDOT".to_string()]
    );
    // A key with no named table has no fabricated options.
    assert!(enum_options(&db, "color").is_empty());

    let props = SelectionProperties::extract(
        &db,
        &SelectionSet::from_refs([ref_of(DocumentId(5), EntityId(1))]),
    );
    let layer = props.rows.iter().find(|row| row.key == "layer").unwrap();
    assert_eq!(layer.kind, PropertyKind::Enum);
    assert_eq!(
        layer.options,
        vec!["0".to_string(), "DOORS".to_string(), "WALLS".to_string()]
    );
}

#[test]
fn enum_options_are_capped_after_sorting() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(6));
    let total = MAX_ENUM_OPTIONS + 8;
    for id in 0..total {
        b.insert_layer(Layer {
            id: LayerId(id as u128),
            name: format!("L{id:04}"),
            visible: true,
        })
        .unwrap();
    }
    b.insert_entity(entity(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        LayerId(0),
    ))
    .unwrap();
    let db = b.finish().unwrap();
    let options = enum_options(&db, "layer");
    assert_eq!(options.len(), MAX_ENUM_OPTIONS);
    // Sorted first, then capped: the list stays deterministic.
    assert_eq!(options[0], format!("L{:04}", 0));
    assert_eq!(
        options[MAX_ENUM_OPTIONS - 1],
        format!("L{:04}", MAX_ENUM_OPTIONS - 1)
    );
}

#[test]
fn layer_is_editable_only_for_a_directly_addressed_entity() {
    let db = database();
    let reference = ref_of(DocumentId(1), EntityId(1));
    let entity = db.entity(EntityId(1)).unwrap();
    assert!(property_row_editable(&db, &reference, entity, "layer"));
    // Keys without a write path stay read-only, including text content.
    for key in ["id", "length", "text", "closed", "font"] {
        assert!(
            !property_row_editable(&db, &reference, entity, key),
            "{key} must stay read-only in this lane"
        );
    }

    let props = SelectionProperties::extract(&db, &SelectionSet::from_refs([reference.clone()]));
    let layer = props.rows.iter().find(|row| row.key == "layer").unwrap();
    assert!(layer.editable);
    assert_eq!(layer.options, vec!["0".to_string()]);

    // A block-instance member is not directly addressable: the future write
    // path addresses top-level entities (same rule as text editing).
    let instance = SelectionRef {
        instance: InstancePath(vec![EntityId(3)]),
        ..reference
    };
    let rows = entity_property_rows(&db, &instance, entity);
    let layer = rows.iter().find(|row| row.key == "layer").unwrap();
    assert!(!layer.editable);
}

#[test]
fn multi_selection_row_is_editable_only_when_every_ref_supports_it() {
    let db = database();
    // Entity 1 is a top-level ref; entity 2 is reached through an instance path,
    // so its layer has no direct write target.
    let set = SelectionSet::from_refs([
        ref_of(DocumentId(1), EntityId(1)),
        SelectionRef {
            instance: InstancePath(vec![EntityId(3)]),
            ..ref_of(DocumentId(1), EntityId(2))
        },
    ]);
    let props = SelectionProperties::extract(&db, &set);
    let layer = props.rows.iter().find(|row| row.key == "layer").unwrap();
    assert_eq!(layer.value.as_deref(), Some("0 (0)"));
    assert!(!layer.mixed);
    assert!(
        !layer.editable,
        "one ref cannot write it, so the row is not editable"
    );
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
        .and_then(|r| r.value.clone());
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
            wrap_width: None,
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
        .and_then(|r| r.value.clone());
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
    let value = |key: &str| {
        rows.iter()
            .find(|r| r.key == key)
            .and_then(|r| r.value.clone())
    };
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
            .and_then(|r| r.value.clone())
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
        wrap_width: None,
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
