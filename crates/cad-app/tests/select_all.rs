//! Synthetic SELECTALL contracts; not UI, GPU or vendor validation.
use std::sync::Arc;

use cad_app::{
    drawing_pick_items, AppMode, Application, Command, CommandId, CommandPayload, Document,
    SelectionSet, SessionState, ToolState,
};
use cad_db::{
    AnnotationDatabase, BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder,
    DynamicBlockState, DynamicBlockVisibility, Layer,
};
use cad_domain::*;

const DOC: DocumentId = DocumentId(1);
const CHILD: EntityId = EntityId((1_u128 << 100) + 7);
const FIRST: EntityId = EntityId((1_u128 << 110) + 11);
const SECOND: EntityId = EntityId(u128::MAX - 1);
const LEAF_LAYER: LayerId = LayerId(7);
const INSERT_LAYER: LayerId = LayerId((1_u128 << 96) + 7);

fn line() -> SemanticGeometry {
    SemanticGeometry::Line {
        start: Point3 { x: 0.0, y: 0.0, z: 0.0 },
        end: Point3 { x: 1.0, y: 0.0, z: 0.0 },
    }
}

fn entity(id: EntityId, layer: LayerId, space: SpaceId, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id.0),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id,
        layer,
        space,
        geometry,
        draw_order: 0,
    }
}

fn builder() -> DrawingDatabaseBuilder {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, visible) in [(LayerId(0), true), (LEAF_LAYER, false), (INSERT_LAYER, true)] {
        builder.insert_layer(Layer { id, name: format!("layer-{}", id.0), visible }).unwrap();
    }
    builder
}

fn fixture(builder: DrawingDatabaseBuilder, mode: AppMode) -> (Application, SessionState) {
    let mut app = Application::new();
    app.workspace.documents.insert(DOC, Document {
        id: DOC,
        drawing: Arc::new(builder.finish().unwrap()),
        annotations: AnnotationDatabase::new(DatabaseId(2)),
        identity: DocumentIdentity::Temporary(1),
        units: UnitContext::drawing_units(),
        resource_keys: Vec::new(),
    });
    let mut session = SessionState::new(DOC, mode);
    session.selection = SelectionSet::from_refs([SelectionRef {
        document: DOC,
        entity: EntityId(999),
        instance: InstancePath(vec![EntityId(998)]),
        sub_element: Some(SubElementId {
            source_key: "prior-face".into(),
            topology_revision: Revision(17),
        }),
    }]);
    (app, session)
}

fn instances(dynamic_visibility: Option<DynamicBlockVisibility>) -> DrawingDatabaseBuilder {
    let mut builder = builder();
    builder.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: vec![CHILD, EntityId(8)],
        dynamic_visibility,
    }).unwrap();
    builder.insert_entity(entity(CHILD, LayerId(0), SpaceId::Block(BlockId(1)), line())).unwrap();
    builder.insert_entity(entity(EntityId(8), LEAF_LAYER, SpaceId::Block(BlockId(1)), line())).unwrap();
    for id in [FIRST, SECOND] {
        builder.insert_entity(entity(id, INSERT_LAYER, SpaceId::Model, SemanticGeometry::Insert {
            block: BlockId(1),
            transform: Transform3::identity(),
        })).unwrap();
    }
    builder
}

fn command(payload: CommandPayload) -> Command {
    Command { schema_version: 1, id: CommandId::SelectAll, document: DOC, viewport: ViewportId(1), payload }
}

fn refused(app: &mut Application, session: &mut SessionState, command: Command) -> CadError {
    let selection = session.selection.clone();
    let generation = session.generation;
    let error = app.execute(session, command).err().expect("command must be refused");
    assert_eq!(session.selection, selection);
    assert!(matches!(session.tool, ToolState::Idle));
    assert_eq!(session.generation, generation);
    assert!(app.history.is_empty());
    error
}

