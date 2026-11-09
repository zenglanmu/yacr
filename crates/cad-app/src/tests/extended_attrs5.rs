//! Wave 5 attribute/xref contracts: `ATTDISP`, `BATTMAN`, `BATTORDER`, `XREF`,
//! `-XREF`, `EXTERNALREFERENCES`, `EXTERNALREFERENCESCLOSE`.
//!
//! `ATTDISP` flips one session token and writes nothing. The six reports are
//! pure reads of the block table and the import-time external-reference
//! provenance: no transaction, no revision bump, no undo step and no
//! `ChangeSet`. Each test pins the diagnostic codes so a report can never
//! silently invent a row.

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
fn attdisp_toggles_a_session_token_and_writes_nothing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    assert!(session.drafting_extras.attdisp(), "ATTDISP defaults ON");

    let off =
        execute(&mut app, &mut session, ExtendedCommandId::AttDisp).expect("ATTDISP is admitted");
    assert!(off.changes.is_none());
    assert!(!session.drafting_extras.attdisp());
    let message = &diagnostic(&off, "attdisp.toggled").message;
    assert!(message.contains("OFF"), "{message}");
    assert!(message.contains("no block-attribute model"), "{message}");
    assert!(message.contains("not written"), "{message}");

    let on =
        execute(&mut app, &mut session, ExtendedCommandId::AttDisp).expect("ATTDISP is admitted");
    assert!(session.drafting_extras.attdisp());
    assert!(diagnostic(&on, "attdisp.toggled").message.contains("ON"));

    assert_eq!(revision(&app), before, "ATTDISP writes no drawing");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn battman_lists_block_members_and_names_the_attribute_gap() {
    let (mut app, mut session) = application_with_block();
    let before = revision(&app);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::BattMan).expect("BATTMAN is admitted");
    assert!(outcome.changes.is_none());
    assert_eq!(count_code(&outcome, "battman.entry"), 1);
    let entry = diagnostic(&outcome, "battman.entry");
    assert!(entry.message.contains("CHAIR"), "{}", entry.message);
    assert!(entry.message.contains("members 1"), "{}", entry.message);
    assert!(diagnostic(&outcome, "battman.summary")
        .message
        .contains("1 block definition(s)"));
    assert!(diagnostic(&outcome, "battman.attributes_not_modeled")
        .message
        .contains("NOT MODELED"));
    assert_eq!(count_code(&outcome, "battorder.entry"), 0);
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn battorder_lists_the_same_blocks_under_its_own_codes() {
    let (mut app, mut session) = application_with_block();
    let before = revision(&app);
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::BattOrder)
        .expect("BATTORDER is admitted");
    assert_eq!(count_code(&outcome, "battorder.entry"), 1);
    assert!(diagnostic(&outcome, "battorder.entry")
        .message
        .contains("CHAIR"));
    assert_eq!(count_code(&outcome, "battman.entry"), 0);
    assert!(diagnostic(&outcome, "battorder.summary")
        .message
        .contains("read-only listing"));
    assert!(diagnostic(&outcome, "battorder.attributes_not_modeled")
        .message
        .contains("display order"));
    assert_eq!(revision(&app), before);
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
