//! `MLEADER` creation options beyond the geometry trio: the text angle **type**
//! (`174` / style `175`, mask `0x2000`), the `STYLE` selector and content-less
//! creation (ADR 0019 F.2 amendment).
//!
//! Split out of `mleader_create` so both files stay inside the code-size limit.
//! Each test pins the stored value, the group it lands in and the DXF round trip;
//! every refusal asserts the revision did not move.

use super::*;

use cad_db::MLeaderStyle;
use cad_domain::MLeaderStyleId;
use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

/// A drawing with layer 0, the pinned `Standard` row and the extra style rows.
fn mleader_app(rows: Vec<MLeaderStyle>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for row in rows {
        b.insert_mleader_style(row).unwrap();
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

fn payload(arrow: Point3, landing: Point3, text: &str) -> CommandPayload {
    CommandPayload::MLeader {
        arrow,
        landing,
        text: text.to_string(),
        style_name: None,
        dogleg_length: None,
        dogleg_angle: None,
        attachment: None,
        text_angle: None,
        content: None,
    }
}

fn with_style(mut request: CommandPayload, name: &str) -> CommandPayload {
    match &mut request {
        CommandPayload::MLeader { style_name, .. } => *style_name = Some(name.to_string()),
        _ => panic!("expected an MLeader payload"),
    }
    request
}

fn with_angle(mut request: CommandPayload, angle: MLeaderTextAngle) -> CommandPayload {
    match &mut request {
        CommandPayload::MLeader { text_angle, .. } => *text_angle = Some(angle),
        _ => panic!("expected an MLeader payload"),
    }
    request
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(CommandId::MLeader, payload))
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

/// The one stored multileader.
fn the_leader(app: &Application) -> MultiLeaderGeometry {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::MultiLeader(leader) => Some(leader.as_ref().clone()),
            _ => None,
        })
        .expect("the multileader entity")
}

/// The re-imported drawing from a DXF-text export of document 1.
fn reimport(app: &Application) -> cad_db::DrawingDatabase {
    let (bytes, _) = export(&ExportRequest {
        database: &app.workspace.documents[&DocumentId(1)].drawing,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
    })
    .expect("the drawing exports");
    let request = ImportRequest {
        document: DocumentId(2),
        database: DatabaseId(2),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("the exported drawing re-imports")
        .database
}

fn exported_text(app: &Application) -> String {
    let (bytes, _) = export(&ExportRequest {
        database: &app.workspace.documents[&DocumentId(1)].drawing,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
    })
    .expect("the drawing exports");
    String::from_utf8(bytes).expect("ASCII DXF")
}

/// Every value written for one DXF group code, in file order.
fn group_values(text: &str, code: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    lines
        .windows(2)
        .filter(|pair| pair[0] == code)
        .map(|pair| pair[1].to_string())
        .collect()
}

fn reimported_leader(database: &cad_db::DrawingDatabase) -> MultiLeaderGeometry {
    database
        .model_space()
        .iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::MultiLeader(leader) => Some(leader.as_ref().clone()),
            _ => None,
        })
        .expect("the created leader re-imports as a MULTILEADER")
}