#[test]
fn model_selection_preserves_exact_full_pick_refs_in_viewer_and_work_without_writes() {
    assert!(!CommandId::SelectAll.requires_work_mode());
    for mode in [AppMode::Viewer, AppMode::Work] {
        let (mut app, mut session) = fixture(instances(None), mode);
        session.layer_overrides.set(LEAF_LAYER, true);
        let drawing = app.workspace.documents[&DOC].drawing.clone();
        let revision = drawing.revision();
        let generation = session.generation;
        let expected = SelectionSet::from_refs(drawing_pick_items(&drawing, DOC).into_iter().map(|item| item.source));
        let outcome = app.execute(&mut session, command(CommandPayload::None)).unwrap();
        assert_eq!(session.selection, expected);
        assert_eq!(session.selection.len(), 4);
        for id in [FIRST, SECOND] {
            assert!(session.selection.contains(&SelectionRef {
                document: DOC, entity: CHILD, instance: InstancePath(vec![id]), sub_element: None,
            }));
        }
        assert!(session.selection.refs().iter().all(|reference| reference.sub_element.is_none()));
        assert!(matches!(session.tool, ToolState::Selecting));
        assert!(outcome.changes.is_none());
        assert!(outcome.diagnostics.is_empty());
        assert!(app.history.is_empty());
        assert_eq!(session.generation, generation);
        assert_eq!(drawing.revision(), revision);
        assert!(Arc::ptr_eq(&drawing, &app.workspace.documents[&DOC].drawing));
    }
}

#[test]
fn hidden_children_are_excluded_and_leaf_override_can_show_them() {
    let (mut app, mut session) = fixture(instances(None), AppMode::Viewer);
    app.execute(&mut session, command(CommandPayload::None)).unwrap();
    assert_eq!(session.selection.len(), 2);
    assert!(session.selection.refs().iter().all(|reference| reference.entity == CHILD));
    session.layer_overrides.set(LEAF_LAYER, true);
    app.execute(&mut session, command(CommandPayload::None)).unwrap();
    assert_eq!(session.selection.len(), 4);
    session.layer_overrides.set(LayerId(0), false);
    app.execute(&mut session, command(CommandPayload::None)).unwrap();
    assert_eq!(session.selection.len(), 2);
    assert!(session.selection.refs().iter().all(|reference| reference.entity == EntityId(8)));
}

#[test]
fn hidden_ancestor_override_excludes_children_even_when_leaf_is_shown() {
    let (mut app, mut session) = fixture(instances(None), AppMode::Work);
    session.layer_overrides.set(LEAF_LAYER, true);
    session.layer_overrides.set(INSERT_LAYER, false);
    app.execute(&mut session, command(CommandPayload::None)).unwrap();
    assert!(session.selection.is_empty());
    session.layer_overrides.set(INSERT_LAYER, true);
    app.execute(&mut session, command(CommandPayload::None)).unwrap();
    assert_eq!(session.selection.len(), 4);
}

#[test]
fn database_hidden_ancestor_excludes_nested_insert_descendants() {
    let mut builder = instances(None);
    builder.insert_block(BlockDefinition {
        id: BlockId(2), entities: vec![EntityId(100)], dynamic_visibility: None,
    }).unwrap();
    builder.insert_entity(entity(EntityId(100), LEAF_LAYER, SpaceId::Block(BlockId(2)), SemanticGeometry::Insert {
        block: BlockId(1), transform: Transform3::identity(),
    })).unwrap();
    builder.insert_entity(entity(EntityId(101), LayerId(0), SpaceId::Model, SemanticGeometry::Insert {
        block: BlockId(2), transform: Transform3::identity(),
    })).unwrap();
    let (mut app, mut session) = fixture(builder, AppMode::Viewer);
    app.execute(&mut session, command(CommandPayload::None)).unwrap();
    assert_eq!(session.selection.len(), 2);
    assert!(session.selection.refs().iter().all(|reference| !reference.instance.0.contains(&EntityId(101))));
    session.layer_overrides.set(LEAF_LAYER, true);
    app.execute(&mut session, command(CommandPayload::None)).unwrap();
    assert!(session.selection.contains(&SelectionRef {
        document: DOC, entity: CHILD, instance: InstancePath(vec![EntityId(101), EntityId(100)]), sub_element: None,
    }));
}

