//! The BIM query/summary session bridge (ADR-0042 D11).
//!
//! What must hold: the wire filter reaches the backend unchanged, the answer is
//! the camelCase page bound to the database revision, paging is the `QueryPage`
//! contract (`limit = 0` = total only), a summary counts components and facets,
//! and a refusal (unknown filter value kind) is named instead of an empty page.

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
        .bim_summary(&request(
            BimFilterWire {
                levels: vec!["Level 1".to_string()],
                ..BimFilterWire::default()
            },
            0,
            10,
        ))
        .expect("the session answers");
    assert_eq!(by_name.total, 2);
    let by_id = session
        .bim_summary(&request(
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
}

#[test]
fn the_summary_counts_components_and_facets() {
    let session = bim_session();
    let response = session
        .bim_summary(&request(BimFilterWire::default(), 0, 100))
        .expect("the session answers");
    assert_eq!(response.total, 2);
    assert_eq!(response.summary.total, 2);
    assert_eq!(response.summary.by_category[0].name, "IfcWall");
    assert_eq!(response.summary.by_category[0].count, 2);
    assert_eq!(response.summary.by_level[0].name, "Level 1");
    assert_eq!(response.database_id, 5);
    assert_eq!(response.revision, 9);
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

    let summary = session
        .bim_summary(&request(BimFilterWire::default(), 0, 10))
        .expect("no document is not an error");
    assert_eq!(summary.total, 0);
    assert_eq!(summary.summary.total, 0);
}

#[test]
fn the_evidence_filter_is_carried() {
    let session = bim_session();
    let heuristic = session
        .bim_summary(&request(
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
    let declared = session
        .bim_summary(&request(
            BimFilterWire {
                evidence: Some(BimEvidenceWire::Declared),
                ..BimFilterWire::default()
            },
            0,
            10,
        ))
        .expect("the session answers");
    assert_eq!(declared.total, 2);
}