#[test]
fn the_typed_angle_mode_is_an_entity_override_and_round_trips() {
    let (mut app, mut session) = mleader_app(Vec::new());
    let request = with_angle(
        payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "Angle"),
        MLeaderTextAngle::ParallelToLastLeaderLine,
    );
    let outcome = execute(&mut app, &mut session, request).expect("the angle mode is stored");
    let leader = the_leader(&app);
    assert_eq!(
        leader.style().text_angle_type,
        MLeaderTextAngle::ParallelToLastLeaderLine
    );
    assert_ne!(
        leader.override_mask() & MASK_TEXT_ANGLE,
        0,
        "an entity value that differs from the style's names the 0x2000 override"
    );
    // The typed option is reported with the group it went to.
    let report = outcome
        .diagnostics
        .iter()
        .find(|row| row.code == "mleader.options_applied")
        .expect("the applied option is reported");
    assert!(report.message.contains("174"), "{}", report.message);

    // Display: `PARALLEL` draws the text along the last leader segment, so the
    // text child's rotation is the (0,0) -> (9.64,2) segment angle.
    let expected = 2.0_f64.atan2(9.64);
    let rotation = leader
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Text { rotation, .. } => Some(*rotation),
            _ => None,
        })
        .expect("the text child");
    assert!(
        (rotation - expected).abs() < 1e-9,
        "{rotation} != {expected}"
    );

    // Entity `174 = 0` and the style mirror `175 = 0` are both written.
    let text = exported_text(&app);
    assert!(
        group_values(&text, "174").contains(&"0".to_string()),
        "entity 174 = 0: {:?}",
        group_values(&text, "174")
    );
    assert!(
        group_values(&text, "175").contains(&"0".to_string()),
        "style 175 = 0: {:?}",
        group_values(&text, "175")
    );

    // Reload: the mask bit makes the entity value win over the style's.
    let database = reimport(&app);
    let reimported = reimported_leader(&database);
    assert_eq!(
        reimported.style().text_angle_type,
        MLeaderTextAngle::ParallelToLastLeaderLine
    );
    assert_ne!(reimported.override_mask() & MASK_TEXT_ANGLE, 0);
}

#[test]
fn a_style_inherited_angle_round_trips_through_the_style_record() {
    let mut standard = MLeaderStyle::standard_metric(MLeaderStyleId(0));
    standard.text_angle_type = MLeaderTextAngle::Optimized;
    let (mut app, mut session) = mleader_app(vec![standard]);
    execute(
        &mut app,
        &mut session,
        payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "Inherited"),
    )
    .expect("the style's angle mode is inherited");
    let leader = the_leader(&app);
    assert_eq!(leader.style().text_angle_type, MLeaderTextAngle::Optimized);
    assert_eq!(
        leader.override_mask() & MASK_TEXT_ANGLE,
        0,
        "an inherited value needs no override bit (a plain creation overrides nothing)"
    );

    let text = exported_text(&app);
    assert!(
        group_values(&text, "174").contains(&"2".to_string()),
        "entity 174 = 2: {:?}",
        group_values(&text, "174")
    );
    assert!(
        group_values(&text, "175").contains(&"2".to_string()),
        "style 175 = 2: {:?}",
        group_values(&text, "175")
    );

    let database = reimport(&app);
    let reimported = reimported_leader(&database);
    assert_eq!(
        reimported.style().text_angle_type,
        MLeaderTextAngle::Optimized,
        "without the mask bit the style value re-resolves on reload"
    );
    assert_eq!(reimported.override_mask() & MASK_TEXT_ANGLE, 0);
}

#[test]
fn a_mode_equal_to_the_style_needs_no_override_bit() {
    let (mut app, mut session) = mleader_app(Vec::new());
    let request = with_angle(
        payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "Same"),
        MLeaderTextAngle::Horizontal,
    );
    execute(&mut app, &mut session, request).expect("HORIZONTAL equals the style default");
    let leader = the_leader(&app);
    assert_eq!(leader.style().text_angle_type, MLeaderTextAngle::Horizontal);
    assert_eq!(
        leader.override_mask(),
        0,
        "asking for the style's own value is not an override"
    );
}

#[test]
fn the_style_selector_resolves_the_named_row_and_refuses_an_unknown_name() {
    let mut callout = MLeaderStyle::standard_metric(MLeaderStyleId(7));
    callout.name = "Callout".to_string();
    callout.dogleg_length = 2.0;
    callout.text_height = 0.3;
    callout.text_angle_type = MLeaderTextAngle::Optimized;
    let (mut app, mut session) = mleader_app(vec![callout]);

    let request = with_style(
        payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "Named"),
        "Callout",
    );
    let outcome = execute(&mut app, &mut session, request).expect("the named style resolves");
    let leader = the_leader(&app);
    assert_eq!(leader.style_name(), "Callout");
    assert_eq!(leader.roots()[0].dogleg_length, 2.0);
    assert!((leader.text().height - 0.3).abs() < 1e-12);
    assert_eq!(
        leader.style().text_angle_type,
        MLeaderTextAngle::Optimized,
        "the named row's snapshot is what creation stores"
    );
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|row| row.code == "mleader.style_selected" && row.message.contains("Callout")),
        "the selector is reported, never silent: {:?}",
        outcome.diagnostics
    );

    // An unknown name is a named refusal that writes nothing.
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        with_style(
            payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "Missing"),
            "NoSuchStyle",
        ),
    )
    .expect_err("an unknown style is refused");
    let message = match error {
        CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
        other => panic!("expected a named refusal, got {other:?}"),
    };
    assert!(message.contains("mleader.style_missing"), "{message}");
    assert!(message.contains("NoSuchStyle"), "{message}");
    assert_eq!(revision(&app), before, "the refusal wrote nothing");
}

