//! The BIM query session bridge (ADR-0042 D11).
//!
//! What must hold: the wire filter reaches the backend unchanged, the answer is
//! the camelCase page bound to the database revision, the summary of the same
//! filtered set rides that one answer (`summary.total == total`), paging is the
//! `QueryPage` contract (`limit = 0` = total only), and a refusal (unknown filter
//! value kind) is named instead of an empty page.

use cad_app::{CommandId, CommandPayload};
use cad_db::{
    BimDefinition, BimDefinitions, BimElement, BimEvidence, BimProperty, BimPropertySet, BimSource,
    BimValue, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer,
};
use cad_domain::{
    DatabaseId, EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry, SpaceId,
};

use super::super::*;
use super::test_support::*;
use crate::bim::{
    BimEvidenceWire, BimFilterWire, BimOpWire, BimPropertyFilterWire, BimQueryRequest,
    BimValueWire, BIM_UNSUPPORTED_VALUE,
};
use crate::host_read::{HostReadAnswer, HostReadRequest, BIM_QUERY_KIND};

const LEVEL: &str = "level:ifc:doc:L1";

fn line(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".to_string(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
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
        draw_order: 0,
    }
}

fn source(id: &str) -> BimSource {
    BimSource {
        connector: "ifc".to_string(),
        document_id: Some("doc".to_string()),
        id: Some(id.to_string()),
        native_id: None,
    }
}

/// Two `IfcWall` components on one declared storey, each with one boolean
/// `Pset_WallCommon` property.
fn bim_database() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(5));
    builder.set_revision(Revision(9));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("layer");
    for (id, name) in [(1_u128, "Wall A"), (2, "Wall B")] {
        builder.insert_entity(line(id)).expect("entity");
        builder
            .set_entity_bim(
                EntityId(id),
                BimElement {
                    category: Some("IfcWall".to_string()),
                    name: Some(name.to_string()),
                    source: source(&format!("wall-{id}")),
                    level: Some(LEVEL.to_string()),
                    element_type: None,
                    material: None,
                    psets: vec![BimPropertySet {
                        name: "Pset_WallCommon".to_string(),
                        properties: vec![BimProperty {
                            name: "IsExternal".to_string(),
                            value: BimValue::Bool(true),
                        }],
                    }],
                    evidence: BimEvidence::Declared,
                },
            )
            .expect("bim");
    }
    let mut definitions = BimDefinitions::new();
    definitions
        .insert(BimDefinition::Level {
            id: LEVEL.to_string(),
            name: "Level 1".to_string(),
            elevation: Some(0.0),
            source: source("storey"),
            evidence: BimEvidence::Declared,
        })
        .expect("level");
    builder
        .set_bim_definitions(definitions)
        .expect("definitions");
    builder.finish().expect("database")
}

/// A session whose open document is the BIM fixture (installed directly, as the
/// other session fixtures do; the query only reads the drawing).
fn bim_session() -> CadSession {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    {
        let mut controller = session.core.controller.borrow_mut();
        let document = controller.document_id;
        controller
            .application
            .workspace
            .documents
            .get_mut(&document)
            .expect("the session's document")
            .drawing = Arc::new(bim_database());
    }
    session
}

fn request(filter: BimFilterWire, offset: usize, limit: usize) -> BimQueryRequest {
    BimQueryRequest::new(filter, offset, limit)
}

fn walls() -> BimFilterWire {
    BimFilterWire {
        categories: vec!["IfcWall".to_string()],
        ..BimFilterWire::default()
    }
}

#[test]
fn a_category_filter_answers_the_wire_page() {
    let session = bim_session();
    let response = session
        .bim_query(&request(walls(), 0, 10))
        .expect("the session answers");
    assert_eq!(response.total, 2);
    assert_eq!(response.database_id, 5);
    assert_eq!(response.revision, 9);
    assert!(!response.stale);
    assert_eq!(response.rows.len(), 2);
    assert_eq!(response.rows[0].level_name.as_deref(), Some("Level 1"));
    assert_eq!(response.rows[0].category.as_deref(), Some("IfcWall"));

    let value = serde_json::to_value(&response).expect("json");
    assert_eq!(value["databaseId"], 5);
    assert_eq!(value["rows"][0]["object"]["kind"], "source");
    assert_eq!(
        value["rows"][0]["propertySummary"][0]["name"],
        "Pset_WallCommon"
    );
}

