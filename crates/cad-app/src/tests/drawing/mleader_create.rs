//! `MLEADER` creation contract tests (ADR 0019 decision F.2, P7c).
//!
//! Pins the style-table resolution, the exact two-point geometry (one root, one
//! line, two straight vertices, a derived dogleg start), the single-entity /
//! one-transaction / one-undo-step shape, the stored `AcDbMultiLeader` identity,
//! the named refusal surface (nothing written), the transform refusals the
//! variant already owns, and the **first export round trip of a created
//! leader**.

use super::*;

use cad_db::MLeaderStyle;
use cad_domain::MLeaderStyleId;
use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

/// A drawing with layer 0, an optional second layer `WALLS` (id 3), the given
/// text styles, and optionally a replacement `Standard` MLEADERSTYLE row.
fn mleader_app(
    styles: Vec<cad_db::Style>,
    standard: Option<MLeaderStyle>,
) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    for style in styles {
        b.insert_style(style).unwrap();
    }
    if let Some(style) = standard {
        b.insert_mleader_style(style).unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
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

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn approx_point(a: Point3, b: Point3) -> bool {
    approx(a.x, b.x) && approx(a.y, b.y) && approx(a.z, b.z)
}

/// The one stored multileader, or a panic naming what was found instead.
fn the_leader(app: &Application) -> (EntityId, MultiLeaderGeometry) {
    let document = &app.workspace.documents[&DocumentId(1)];
    let entities = document.drawing.model_space();
    let entity = entities
        .iter()
        .find(|entity| matches!(entity.geometry, SemanticGeometry::MultiLeader(_)))
        .expect("the multileader entity");
    match &entity.geometry {
        SemanticGeometry::MultiLeader(leader) => (entity.id, leader.as_ref().clone()),
        other => panic!("expected a multileader, got {other:?}"),
    }
}

fn payload(arrow: Point3, landing: Point3, text: &str) -> CommandPayload {
    CommandPayload::MLeader {
        arrow,
        landing,
        text: text.to_string(),
    }
}

#[test]
fn create_mleader_inserts_one_acdbmultileader_in_one_transaction_and_one_undo_step() {
    let (mut app, mut session) = mleader_app(Vec::new(), None);
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(3)),
    )
    .unwrap();
    let before = revision(&app);
    let arrow = p3(0.0, 0.0, 0.0);
    let landing = p3(10.0, 2.0, 0.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        payload(arrow, landing, "Note"),
    )
    .unwrap();

    assert_eq!(outcome.objects.len(), 1, "one MULTILEADER for one capture");
    let changes = outcome.changes.expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(changes.changes.len(), 1, "one transaction");

    let (entity_id, leader) = the_leader(&app);
    assert_eq!(EntityId(outcome.objects[0].0), entity_id);

    // ─── Definition ───
    assert_eq!(leader.content(), MLeaderContent::Text);
    assert_eq!(leader.style_name(), "Standard");
    assert_eq!(
        leader.override_mask(),
        0,
        "a created leader overrides nothing"
    );
    assert_eq!(leader.roots().len(), 1, "one root");
    assert_eq!(leader.lines().len(), 1, "one line");
    let root = &leader.roots()[0];
    assert_eq!(root.leader_index, 0);
    assert!(approx_point(root.landing_point, landing));
    assert!(approx_point(
        root.dogleg_vector,
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0
        }
    ));
    let line = &leader.lines()[0];
    assert_eq!(line.root_index, 0);
    assert_eq!(line.index, 0);
    assert_eq!(line.path_type, MLeaderPathType::StraightLineSegments);
    assert_eq!(line.override_flags, 0, "no per-line overrides");
    assert_eq!(
        line.points.len(),
        2,
        "exactly two vertices: the arrow and the derived dogleg start"
    );
    assert!(approx_point(line.points[0], arrow));
    // The dogleg start = landing - (style dogleg length · side); the side is +X
    // because the landing point lies to the right of the arrow.
    assert!(approx_point(
        line.points[1],
        p3(landing.x - 0.36, landing.y, landing.z)
    ));

    // ─── Text ───
    let text = leader.text();
    assert_eq!(text.string, "Note");
    assert!(
        approx_point(text.location, landing),
        "text at the landing point"
    );
    assert!(approx(text.height, 0.18), "style text height");
    assert_eq!(text.rotation, 0.0);
    assert!(approx(text.width, 0.0));
    assert_eq!(text.attachment_point, 2, "centred horizontally");
    assert_eq!(text.left_attachment, 1, "the style's vertical attachment");

    // ─── Derived display: path, arrowhead, dogleg, text ───
    let display = leader.display();
    assert_eq!(display.len(), 4, "{display:?}");
    match &display[0] {
        SemanticGeometry::Polyline { points, closed, .. } => {
            assert_eq!(points.len(), 2);
            assert!(approx_point(points[0], arrow));
            assert!(approx_point(points[1], p3(9.64, 2.0, 0.0)));
            assert!(!closed);
        }
        other => panic!("expected the leader path, got {other:?}"),
    }
    match &display[1] {
        SemanticGeometry::Mesh(mesh) => {
            assert!(approx_point(mesh.vertices[0], arrow), "the arrow tip");
            // The base centre sits one arrowhead size behind the tip.
            let base = Point3 {
                x: (mesh.vertices[1].x + mesh.vertices[2].x) / 2.0,
                y: (mesh.vertices[1].y + mesh.vertices[2].y) / 2.0,
                z: (mesh.vertices[1].z + mesh.vertices[2].z) / 2.0,
            };
            let dx = base.x - arrow.x;
            let dy = base.y - arrow.y;
            assert!(approx(dx.hypot(dy), 0.18), "arrowhead size {dx:?},{dy:?}");
        }
        other => panic!("expected the arrowhead mesh, got {other:?}"),
    }
    match &display[2] {
        SemanticGeometry::Line { start, end } => {
            assert!(approx_point(*start, p3(9.64, 2.0, 0.0)));
            // Starts at the last vertex and stops the landing gap short.
            assert!(approx_point(*end, p3(10.0 - 0.09, 2.0, 0.0)));
        }
        other => panic!("expected the dogleg line, got {other:?}"),
    }
    match &display[3] {
        SemanticGeometry::Text {
            text,
            position,
            height,
            font,
            ..
        } => {
            assert_eq!(text, "Note");
            assert!(approx_point(*position, landing));
            assert!(approx(*height, 0.18));
            assert_eq!(*font, None);
        }
        other => panic!("expected the annotation text, got {other:?}"),
    }

    // ─── Stored identity / placement ───
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(entity_id)
        .unwrap();
    assert_eq!(entity.object.type_key, "AcDbMultiLeader");
    assert_eq!(entity.layer, LayerId(3), "the active layer");
    assert_eq!(entity.space, SpaceId::Model);
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
fn the_created_snapshot_is_the_resolved_style_table_row() {
    let (mut app, mut session) = mleader_app(vec![style(3, "Standard", Some("arial.ttf"))], None);
    execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "Note"),
    )
    .unwrap();
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let resolved = drawing
        .resolve_mleader_style("Standard")
        .expect("the pinned Standard resolves");
    let (_, leader) = the_leader(&app);
    // Every field the entity stores is the table's effective value: one source.
    assert_eq!(leader.style(), &resolved.values);
    assert_eq!(leader.style().text_style, StyleId(3));
    assert_eq!(leader.style().text_font.as_deref(), Some("arial.ttf"));
    assert_eq!(leader.text().left_attachment, resolved.left_attachment);
    assert!(approx(leader.text().height, resolved.values.text_height));
}

