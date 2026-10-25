//! MTEXT creation contract tests (command core only).
//!
//! Pins the load-bearing facts of `CommandId::CreateMText`:
//!
//! * the created entity is stamped **`AcDbMText`** (through the create funnel's
//!   type-key override), not `class_name()`'s `AcDbText`;
//! * the draft's line breaks are stored as `\P` by the shared newline policy, so a
//!   later `TEXTEDIT` of the same entity accepts line breaks;
//! * the payload is used verbatim — `None` width stays `None`, an unusable width
//!   is **refused** (never clamped or dropped), and height/rotation/alignment
//!   survive;
//! * one transaction, one revision bump, one undo step;
//! * every refusal writes nothing (no revision bump, no entity, no undo entry);
//! * single-line `TEXT` now **refuses** a newline instead of storing an
//!   uneditable multi-line `AcDbText`.

use super::*;

const HEIGHT: f64 = 2.5;
const ROTATION: f64 = 0.5;
const WIDTH: f64 = 5.0;

fn mtext_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([11u8; 32]),
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

fn mtext_payload(text: &str) -> CommandPayload {
    CommandPayload::MText {
        position: p3(1.0, 2.0, 0.0),
        width: Some(WIDTH),
        text: text.into(),
        height: HEIGHT,
        rotation: ROTATION,
        h_align: TextAlignH::Center,
        v_align: TextAlignV::Middle,
    }
}

fn text_payload(text: &str) -> CommandPayload {
    CommandPayload::Text {
        position: p3(0.0, 0.0, 0.0),
        text: text.into(),
        height: HEIGHT,
    }
}

/// The only text entity in model space (panics when there is none).
fn stored_text_entity(app: &Application) -> DbEntity {
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    drawing
        .model_space()
        .iter()
        .find(|entity| matches!(entity.geometry, SemanticGeometry::Text { .. }))
        .map(|entity| (*entity).clone())
        .expect("the text entity")
}

fn text_count(app: &Application) -> usize {
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    drawing
        .model_space()
        .iter()
        .filter(|entity| matches!(entity.geometry, SemanticGeometry::Text { .. }))
        .count()
}

fn invalid_input(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) => message,
        other => panic!("expected an InvalidInput refusal, got {other:?}"),
    }
}

#[test]
fn create_mtext_stores_one_acdbmtext_with_paragraph_breaks_in_one_transaction() {
    let (mut app, mut session) = mtext_app();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateMText,
        mtext_payload("a\nb"),
    )
    .expect("the MTEXT is created");

    assert_eq!(outcome.objects.len(), 1, "one entity for one MTEXT");
    let changes = outcome.changes.clone().expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(changes.changes.len(), 1, "one transaction");

    let entity = stored_text_entity(&app);
    assert_eq!(EntityId(outcome.objects[0].0), entity.id);
    // The load-bearing type key: MTEXT, never `class_name()`'s `AcDbText`.
    assert_eq!(entity.object.type_key, "AcDbMText");
    assert_eq!(entity.layer, LayerId(0), "the active layer");
    assert_eq!(entity.space, SpaceId::Model);
    match &entity.geometry {
        SemanticGeometry::Text {
            text,
            position,
            style,
            height,
            rotation,
            font,
            h_align,
            v_align,
            wrap_width,
        } => {
            assert_eq!(text, "a\\Pb", "`\\n` is stored as `\\P`");
            assert_eq!(*position, p3(1.0, 2.0, 0.0));
            assert_eq!(*height, HEIGHT);
            assert_eq!(*rotation, ROTATION, "the payload rotation is kept");
            assert_eq!(*h_align, TextAlignH::Center);
            assert_eq!(*v_align, TextAlignV::Middle);
            assert_eq!(*wrap_width, Some(WIDTH), "the width is the wrap boundary");
            // No STYLE table entry: the documented `StyleId(0)` fallback and no
            // fabricated font key.
            assert_eq!(*style, StyleId(0));
            assert_eq!(*font, None);
        }
        other => panic!("expected text, got {other:?}"),
    }

    // The diagnostic names the unmodelled facets instead of implying them.
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "drawing.mtext_created")
        .expect("the creation diagnostic");
    for reason in [
        "text.line_spacing_style_unsupported",
        "text.columns_unsupported",
        "text.background_unsupported",
        "text.drawing_direction_unsupported",
    ] {
        assert!(
            diagnostic.message.contains(reason),
            "the diagnostic must name {reason}: {}",
            diagnostic.message
        );
    }
    assert!(
        diagnostic.message.is_ascii(),
        "no CJK: {}",
        diagnostic.message
    );

    // One undo step, and it removes exactly the created entity.
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(text_count(&app), 0, "undo removes the MTEXT");
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(text_count(&app), 1, "redo restores it");
}

