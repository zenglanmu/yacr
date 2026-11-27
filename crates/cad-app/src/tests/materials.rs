//! `PAINT` / `MATERIALS` command contracts.
//!
//! The paint is a document-table write: one revision, one undo step, the entity
//! rows untouched. These tests pin the built-in/din-document resolution, the
//! named refusals, the report's read-only nature and the undo/redo round trip.

use super::*;
use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

fn app_with(
    entities: Vec<DbEntity>,
    block: Option<BlockDefinition>,
) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    if let Some(block) = block {
        builder.insert_block(block).unwrap();
    }
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let (app, session, _) = crate::tests::inquiry::app_from_builder(builder);
    (app, session)
}

fn ent(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: order,
    }
}

fn ref_of(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn select(app: &mut Application, session: &mut SessionState, entities: &[u128]) {
    app.execute(
        session,
        command(
            CommandId::Select,
            CommandPayload::Selection(entities.iter().map(|id| ref_of(*id)).collect()),
        ),
    )
    .expect("selection");
}

fn paint(
    app: &mut Application,
    session: &mut SessionState,
    name: &str,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(
            CommandId::Extended(ExtendedCommandId::Paint),
            CommandPayload::PaintMaterial(name.into()),
        ),
    )
}

fn list(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(
            CommandId::Extended(ExtendedCommandId::Materials),
            CommandPayload::None,
        ),
    )
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn materials(app: &Application) -> cad_domain::material::MaterialDocument {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .materials()
        .clone()
}

fn refusal_text(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) | CadError::Unsupported(text) => text,
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

fn line(id: u128) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Line {
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 2.0,
                y: 0.0,
                z: 0.0,
            },
        },
        id as i64,
    )
}

#[test]
fn painting_a_builtin_creates_the_row_and_assigns_the_selection() {
    let (mut app, mut session) = app_with(vec![line(7)], None);
    select(&mut app, &mut session, &[7]);
    let before = revision(&app);
    let outcome = paint(&mut app, &mut session, "Red").expect("paint");
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");
    let document = materials(&app);
    let row = document.resolve("Red").expect("the built-in was copied in");
    assert_eq!(row.color, [178, 34, 34]);
    assert_eq!(row.transparency, 0.0);
    assert_eq!(
        document
            .assignment(EntityId(7))
            .map(|row| row.name.as_str()),
        Some("Red")
    );
    // The entity row itself is untouched (no geometry write).
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(7))
        .is_some());
    let applied = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "paint.applied")
        .expect("the success is named");
    assert!(applied.message.contains("Red"), "{}", applied.message);
    assert!(applied.message.contains("#b22222"), "{}", applied.message);
}

#[test]
fn undo_and_redo_round_trip_the_material_document() {
    let (mut app, mut session) = app_with(vec![line(7)], None);
    select(&mut app, &mut session, &[7]);
    paint(&mut app, &mut session, "Glass").expect("paint");
    assert_eq!(materials(&app).len(), 1);
    let painted = materials(&app);

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo");
    assert!(
        materials(&app).is_empty(),
        "undo removes the material table row"
    );
    assert!(app.can_redo(&DocumentId(1)));

    app.execute(&mut session, command(CommandId::Redo, CommandPayload::None))
        .expect("redo");
    let redone = materials(&app);
    assert_eq!(redone, painted);
    assert!(redone.resolve("Glass").is_some());
    assert!(redone.assignment(EntityId(7)).is_some());
}

#[test]
fn painting_the_same_material_twice_is_refused_as_no_change() {
    let (mut app, mut session) = app_with(vec![line(7)], None);
    select(&mut app, &mut session, &[7]);
    paint(&mut app, &mut session, "Blue").expect("the first paint");
    let before = revision(&app);
    let depth = app.history[&DocumentId(1)].undo_depth();
    let text = refusal_text(paint(&mut app, &mut session, "blue").unwrap_err());
    assert!(text.starts_with("paint.no_change:"), "{text}");
    assert_eq!(revision(&app), before, "nothing is written");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), depth);
}

#[test]
fn an_unknown_material_name_is_refused_by_name() {
    let (mut app, mut session) = app_with(vec![line(7)], None);
    select(&mut app, &mut session, &[7]);
    let before = revision(&app);
    let text = refusal_text(paint(&mut app, &mut session, "Chartreuse").unwrap_err());
    assert!(text.starts_with("paint.unknown_material:"), "{text}");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn an_empty_selection_or_a_sub_element_ref_is_refused_by_name() {
    let (mut app, mut session) = app_with(vec![line(7)], None);
    let text = refusal_text(paint(&mut app, &mut session, "Red").unwrap_err());
    assert!(text.starts_with("paint.selection_required:"), "{text}");

    session.selection.replace([SelectionRef {
        sub_element: Some(SubElementId {
            source_key: "face-0".into(),
            topology_revision: Revision(0),
        }),
        ..ref_of(7)
    }]);
    let text = refusal_text(paint(&mut app, &mut session, "Red").unwrap_err());
    assert!(text.starts_with("paint.not_addressable:"), "{text}");
}

#[test]
fn a_block_reference_is_refused_instead_of_painting_nothing() {
    let block = BlockDefinition {
        id: BlockId(1),
        entities: Vec::new(),
        dynamic_visibility: None,
    };
    let insert = ent(
        9,
        SemanticGeometry::Insert {
            block: BlockId(1),
            transform: Transform3::identity(),
            attributes: Vec::new(),
        },
        9,
    );
    let (mut app, mut session) = app_with(vec![insert], Some(block));
    select(&mut app, &mut session, &[9]);
    let text = refusal_text(paint(&mut app, &mut session, "Red").unwrap_err());
    assert!(text.starts_with("paint.insert_not_paintable:"), "{text}");
}

#[test]
fn materials_reports_the_document_and_the_palette_without_writing() {
    let (mut app, mut session) = app_with(vec![line(7)], None);
    select(&mut app, &mut session, &[7]);
    paint(&mut app, &mut session, "Wood").expect("paint");
    let before = revision(&app);
    let depth = app.history[&DocumentId(1)].undo_depth();
    let outcome = list(&mut app, &mut session).expect("the report runs");
    assert_eq!(revision(&app), before, "the report never writes");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), depth);
    let report = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "materials.listed")
        .expect("the report is named");
    assert!(report.message.contains("Wood"), "{}", report.message);
    assert!(report.message.contains("Glass"), "{}", report.message);
}

#[test]
fn a_document_material_wins_over_a_builtin_of_the_same_name() {
    // Build a document whose `Red` row is a different colour: paint must use the
    // document row, not the built-in.
    let (mut app, mut session) = app_with(vec![line(7)], None);
    {
        let mut materials = cad_domain::material::MaterialDocument::new();
        materials.ensure("Red", [1, 2, 3], 0.0).unwrap();
        let document = app.workspace.documents.get_mut(&DocumentId(1)).unwrap();
        std::sync::Arc::make_mut(&mut document.drawing)
            .apply_material_document(materials, TransactionId(99), "fixture")
            .unwrap();
    }
    select(&mut app, &mut session, &[7]);
    paint(&mut app, &mut session, "red").expect("paint");
    let document = materials(&app);
    let row = document.resolve("Red").expect("the document row");
    assert_eq!(row.color, [1, 2, 3], "the document row wins");
}