#[test]
fn create_mleader_honours_a_custom_standard_row() {
    // A non-default row proves the geometry really comes from the table, not
    // from constants hidden in the command.
    let mut standard = MLeaderStyle::standard_metric(MLeaderStyleId(0));
    standard.dogleg_length = 2.0;
    standard.landing_gap = 0.1;
    standard.arrowhead_size = 0.5;
    standard.text_height = 0.3;
    standard.left_attachment = 4;
    let (mut app, mut session) = mleader_app(Vec::new(), Some(standard));
    execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "Custom"),
    )
    .unwrap();
    let (_, leader) = the_leader(&app);
    let style = leader.style();
    assert!(approx(style.landing_distance, 2.0));
    assert!(approx(style.landing_gap, 0.1));
    assert!(approx(style.arrowhead_size, 0.5));
    assert!(approx(style.text_height, 0.3));
    assert_eq!(leader.roots()[0].dogleg_length, 2.0);
    // The derived dogleg start is landing - 2.0 on X.
    assert!(approx_point(leader.lines()[0].points[1], p3(8.0, 2.0, 0.0)));
    assert!(approx(leader.text().height, 0.3));
    assert_eq!(leader.text().left_attachment, 4);
    match &leader.display()[2] {
        SemanticGeometry::Line { start, end } => {
            assert!(approx_point(*start, p3(8.0, 2.0, 0.0)));
            // 2.0 dogleg minus the 0.1 landing gap.
            assert!(approx_point(*end, p3(9.9, 2.0, 0.0)));
        }
        other => panic!("expected the dogleg line, got {other:?}"),
    }
    assert!(
        leader.transformed(&Transform3::scale(2.0)).is_err(),
        "the variant still refuses scaling"
    );
}

