//! `FINDINCLUDE` scope contracts for `-FIND` / `FIND` (2026-11-24).
//!
//! The session token gates the read-only block channel: ON (AutoCAD's default)
//! searches local block definitions and the blocks a loaded external reference
//! materialized, naming the latter as an external reference; OFF excludes that
//! channel and says so. External references whose content was never attached are
//! always named with their recorded status ("missing", ...), independent of the
//! token. Both directions write nothing and record no undo step.

use super::*;
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, ExternalReference,
    ExternalReferenceStatus, Layer,
};
use cad_domain::Transform3;

const FRAME_BLOCK: BlockId = BlockId(7);
const FRAME_MEMBER: EntityId = EntityId(41);
const XREF_BLOCK: BlockId = BlockId(8);
const XREF_MEMBER: EntityId = EntityId(42);
const MODEL_TEXT: EntityId = EntityId(50);

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn text(value: &str) -> SemanticGeometry {
    SemanticGeometry::Text {
        text: value.into(),
        position: point(0.0, 0.0),
        style: StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: None,
        width_factor: 1.0,
        line_spacing: cad_domain::TextLineSpacing::default(),
        mirrored: false,
    }
}

fn entity(id: EntityId, space: SpaceId, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id.0),
            type_key: "AcDbText".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id,
        layer: LayerId(0),
        space,
        geometry,
        draw_order: 0,
    }
}

fn reference(
    name: &str,
    block: Option<BlockId>,
    status: ExternalReferenceStatus,
) -> ExternalReference {
    ExternalReference {
        name: name.into(),
        raw_path: format!("{name}.dwg"),
        resolved_path: status.is_loaded().then(|| format!("{name}.dwg")),
        block,
        overlay: false,
        depth: 0,
        status,
        reason: (!status.is_loaded()).then(|| format!("xref.{}", status.as_str())),
    }
}

/// `Frame` (local block) and `STRUCT` (loaded external reference) each hold one
/// TEXT; `LOST` never attached; model space holds one editable TEXT.
fn find_app() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: FRAME_BLOCK,
        entities: vec![FRAME_MEMBER],
        dynamic_visibility: None,
    })
    .unwrap();
    b.set_block_metadata(FRAME_BLOCK, "Frame", point(0.0, 0.0))
        .unwrap();
    b.insert_entity(entity(
        FRAME_MEMBER,
        SpaceId::Block(FRAME_BLOCK),
        text("frame label"),
    ))
    .unwrap();
    b.insert_block(BlockDefinition {
        id: XREF_BLOCK,
        entities: vec![XREF_MEMBER],
        dynamic_visibility: None,
    })
    .unwrap();
    b.set_block_metadata(XREF_BLOCK, "STRUCT", point(0.0, 0.0))
        .unwrap();
    b.insert_entity(entity(
        XREF_MEMBER,
        SpaceId::Block(XREF_BLOCK),
        text("STRUCT grid"),
    ))
    .unwrap();
    b.insert_entity(entity(
        MODEL_TEXT,
        SpaceId::Model,
        text("shared word in model space"),
    ))
    .unwrap();
    // A model-space INSERT so the block channel is reachable through a real
    // placement, exactly like the viewport reaches it.
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(60),
            type_key: "AcDbInsert".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(60),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Insert {
            block: FRAME_BLOCK,
            transform: Transform3::identity(),
            attributes: Vec::new(),
        },
        draw_order: 1,
    })
    .unwrap();
    b.insert_external_reference(reference(
        "STRUCT",
        Some(XREF_BLOCK),
        ExternalReferenceStatus::Loaded,
    ))
    .unwrap();
    b.insert_external_reference(reference("LOST", None, ExternalReferenceStatus::Missing))
        .unwrap();
    let drawing = b.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

fn run(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    needle: &str,
    replacement: Option<&str>,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(
            id,
            CommandPayload::TextFind {
                needle: needle.into(),
                replacement: replacement.map(str::to_string),
            },
        ),
    )
}

fn report_message(outcome: &CommandOutcome) -> &str {
    &outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "find.result")
        .expect("a find report")
        .message
}

