//! TEXT creation contract tests (ADR 0019 decision F.1, P7a).
//!
//! Pins the single-entity / one-transaction / one-undo shape, the stored
//! `AcDbText` identity, the style/font resolution, the documented height channel,
//! the named refusal surface (nothing written), and the first **export round trip
//! of a created TEXT**.

use super::*;

use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

/// A drawing with layer 0 (and optionally `STANDARD` plus a second style) whose
/// active layer is 0.
fn text_app(styles: Vec<cad_db::Style>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for style in styles {
        b.insert_style(style).unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
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

fn style(id: u128, name: &str, font: Option<&str>) -> cad_db::Style {
    cad_db::Style {
        id: StyleId(id),
        name: name.into(),
        resource_keys: font.map(|font| vec![font.to_string()]).unwrap_or_default(),
    }
}

fn the_text(app: &Application) -> (EntityId, String, Point3, f64, f64, StyleId) {
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let entities = drawing.model_space();
    let entity = entities
        .iter()
        .find(|entity| matches!(entity.geometry, SemanticGeometry::Text { .. }))
        .expect("the text entity");
    match &entity.geometry {
        SemanticGeometry::Text {
            text,
            position,
            height,
            rotation,
            style,
            ..
        } => (
            entity.id,
            text.clone(),
            *position,
            *height,
            *rotation,
            *style,
        ),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn create_text_inserts_one_acdbt_text_in_one_transaction_and_one_undo_step() {
    let (mut app, mut session) = text_app(Vec::new());
    let before = revision(&app);
    let position = p3(1.5, -2.5, 0.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateText,
        CommandPayload::Text {
            position,
            text: "AB".into(),
            height: 2.5,
        },
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "one entity for one text");
    let changes = outcome.changes.expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(changes.changes.len(), 1, "one transaction");

    let (entity_id, text, stored_position, height, rotation, style_id) = the_text(&app);
    assert_eq!(EntityId(outcome.objects[0].0), entity_id);
    assert_eq!(text, "AB");
    assert_eq!(stored_position, position);
    assert_eq!(height, 2.5);
    assert_eq!(rotation, 0.0, "creation has no rotation prompt");
    // No STYLE table entry: the documented `StyleId(0)` fallback, never an
    // invented style identity.
    assert_eq!(style_id, StyleId(0));
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(entity_id)
        .unwrap();
    assert_eq!(entity.object.type_key, "AcDbText");
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);
    assert!(matches!(
        entity.geometry,
        SemanticGeometry::Text {
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            ..
        }
    ));

    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(entity_id)
        .is_none());
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(entity_id)
        .is_some());
}

#[test]
fn create_text_resolves_the_standard_style_identity_and_font() {
    let (mut app, mut session) = text_app(vec![
        style(0, "0style", Some("other.shx")),
        style(3, "Standard", Some("arial.ttf")),
    ]);
    execute(
        &mut app,
        &mut session,
        CommandId::CreateText,
        CommandPayload::Text {
            position: p3(0.0, 0.0, 0.0),
            text: "styled".into(),
            height: 3.0,
        },
    )
    .unwrap();
    let (_, _, _, _, _, style_id) = the_text(&app);
    assert_eq!(style_id, StyleId(3), "`Standard` is resolved by name");
    let document = &app.workspace.documents[&DocumentId(1)];
    let entities = document.drawing.model_space();
    let entity = entities
        .iter()
        .find(|entity| matches!(entity.geometry, SemanticGeometry::Text { .. }))
        .unwrap();
    match &entity.geometry {
        SemanticGeometry::Text { font, .. } => {
            assert_eq!(font.as_deref(), Some("arial.ttf"), "the style's face");
        }
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn create_text_refuses_empty_over_long_and_invalid_numbers_without_writing() {
    let (mut app, mut session) = text_app(Vec::new());
    let rev = revision(&app);
    let cases: Vec<(&str, CommandPayload)> = vec![
        (
            "tool.text_empty",
            CommandPayload::Text {
                position: p3(0.0, 0.0, 0.0),
                text: String::new(),
                height: 2.5,
            },
        ),
        (
            "tool.text_empty",
            CommandPayload::Text {
                position: p3(0.0, 0.0, 0.0),
                text: "  \t\n".into(),
                height: 2.5,
            },
        ),
        (
            "tool.text_too_long",
            CommandPayload::Text {
                position: p3(0.0, 0.0, 0.0),
                text: "a".repeat(crate::MAX_TOOL_TEXT_CHARS + 1),
                height: 2.5,
            },
        ),
        (
            "position must be finite",
            CommandPayload::Text {
                position: p3(f64::NAN, 0.0, 0.0),
                text: "AB".into(),
                height: 2.5,
            },
        ),
        (
            "height must be finite and positive",
            CommandPayload::Text {
                position: p3(0.0, 0.0, 0.0),
                text: "AB".into(),
                height: 0.0,
            },
        ),
        (
            "height must be finite and positive",
            CommandPayload::Text {
                position: p3(0.0, 0.0, 0.0),
                text: "AB".into(),
                height: f64::INFINITY,
            },
        ),
    ];
    for (expected, payload) in cases {
        let error = match execute(&mut app, &mut session, CommandId::CreateText, payload) {
            Ok(_) => panic!("{expected} must refuse"),
            Err(error) => error,
        };
        match error {
            CadError::InvalidInput(message) => assert!(
                message.contains(expected),
                "the refusal must contain {expected:?}: {message}"
            ),
            other => panic!("expected InvalidInput for {expected}, got {other:?}"),
        }
        assert_eq!(revision(&app), rev, "{expected}: a refusal must not write");
    }
    // A mismatched payload is refused too (never a silent empty success).
    let error = execute(
        &mut app,
        &mut session,
        CommandId::CreateText,
        CommandPayload::Points(vec![p3(0.0, 0.0, 0.0)]),
    )
    .unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_text_on_a_missing_layer_is_refused_by_the_shared_funnel() {
    let (mut app, mut session) = text_app(Vec::new());
    let rev = revision(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(99)),
    )
    .unwrap();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::CreateText,
        CommandPayload::Text {
            position: p3(0.0, 0.0, 0.0),
            text: "AB".into(),
            height: 2.5,
        },
    )
    .unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn the_text_height_parameter_default_is_the_documented_constant() {
    assert_eq!(
        ToolParameter::TextHeight.default_value(),
        ToolValue::Number(crate::draw_tool::TEXT_DEFAULT_HEIGHT)
    );
    assert_eq!(crate::draw_tool::TEXT_DEFAULT_HEIGHT, 2.5);
    ToolParameter::TextHeight
        .validate(ToolValue::Number(2.5))
        .expect("the default is a valid value");
    for refused in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(
            ToolParameter::TextHeight
                .validate(ToolValue::Number(refused))
                .is_err(),
            "{refused} must be refused"
        );
    }
    let session = SessionState::new(DocumentId(1), AppMode::Work);
    assert_eq!(
        session.tool_parameter(ToolParameter::TextHeight),
        ToolValue::Number(2.5)
    );
}

#[test]
fn a_created_text_exports_as_text_and_reimports() {
    let (mut app, mut session) = text_app(vec![style(3, "Standard", Some("arial.ttf"))]);
    execute(
        &mut app,
        &mut session,
        CommandId::CreateText,
        CommandPayload::Text {
            position: p3(4.0, 5.0, 0.0),
            text: "round trip".into(),
            height: 3.5,
        },
    )
    .unwrap();

    let (bytes, report) = export(&ExportRequest {
        database: &app.workspace.documents[&DocumentId(1)].drawing,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
    })
    .expect("the drawing exports");
    let text = std::str::from_utf8(&bytes).expect("ASCII DXF");
    assert!(
        text.lines().any(|line| line.trim() == "TEXT"),
        "a real TEXT record is written (single-line text is not MTEXT)"
    );
    assert!(!text.lines().any(|line| line.trim() == "MTEXT"), "{text}");
    assert_eq!(report.counts.dropped, 0, "{:?}", report.entries);

    let request = ImportRequest {
        document: DocumentId(2),
        database: DatabaseId(2),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
    };
    let imported = AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("the exported drawing re-imports");
    let (stored_text, position, height) = imported
        .database
        .model_space()
        .iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::Text {
                text,
                position,
                height,
                ..
            } => Some((text.clone(), *position, *height)),
            _ => None,
        })
        .expect("the exported text re-imports as Text");
    assert_eq!(stored_text, "round trip");
    assert!((position.x - 4.0).abs() < 1e-9, "{position:?}");
    assert!((position.y - 5.0).abs() < 1e-9, "{position:?}");
    assert!((height - 3.5).abs() < 1e-9, "{height}");
}

