//! The classic-panel projections of `cad_status`: drawing block definitions,
//! session-only groups and raster IMAGE entities, each copied from its authority.

use std::collections::BTreeSet;

use super::super::super::*;
use super::super::test_support::*;

fn line_entity(id: u128, space: cad_domain::SpaceId) -> cad_db::DbEntity {
    cad_db::DbEntity {
        object: cad_db::DbObject {
            id: cad_domain::ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: cad_domain::Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: cad_domain::LayerId(0),
        space,
        geometry: SemanticGeometry::Line {
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
        draw_order: id as i64,
    }
}

fn image_entity(
    id: u128,
    space: cad_domain::SpaceId,
    file: Option<&str>,
    visible: bool,
    clip: Option<cad_domain::ImageClip>,
) -> cad_db::DbEntity {
    cad_db::DbEntity {
        object: cad_db::DbObject {
            id: cad_domain::ObjectId(id),
            type_key: "AcDbRasterImage".into(),
            revision: cad_domain::Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: cad_domain::LayerId(0),
        space,
        geometry: SemanticGeometry::Image {
            origin: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            u: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            v: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            pixels: [640.0, 480.0],
            file: file.map(str::to_string),
            clip,
            visible,
            adjust: cad_domain::ImageAdjust::default(),
        },
        draw_order: id as i64,
    }
}

fn session_with_classic_rows() -> CadSession {
    let session = session(std::rc::Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let mut builder = cad_db::DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1));
    builder
        .insert_layer(cad_db::Layer {
            id: cad_domain::LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    // Block 1 is named and carries two members; block 2 is deliberately unnamed
    // and carries one image member.
    builder
        .insert_block(cad_db::BlockDefinition {
            id: cad_domain::BlockId(1),
            entities: vec![EntityId(10), EntityId(11)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(
            cad_domain::BlockId(1),
            "Widget",
            Point3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
        )
        .unwrap();
    builder
        .insert_block(cad_db::BlockDefinition {
            id: cad_domain::BlockId(2),
            entities: vec![EntityId(21)],
            dynamic_visibility: None,
        })
        .unwrap();
    for id in [10, 11] {
        builder
            .insert_entity(line_entity(
                id,
                cad_domain::SpaceId::Block(cad_domain::BlockId(1)),
            ))
            .unwrap();
    }
    builder
        .insert_entity(image_entity(
            20,
            cad_domain::SpaceId::Model,
            Some("logo.png"),
            true,
            Some(cad_domain::ImageClip {
                vertices: vec![[0.0, 0.0], [10.0, 10.0]],
                inside: true,
            }),
        ))
        .unwrap();
    builder
        .insert_entity(image_entity(
            21,
            cad_domain::SpaceId::Block(cad_domain::BlockId(2)),
            None,
            false,
            None,
        ))
        .unwrap();
    let drawing = builder.finish().expect("classic fixture builds");
    let document = {
        let mut controller = session.core.controller.borrow_mut();
        let document = controller.document_id;
        controller
            .application
            .workspace
            .documents
            .get_mut(&document)
            .expect("the session's document")
            .drawing = std::sync::Arc::new(drawing);
        document
    };
    {
        let mut controller = session.core.controller.borrow_mut();
        let member = |entity| SelectionRef {
            document,
            entity: EntityId(entity),
            instance: InstancePath::default(),
            sub_element: None,
        };
        controller
            .session
            .groups
            .create("Walls", vec![member(20), member(21)]);
    }
    session
}

#[test]
fn classic_rows_are_projected_from_their_authorities() {
    let status = session_with_classic_rows().snapshot(None, None, 0);

    assert_eq!(status.blocks.len(), 2, "one row per block definition");
    assert_eq!(status.blocks[0].name.as_deref(), Some("Widget"));
    assert_eq!(status.blocks[0].member_count, 2);
    assert_eq!(status.blocks[0].base_point, Some([1.0, 2.0, 3.0]));
    assert_eq!(
        status.blocks[1].name, None,
        "an unnamed block invents no name"
    );
    assert_eq!(status.blocks[1].member_count, 1);
    assert_eq!(
        status.blocks[1].base_point, None,
        "missing metadata invents no zero base point"
    );

    assert_eq!(status.groups.len(), 1);
    assert_eq!(status.groups[0].name, "Walls");
    assert_eq!(status.groups[0].member_count, 2);
    assert_eq!(
        status.groups[0]
            .members
            .iter()
            .map(|member| member.entity_id.as_str())
            .collect::<Vec<_>>(),
        vec!["20", "21"]
    );
    assert!(status.groups[0]
        .members
        .iter()
        .all(|member| member.instance_path.is_empty()));

    assert_eq!(status.images.len(), 2, "one row per IMAGE entity");
    let first = &status.images[0];
    assert_eq!(first.id, "20");
    assert_eq!(first.space, "model");
    assert_eq!(first.space_id, None);
    assert_eq!(first.file.as_deref(), Some("logo.png"));
    assert_eq!(first.pixels, [640.0, 480.0]);
    assert!(first.visible);
    assert_eq!(
        first.clip,
        Some(CadImageClipStatus {
            vertex_count: 2,
            inside: true,
        })
    );
    let second = &status.images[1];
    assert_eq!(second.id, "21");
    assert_eq!(second.space, "block");
    assert_eq!(second.space_id, Some(2));
    assert_eq!(second.file, None);
    assert!(!second.visible);
    assert_eq!(second.clip, None);
}

#[test]
fn classic_rows_serialize_as_camel_case_and_are_never_omitted() {
    let status = session_with_classic_rows().snapshot(None, None, 0);
    let value = serde_json::to_value(&status).expect("the status serializes");

    let block_keys: BTreeSet<&str> = value["blocks"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        block_keys,
        ["name", "memberCount", "basePoint"]
            .into_iter()
            .collect::<BTreeSet<_>>()
    );
    let group_keys: BTreeSet<&str> = value["groups"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        group_keys,
        ["name", "memberCount", "members"]
            .into_iter()
            .collect::<BTreeSet<_>>()
    );
    let image_keys: BTreeSet<&str> = value["images"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        image_keys,
        ["id", "space", "spaceId", "file", "pixels", "visible", "clip",]
            .into_iter()
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(value["images"][0]["clip"]["vertexCount"], 2);
    assert_eq!(value["images"][0]["clip"]["inside"], true);

    let empty = serde_json::to_value(unsupported_status(None)).unwrap();
    assert_eq!(empty["blocks"], serde_json::json!([]));
    assert_eq!(empty["groups"], serde_json::json!([]));
    assert_eq!(empty["images"], serde_json::json!([]));
}