fn stored(app: &Application, id: EntityId) -> String {
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(id)
        .expect("entity exists")
        .geometry
    {
        SemanticGeometry::Text { text, .. } => text.clone(),
        other => panic!("expected a text geometry, got {other:?}"),
    }
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(cad_history::History::undo_depth)
        .unwrap_or(0)
}

fn toggle(app: &mut Application, session: &mut SessionState) {
    app.execute(
        session,
        command(
            CommandId::Extended(ExtendedCommandId::FindInclude),
            CommandPayload::None,
        ),
    )
    .expect("FINDINCLUDE toggles");
}

/// ON (the default) searches the block-definition channel and names a loaded
/// external reference as such.
#[test]
fn findinclude_on_searches_block_and_loaded_xref_text_and_names_the_xref() {
    let (mut app, mut session) = find_app();
    let before = revision(&app);
    let outcome = run(&mut app, &mut session, CommandId::FindReport, "grid", None).unwrap();
    let message = report_message(&outcome);
    assert!(
        message.contains("0 match(es)"),
        "no top-level hit: {message}"
    );
    assert!(
        message.contains("external reference 'STRUCT': 1 match(es)"),
        "a loaded xref block is named as an external reference: {message}"
    );
    assert!(
        !message.contains("excluded by FINDINCLUDE"),
        "ON must not claim an exclusion: {message}"
    );
    assert_eq!(revision(&app), before, "the report writes nothing");
    assert_eq!(undo_depth(&app), 0);
}

/// OFF excludes block and xref text **by name**; a `0 match(es)` is never a
/// silent miss.
#[test]
fn findinclude_off_excludes_the_block_channel_and_names_the_boundary() {
    let (mut app, mut session) = find_app();
    let before = revision(&app);
    toggle(&mut app, &mut session);
    assert!(!session.drafting_extras.find_include());
    let outcome = run(&mut app, &mut session, CommandId::FindReport, "grid", None).unwrap();
    let message = report_message(&outcome);
    assert!(
        message.contains("excluded by FINDINCLUDE OFF"),
        "the excluded channel is named: {message}"
    );
    assert!(
        !message.contains("external reference 'STRUCT'"),
        "OFF must not report xref text it did not search: {message}"
    );
    assert_eq!(revision(&app), before, "the toggle + report write nothing");
    assert_eq!(undo_depth(&app), 0);
}

/// OFF narrows the *scope*, not the replace path: model text is still replaced
/// in one undo step while block-definition text stays untouched.
#[test]
fn findinclude_off_still_replaces_model_text_and_never_block_text() {
    let (mut app, mut session) = find_app();
    toggle(&mut app, &mut session);
    let before = revision(&app);
    let outcome = run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        "shared",
        Some("kept"),
    )
    .unwrap();
    let message = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "find.replaced")
        .expect("a replace report")
        .message
        .clone();
    assert!(message.contains("model space"), "{message}");
    assert_eq!(stored(&app, MODEL_TEXT), "kept word in model space");
    assert_eq!(
        stored(&app, FRAME_MEMBER),
        "frame label",
        "block text is never edited by FIND"
    );
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");
}

/// A reference whose content was never attached is named with its status,
/// independent of `FINDINCLUDE`.
#[test]
fn find_names_an_unattached_external_reference_as_unsearched() {
    let (mut app, mut session) = find_app();
    let outcome = run(&mut app, &mut session, CommandId::FindReport, "grid", None).unwrap();
    let message = report_message(&outcome);
    assert!(
        message.contains("'LOST' (missing)"),
        "the unloaded reference and its status are named: {message}"
    );
    assert!(
        message.contains("external references not searched (content not attached)"),
        "{message}"
    );

    toggle(&mut app, &mut session);
    let outcome = run(&mut app, &mut session, CommandId::FindReport, "grid", None).unwrap();
    let message = report_message(&outcome);
    assert!(
        message.contains("'LOST' (missing)"),
        "OFF still names the unattached reference: {message}"
    );
}
