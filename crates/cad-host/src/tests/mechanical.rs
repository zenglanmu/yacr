//! The mechanical FeatureManager tree read (ADR-0053 mechanical stage).
//!
//! These contracts pin the projection, not a renderer: features come first in
//! evaluation order with the honest `schema_only` status (this build has the
//! schema half of ADR-0043 P2 only), bodies follow by entity id with `ready` /
//! `unavailable` reflecting the exact B-rep, and a host with no document answers
//! an explicit empty tree.

use super::*;

use cad_db::{
    DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, FeatureDefinition,
    FeatureDefinitions, FeatureOperation, Layer,
};
use cad_domain::param::ParamBindings;
use cad_domain::{
    Body3d, BodyDisplay, BodyExact, BodyKind, Completeness, DatabaseId, EntityId, LayerId,
    ObjectId, Precision, Revision, SemanticGeometry, SpaceId,
};

fn body_entity(id: u128, exact: bool) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDb3dSolid".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Body3d(Box::new(Body3d {
            kind: BodyKind::Solid,
            exact: if exact {
                BodyExact::Exact(Box::default())
            } else {
                BodyExact::Unavailable {
                    reason: "fixture has no exact B-rep".into(),
                }
            },
            display: BodyDisplay {
                mesh: cad_domain::Mesh {
                    vertices: Vec::new(),
                    triangles: Vec::new(),
                    normals: Vec::new(),
                    face_sources: Vec::new(),
                    colors: Vec::new(),
                    uvs: Vec::new(),
                    texture: None,
                    pbr: None,
                },
                edges: Vec::new(),
                tolerance: None,
                precision: Precision::Analytic,
                completeness: Completeness::Complete,
            },
        })),
        draw_order: id as i64,
    }
}

fn feature(id: u128, order: u32, operation: FeatureOperation) -> FeatureDefinition {
    FeatureDefinition {
        id: EntityId(id),
        order,
        operation,
        inputs: Vec::new(),
        parameters: ParamBindings::new(),
    }
}

fn tree_core() -> HostCore {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(9));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("layer");
    builder.insert_entity(body_entity(5, true)).expect("body 5");
    builder
        .insert_entity(body_entity(6, false))
        .expect("body 6");
    let mut features = FeatureDefinitions::new();
    features
        .insert(feature(6, 1, FeatureOperation::Extrude))
        .expect("feature 8");
    features
        .insert(feature(5, 0, FeatureOperation::Revolve))
        .expect("feature 7");
    builder.set_features(features).expect("features");
    let database: DrawingDatabase = builder.finish().expect("mechanical database");
    let core = core();
    {
        let mut controller = core.controller.borrow_mut();
        let document = controller.document_id;
        controller
            .application
            .workspace
            .documents
            .get_mut(&document)
            .expect("the host always has a document")
            .drawing = Arc::new(database);
    }
    core
}

fn no_document_core() -> HostCore {
    let platform = HostPlatform {
        files: Rc::new(FakeFiles::default()),
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    let mut options = options();
    options.document_at_start = StartDocument::Empty;
    HostCore::new(options, platform, RenderSessionHandle::default()).expect("empty host core")
}

#[test]
fn features_come_first_in_evaluation_order_and_are_schema_only() {
    let tree = tree_core().mechanical_tree();
    assert_eq!(tree.total, 4, "two features plus two bodies");
    assert!(!tree.truncated);
    let rows = &tree.rows;
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows[..2]
            .iter()
            .map(|row| (row.kind, row.id, row.order))
            .collect::<Vec<_>>(),
        vec![("feature", 5, Some(0)), ("feature", 6, Some(1))],
        "features are ordered by `order`, not by entity id"
    );
    assert_eq!(rows[0].token, "revolve");
    assert_eq!(rows[1].token, "extrude");
    for row in &rows[..2] {
        assert_eq!(
            row.status, "schema_only",
            "P2 has no evaluator; a feature must not read as geometry"
        );
        assert_eq!(row.inputs, 0);
        assert_eq!(row.parameters, 0);
    }
}

#[test]
fn bodies_follow_by_id_and_report_their_exactness() {
    let tree = tree_core().mechanical_tree();
    let bodies: Vec<_> = tree.rows.iter().filter(|row| row.kind == "body").collect();
    assert_eq!(bodies.len(), 2);
    assert_eq!((bodies[0].id, bodies[0].status), (5, "ready"));
    assert_eq!((bodies[1].id, bodies[1].status), (6, "unavailable"));
    for body in bodies {
        assert_eq!(body.token, "solid");
        assert_eq!(body.order, None);
    }
}

#[test]
fn no_document_is_an_explicit_empty_tree() {
    let tree = no_document_core().mechanical_tree();
    assert_eq!(tree.revision, 0);
    assert!(tree.rows.is_empty());
    assert_eq!(tree.total, 0);
    assert!(!tree.truncated);
}
