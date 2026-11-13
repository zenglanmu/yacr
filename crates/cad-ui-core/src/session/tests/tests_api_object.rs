//! The session-bound object reads (`database.getObjectId`,
//! `transaction.getObject`): the client shape they answer, and every refusal.

use super::super::*;
use super::test_support::*;

use cad_db::{
    DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, EntityColor as DbEntityColor,
    EntityLineType, EntityRenderAttributes, Layer, LinetypePattern,
};
use cad_domain::{DatabaseId, LayerId, ObjectId, Revision, SpaceId};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

fn point(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// One top-level entity with an optional imported handle.
fn entity(id: u128, handle: Option<&str>, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "TEST".to_string(),
            revision: Revision(0),
            source_handle: handle.map(str::to_string),
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: 0,
    }
}

/// A drawing holding `entities` on layer `0`, with optional render attributes.
fn drawing_with(
    entities: Vec<DbEntity>,
    attributes: &[(EntityId, EntityRenderAttributes)],
) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("the layer inserts");
    for entity in entities {
        builder.insert_entity(entity).expect("the entity inserts");
    }
    for (id, attributes) in attributes {
        builder
            .set_entity_render_attributes(*id, attributes.clone())
            .expect("the attributes attach to a real entity");
    }
    builder.finish().expect("the database builds")
}

/// The entity JSON through the real projection.
fn entity_json(drawing: &DrawingDatabase, id: u128) -> serde_json::Value {
    let entity = drawing.entity(EntityId(id)).expect("the entity exists");
    super::super::api_object::entity_value(drawing, entity).expect("the entity projects")
}

#[test]
fn a_handle_lookup_is_case_insensitive_and_answers_null_when_absent() {
    let drawing = drawing_with(
        vec![
            entity(
                7,
                Some("2F"),
                SemanticGeometry::Line {
                    start: point(0.0, 0.0, 0.0),
                    end: point(1.0, 0.0, 0.0),
                },
            ),
            entity(9, None, SemanticGeometry::Point(point(1.0, 1.0, 0.0))),
        ],
        &[],
    );
    assert_eq!(
        super::super::api_object::find_object_id(&drawing, "2F").as_deref(),
        Some("7")
    );
    assert_eq!(
        super::super::api_object::find_object_id(&drawing, "2f").as_deref(),
        Some("7"),
        "the comparison is case-insensitive"
    );
    assert_eq!(
        super::super::api_object::find_object_id(&drawing, "FF"),
        None,
        "an unknown handle is a real null, never an error"
    );
}

#[test]
fn a_line_reports_exactly_the_client_fields() {
    let drawing = drawing_with(
        vec![entity(
            7,
            Some("2F"),
            SemanticGeometry::Line {
                start: point(0.0, 0.0, 0.0),
                end: point(3.0, 4.0, 0.0),
            },
        )],
        &[],
    );
    assert_eq!(
        entity_json(&drawing, 7),
        serde_json::json!({
            "objectId": "7",
            "handle": "2F",
            "typeName": "Line",
            "layer": "0",
            "color": { "method": "byLayer" },
            "linetype": { "method": "byLayer" },
            "startPoint": [0.0, 0.0, 0.0],
            "endPoint": [3.0, 4.0, 0.0],
            "length": 5.0,
        })
    );
}

#[test]
fn an_entity_without_an_imported_handle_reports_null() {
    let drawing = drawing_with(
        vec![entity(
            8,
            None,
            SemanticGeometry::Line {
                start: point(0.0, 0.0, 0.0),
                end: point(1.0, 0.0, 0.0),
            },
        )],
        &[],
    );
    assert_eq!(
        entity_json(&drawing, 8)["handle"],
        serde_json::Value::Null,
        "a host-created entity has no handle and none is minted"
    );
}

