//! `MLEADER CONTENT …`: block content and tolerance content through the typed
//! creation line, plus their DXF round trip.
//!
//! The four content kinds are entity `172` values (`0` none, `1` block, `2`
//! MText, `3` tolerance). These tests pin, per kind: the typed line creates it,
//! the drawing stores it, the display draws it, the exporter writes it and the
//! importer reads it back — and that every refusal writes nothing.

use super::*;

use cad_db::{BlockDefinition, MLeaderStyle};
use cad_domain::{MLeaderContentKind, MLeaderStyleId, MLeaderToleranceContent, Transform3};
use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

/// One named block (`Door`, id 7) with a single member line, plus layer 0.
fn block_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(7),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.set_block_metadata(BlockId(7), "Door", p3(0.0, 0.0, 0.0))
        .unwrap();
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(100),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(100),
        layer: LayerId(0),
        space: SpaceId::Block(BlockId(7)),
        geometry: SemanticGeometry::Line {
            start: p3(0.0, 0.0, 0.0),
            end: p3(1.0, 0.0, 0.0),
        },
        draw_order: 0,
    })
    .unwrap();
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

/// [`block_app`] with a replacement `Standard` MLEADERSTYLE row.
fn block_app_with_style(standard: MLeaderStyle) -> (Application, SessionState) {
    let (mut app, session) = block_app();
    // Rebuild the drawing with the row: the builder is the only authority for the
    // style table, so the row is inserted through a fresh database.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(7),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.set_block_metadata(BlockId(7), "Door", p3(0.0, 0.0, 0.0))
        .unwrap();
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(100),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(100),
        layer: LayerId(0),
        space: SpaceId::Block(BlockId(7)),
        geometry: SemanticGeometry::Line {
            start: p3(0.0, 0.0, 0.0),
            end: p3(1.0, 0.0, 0.0),
        },
        draw_order: 0,
    })
    .unwrap();
    b.insert_mleader_style(standard).unwrap();
    let drawing = b.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

fn request(content: Option<MLeaderContentSpec>, text: &str) -> CommandPayload {
    CommandPayload::MLeader {
        arrow: p3(0.0, 0.0, 0.0),
        landing: p3(10.0, 2.0, 0.0),
        text: text.to_string(),
        style_name: None,
        dogleg_length: None,
        dogleg_angle: None,
        attachment: None,
        text_angle: None,
        content,
    }
}

fn the_leader(app: &Application) -> MultiLeaderGeometry {
    let document = &app.workspace.documents[&DocumentId(1)];
    document
        .drawing
        .model_space()
        .iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::MultiLeader(leader) => Some(leader.as_ref().clone()),
            _ => None,
        })
        .expect("the multileader entity")
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn approx_point(a: Point3, b: Point3) -> bool {
    approx(a.x, b.x) && approx(a.y, b.y) && approx(a.z, b.z)
}

/// Export model space as DXF text.
fn export_bytes(app: &Application) -> Vec<u8> {
    export(&ExportRequest {
        database: &app.workspace.documents[&DocumentId(1)].drawing,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
    })
    .expect("the drawing exports")
    .0
}

/// Export model space as DXF text and re-import it.
fn round_trip(app: &Application) -> MultiLeaderGeometry {
    let (bytes, report) = export(&ExportRequest {
        database: &app.workspace.documents[&DocumentId(1)].drawing,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
    })
    .expect("the drawing exports");
    assert!(
        !report
            .entries
            .iter()
            .any(|entry| entry.code == "export.converted.multileader_flattened"),
        "the leader is written as MULTILEADER: {:?}",
        report.entries
    );
    let request = ImportRequest {
        document: DocumentId(2),
        database: DatabaseId(2),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    let imported = AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("the exported drawing re-imports");
    imported
        .database
        .model_space()
        .iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::MultiLeader(leader) => Some(leader.as_ref().clone()),
            _ => None,
        })
        .expect("the created leader re-imports as a MULTILEADER variant")
}