#[test]
fn textedit_accepts_a_newline_on_a_created_mtext() {
    // The type-key override is what makes this work: `TEXTEDIT` converts a
    // newline only for an `AcDbMText` source and would refuse an `AcDbText`.
    let (mut app, mut session) = mtext_app();
    let created = execute(
        &mut app,
        &mut session,
        CommandId::CreateMText,
        mtext_payload("a\nb"),
    )
    .expect("the MTEXT is created");
    let id = created.objects[0];
    let after_create = revision(&app);

    execute(
        &mut app,
        &mut session,
        CommandId::TextEdit,
        CommandPayload::TextEdit {
            target: ref_of(id.0),
            text: "x\ny".into(),
        },
    )
    .expect("TEXTEDIT of a created MTEXT accepts a newline");

    assert_eq!(revision(&app).0, after_create.0 + 1, "one revision bump");
    let entity = stored_text_entity(&app);
    assert_eq!(entity.object.type_key, "AcDbMText", "the type key is kept");
    match &entity.geometry {
        SemanticGeometry::Text {
            text, wrap_width, ..
        } => {
            assert_eq!(text, "x\\Py", "the edit stores `\\P` too");
            // Editing the string never changes the layout boundary.
            assert_eq!(*wrap_width, Some(WIDTH));
        }
        other => panic!("expected text, got {other:?}"),
    }
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 2, "one step each");
}

#[test]
fn create_mtext_refuses_empty_and_whitespace_only_text_without_writing() {
    let (mut app, mut session) = mtext_app();
    let before = revision(&app);
    for text in ["", " ", "\t", "\n  \r\n"] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::CreateMText,
            mtext_payload(text),
        )
        .expect_err("empty text must refuse");
        let message = invalid_input(error);
        assert!(
            message.contains("tool.text_empty"),
            "{text:?}: the refusal must name the empty-text reason: {message}"
        );
        assert_eq!(revision(&app), before, "{text:?}: a refusal must not write");
        assert_eq!(text_count(&app), 0, "{text:?}: nothing is created");
        assert!(!app.can_undo(&DocumentId(1)), "{text:?}: no undo entry");
    }
}

#[test]
fn create_mtext_refuses_an_unusable_width_and_never_fabricates_one() {
    let (mut app, mut session) = mtext_app();
    let before = revision(&app);
    for width in [0.0, -2.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::CreateMText,
            CommandPayload::MText {
                position: p3(0.0, 0.0, 0.0),
                width: Some(width),
                text: "AB".into(),
                height: HEIGHT,
                rotation: 0.0,
                h_align: TextAlignH::Left,
                v_align: TextAlignV::Baseline,
            },
        )
        .expect_err("an unusable width must refuse");
        let message = invalid_input(error);
        assert!(
            message.contains("width must be finite and positive"),
            "{width}: {message}"
        );
        assert_eq!(revision(&app), before, "{width}: a refusal must not write");
        assert_eq!(text_count(&app), 0);
    }
    assert!(!app.can_undo(&DocumentId(1)));

    // `None` means "no wrap" and is stored as `None` — never replaced by a
    // made-up boundary.
    execute(
        &mut app,
        &mut session,
        CommandId::CreateMText,
        CommandPayload::MText {
            position: p3(0.0, 0.0, 0.0),
            width: None,
            text: "AB".into(),
            height: HEIGHT,
            rotation: 0.0,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
        },
    )
    .expect("an unwrapped MTEXT is created");
    match &stored_text_entity(&app).geometry {
        SemanticGeometry::Text { wrap_width, .. } => assert_eq!(*wrap_width, None),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn create_text_now_refuses_a_newline_but_still_creates_a_single_line_text() {
    // The honesty bug this lane fixes: a `Text` payload containing `\n` used to
    // be stored as an `AcDbText` that rendered over several lines and could never
    // be re-edited by `TEXTEDIT`.
    let (mut app, mut session) = mtext_app();
    let before = revision(&app);
    for text in ["a\nb", "a\rb", "a\r\nb"] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::CreateText,
            text_payload(text),
        )
        .expect_err("a newline in TEXT must refuse");
        let message = invalid_input(error);
        assert!(
            message.starts_with("text.single_line_newline: "),
            "{text:?}: {message}"
        );
        assert!(
            message.contains("MTEXT"),
            "{text:?}: the refusal must name the supported channel: {message}"
        );
        assert_eq!(revision(&app), before, "{text:?}: a refusal must not write");
        assert_eq!(text_count(&app), 0);
    }
    assert!(!app.can_undo(&DocumentId(1)));

    // A single-line value still works and is still an `AcDbText`.
    execute(
        &mut app,
        &mut session,
        CommandId::CreateText,
        text_payload("AB"),
    )
    .expect("a single-line TEXT is created");
    let entity = stored_text_entity(&app);
    assert_eq!(entity.object.type_key, "AcDbText");
    match &entity.geometry {
        SemanticGeometry::Text {
            text, wrap_width, ..
        } => {
            assert_eq!(text, "AB");
            assert_eq!(*wrap_width, None);
        }
        other => panic!("expected text, got {other:?}"),
    }
    assert_eq!(revision(&app).0, before.0 + 1);
}

