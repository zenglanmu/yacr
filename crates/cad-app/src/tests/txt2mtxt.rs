//! `TXT2MTXT` settings contracts: combine on/off, text ordering, word-wrap
//! (including the first *stored* boundary rule), per-source height preservation
//! through the MTEXT `\H` code and the declared uniform-line-spacing refusal.
//!
//! The base command contract lives in `tests/text_commands.rs`; these tests pin
//! the 2026-11 settings payload and the one-transaction source replacement.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn text_entity(
    id: u128,
    text: &str,
    position: Point3,
    rotation: f64,
    order: i64,
    type_key: &str,
) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(7),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Text {
            text: text.into(),
            position,
            style: StyleId(0),
            height: 2.0,
            rotation,
            font: Some("arial.ttf".into()),
            h_align: TextAlignH::Center,
            v_align: TextAlignV::Middle,
            wrap_width: None,
            width_factor: 1.0,
        },
        draw_order: order,
    }
}

fn app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "NOTES")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([72u8; 32]),
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

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        Command {
            schema_version: 1,
            id,
            document: DocumentId(1),
            viewport: ViewportId(1),
            payload,
        },
    )
}

fn ref_of(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn select(session: &mut SessionState, entities: &[u128]) {
    session
        .selection
        .replace(entities.iter().map(|id| ref_of(*id)).collect::<Vec<_>>());
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn entity(app: &Application, id: u128) -> Option<DbEntity> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .cloned()
}

// --- TXT2MTXT settings -------------------------------------------------------

fn txt2mtext_payload(options: crate::command_payload::Txt2MTextOptions) -> CommandPayload {
    CommandPayload::Annotation(Box::new(AnnotationBatchPayload::Utility(Box::new(
        crate::command_payload::UtilityPayload::Txt2MText(options),
    ))))
}

#[test]
fn txt2mtxt_combine_off_creates_one_mtext_per_source() {
    let a = text_entity(1, "A", p3(0.0, 9.0, 0.0), 0.0, 0, "AcDbText");
    let b = text_entity(2, "B", p3(0.0, 2.0, 0.0), 0.0, 1, "AcDbText");
    let (mut app, mut session) = app_with(vec![a, b]);
    select(&mut session, &[1, 2]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Txt2MText,
        txt2mtext_payload(crate::command_payload::Txt2MTextOptions {
            combine: false,
            order: crate::command_payload::TextOrder::TopDown,
            word_wrap: true,
            force_uniform_line_spacing: false,
        }),
    )
    .expect("one MTEXT per source");
    assert_eq!(outcome.objects.len(), 2);
    assert!(entity(&app, 1).is_none() && entity(&app, 2).is_none());
    for object in &outcome.objects {
        let replacement = entity(&app, object.0).expect("replacement exists");
        assert_eq!(replacement.object.type_key, "AcDbMText");
        assert_eq!(
            match &replacement.geometry {
                SemanticGeometry::Text { position, .. } => *position,
                _ => panic!("expected Text"),
            },
            if replacement.id.0 == outcome.objects[0].0 {
                p3(0.0, 9.0, 0.0)
            } else {
                p3(0.0, 2.0, 0.0)
            }
        );
    }
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(entity(&app, 1).is_some() && entity(&app, 2).is_some());
}

#[test]
fn txt2mtxt_selection_order_and_word_wrap_settings_are_honoured() {
    let a = text_entity(1, "A", p3(0.0, 9.0, 0.0), 0.0, 0, "AcDbText");
    let b = text_entity(2, "B", p3(0.0, 2.0, 0.0), 0.0, 1, "AcDbText");
    let (mut app, mut session) = app_with(vec![a, b]);
    select(&mut session, &[2, 1]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Txt2MText,
        txt2mtext_payload(crate::command_payload::Txt2MTextOptions {
            combine: true,
            order: crate::command_payload::TextOrder::Selection,
            word_wrap: true,
            force_uniform_line_spacing: false,
        }),
    )
    .unwrap();
    let combined = entity(&app, outcome.objects[0].0).unwrap();
    let SemanticGeometry::Text { text, .. } = &combined.geometry else {
        panic!("expected Text");
    };
    assert_eq!(text, "B\\PA", "selection order is kept");

    let mut source = text_entity(3, "wide", p3(0.0, 0.0, 0.0), 0.0, 0, "AcDbMText");
    if let SemanticGeometry::Text { wrap_width, .. } = &mut source.geometry {
        *wrap_width = Some(12.5);
    }
    let (mut app, mut session) = app_with(vec![source]);
    select(&mut session, &[3]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Txt2MText,
        txt2mtext_payload(crate::command_payload::Txt2MTextOptions {
            combine: true,
            order: crate::command_payload::TextOrder::TopDown,
            word_wrap: false,
            force_uniform_line_spacing: false,
        }),
    )
    .unwrap();
    let combined = entity(&app, outcome.objects[0].0).unwrap();
    let SemanticGeometry::Text { wrap_width, .. } = combined.geometry else {
        panic!("expected Text");
    };
    assert_eq!(wrap_width, None, "word-wrap off stores no width");
}

#[test]
fn txt2mtxt_uniform_line_spacing_is_a_named_refusal_without_writing() {
    let (mut app, mut session) = app_with(vec![text_entity(
        1,
        "A",
        p3(0.0, 0.0, 0.0),
        0.0,
        0,
        "AcDbText",
    )]);
    select(&mut session, &[1]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Txt2MText,
        txt2mtext_payload(crate::command_payload::Txt2MTextOptions {
            combine: true,
            order: crate::command_payload::TextOrder::TopDown,
            word_wrap: true,
            force_uniform_line_spacing: true,
        }),
    )
    .expect_err("the line-spacing factor is not modelled");
    assert!(
        matches!(error, CadError::Unsupported(ref message) if message.contains("txt2mtxt.uniform_line_spacing")),
        "{error:?}"
    );
    assert_eq!(revision(&app), before, "the refusal writes nothing");
    assert_eq!(undo_depth(&app), 0);
    assert!(entity(&app, 1).is_some());
}

// --- Content and height preservation ----------------------------------------

/// The stored MTEXT geometry of the one replacement, or a panic.
fn combined_geometry(app: &Application, outcome: &CommandOutcome) -> SemanticGeometry {
    assert_eq!(outcome.objects.len(), 1, "one combined MTEXT");
    entity(app, outcome.objects[0].0)
        .expect("the replacement exists")
        .geometry
}

fn with_height(mut entity: DbEntity, height: f64) -> DbEntity {
    if let SemanticGeometry::Text { height: stored, .. } = &mut entity.geometry {
        *stored = height;
    }
    entity
}

fn with_wrap(mut entity: DbEntity, wrap: Option<f64>) -> DbEntity {
    if let SemanticGeometry::Text { wrap_width, .. } = &mut entity.geometry {
        *wrap_width = wrap;
    }
    entity
}

#[test]
fn txt2mtxt_preserves_a_differing_source_height_as_a_per_run_height_code() {
    // First sorted source (A) is the base height 2; B stores 4, so B's run gets
    // the 2x MTEXT height code around its own unchanged string.
    let a = with_height(
        text_entity(1, "A", p3(0.0, 9.0, 0.0), 0.0, 0, "AcDbText"),
        2.0,
    );
    let b = with_height(
        text_entity(2, "B", p3(0.0, 2.0, 0.0), 0.0, 1, "AcDbText"),
        4.0,
    );
    let (mut app, mut session) = app_with(vec![a, b]);
    select(&mut session, &[1, 2]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Txt2MText,
        CommandPayload::None,
    )
    .expect("combine two heights");

    let SemanticGeometry::Text { text, height, .. } = combined_geometry(&app, &outcome) else {
        panic!("expected Text");
    };
    assert_eq!(text, "A\\P{\\H2x;B}", "content and height both survive");
    assert_eq!(height, 2.0, "the first source stays the base height");
    assert_eq!(
        undo_depth(&app),
        1,
        "the height overrides are part of the same transaction"
    );
}

#[test]
fn txt2mtxt_skips_a_height_override_for_unbalanced_braces_and_names_it() {
    // `B{` cannot be wrapped in a `{...}` group without re-framing the markup
    // that follows, so the raw string is kept and the skip is reported.
    let a = with_height(
        text_entity(1, "A", p3(0.0, 9.0, 0.0), 0.0, 0, "AcDbText"),
        2.0,
    );
    let b = with_height(
        text_entity(2, "B{", p3(0.0, 2.0, 0.0), 0.0, 1, "AcDbText"),
        4.0,
    );
    let (mut app, mut session) = app_with(vec![a, b]);
    select(&mut session, &[1, 2]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Txt2MText,
        CommandPayload::None,
    )
    .expect("combine");

    let SemanticGeometry::Text { text, .. } = combined_geometry(&app, &outcome) else {
        panic!("expected Text");
    };
    assert_eq!(text, "A\\PB{", "the raw string is never rewritten");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "txt2mtxt.height_override_skipped"),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn txt2mtxt_keeps_the_first_stored_wrap_width_when_the_first_source_has_none() {
    // A plain TEXT source has no boundary; the later MTEXT's declared width must
    // not be dropped, and no width is invented when none exists.
    let a = text_entity(1, "A", p3(0.0, 9.0, 0.0), 0.0, 0, "AcDbText");
    let b = with_wrap(
        text_entity(2, "B", p3(0.0, 2.0, 0.0), 0.0, 1, "AcDbMText"),
        Some(9.5),
    );
    let (mut app, mut session) = app_with(vec![a, b]);
    select(&mut session, &[1, 2]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Txt2MText,
        txt2mtext_payload(crate::command_payload::Txt2MTextOptions {
            combine: true,
            order: crate::command_payload::TextOrder::TopDown,
            word_wrap: true,
            force_uniform_line_spacing: false,
        }),
    )
    .expect("combine with word-wrap on");
    let SemanticGeometry::Text { wrap_width, .. } = combined_geometry(&app, &outcome) else {
        panic!("expected Text");
    };
    assert_eq!(wrap_width, Some(9.5), "the stored boundary is kept");

    // No source declares a width -> `None`, never an invented value.
    let (mut app, mut session) = app_with(vec![
        text_entity(1, "A", p3(0.0, 9.0, 0.0), 0.0, 0, "AcDbText"),
        text_entity(2, "B", p3(0.0, 2.0, 0.0), 0.0, 1, "AcDbMText"),
    ]);
    select(&mut session, &[1, 2]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Txt2MText,
        CommandPayload::None,
    )
    .expect("combine without a stored width");
    let SemanticGeometry::Text { wrap_width, .. } = combined_geometry(&app, &outcome) else {
        panic!("expected Text");
    };
    assert_eq!(wrap_width, None);
}

#[test]
fn txt2mtxt_separate_preserves_each_source_height_and_clears_wrap_only_when_off() {
    let a = with_wrap(
        with_height(
            text_entity(1, "A", p3(0.0, 9.0, 0.0), 0.0, 0, "AcDbMText"),
            2.0,
        ),
        Some(5.0),
    );
    let b = with_height(
        text_entity(2, "B", p3(0.0, 2.0, 0.0), 0.0, 1, "AcDbMText"),
        4.0,
    );
    let (mut app, mut session) = app_with(vec![a, b]);
    select(&mut session, &[1, 2]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Txt2MText,
        txt2mtext_payload(crate::command_payload::Txt2MTextOptions {
            combine: false,
            order: crate::command_payload::TextOrder::TopDown,
            word_wrap: false,
            force_uniform_line_spacing: false,
        }),
    )
    .expect("separate");
    assert_eq!(outcome.objects.len(), 2);
    let mut heights = Vec::new();
    for object in &outcome.objects {
        let replacement = entity(&app, object.0).expect("replacement exists");
        let SemanticGeometry::Text {
            height, wrap_width, ..
        } = &replacement.geometry
        else {
            panic!("expected Text");
        };
        heights.push(*height);
        assert_eq!(*wrap_width, None, "word-wrap off clears the boundary");
    }
    heights.sort_by(f64::total_cmp);
    assert_eq!(heights, vec![2.0, 4.0], "each source keeps its own height");
}