/// Dispatch one TEXT creation with the given payload (the create path under test).
fn create_text(app: &mut Application, session: &mut SessionState, position: Point3) {
    execute(
        app,
        session,
        CommandId::CreateText,
        CommandPayload::Text {
            position,
            text: "ROT".into(),
            height: 2.5,
        },
    )
    .expect("the text is created");
}

#[test]
fn create_text_defaults_to_the_zero_rotation_of_the_session_channel() {
    // The parameter's default reproduces the previously fixed value byte for byte:
    // an untouched session stores exactly `0.0` radians.
    let (mut app, mut session) = text_app(Vec::new());
    assert_eq!(
        session.tool_parameter(ToolParameter::TextRotation),
        ToolValue::Number(0.0),
        "the session starts at the documented default"
    );
    create_text(&mut app, &mut session, p3(0.0, 0.0, 0.0));
    assert_eq!(the_text(&app).4, 0.0, "unrotated, exactly as before");
}

#[test]
fn create_text_converts_the_session_degrees_once_into_stored_radians() {
    // The channel carries the degrees AutoCAD prompts with; the domain stores
    // radians. 30° is π/6 and the conversion happens exactly once.
    let (mut app, mut session) = text_app(Vec::new());
    session
        .set_tool_parameter(ToolParameter::TextRotation, ToolValue::Number(30.0))
        .expect("30 degrees is a finite value");
    create_text(&mut app, &mut session, p3(1.0, 2.0, 0.0));
    // Exactly one conversion: the stored value is `30°.to_radians()`, which is
    // π/6 to the last bit of the two literals' rounding.
    let stored = the_text(&app).4;
    assert_eq!(
        stored,
        30.0_f64.to_radians(),
        "the channel's degrees are converted once with `.to_radians()`"
    );
    assert!(
        (stored - std::f64::consts::FRAC_PI_6).abs() < 1e-15,
        "30 degrees is π/6 radians, got {stored}"
    );
}