#[test]
fn create_mleader_refuses_bad_text_points_and_payloads_without_writing() {
    let (mut app, mut session) = mleader_app(Vec::new(), None);
    let rev = revision(&app);
    let cases: Vec<(&str, CommandPayload)> = vec![
        (
            "tool.text_empty",
            payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), ""),
        ),
        (
            "tool.text_empty",
            payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "  \t\n"),
        ),
        (
            "tool.text_too_long",
            CommandPayload::MLeader {
                arrow: p3(0.0, 0.0, 0.0),
                landing: p3(10.0, 2.0, 0.0),
                text: "a".repeat(MAX_TOOL_TEXT_CHARS + 1),
            },
        ),
        (
            "mleader.points_not_finite",
            payload(p3(f64::NAN, 0.0, 0.0), p3(10.0, 2.0, 0.0), "Note"),
        ),
        (
            "mleader.plane",
            payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 1.0), "Note"),
        ),
        (
            // Closer than the style's 0.36 dogleg length.
            "mleader.landing_too_close",
            payload(p3(0.0, 0.0, 0.0), p3(0.2, 0.0, 0.0), "Note"),
        ),
    ];
    for (expected, payload) in cases {
        let error = match execute(&mut app, &mut session, CommandId::MLeader, payload) {
            Ok(_) => panic!("{expected} must refuse"),
            Err(error) => error,
        };
        let message = match error {
            CadError::InvalidInput(message) => message,
            CadError::Unsupported(message) => message,
            other => panic!("expected a named refusal for {expected}, got {other:?}"),
        };
        assert!(
            message.contains(expected),
            "the refusal must contain {expected:?}: {message}"
        );
        assert_eq!(revision(&app), rev, "{expected}: a refusal must not write");
    }
    // A mismatched payload is refused too (never a silent empty success).
    let error = execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        CommandPayload::Points(vec![p3(0.0, 0.0, 0.0)]),
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.contains("mleader.payload")),
        "{error:?}"
    );
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_mleader_refuses_a_spline_style_without_writing() {
    let mut standard = MLeaderStyle::standard_metric(MLeaderStyleId(0));
    standard.path_type = MLeaderPathType::Spline;
    let (mut app, mut session) = mleader_app(Vec::new(), Some(standard));
    let rev = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "Note"),
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::Unsupported(ref message) if message.contains("mleader.path_type")),
        "{error:?}"
    );
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_mleader_refuses_block_tolerance_and_content_less_styles_without_writing() {
    for (content, code) in [
        (MLeaderStyle::CONTENT_BLOCK, "mleader.content_unsupported"),
        (
            MLeaderStyle::CONTENT_TOLERANCE,
            "mleader.content_unsupported",
        ),
        (MLeaderStyle::CONTENT_NONE, "mleader.text_required"),
    ] {
        let mut standard = MLeaderStyle::standard_metric(MLeaderStyleId(0));
        standard.content_type = content;
        let (mut app, mut session) = mleader_app(Vec::new(), Some(standard));
        let rev = revision(&app);
        let error = execute(
            &mut app,
            &mut session,
            CommandId::MLeader,
            payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "Note"),
        )
        .unwrap_err();
        let message = match error {
            CadError::Unsupported(message) | CadError::InvalidInput(message) => message,
            other => panic!("expected a named refusal for content {content}, got {other:?}"),
        };
        assert!(
            message.contains(code),
            "content {content} must refuse with {code:?}: {message}"
        );
        assert_eq!(revision(&app), rev, "content {content} must not write");
        assert!(!app.can_undo(&DocumentId(1)));
    }
}