#[test]
fn the_other_decodable_shapes_report_their_own_fields() {
    let drawing = drawing_with(
        vec![
            entity(
                11,
                None,
                SemanticGeometry::Circle {
                    center: point(1.0, 2.0, 0.0),
                    normal: point(0.0, 0.0, 1.0),
                    radius: 2.0,
                },
            ),
            entity(
                12,
                None,
                SemanticGeometry::Arc {
                    center: point(0.0, 0.0, 0.0),
                    normal: point(0.0, 0.0, 1.0),
                    radius: 1.0,
                    start: 0.0,
                    sweep: std::f64::consts::FRAC_PI_2,
                },
            ),
            entity(
                13,
                None,
                SemanticGeometry::Text {
                    text: "hello".to_string(),
                    position: point(5.0, 6.0, 0.0),
                    style: cad_domain::StyleId(0),
                    height: 2.5,
                    rotation: 0.0,
                    font: None,
                    h_align: cad_domain::TextAlignH::Left,
                    v_align: cad_domain::TextAlignV::Baseline,
                    wrap_width: None,
                    width_factor: 1.0,
                },
            ),
        ],
        &[],
    );

    let circle = entity_json(&drawing, 11);
    assert_eq!(circle["typeName"], serde_json::json!("Circle"));
    assert_eq!(circle["center"], serde_json::json!([1.0, 2.0, 0.0]));
    assert_eq!(circle["radius"], serde_json::json!(2.0));
    assert_eq!(
        circle["length"],
        serde_json::json!(std::f64::consts::TAU * 2.0),
        "the circumference"
    );

    let arc = entity_json(&drawing, 12);
    assert_eq!(arc["typeName"], serde_json::json!("Arc"));
    assert_eq!(arc["startAngle"], serde_json::json!(0.0));
    assert_eq!(
        arc["endAngle"],
        serde_json::json!(std::f64::consts::FRAC_PI_2),
        "the sweep's own end, never normalized"
    );
    assert_eq!(
        arc["length"],
        serde_json::json!(std::f64::consts::FRAC_PI_2)
    );

    let text = entity_json(&drawing, 13);
    assert_eq!(text["typeName"], serde_json::json!("DBText"));
    assert_eq!(text["textString"], serde_json::json!("hello"));
    assert_eq!(text["position"], serde_json::json!([5.0, 6.0, 0.0]));
    assert_eq!(text["height"], serde_json::json!(2.5));
    assert_eq!(text["rotation"], serde_json::json!(0.0));
}

#[test]
fn explicit_color_and_linetype_are_the_client_forms() {
    let attributes = EntityRenderAttributes {
        color: DbEntityColor::Explicit([255, 0, 128]),
        linetype: EntityLineType::Explicit {
            name: "DASHED".to_string(),
            pattern: LinetypePattern::continuous(),
            scale: 1.0,
        },
        ..EntityRenderAttributes::default()
    };
    let drawing = drawing_with(
        vec![entity(
            21,
            None,
            SemanticGeometry::Line {
                start: point(0.0, 0.0, 0.0),
                end: point(1.0, 0.0, 0.0),
            },
        )],
        &[(EntityId(21), attributes)],
    );
    let value = entity_json(&drawing, 21);
    assert_eq!(
        value["color"],
        serde_json::json!({ "method": "byColor", "rgb": "#ff0080" })
    );
    assert_eq!(
        value["linetype"],
        serde_json::json!({ "method": "named", "name": "DASHED" })
    );
}

#[test]
fn a_geometry_without_a_client_shape_is_refused_by_name() {
    let drawing = drawing_with(
        vec![entity(
            31,
            None,
            SemanticGeometry::Point(point(0.0, 0.0, 0.0)),
        )],
        &[],
    );
    let entity = drawing.entity(EntityId(31)).expect("the entity exists");
    assert_eq!(
        super::super::api_object::entity_value(&drawing, entity).expect_err("no client shape"),
        "unsupported-runtime: cad_api.DatabaseServices.transaction.getObject: Point"
    );
}