#[test]
fn create_mtext_refuses_invalid_numbers_and_a_wrong_payload_without_writing() {
    let (mut app, mut session) = mtext_app();
    let before = revision(&app);
    let valid = mtext_payload("AB");
    let cases: Vec<(&str, CommandPayload)> = vec![
        (
            "position must be finite",
            CommandPayload::MText {
                position: p3(f64::NAN, 0.0, 0.0),
                width: None,
                text: "AB".into(),
                height: HEIGHT,
                rotation: 0.0,
                h_align: TextAlignH::Left,
                v_align: TextAlignV::Baseline,
            },
        ),
        (
            "height must be finite and positive",
            CommandPayload::MText {
                position: p3(0.0, 0.0, 0.0),
                width: None,
                text: "AB".into(),
                height: 0.0,
                rotation: 0.0,
                h_align: TextAlignH::Left,
                v_align: TextAlignV::Baseline,
            },
        ),
        (
            "height must be finite and positive",
            CommandPayload::MText {
                position: p3(0.0, 0.0, 0.0),
                width: None,
                text: "AB".into(),
                height: f64::INFINITY,
                rotation: 0.0,
                h_align: TextAlignH::Left,
                v_align: TextAlignV::Baseline,
            },
        ),
        (
            "rotation must be finite",
            CommandPayload::MText {
                position: p3(0.0, 0.0, 0.0),
                width: None,
                text: "AB".into(),
                height: HEIGHT,
                rotation: f64::NAN,
                h_align: TextAlignH::Left,
                v_align: TextAlignV::Baseline,
            },
        ),
        (
            "tool.text_too_long",
            CommandPayload::MText {
                position: p3(0.0, 0.0, 0.0),
                width: None,
                text: "a".repeat(crate::MAX_TOOL_TEXT_CHARS + 1),
                height: HEIGHT,
                rotation: 0.0,
                h_align: TextAlignH::Left,
                v_align: TextAlignV::Baseline,
            },
        ),
    ];
    for (expected, payload) in cases {
        let error = execute(&mut app, &mut session, CommandId::CreateMText, payload)
            .expect_err("the payload must refuse");
        let message = invalid_input(error);
        assert!(
            message.contains(expected),
            "the refusal must contain {expected:?}: {message}"
        );
        assert_eq!(
            revision(&app),
            before,
            "{expected}: a refusal must not write"
        );
        assert_eq!(text_count(&app), 0);
    }
    // A mismatched payload is refused too (never a silent empty success): a
    // payload-less dispatch (direct caller / IPC) reaches exactly this path. The
    // bare `MTEXT` selector itself now starts the interactive tool, which always
    // builds the full payload.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::CreateMText,
        CommandPayload::None,
    )
    .expect_err("a payload-less MTEXT dispatch must refuse");
    let message = invalid_input(error);
    assert!(message.contains("MTEXT needs an MText"), "{message}");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));

    // The untouched app still accepts the valid payload.
    execute(&mut app, &mut session, CommandId::CreateMText, valid)
        .expect("the valid payload is created");
    assert_eq!(text_count(&app), 1);
}