#[test]
fn create_text_stores_a_negative_rotation_verbatim_without_wrapping() {
    let (mut app, mut session) = text_app(Vec::new());
    session
        .set_tool_parameter(ToolParameter::TextRotation, ToolValue::Number(-30.0))
        .expect("-30 degrees is a finite value");
    create_text(&mut app, &mut session, p3(1.0, 2.0, 0.0));
    let stored = the_text(&app).4;
    assert_eq!(
        stored,
        -30.0_f64.to_radians(),
        "no wrap and no clamp: a negative angle stays negative"
    );
}

#[test]
fn create_text_refuses_a_forced_non_finite_rotation_and_writes_nothing() {
    // The channel refuses NaN/∞, so this forced store value reaches the command's
    // own guard: it must refuse by name and write nothing.
    for refused in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let (mut app, mut session) = text_app(Vec::new());
        let before = revision(&app);
        session.force_tool_parameter(ToolParameter::TextRotation, ToolValue::Number(refused));
        let error = execute(
            &mut app,
            &mut session,
            CommandId::CreateText,
            CommandPayload::Text {
                position: p3(0.0, 0.0, 0.0),
                text: "BAD".into(),
                height: 2.5,
            },
        )
        .expect_err("a non-finite rotation must refuse");
        let message = match error {
            CadError::InvalidInput(message) => message,
            other => panic!("expected an InvalidInput refusal, got {other:?}"),
        };
        assert!(message.contains("textRotation"), "{message}");
        assert!(message.contains("finite"), "{message}");
        assert_eq!(revision(&app), before, "{refused}: no transaction ran");
        assert!(!app.can_undo(&DocumentId(1)), "{refused}: no undo entry");
        assert!(
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .model_space()
                .is_empty(),
            "{refused}: nothing written"
        );
    }
}

#[test]
fn the_text_creation_parameter_channel_refuses_by_name_and_writes_nothing() {
    let (app, mut session) = text_app(Vec::new());
    let before = revision(&app);
    for (parameter, refused, expected) in [
        (ToolParameter::TextRotation, f64::NAN, "must be finite"),
        (ToolParameter::TextRotation, f64::INFINITY, "must be finite"),
        (ToolParameter::MTextAttachment, 0.0, "between 1 and 9"),
        (ToolParameter::MTextAttachment, 10.0, "between 1 and 9"),
        (ToolParameter::MTextAttachment, 1.5, "between 1 and 9"),
        (ToolParameter::MTextAttachment, f64::NAN, "between 1 and 9"),
    ] {
        let error = session
            .set_tool_parameter(parameter, ToolValue::Number(refused))
            .expect_err("the channel refuses the value");
        let message = match error {
            CadError::InvalidInput(message) => message,
            other => panic!("expected an InvalidInput refusal, got {other:?}"),
        };
        assert!(
            message.contains(parameter.key()),
            "the refusal names '{}': {message}",
            parameter.key()
        );
        assert!(message.contains(expected), "{message}");
        // The store keeps its previous value: a refusal never mutates.
        assert_eq!(
            session.tool_parameter(parameter),
            parameter.default_value(),
            "{} keeps its default",
            parameter.key()
        );
    }
    assert_eq!(revision(&app), before, "no channel refusal writes");
    assert!(!app.can_undo(&DocumentId(1)));
}