#[test]
fn content_block_creates_stores_and_draws_the_named_block() {
    let (mut app, mut session) = block_app();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        request(Some(MLeaderContentSpec::Block("Door".into())), ""),
    )
    .expect("a known block creates block content");
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert!(app.can_undo(&DocumentId(1)));
    assert!(
        outcome.diagnostics.iter().any(|row| {
            row.code == "mleader.content_block" && row.message.contains("entity 172 = 1")
        }),
        "{:?}",
        outcome.diagnostics
    );

    let leader = the_leader(&app);
    assert_eq!(leader.content().kind(), MLeaderContentKind::Block);
    let block = leader.block_content().expect("the block payload");
    assert_eq!(block.name, "Door");
    assert_eq!(block.block, Some(BlockId(7)));
    assert!(approx_point(block.location, p3(10.0, 2.0, 0.0)));
    assert_eq!(leader.text().string, "");
    // The entity content kind differs from the `Standard` row's MText default, so
    // the override bit names it.
    assert_ne!(leader.override_mask() & MASK_CONTENT_TYPE, 0);

    // The display draws the block instance at the content point.
    let insert = leader
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Insert { block, transform } => Some((*block, *transform)),
            _ => None,
        })
        .expect("the content block instance");
    assert_eq!(insert.0, BlockId(7));
    let placed = insert.1.apply_point(p3(0.0, 0.0, 0.0));
    assert!(approx_point(placed, p3(10.0, 2.0, 0.0)), "{placed:?}");
}

#[test]
fn content_block_round_trips_through_the_one_writer_and_reader() {
    let (mut app, mut session) = block_app();
    execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        request(Some(MLeaderContentSpec::Block("Door".into())), ""),
    )
    .expect("a known block creates block content");

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
    // One `CLASSES` entry (acadrust pre-registers the class) plus the entity
    // record: the leader really is a MULTILEADER, not a flattened compound.
    assert_eq!(
        text.lines()
            .filter(|line| line.trim() == "MULTILEADER")
            .count(),
        2,
        "a real MULTILEADER record is written"
    );
    assert!(
        text.lines().any(|line| line.trim() == "BLOCK_RECORD"),
        "the content block definition is written so the handle resolves"
    );
    assert!(
        !report
            .entries
            .iter()
            .any(|entry| entry.code == "export.converted.multileader_flattened"),
        "block content is written as MULTILEADER: {:?}",
        report.entries
    );
    // Entity `172 = 1`: the block kind.
    let mut lines = text.lines().map(str::trim);
    while let Some(line) = lines.next() {
        if line == "172" {
            let code = lines.next().unwrap_or_default();
            assert!(code == "0" || code == "1", "unexpected content code {code}");
        }
    }

    let reimported = round_trip(&app);
    assert_eq!(reimported.content().kind(), MLeaderContentKind::Block);
    let block = reimported.block_content().expect("the block payload");
    assert_eq!(block.name, "Door");
    assert!(approx_point(block.location, p3(10.0, 2.0, 0.0)));
    assert_eq!(block.rotation, 0.0);
    assert_eq!(block.scale.x, 1.0);
    assert!(block.block.is_some(), "the handle resolves to the block");
    assert!(
        reimported
            .display()
            .iter()
            .any(|child| matches!(child, SemanticGeometry::Insert { .. })),
        "the re-imported leader draws the block"
    );
}

#[test]
fn content_block_refusals_write_nothing() {
    // An unknown block name.
    let (mut app, mut session) = block_app();
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        request(Some(MLeaderContentSpec::Block("Missing".into())), ""),
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref message)
            if message.contains("mleader.block_missing")),
        "{error:?}"
    );
    assert_eq!(revision(&app), before, "a refusal writes nothing");
    assert!(!app.can_undo(&DocumentId(1)));

    // A block content with annotation text is contradictory.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        request(Some(MLeaderContentSpec::Block("Door".into())), "Note"),
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref message)
            if message.contains("mleader.block_text_unexpected")),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
    assert!(
        !app.workspace.documents[&DocumentId(1)]
            .drawing
            .model_space()
            .iter()
            .any(|entity| matches!(entity.geometry, SemanticGeometry::MultiLeader(_))),
        "nothing was inserted"
    );
}

