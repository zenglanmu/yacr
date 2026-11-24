//! Wave 5 attribute/xref contracts: `ATTDISP`, `BATTMAN`, `BATTORDER`, `XREF`,
//! `-XREF`, `EXTERNALREFERENCES`, `EXTERNALREFERENCESCLOSE`.
//!
//! `ATTDISP` sets one session display mode and writes nothing. The reports are
//! pure reads of the block table and the import-time external-reference
//! provenance: no transaction, no revision bump, no undo step and no
//! `ChangeSet`. Each test pins the diagnostic codes so a report can never
//! silently invent a row. The attribute-manager contract itself lives in
//! [`crate::tests::attribute_commands`].

use super::*;
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, ExternalReference,
    ExternalReferenceStatus, Layer,
};

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    ext: ExtendedCommandId,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Extended(ext), CommandPayload::None),
    )
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn diagnostic<'a>(outcome: &'a CommandOutcome, code: &str) -> &'a Diagnostic {
    outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("missing {code}"))
}

fn count_code(outcome: &CommandOutcome, code: &str) -> usize {
    outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == code)
        .count()
}

fn line_entity(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Block(BlockId(1)),
        geometry: SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        draw_order: 0,
    }
}

/// A drawing with one named block (`CHAIR`) that owns one member entity.
fn application_with_block() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder.insert_entity(line_entity(10)).unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(10)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(BlockId(1), "CHAIR", point(0.0, 0.0))
        .unwrap();
    let drawing = builder.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

/// A drawing whose database carries one loaded xref and one missing one.
fn application_with_xrefs() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_external_reference(ExternalReference {
            name: "XREF-A".into(),
            raw_path: r".\xref\base.dwg".into(),
            resolved_path: Some("/work/xref/base.dwg".into()),
            block: None,
            overlay: false,
            depth: 0,
            status: ExternalReferenceStatus::Loaded,
            reason: None,
        })
        .unwrap();
    builder
        .insert_external_reference(ExternalReference {
            name: "XREF-B".into(),
            raw_path: r".\xref\missing.dwg".into(),
            resolved_path: None,
            block: None,
            overlay: true,
            depth: 1,
            status: ExternalReferenceStatus::Missing,
            reason: Some("file-not-found".into()),
        })
        .unwrap();
    let drawing = builder.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

