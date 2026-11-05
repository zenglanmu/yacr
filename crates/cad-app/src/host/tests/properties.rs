//! `SETENTITYPROPERTY` host-path contracts: the panel's write method applied to
//! the controller's **current** selection.
//!
//! No id travels with the call (the host resolves its own selection at apply
//! time), refusals are named and write nothing, and a successful edit is one
//! transaction / one undo step that keeps the selection intact.

use super::*;

fn demo() -> HostController {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    controller
}

fn ref_for(controller: &HostController, entity: u128) -> SelectionRef {
    crate::selection::model_ref(controller.document_id, EntityId(entity))
}

fn stored_layer(controller: &HostController, entity: u128) -> LayerId {
    controller
        .drawing()
        .expect("a document")
        .entity(EntityId(entity))
        .expect("the entity")
        .layer
}

#[test]
fn host_applies_a_property_to_the_current_selection_as_one_undo_step() {
    let mut controller = demo();
    controller
        .set_selection(vec![ref_for(&controller, 1), ref_for(&controller, 2)])
        .unwrap();
    let before = controller.drawing().unwrap().revision();

    controller
        .set_selection_property("layer", "0")
        .expect("the demo layer 0 exists");

    assert_eq!(stored_layer(&controller, 1), LayerId(0));
    assert_eq!(stored_layer(&controller, 2), LayerId(0));
    assert_eq!(
        controller.drawing().unwrap().revision(),
        Revision(before.0 + 1),
        "one transaction"
    );
    assert_eq!(controller.selection().len(), 2, "selection preserved");
    let props = controller.selection_properties().unwrap();
    let layer = props.rows.iter().find(|row| row.key == "layer").unwrap();
    assert_eq!(layer.value.as_deref(), Some("0 (0)"));

    // One undo step restores both.
    controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::Undo,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .expect("undo");
    assert_eq!(stored_layer(&controller, 1), LayerId(1));
    assert_eq!(stored_layer(&controller, 2), LayerId(1));
    assert!(!controller.history_availability().can_undo);
}

#[test]
fn host_refuses_an_invalid_value_without_writing_or_recording() {
    let mut controller = demo();
    controller
        .set_selection(vec![ref_for(&controller, 5)])
        .unwrap();
    let before = controller.drawing().unwrap().revision();

    let error = controller
        .set_selection_property("transparency", "91")
        .unwrap_err();
    assert!(
        format!("{error:?}").contains("property.invalid_value"),
        "got {error:?}"
    );
    assert_eq!(controller.drawing().unwrap().revision(), before);
    assert!(!controller.history_availability().can_undo);
}

#[test]
fn host_refuses_an_empty_selection_explicitly() {
    let mut controller = demo();
    controller.set_selection(Vec::new()).unwrap();
    let error = controller.set_selection_property("layer", "0").unwrap_err();
    assert!(
        format!("{error:?}").contains("property.selection_empty"),
        "got {error:?}"
    );
    assert!(!controller.history_availability().can_undo);
}

#[test]
fn host_applies_a_display_attribute_to_every_selected_entity() {
    let mut controller = demo();
    controller
        .set_selection(vec![ref_for(&controller, 1), ref_for(&controller, 2)])
        .unwrap();
    // The dispatch arm in `cad-host` calls exactly this controller method; the
    // request itself carries no id (see `InteractionRequest::SetSelectedProperty`).
    controller
        .set_selection_property("color", "aci:1")
        .expect("aci:1 is a palette colour");
    for entity in [1u128, 2] {
        assert_eq!(
            controller
                .drawing()
                .unwrap()
                .entity_render_attributes(EntityId(entity))
                .color,
            cad_db::EntityColor::Explicit([255, 0, 0])
        );
    }
    assert_eq!(controller.selection().len(), 2);
}

#[test]
fn selection_properties_report_vendor_derived_rows() {
    let mut controller = demo();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "TCH_COLUMN".into(),
                revision: Revision(0),
                source_handle: Some("2C".into()),
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Opaque {
                type_key: "TCH_COLUMN".into(),
                version: 0,
                payload: Vec::new(),
            },
            draw_order: 0,
        })
        .unwrap();
    controller
        .application
        .workspace
        .documents
        .get_mut(&controller.document_id)
        .unwrap()
        .drawing = Arc::new(builder.finish().unwrap());
    controller
        .application
        .set_vendor_representations(Some(Arc::new(
            crate::vendor::VendorRepresentationStore::new(
                5,
                vec![crate::vendor::VendorEntityPrimitives {
                    handle: 0x2C,
                    primitives: vec![crate::vendor::VendorPrimitive::Lines(vec![
                        cad_domain::Point3 {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        },
                        cad_domain::Point3 {
                            x: 1.0,
                            y: 0.0,
                            z: 0.0,
                        },
                    ])],
                }],
            ),
        )));

    controller
        .set_selection(vec![ref_for(&controller, 1)])
        .expect("select the vendor entity");
    let properties = controller.selection_properties().expect("properties");
    let value = |key: &str| {
        properties
            .rows
            .iter()
            .find(|row| row.key == key)
            .and_then(|row| row.value.clone())
    };
    assert_eq!(value("vendor_derived").as_deref(), Some("true"));
    assert_eq!(value("vendor_primitives").as_deref(), Some("1"));
    assert_eq!(value("vendor_revision").as_deref(), Some("5"));
}