#[test]
fn create_mleader_refuses_a_missing_layer_and_viewer_mode_without_writing() {
    let (mut app, mut session) = mleader_app(Vec::new(), None);
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
        CommandId::MLeader,
        payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "Note"),
    )
    .unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));

    // Viewer mode refuses the Work-only command before any write.
    let (mut app, mut session) = mleader_app(Vec::new(), None);
    let rev = revision(&app);
    session.switch_mode(AppMode::Viewer).unwrap();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "Note"),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            CadError::PermissionDenied | CadError::InvalidInput(_)
        ),
        "{error:?}"
    );
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn the_text_less_path_is_unreachable() {
    // The command refuses an empty/whitespace text (no content-less creation).
    let (mut app, mut session) = mleader_app(Vec::new(), None);
    for text in ["", "   "] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::MLeader,
            payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), text),
        )
        .unwrap_err();
        assert!(
            matches!(error, CadError::InvalidInput(ref message) if message.contains("tool.text_empty")),
            "{error:?}"
        );
    }
    // The pure tool cannot produce an intent without the text either.
    let mut tool = crate::DrawTool::new(crate::draw_tool::DrawToolKind::MLeader, 0);
    tool.push_point(p3(0.0, 0.0, 0.0)).unwrap();
    tool.push_point(p3(10.0, 2.0, 0.0)).unwrap();
    assert!(!tool.preview().can_confirm(), "text is terminal");
    let error = tool.intent().unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.contains("tool.text_required")),
        "{error:?}"
    );
    // And the creation constructor itself has no content-less arm: it always
    // stores `MLeaderContent::Text`.
    let resolution = app.workspace.documents[&DocumentId(1)]
        .drawing
        .resolve_mleader_style("Standard")
        .unwrap();
    let created = MultiLeaderGeometry::created(
        resolution.values,
        "Standard".into(),
        resolution.left_attachment,
        p3(0.0, 0.0, 0.0),
        p3(10.0, 2.0, 0.0),
        "Note".into(),
    )
    .expect("created");
    assert_eq!(created.content(), MLeaderContent::Text);
}

#[test]
fn a_created_multileader_refuses_scale_mirror_and_non_uniform_transforms() {
    let (mut app, mut session) = mleader_app(Vec::new(), None);
    execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "Note"),
    )
    .unwrap();
    let (_, leader) = the_leader(&app);
    let mut non_uniform = Transform3::identity();
    non_uniform.matrix[0][0] = 2.0;
    for transform in [
        Transform3::scale(2.0),
        non_uniform,
        // A mirror (negative determinant) is a reflection, not a rigid motion.
        Transform3::scale(-1.0),
    ] {
        let error = leader.transformed(&transform).unwrap_err();
        assert!(
            matches!(error, CadError::Unsupported(ref message) if message.contains("multileader")),
            "{error:?}"
        );
    }
    // A rigid motion still works (translation + rotation only).
    leader
        .transformed(&Transform3::translation(p3(1.0, 0.0, 0.0)))
        .expect("a translation is rigid");
}

#[test]
fn a_created_multileader_exports_as_a_real_multileader_and_reimports() {
    let (mut app, mut session) = mleader_app(Vec::new(), None);
    execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        payload(p3(0.0, 0.0, 0.0), p3(10.0, 2.0, 0.0), "round trip"),
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
        text.lines().any(|line| line.trim() == "MULTILEADER"),
        "a real MULTILEADER record is written, not a flattened display"
    );
    assert!(
        text.lines().any(|line| line.trim() == "MLEADERSTYLE"),
        "the style record the entity references is written"
    );
    assert_eq!(report.counts.dropped, 0, "{:?}", report.entries);
    assert!(
        !report
            .entries
            .iter()
            .any(|entry| entry.code == "export.converted.multileader_flattened"),
        "a rigid model-space leader is written as MULTILEADER, not flattened: {:?}",
        report.entries
    );

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
    let reimported = imported
        .database
        .model_space()
        .iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::MultiLeader(leader) => Some(leader.as_ref()),
            _ => None,
        })
        .expect("the created leader re-imports as a MULTILEADER variant");
    assert_eq!(reimported.override_mask(), 0);
    assert_eq!(reimported.content(), MLeaderContent::Text);
    assert_eq!(reimported.style_name(), "Standard");
    assert_eq!(reimported.roots().len(), 1);
    assert_eq!(reimported.lines().len(), 1);
    let root = &reimported.roots()[0];
    assert!(approx_point(root.landing_point, p3(10.0, 2.0, 0.0)));
    assert!(approx_point(
        root.dogleg_vector,
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0
        }
    ));
    assert!(approx(root.dogleg_length, 0.36));
    let line = &reimported.lines()[0];
    assert_eq!(line.points.len(), 2);
    assert!(approx_point(line.points[0], p3(0.0, 0.0, 0.0)));
    assert!(approx_point(line.points[1], p3(9.64, 2.0, 0.0)));
    let text = reimported.text();
    assert_eq!(text.string, "round trip");
    assert!(approx_point(text.location, p3(10.0, 2.0, 0.0)));
    assert!(approx(text.height, 0.18));
    assert_eq!(text.attachment_point, 2);
    assert_eq!(text.left_attachment, 1);
    // The style snapshot survives the round trip (the written MLEADERSTYLE record
    // is the same resolved snapshot).
    assert!(approx(reimported.style().landing_distance, 0.36));
    assert!(approx(reimported.style().landing_gap, 0.09));
    assert!(approx(reimported.style().arrowhead_size, 0.18));
    assert!(approx(reimported.style().text_height, 0.18));
}