#[test]
fn a_blank_explicit_linetype_name_is_refused_rather_than_guessed() {
    let attributes = EntityRenderAttributes {
        linetype: EntityLineType::Explicit {
            name: "   ".to_string(),
            pattern: LinetypePattern::continuous(),
            scale: 1.0,
        },
        ..EntityRenderAttributes::default()
    };
    let drawing = drawing_with(
        vec![entity(
            32,
            None,
            SemanticGeometry::Line {
                start: point(0.0, 0.0, 0.0),
                end: point(1.0, 0.0, 0.0),
            },
        )],
        &[(EntityId(32), attributes)],
    );
    let entity = drawing.entity(EntityId(32)).expect("the entity exists");
    assert_eq!(
        super::super::api_object::entity_value(&drawing, entity).expect_err("no honest linetype"),
        "unsupported-runtime: cad_api.DatabaseServices.transaction.getObject: linetype"
    );
}

/// A `DatabaseServices` submission through the real session funnel.
fn submit(
    session: &CadSession,
    request_id: &str,
    operation: &str,
    parameters: serde_json::Value,
) -> CadApiOutcome {
    session.api_submit(CadApiSubmission {
        request_id: request_id.to_string(),
        namespace: "DatabaseServices".to_string(),
        operation: operation.to_string(),
        parameters: Some(parameters),
    })
}

fn refusal(outcome: CadApiOutcome) -> String {
    assert!(!outcome.ok, "expected a refusal, got {outcome:?}");
    assert_eq!(outcome.value, None, "a refusal carries no value");
    outcome.error.expect("a refusal marker")
}

/// The demo session's document id and database id.
fn identifiers(session: &CadSession) -> (String, String) {
    let snapshot = session.api_snapshot();
    let document = snapshot
        .document
        .expect("the demo session holds a document");
    (document.id, document.database_id)
}

#[test]
fn the_get_object_id_arguments_are_validated_by_name() {
    let session = fresh_session();
    let (_document, database) = identifiers(&session);

    for (parameters, expected) in [
        (
            serde_json::json!({ "database": "999", "handle": "2F" }),
            "api.invalid_argument: database",
        ),
        (
            serde_json::json!({ "database": database, "handle": "  " }),
            "api.invalid_argument: handle",
        ),
        (
            serde_json::json!({ "database": database, "handle": "XY" }),
            "api.invalid_argument: handle",
        ),
        (
            serde_json::json!({ "database": database }),
            "api.invalid_argument: handle",
        ),
    ] {
        let outcome = submit(&session, "id-1", "database.getObjectId", parameters);
        assert_eq!(refusal(outcome), expected);
    }
}

#[test]
fn a_handle_that_names_no_entity_is_null() {
    let session = fresh_session();
    let (_document, database) = identifiers(&session);
    let outcome = submit(
        &session,
        "id-2",
        "database.getObjectId",
        serde_json::json!({ "database": database, "handle": "FFFF" }),
    );
    assert!(outcome.ok, "{outcome:?}");
    assert_eq!(
        outcome.value.expect("a value"),
        serde_json::json!({ "objectId": null })
    );
}

#[test]
fn the_get_object_arguments_and_targets_are_validated_by_name() {
    let session = fresh_session();
    let (document, _database) = identifiers(&session);
    let valid = serde_json::json!({
        "document": document,
        "transactionId": "tx-1",
        "objectId": "1",
        "mode": "read",
    });

    let mut wrong_document = valid.clone();
    wrong_document["document"] = serde_json::json!("999");
    let mut blank_id = valid.clone();
    blank_id["objectId"] = serde_json::json!(" ");
    let mut bad_id = valid.clone();
    bad_id["objectId"] = serde_json::json!("xyz");
    let mut bad_mode = valid.clone();
    bad_mode["mode"] = serde_json::json!("exclusive");
    let mut blank_transaction = valid.clone();
    blank_transaction["transactionId"] = serde_json::json!("");

    for (parameters, expected) in [
        (wrong_document, "api.invalid_argument: document"),
        (blank_transaction, "api.invalid_argument: transactionId"),
        (blank_id, "api.invalid_argument: objectId"),
        (bad_id, "api.invalid_argument: objectId"),
        (bad_mode, "api.invalid_argument: mode"),
    ] {
        let outcome = submit(&session, "id-3", "transaction.getObject", parameters);
        assert_eq!(refusal(outcome), expected);
    }

    // A well-formed id that names no entity is the named unknown-object refusal.
    let unknown = serde_json::json!({
        "document": document,
        "transactionId": "tx-2",
        "objectId": "999999",
        "mode": "notify",
    });
    assert_eq!(
        refusal(submit(&session, "id-4", "transaction.getObject", unknown)),
        "cad_api.unknown_object: 999999"
    );
}

