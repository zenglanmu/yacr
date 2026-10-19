//! EXPLODE self-referential-block contract test (ADR 0016 decision G).
//!
//! Kept in its own module because it pins the specific claim that justified
//! deleting the export layer's block-reference cycle stack and depth cap: the
//! one-level INSERT walk never descends into a block definition, so a block that
//! references itself explodes once and yields the self-reference again.

use super::*;

use cad_db::{BlockDefinition, DbEntity, DbObject};

/// A `DbEntity` with an explicit layer, space and type key.
fn part(
    id: u128,
    geometry: SemanticGeometry,
    layer: LayerId,
    space: SpaceId,
    type_key: &str,
    order: i64,
) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer,
        space,
        geometry,
        draw_order: order,
    }
}

fn insert(block: u128, transform: Transform3) -> SemanticGeometry {
    SemanticGeometry::Insert {
        block: cad_domain::BlockId(block),
        transform,
    }
}

/// Wrap a finished builder into a Work-mode app/session pair.
fn app_from(drawing: cad_db::DrawingDatabase) -> (Application, SessionState) {
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([22u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

fn select_one(app: &mut Application, session: &mut SessionState, entity: u128) {
    execute(
        app,
        session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(entity)]),
    )
    .unwrap();
}

fn explode(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::ExplodeEntities,
        CommandPayload::None,
    )
}

fn model_geometry(app: &Application) -> Vec<(EntityId, SemanticGeometry)> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .map(|entity| (entity.id, entity.geometry.clone()))
        .collect()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

/// Pins the claim that justified deleting the export layer's block-reference
/// cycle stack and depth cap (ADR 0016 decision G): the one-level INSERT walk
/// never descends into a block definition, so a block that references itself
/// explodes **once** — no recursion, no error — and yields the self-reference
/// again with the placement composed in. Exploding the produced reference simply
/// repeats this finitely; it never loops and never expands to leaves.
#[test]
fn explode_self_referential_block_yields_the_reference_again() {
    // Block 1's only member is an INSERT of block 1 at (0,5); the model entity 1
    // is an INSERT of block 1 at (10,0).
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(1),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(part(
        100,
        insert(1, Transform3::translation(p3(0.0, 5.0, 0.0))),
        LayerId(0),
        SpaceId::Block(cad_domain::BlockId(1)),
        "AcDbBlockReference",
        0,
    ))
    .unwrap();
    b.insert_entity(part(
        1,
        insert(1, Transform3::translation(p3(10.0, 0.0, 0.0))),
        LayerId(0),
        SpaceId::Model,
        "AcDbBlockReference",
        1,
    ))
    .unwrap();
    let (mut app, mut session) = app_from(b.finish().unwrap());

    select_one(&mut app, &mut session, 1);
    let outcome = explode(&mut app, &mut session)
        .expect("a self-reference explodes once, without recursion or an error");
    assert_eq!(
        outcome.objects.len(),
        1,
        "exactly the self-reference is produced"
    );
    let produced = model_geometry(&app);
    assert_eq!(produced.len(), 1, "no expansion to leaf geometry");
    let reference_id = produced[0].0;
    match &produced[0].1 {
        SemanticGeometry::Insert { block, transform } => {
            assert_eq!(*block, cad_domain::BlockId(1), "still a block-1 reference");
            // The composed transform (10,0)∘(0,5) = (10,5), with unit scale.
            assert!(
                (transform.matrix[0][3] - 10.0).abs() < 1e-9,
                "{transform:?}"
            );
            assert!((transform.matrix[1][3] - 5.0).abs() < 1e-9, "{transform:?}");
            assert!((transform.matrix[0][0] - 1.0).abs() < 1e-9, "{transform:?}");
            assert!((transform.matrix[1][1] - 1.0).abs() < 1e-9, "{transform:?}");
        }
        other => panic!("expected the self-reference INSERT, got {other:?}"),
    }
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");

    // Explode the produced reference again: still one self-reference, now at
    // (10,10). The walk repeats finitely and never reaches leaf geometry.
    session.selection = SelectionSet::from_refs(vec![SelectionRef {
        document: DocumentId(1),
        entity: reference_id,
        instance: InstancePath::default(),
        sub_element: None,
    }]);
    explode(&mut app, &mut session).expect("the produced self-reference explodes once again");
    let produced = model_geometry(&app);
    assert_eq!(produced.len(), 1, "no recursion, no expansion to leaves");
    match &produced[0].1 {
        SemanticGeometry::Insert { block, transform } => {
            assert_eq!(*block, cad_domain::BlockId(1));
            assert!(
                (transform.matrix[0][3] - 10.0).abs() < 1e-9,
                "{transform:?}"
            );
            assert!(
                (transform.matrix[1][3] - 10.0).abs() < 1e-9,
                "{transform:?}"
            );
        }
        other => panic!("expected the self-reference INSERT, got {other:?}"),
    }
}