#[test]
fn the_style_table_can_write_the_angle_type_and_the_named_row_creates_from_it() {
    // `MLEADERSTYLE MODIFY` is the only style-editing surface, and its field list
    // is documented as "the fields of `cad_db::MLeaderStyle`": the new column must
    // be writable there or a user could never set it without an imported style.
    let (mut app, mut session) = mleader_app(Vec::new());
    app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::MLeaderStyleList),
            CommandPayload::StyleTable(StyleTableCommand {
                kind: StyleTableKind::MLeaderStyle,
                op: StyleTableOp::Modify {
                    name: "Standard".to_string(),
                    assignments: vec![("text_angle_type".to_string(), "OPTIMIZED".to_string())],
                },
            }),
        ),
    )
    .expect("the angle type is a writable style field");
    let row = app.workspace.documents[&DocumentId(1)]
        .drawing
        .mleader_style_by_name("Standard")
        .expect("the row");
    assert_eq!(row.text_angle_type, MLeaderTextAngle::Optimized);

    let request = with_style(
        payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "FromRow"),
        "Standard",
    );
    execute(&mut app, &mut session, request).expect("the edited row creates");
    let leader = the_leader(&app);
    assert_eq!(leader.style().text_angle_type, MLeaderTextAngle::Optimized);
    assert_eq!(leader.override_mask() & MASK_TEXT_ANGLE, 0);

    // A rotation and an unknown word are both refused by the field grammar.
    for value in ["90", "diagonal"] {
        let error = app
            .execute(
                &mut session,
                command(
                    CommandId::Extended(ExtendedCommandId::MLeaderStyleList),
                    CommandPayload::StyleTable(StyleTableCommand {
                        kind: StyleTableKind::MLeaderStyle,
                        op: StyleTableOp::Modify {
                            name: "Standard".to_string(),
                            assignments: vec![("text_angle_type".to_string(), value.to_string())],
                        },
                    }),
                ),
            )
            .unwrap_err();
        let message = match error {
            CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
            other => panic!("expected a named refusal for {value}, got {other:?}"),
        };
        assert!(message.contains("mleaderstyle.value_range"), "{message}");
    }
}

#[test]
fn a_default_created_leader_keeps_the_writer_base_angle_codes() {
    // The field's default is the mode the writer already emitted (`1`, horizontal,
    // the value the created geometry's text rotation of `0` produces), so a leader
    // created without the option is byte-identical to a pre-field build.
    let (mut app, mut session) = mleader_app(Vec::new());
    execute(
        &mut app,
        &mut session,
        payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "Default"),
    )
    .expect("the plain creation");
    let leader = the_leader(&app);
    assert_eq!(leader.style().text_angle_type, MLeaderTextAngle::Horizontal);
    assert_eq!(leader.style().text_angle_type.dxf(), 1);
    assert_eq!(leader.override_mask(), 0, "the default overrides nothing");

    let text = exported_text(&app);
    assert!(
        group_values(&text, "174").contains(&"1".to_string()),
        "entity 174 = 1: {:?}",
        group_values(&text, "174")
    );
    assert!(
        group_values(&text, "175").contains(&"1".to_string()),
        "style 175 = 1: {:?}",
        group_values(&text, "175")
    );
    let database = reimport(&app);
    assert_eq!(
        reimported_leader(&database).style().text_angle_type,
        MLeaderTextAngle::Horizontal
    );
}