#[test]
fn malformed_payload_preserves_prior_selection() {
    let (mut app, mut session) = fixture(instances(None), AppMode::Viewer);
    assert!(matches!(refused(&mut app, &mut session, command(CommandPayload::Selection(Vec::new()))), CadError::InvalidInput(_)));
}

#[test]
fn stale_document_preserves_prior_selection() {
    let (mut app, mut session) = fixture(instances(None), AppMode::Viewer);
    let mut request = command(CommandPayload::None);
    request.document = DocumentId(2);
    assert!(matches!(refused(&mut app, &mut session, request), CadError::StaleResult));
}

#[test]
fn missing_document_preserves_prior_selection() {
    let (mut app, mut session) = fixture(instances(None), AppMode::Work);
    app.workspace.documents.remove(&DOC);
    assert!(matches!(refused(&mut app, &mut session, command(CommandPayload::None)), CadError::InvalidInput(_)));
}

#[test]
fn nonmodel_spaces_are_unsupported_without_clearing_selection() {
    for space in [SpaceId::Paper(LayoutId(1)), SpaceId::Block(BlockId(1))] {
        let (mut app, mut session) = fixture(instances(None), AppMode::Viewer);
        session.active_space = space;
        assert!(matches!(refused(&mut app, &mut session, command(CommandPayload::None)), CadError::Unsupported(_)));
    }
}

#[test]
fn visible_opaque_geometry_refuses_but_hidden_opaque_is_not_pickable() {
    let mut builder = instances(None);
    builder.insert_entity(entity(EntityId(50), INSERT_LAYER, SpaceId::Model, SemanticGeometry::Opaque {
        type_key: "unknown".into(), version: 1, payload: vec![1],
    })).unwrap();
    let (mut app, mut session) = fixture(builder, AppMode::Viewer);
    assert!(matches!(refused(&mut app, &mut session, command(CommandPayload::None)), CadError::Unsupported(_)));
    session.layer_overrides.set(INSERT_LAYER, false);
    app.execute(&mut session, command(CommandPayload::None)).unwrap();
    assert!(session.selection.is_empty());
}

#[test]
fn cyclic_insert_traversal_refuses_instead_of_publishing_partial_selection() {
    let mut builder = instances(None);
    builder.insert_block(BlockDefinition {
        id: BlockId(2), entities: vec![EntityId(200)], dynamic_visibility: None,
    }).unwrap();
    builder.insert_entity(entity(EntityId(200), LayerId(0), SpaceId::Block(BlockId(2)), SemanticGeometry::Insert {
        block: BlockId(2), transform: Transform3::identity(),
    })).unwrap();
    builder.insert_entity(entity(EntityId(201), LayerId(0), SpaceId::Model, SemanticGeometry::Insert {
        block: BlockId(2), transform: Transform3::identity(),
    })).unwrap();
    let (mut app, mut session) = fixture(builder, AppMode::Viewer);
    assert!(matches!(refused(&mut app, &mut session, command(CommandPayload::None)), CadError::Unsupported(_)));
}

#[test]
fn resolved_dynamic_visibility_excludes_hidden_members() {
    let visibility = DynamicBlockVisibility {
        member_entities: vec![CHILD, EntityId(8)],
        states: vec![DynamicBlockState { name: "leaf".into(), entities: vec![EntityId(8)] }],
        active_state: Some("leaf".into()),
    };
    let (mut app, mut session) = fixture(instances(Some(visibility)), AppMode::Viewer);
    session.layer_overrides.set(LEAF_LAYER, true);
    app.execute(&mut session, command(CommandPayload::None)).unwrap();
    assert_eq!(session.selection.len(), 2);
    assert!(session.selection.refs().iter().all(|reference| reference.entity == EntityId(8)));
}