#[test]
fn attdisp_sets_a_session_display_mode_and_writes_nothing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    assert_eq!(
        session.drafting_extras.attribute_display(),
        crate::AttributeDisplayMode::Normal,
        "ATTDISP defaults to Normal"
    );

    let off =
        execute(&mut app, &mut session, ExtendedCommandId::AttDisp).expect("ATTDISP is admitted");
    assert!(off.changes.is_none());
    assert_eq!(
        session.drafting_extras.attribute_display(),
        crate::AttributeDisplayMode::Off
    );
    let message = &diagnostic(&off, "attdisp.mode").message;
    assert!(message.contains("OFF"), "{message}");
    assert!(message.contains("session display setting"), "{message}");
    assert!(message.contains("no drawing row"), "{message}");
    assert!(!message.contains("no block-attribute model"), "{message}");

    let normal =
        execute(&mut app, &mut session, ExtendedCommandId::AttDisp).expect("ATTDISP is admitted");
    assert_eq!(
        session.drafting_extras.attribute_display(),
        crate::AttributeDisplayMode::Normal
    );
    assert!(diagnostic(&normal, "attdisp.mode")
        .message
        .contains("NORMAL"));

    assert_eq!(revision(&app), before, "ATTDISP writes no drawing");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn battman_reports_that_no_block_has_attributes() {
    let (mut app, mut session) = application_with_block();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::BattMan).expect("BATTMAN is admitted");
    assert!(outcome.changes.is_none());
    assert!(diagnostic(&outcome, "battman.no_attributes")
        .message
        .contains("no block"));
    assert!(diagnostic(&outcome, "battman.summary")
        .message
        .contains("0 block(s)"));
    assert_eq!(count_code(&outcome, "battman.block"), 0);
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn battorder_lists_no_order_when_no_block_has_attributes() {
    let (mut app, mut session) = application_with_block();
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::BattOrder)
        .expect("BATTORDER is admitted");
    assert_eq!(count_code(&outcome, "battorder.entry"), 0);
    assert!(diagnostic(&outcome, "battorder.summary")
        .message
        .contains("0 block(s)"));
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn xref_reports_real_provenance_under_its_own_codes() {
    let (mut app, mut session) = application_with_xrefs();
    let before = revision(&app);

    let xref = execute(&mut app, &mut session, ExtendedCommandId::XRef).expect("XREF is admitted");
    assert!(xref.changes.is_none());
    assert_eq!(count_code(&xref, "xref.entry"), 2);
    let first = diagnostic(&xref, "xref.entry");
    assert!(first.message.contains("XREF-A"), "{}", first.message);
    assert!(first.message.contains("loaded"), "{}", first.message);
    assert!(diagnostic(&xref, "xref.summary")
        .message
        .contains("2 external reference(s)"));
    assert!(diagnostic(&xref, "xref.summary")
        .message
        .contains("NOT IMPLEMENTED"));
    assert_eq!(count_code(&xref, "-xref.entry"), 0);

    let dash =
        execute(&mut app, &mut session, ExtendedCommandId::DashXRef).expect("-XREF is admitted");
    assert_eq!(count_code(&dash, "-xref.entry"), 2);
    assert_eq!(count_code(&dash, "xref.entry"), 0);
    let missing = dash
        .diagnostics
        .iter()
        .find(|entry| entry.message.contains("XREF-B"))
        .expect("XREF-B is listed");
    assert!(missing.message.contains("missing"), "{}", missing.message);
    assert!(
        missing.message.contains("file-not-found"),
        "{}",
        missing.message
    );

    let external = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::ExternalReferences,
    )
    .expect("EXTERNALREFERENCES is admitted");
    assert_eq!(count_code(&external, "externalreferences.entry"), 2);
    assert!(diagnostic(&external, "externalreferences.summary")
        .message
        .contains("2 external reference(s)"));
    assert!(
        diagnostic(&external, "externalreferences.palette_not_implemented")
            .message
            .contains("no External References palette")
    );

    assert_eq!(revision(&app), before, "xref reports write no drawing");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn classicxref_lists_the_same_rows_and_names_the_unsupported_actions() {
    let (mut app, mut session) = application_with_xrefs();
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::ClassicXRef)
        .expect("CLASSICXREF is admitted");
    assert!(outcome.changes.is_none());
    assert_eq!(count_code(&outcome, "classicxref.entry"), 2);
    assert!(diagnostic(&outcome, "classicxref.entry")
        .message
        .contains("XREF-A"));
    assert!(diagnostic(&outcome, "classicxref.summary")
        .message
        .contains("2 external reference(s)"));
    assert!(diagnostic(&outcome, "classicxref.attach_unsupported")
        .message
        .contains("attaching"));
    assert!(diagnostic(&outcome, "classicxref.detach_unsupported")
        .message
        .contains("detaching"));
    assert!(diagnostic(&outcome, "classicxref.reload_unsupported")
        .message
        .contains("reloading"));
    assert_eq!(revision(&app), before, "CLASSICXREF writes nothing");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn xref_on_a_drawing_without_references_reports_zero_rows() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    for (ext, summary, entry) in [
        (ExtendedCommandId::XRef, "xref.summary", "xref.entry"),
        (ExtendedCommandId::DashXRef, "-xref.summary", "-xref.entry"),
        (
            ExtendedCommandId::ExternalReferences,
            "externalreferences.summary",
            "externalreferences.entry",
        ),
    ] {
        let outcome = execute(&mut app, &mut session, ext).expect("report is admitted");
        assert_eq!(count_code(&outcome, entry), 0, "no xref is invented");
        assert!(diagnostic(&outcome, summary)
            .message
            .contains("0 external reference(s)"));
    }
    assert_eq!(revision(&app), before);
}

#[test]
fn externalreferencesclose_is_a_named_noop() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::ExternalReferencesClose,
    )
    .expect("EXTERNALREFERENCESCLOSE is admitted");
    assert!(outcome.changes.is_none());
    assert!(
        diagnostic(&outcome, "externalreferences.close_not_implemented")
            .message
            .contains("nothing to close")
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn a_wave5_report_payload_is_refused_and_changes_nothing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let refused = app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::BattMan),
            CommandPayload::Points(vec![point(1.0, 2.0)]),
        ),
    );
    assert!(refused.is_err(), "BATTMAN takes no payload");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn attdisp_typed_modes_are_session_state_only() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    for mode in [
        crate::AttributeDisplayMode::On,
        crate::AttributeDisplayMode::Normal,
        crate::AttributeDisplayMode::Off,
    ] {
        let outcome = app
            .execute(
                &mut session,
                command(
                    CommandId::Extended(ExtendedCommandId::AttDisp),
                    CommandPayload::Attribute(crate::AttributeCommand::Display(mode)),
                ),
            )
            .expect("ATTDISP accepts its typed mode");
        assert!(outcome.changes.is_none());
        assert_eq!(session.drafting_extras.attribute_display(), mode);
        assert!(outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "attdisp.mode"));
    }
    assert_eq!(revision(&app), before, "ATTDISP never writes the drawing");
    assert!(!app.can_undo(&DocumentId(1)));
}