#[test]
fn a_created_entity_reads_back_as_the_client_object() {
    let session = fresh_session();
    let (document, _database) = identifiers(&session);
    let revision: u64 = session
        .api_snapshot()
        .document
        .expect("a document")
        .database_version
        .parse()
        .expect("a decimal revision");
    let commit = submit(
        &session,
        "id-5",
        "transaction.commit",
        serde_json::json!({
            "document": document,
            "transactionId": "tx-3",
            "expectedRevision": revision,
            "edits": [{
                "typeName": "Line",
                "layer": "0",
                "space": "model",
                "parameters": { "start": [0.0, 0.0, 0.0], "end": [3.0, 4.0, 0.0] },
            }],
        }),
    );
    assert!(commit.ok, "{commit:?}");
    let created = commit.value.expect("a value")["created"][0]
        .as_str()
        .expect("a decimal id")
        .to_string();

    let read = submit(
        &session,
        "id-6",
        "transaction.getObject",
        serde_json::json!({
            "document": document,
            "transactionId": "tx-4",
            "objectId": created,
            "mode": "write",
        }),
    );
    assert!(read.ok, "{read:?}");
    let object = &read.value.expect("a value")["object"];
    assert_eq!(object["typeName"], serde_json::json!("Line"));
    assert_eq!(object["objectId"], serde_json::json!(created));
    assert_eq!(object["handle"], serde_json::Value::Null);
    assert_eq!(object["layer"], serde_json::json!("0"));
    assert_eq!(object["startPoint"], serde_json::json!([0.0, 0.0, 0.0]));
    assert_eq!(object["endPoint"], serde_json::json!([3.0, 4.0, 0.0]));
    assert_eq!(object["length"], serde_json::json!(5.0));
    assert_eq!(object["color"], serde_json::json!({ "method": "byLayer" }));
    assert_eq!(
        object["linetype"],
        serde_json::json!({ "method": "byLayer" })
    );
}

#[test]
fn the_object_reads_without_a_document_are_named() {
    let session = fresh_session();
    let (_document, database) = identifiers(&session);
    session
        .command("CLOSE")
        .expect("closing the demo document is a host command");
    let id_refusal = submit(
        &session,
        "id-7",
        "database.getObjectId",
        serde_json::json!({ "database": database, "handle": "2F" }),
    );
    assert_eq!(refusal(id_refusal), API_CALL_NO_DOCUMENT);
    let object_refusal = submit(
        &session,
        "id-8",
        "transaction.getObject",
        serde_json::json!({
            "document": "1",
            "transactionId": "tx-5",
            "objectId": "1",
            "mode": "read",
        }),
    );
    assert_eq!(refusal(object_refusal), API_CALL_NO_DOCUMENT);
}

#[test]
fn the_synchronous_read_lane_still_refuses_both_operations() {
    let session = fresh_session();
    let snapshot = session.api_snapshot();
    for operation in [API_CALL_GET_OBJECT_ID, API_CALL_GET_OBJECT] {
        let outcome = dispatch_api_call(
            Some(&snapshot),
            "DatabaseServices",
            operation,
            Some(&serde_json::json!({})),
        );
        assert_eq!(
            refusal(outcome),
            format!("unsupported-runtime: cad_api.DatabaseServices.{operation}"),
            "{operation} is a live-session read, not a snapshot projection"
        );
    }
}