#[test]
fn an_unknown_category_selects_nothing_without_an_error() {
    let session = bim_session();
    let response = session
        .bim_query(&request(
            BimFilterWire {
                categories: vec!["IfcDoor".to_string()],
                ..BimFilterWire::default()
            },
            0,
            10,
        ))
        .expect("an unmatched category is an empty page, not an error");
    assert_eq!(response.total, 0);
    assert!(response.rows.is_empty());
    assert_eq!(response.database_id, 5, "the page is still revision-bound");
}

#[test]
fn a_level_filter_resolves_the_display_name() {
    let session = bim_session();
    let by_name = session
        .bim_query(&request(
            BimFilterWire {
                levels: vec!["Level 1".to_string()],
                ..BimFilterWire::default()
            },
            0,
            10,
        ))
        .expect("the session answers");
    assert_eq!(by_name.total, 2);
    assert_eq!(by_name.summary.total, 2);
    let by_id = session
        .bim_query(&request(
            BimFilterWire {
                levels: vec![LEVEL.to_string()],
                ..BimFilterWire::default()
            },
            0,
            10,
        ))
        .expect("the session answers");
    assert_eq!(by_id.total, 2, "the definition id is accepted too");
}

#[test]
fn a_property_predicate_selects_the_declaring_elements() {
    let session = bim_session();
    let response = session
        .bim_query(&request(
            BimFilterWire {
                property: Some(BimPropertyFilterWire {
                    pset: "Pset_WallCommon".to_string(),
                    property: "IsExternal".to_string(),
                    op: BimOpWire::Eq,
                    value: BimValueWire::Bool { value: true },
                }),
                ..BimFilterWire::default()
            },
            0,
            10,
        ))
        .expect("the session answers");
    assert_eq!(response.total, 2);
    let absent = session
        .bim_query(&request(
            BimFilterWire {
                property: Some(BimPropertyFilterWire {
                    pset: "Pset_WallCommon".to_string(),
                    property: "IsCurtainWall".to_string(),
                    op: BimOpWire::Ne,
                    value: BimValueWire::Bool { value: false },
                }),
                ..BimFilterWire::default()
            },
            0,
            10,
        ))
        .expect("the session answers");
    assert_eq!(
        absent.total, 0,
        "a property an element does not declare cannot satisfy Ne"
    );
}

#[test]
fn paging_is_the_query_page_contract() {
    let session = bim_session();
    let page = session
        .bim_query(&request(BimFilterWire::default(), 1, 1))
        .expect("the session answers");
    assert_eq!(page.total, 2, "total is every match, not the page");
    assert_eq!(page.rows.len(), 1);
    assert_eq!((page.offset, page.limit), (1, 1));

    let total_only = session
        .bim_query(&request(BimFilterWire::default(), 0, 0))
        .expect("the session answers");
    assert_eq!(total_only.total, 2);
    assert!(
        total_only.rows.is_empty(),
        "limit=0 requests only the total"
    );
    assert_eq!(
        total_only.summary.total, 2,
        "the summary is one aggregate: limit=0 still answers the counts"
    );
    assert_eq!(total_only.summary.by_category[0].count, 2);
}

#[test]
fn the_summary_rides_the_query_and_counts_components_and_facets() {
    let session = bim_session();
    let response = session
        .bim_query(&request(BimFilterWire::default(), 0, 100))
        .expect("the session answers");
    assert_eq!(response.total, 2);
    assert_eq!(
        response.summary.total, response.total,
        "one answer, one filtered set"
    );
    assert_eq!(response.summary.by_category[0].name, "IfcWall");
    assert_eq!(response.summary.by_category[0].count, 2);
    assert_eq!(response.summary.by_level[0].name, "Level 1");
    assert_eq!(response.database_id, 5);
    assert_eq!(response.revision, 9);

    // The facet counts follow the filter: a filtered answer's facets are the
    // matched set's own, and its summary total is the matched total.
    let walls = session
        .bim_query(&request(walls(), 0, 100))
        .expect("the session answers");
    assert_eq!(walls.summary.total, walls.total);
    assert_eq!(walls.summary.by_category.len(), 1);
    assert_eq!(walls.summary.by_category[0].count, 2);
    let doors = session
        .bim_query(&request(
            BimFilterWire {
                categories: vec!["IfcDoor".to_string()],
                ..BimFilterWire::default()
            },
            0,
            100,
        ))
        .expect("an unmatched category is an empty page, not an error");
    assert_eq!(doors.total, 0);
    assert_eq!(doors.summary.total, 0);
    assert!(doors.summary.by_category.is_empty());
}