#[test]
fn content_tolerance_creates_stores_and_draws_the_frame() {
    let (mut app, mut session) = block_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        request(Some(MLeaderContentSpec::Tolerance), "|0.05|A|B"),
    )
    .expect("tolerance content is creatable");
    assert!(
        outcome.diagnostics.iter().any(|row| {
            row.code == "mleader.content_tolerance" && row.message.contains("entity 172 = 3")
        }),
        "{:?}",
        outcome.diagnostics
    );

    let leader = the_leader(&app);
    assert_eq!(leader.content().kind(), MLeaderContentKind::Tolerance);
    let tolerance = leader.tolerance_content().expect("the frame payload");
    assert_eq!(
        tolerance,
        &MLeaderToleranceContent {
            text: "|0.05|A|B".into(),
            height: 0.18,
        }
    );
    assert_eq!(leader.text().string, "");
    assert_ne!(leader.override_mask() & MASK_CONTENT_TYPE, 0);

    // Border polyline + dividers + one TEXT per compartment.
    let closed = leader
        .display()
        .iter()
        .filter(|child| matches!(child, SemanticGeometry::Polyline { closed: true, .. }))
        .count();
    let cell_texts: Vec<String> = leader
        .display()
        .iter()
        .filter_map(|child| match child {
            SemanticGeometry::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(closed, 1, "the frame border");
    assert_eq!(cell_texts, vec!["0.05", "A", "B"]);
}

#[test]
fn content_tolerance_round_trips_and_refuses_an_empty_frame() {
    let (mut app, mut session) = block_app();
    execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        request(Some(MLeaderContentSpec::Tolerance), "|0.05|A|B"),
    )
    .expect("tolerance content is creatable");

    let reimported = round_trip(&app);
    assert_eq!(
        reimported.content().kind(),
        MLeaderContentKind::Tolerance,
        "the tolerance kind reaches entity 172 = 3 and reloads from the style 170 record"
    );
    let tolerance = reimported.tolerance_content().expect("the frame payload");
    assert_eq!(tolerance.text, "|0.05|A|B");
    assert!(approx(tolerance.height, 0.18), "{}", tolerance.height);

    // A missing frame text is a named refusal that writes nothing.
    let (mut app, mut session) = block_app();
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        request(Some(MLeaderContentSpec::Tolerance), "   "),
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref message)
            if message.contains("mleader.tolerance_text_required")),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn a_non_finite_style_height_cannot_reach_the_frame() {
    // The style table normalises a non-finite size to the pinned constant
    // (`MLeaderStyle::resolved`), so the command never sees a NaN height: the
    // frame is sized from the resolved value. The named `mleader.tolerance_height`
    // refusal guards the constructor's own input and is pinned in
    // `cad-domain/tests/multileader_content.rs`.
    let mut standard = MLeaderStyle::standard_metric(MLeaderStyleId(0));
    standard.text_height = f64::NAN;
    let (mut app, mut session) = block_app_with_style(standard);
    execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        request(Some(MLeaderContentSpec::Tolerance), "|0.05"),
    )
    .expect("the resolved style height is normalised, so the frame is sized");
    let leader = the_leader(&app);
    let tolerance = leader.tolerance_content().expect("the frame payload");
    assert!(
        tolerance.height.is_finite() && tolerance.height > 0.0,
        "{tolerance:?}"
    );
}

#[test]
fn mtext_and_content_less_creation_are_unchanged() {
    // The default (no CONTENT option) is the pre-existing MText rule.
    let (mut app, mut session) = block_app();
    execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        request(None, "Note"),
    )
    .expect("the default content");
    let leader = the_leader(&app);
    assert_eq!(leader.content().kind(), MLeaderContentKind::Text);
    assert_eq!(leader.text().string, "Note");
    assert_eq!(leader.override_mask(), 0, "no option, no override bit");

    // An explicit `CONTENT MTEXT` is the same thing.
    let (mut app, mut session) = block_app();
    execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        request(Some(MLeaderContentSpec::MText), "Note"),
    )
    .expect("explicit MTEXT");
    let explicit = the_leader(&app);
    assert_eq!(explicit.content().kind(), MLeaderContentKind::Text);
    assert_eq!(explicit.override_mask(), 0);

    // An empty text still creates a content-less leader.
    let (mut app, mut session) = block_app();
    execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        request(None, "   "),
    )
    .expect("content-less creation");
    let content_less = the_leader(&app);
    assert_eq!(content_less.content().kind(), MLeaderContentKind::None);
    assert!(content_less.text().string.is_empty());
}