#[test]
fn unresolved_dynamic_visibility_refuses_without_guessing() {
    let visibility = DynamicBlockVisibility {
        member_entities: vec![CHILD],
        states: vec![DynamicBlockState { name: "child".into(), entities: vec![CHILD] }],
        active_state: None,
    };
    let (mut app, mut session) = fixture(instances(Some(visibility)), AppMode::Viewer);
    assert!(matches!(refused(&mut app, &mut session, command(CommandPayload::None)), CadError::Unsupported(_)));
}

#[test]
fn empty_model_is_an_explicit_successful_empty_selection() {
    let (mut app, mut session) = fixture(builder(), AppMode::Viewer);
    app.execute(&mut session, command(CommandPayload::None)).unwrap();
    assert!(session.selection.is_empty());
    assert!(app.history.is_empty());
}

#[test]
fn depth_limited_insert_traversal_preserves_prior_selection() {
    let mut builder = builder();
    for depth in 0..=cad_db::MAX_INSTANCE_DEPTH {
        let block = BlockId(depth as u128);
        let id = EntityId(depth as u128 + 1);
        builder.insert_block(BlockDefinition {
            id: block, entities: vec![id], dynamic_visibility: None,
        }).unwrap();
        let geometry = if depth == cad_db::MAX_INSTANCE_DEPTH {
            line()
        } else {
            SemanticGeometry::Insert {
                block: BlockId(depth as u128 + 1), transform: Transform3::identity(),
            }
        };
        builder.insert_entity(entity(id, LayerId(0), SpaceId::Block(block), geometry)).unwrap();
    }
    builder.insert_entity(entity(EntityId(1000), LayerId(0), SpaceId::Model, SemanticGeometry::Insert {
        block: BlockId(0), transform: Transform3::identity(),
    })).unwrap();
    let (mut app, mut session) = fixture(builder, AppMode::Viewer);
    assert!(matches!(refused(&mut app, &mut session, command(CommandPayload::None)), CadError::Unsupported(_)));
}

#[test]
fn missing_insert_definition_is_unsupported_not_an_empty_success() {
    let mut builder = builder();
    builder.insert_entity(entity(EntityId(1), LayerId(0), SpaceId::Model, SemanticGeometry::Insert {
        block: BlockId(99), transform: Transform3::identity(),
    })).unwrap();
    let (mut app, mut session) = fixture(builder, AppMode::Viewer);
    assert!(matches!(refused(&mut app, &mut session, command(CommandPayload::None)), CadError::Unsupported(_)));
}

#[test]
fn compound_with_opaque_or_unexpanded_insert_cannot_claim_complete_selection() {
    for unsupported in [
        SemanticGeometry::Opaque { type_key: "unknown".into(), version: 1, payload: vec![] },
        SemanticGeometry::Insert { block: BlockId(1), transform: Transform3::identity() },
    ] {
        let mut builder = instances(None);
        builder.insert_entity(entity(EntityId(50), LayerId(0), SpaceId::Model, SemanticGeometry::Compound(vec![line(), unsupported]))).unwrap();
        let (mut app, mut session) = fixture(builder, AppMode::Viewer);
        assert!(matches!(refused(&mut app, &mut session, command(CommandPayload::None)), CadError::Unsupported(_)));
    }
}

#[test]
fn mismatched_open_document_identity_preserves_selection() {
    let (mut app, mut session) = fixture(instances(None), AppMode::Viewer);
    app.workspace.documents.get_mut(&DOC).unwrap().id = DocumentId(2);
    assert!(matches!(refused(&mut app, &mut session, command(CommandPayload::None)), CadError::StaleResult));
}