#[test]
fn an_unknown_value_kind_is_refused_before_the_backend_runs() {
    let session = bim_session();
    let error = session
        .bim_query(&request(
            BimFilterWire {
                property: Some(BimPropertyFilterWire {
                    pset: "Pset_WallCommon".to_string(),
                    property: "IsExternal".to_string(),
                    op: BimOpWire::Eq,
                    value: BimValueWire::Unsupported {
                        token: "date".to_string(),
                    },
                }),
                ..BimFilterWire::default()
            },
            0,
            10,
        ))
        .expect_err("an unknown kind is not a query");
    assert!(error.starts_with(BIM_UNSUPPORTED_VALUE), "{error}");
}

#[test]
fn no_document_is_an_empty_success() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    session
        .core
        .command(CommandId::CloseDrawing, CommandPayload::None)
        .expect("the demo document closes");
    assert!(
        session.core.controller.borrow().drawing().is_none(),
        "the fixture really has no document"
    );

    let response = session
        .bim_query(&request(BimFilterWire::default(), 0, 10))
        .expect("no document is not an error");
    assert_eq!(response.total, 0);
    assert!(response.rows.is_empty());
    assert_eq!(response.database_id, 0);
    assert_eq!(response.revision, 0);
    assert_eq!(response.summary.total, 0, "no document answers no counts");
    assert!(response.summary.by_category.is_empty());
}

#[test]
fn the_evidence_filter_is_carried() {
    let session = bim_session();
    let heuristic = session
        .bim_query(&request(
            BimFilterWire {
                evidence: Some(BimEvidenceWire::Heuristic),
                ..BimFilterWire::default()
            },
            0,
            10,
        ))
        .expect("the session answers");
    assert_eq!(
        heuristic.total, 0,
        "the fixture's rows are declared, not heuristic"
    );
    assert_eq!(heuristic.summary.total, 0);
    let declared = session
        .bim_query(&request(
            BimFilterWire {
                evidence: Some(BimEvidenceWire::Declared),
                ..BimFilterWire::default()
            },
            0,
            10,
        ))
        .expect("the session answers");
    assert_eq!(declared.total, 2);
    assert_eq!(declared.summary.total, 2);
}

#[test]
fn the_generic_host_read_answers_the_same_page_as_the_session_entry() {
    // The wire row now goes through `HostReadRequest`/`HostReadAnswer`; the
    // session's direct entry must still be the same answer, character for
    // character, or the refactor changed behaviour.
    let session = bim_session();
    let direct = session
        .bim_query(&request(walls(), 0, 10))
        .expect("the session answers");
    let answer = session
        .host_read(&HostReadRequest::BimQuery {
            filter: walls(),
            offset: 0,
            limit: 10,
        })
        .expect("the generic entry answers");
    assert_eq!(answer, HostReadAnswer::BimQuery(direct.clone()));
    // The envelope's tag is the documented kind and the inner value is the
    // unchanged response (the wire row unwraps it).
    let value = serde_json::to_value(&answer).expect("serialize");
    assert_eq!(value["kind"], BIM_QUERY_KIND);
    let inner = serde_json::to_value(&direct).expect("serialize");
    for key in [
        "databaseId",
        "revision",
        "total",
        "offset",
        "limit",
        "rows",
        "stale",
        "summary",
    ] {
        assert_eq!(value[key], inner[key], "field {key}");
    }
}

#[test]
fn the_query_row_refuses_by_name_without_a_host_read_seam() {
    // The desktop transport has no session on the IPC thread: without an
    // installed seam the shared layer answers by marker, never an empty page.
    let reads = crate::session::wire::QueueReads::default();
    let error = crate::session::host_read::bim_query(&reads, BimFilterWire::default(), 0, 10)
        .expect_err("no host-read seam on this host");
    assert_eq!(error, crate::bim::BIM_UNSUPPORTED);
    assert_eq!(error, "unsupported-runtime: bimQuery");
}