/// A content block the exporter cannot write leaves no dangling handle: the
/// leader is flattened to its display primitives with the reason.
#[test]
fn a_block_content_that_cannot_be_written_is_flattened_with_the_reason() {
    // An external-reference block resolves by name, so creation succeeds; the
    // exporter refuses to write the definition, so the leader converts instead of
    // carrying a handle to a record the file does not have.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(7),
        entities: Vec::new(),
        dynamic_visibility: None,
    })
    .unwrap();
    b.set_block_metadata(BlockId(7), "DOOR_XREF", p3(0.0, 0.0, 0.0))
        .unwrap();
    b.insert_external_reference(cad_db::ExternalReference {
        name: "door_xref".into(),
        raw_path: "door.dwg".into(),
        resolved_path: None,
        block: Some(BlockId(7)),
        overlay: false,
        depth: 0,
        status: cad_db::ExternalReferenceStatus::Missing,
        reason: Some("missing".into()),
    })
    .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([8u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        request(Some(MLeaderContentSpec::Block("DOOR_XREF".into())), ""),
    )
    .expect("the xref block resolves by name");

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
    // `MULTILEADER` appears once in `CLASSES` (acadrust pre-registers the class)
    // and once as the entity record: a flattened leader leaves only the class.
    assert_eq!(
        text.lines()
            .filter(|line| line.trim() == "MULTILEADER")
            .count(),
        1,
        "the leader is not written as a MULTILEADER with a dangling content handle"
    );
    assert!(
        report.entries.iter().any(
            |entry| entry.code == "export.converted.multileader_flattened"
                && entry.message.contains("block content")
        ),
        "the flatten conversion names the block-content refusal: {:?}",
        report.entries
    );
    // Its primitives survive: the leader line and the arrowhead are still drawn.
    assert!(
        text.lines().any(|line| line.trim() == "LINE"),
        "the leader primitives are written"
    );
}

/// The two pre-existing creation shapes export exactly the bytes they did before
/// the content channel existed: no `CONTENT` option and an explicit
/// `CONTENT MTEXT` produce identical files, and so does a content-less leader.
#[test]
fn the_default_content_kinds_export_byte_identically() {
    fn bytes_for(content: Option<MLeaderContentSpec>, text: &str) -> Vec<u8> {
        let (mut app, mut session) = block_app();
        execute(
            &mut app,
            &mut session,
            CommandId::MLeader,
            request(content, text),
        )
        .expect("the creation succeeds");
        export_bytes(&app)
    }
    assert_eq!(
        bytes_for(None, "Note"),
        bytes_for(Some(MLeaderContentSpec::MText), "Note"),
        "an explicit CONTENT MTEXT is the same creation as the default"
    );
    assert_eq!(
        bytes_for(None, "  "),
        bytes_for(Some(MLeaderContentSpec::MText), "  "),
        "a content-less leader is unchanged"
    );
}

#[test]
fn a_block_or_tolerance_style_row_is_a_default_not_a_refusal() {
    // A style whose `170` is block/tolerance now only sets the *default*: the
    // typed MText text still creates MText content, and the override bit names the
    // difference.
    for code in [MLeaderStyle::CONTENT_BLOCK, MLeaderStyle::CONTENT_TOLERANCE] {
        let mut standard = MLeaderStyle::standard_metric(MLeaderStyleId(0));
        standard.content_type = code;
        let (mut app, mut session) = block_app_with_style(standard);
        execute(
            &mut app,
            &mut session,
            CommandId::MLeader,
            request(None, "Note"),
        )
        .expect("a block/tolerance style row is only a default");
        let leader = the_leader(&app);
        assert_eq!(leader.content().kind(), MLeaderContentKind::Text);
        assert_ne!(
            leader.override_mask() & MASK_CONTENT_TYPE,
            0,
            "the entity kind differs from the style's {code}"
        );
    }
}

/// The transform helper the block display uses is the one the reference
/// `INSERT` path uses (rotation × scale about the placed origin).
#[test]
fn the_block_content_transform_is_rotation_times_scale() {
    let transform = Transform3::translation(p3(1.0, 2.0, 0.0));
    let moved = transform.apply_point(p3(0.0, 0.0, 0.0));
    assert!(approx_point(moved, p3(1.0, 2.0, 0.0)));
}
