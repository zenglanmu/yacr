//! `PDMODE <n>` / `PDSIZE <n>` contracts: the command-line form of the two
//! drawing-global POINT display style sysvars
//! ([`CommandId::SetPointDisplayMode`] / [`CommandId::SetPointDisplaySize`]).
//!
//! The evidence these pin, against the real command funnel
//! (`Application::execute`):
//!
//! * the value reaches the drawing header **raw** (`point_display_mode` /
//!   `point_display_size`), including signs, zero, the `32`/`64`/`96` shape bits
//!   and the `i16` extremes — the database never interprets or clamps;
//! * a real write bumps the document revision exactly once with **no** entity
//!   change, which is the display refresh signal (the overlay derives the POINT
//!   style per base revision) — and writes the already stored value is a
//!   documented no-op (no bump, `(already stored, no change)`);
//! * **no undo entry** is created (`undo_depth()` unchanged): there is no
//!   header-variable patch kind, so the command deliberately records no history
//!   step instead of faking one;
//! * a wrong/absent payload is refused **by name** (`PDMODE`/`PDSIZE` in the
//!   message) and a non-finite `PDSIZE` is a named `InvalidInput`; both change no
//!   value, no revision and no history;
//! * Viewer mode refuses both through the existing document-mutation rule
//!   (`requires_work_mode` + `SessionState::authorize` → `PermissionDenied`).
//!
//! The **textual** grammar (a non-numeric argument, a fractional `PDMODE 3.5`, an
//! out-of-`i16` `PDMODE 40000`) lives one layer up, in
//! `cad_ui_core::session::commands` (`parse_pdmode_argument` /
//! `parse_pdsize_argument`), because `cad-app` sits below `cad-ui-core` in the
//! dependency order and must not own a second command-line grammar. Its contracts
//! are pinned in `cad-ui-core`'s `point_style` session tests; here we pin that no
//! such value can be *constructed* as a payload (`i16`/`f64`) and that the
//! absent-payload refusal is explicit.

use super::*;

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn drawing(app: &Application) -> &cad_db::DrawingDatabase {
    &app.workspace.documents[&DocumentId(1)].drawing
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

fn set_mode(
    app: &mut Application,
    session: &mut SessionState,
    mode: i16,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::SetPointDisplayMode,
        CommandPayload::PointDisplayMode(mode),
    )
}

fn set_size(
    app: &mut Application,
    session: &mut SessionState,
    size: f64,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::SetPointDisplaySize,
        CommandPayload::PointDisplaySize(size),
    )
}

/// `PDMODE <n>` stores the raw mode, bumps the revision once and records no undo
/// entry.
#[test]
fn pdmode_stores_the_raw_mode_and_refreshes_through_the_revision() {
    let (mut app, mut session) = application_with_document();
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);
    assert_eq!(drawing(&app).point_display_mode(), 0, "the DWG default");

    let outcome = set_mode(&mut app, &mut session, 35).expect("35 is a legal $PDMODE");
    assert_eq!(
        drawing(&app).point_display_mode(),
        35,
        "the mode is stored raw (cross plus a requested circle: bits 32|3)"
    );
    assert_eq!(
        revision(&app),
        Revision(before_revision.0 + 1),
        "one revision bump is the display refresh signal"
    );
    assert_eq!(
        outcome
            .changes
            .as_ref()
            .map(|changes| changes.changes.len()),
        Some(0),
        "a header write changes no entity"
    );
    assert_eq!(
        undo_depth(&app),
        before_undo,
        "there is no header patch kind, so no undo entry is recorded"
    );
    let diagnostic = &outcome.diagnostics[0];
    assert_eq!(diagnostic.code, "drawing.point_display_mode");
    assert!(diagnostic.message.contains("35"), "{diagnostic:?}");
}

/// Every `i16` mode is stored verbatim: the database never invents a glyph.
#[test]
fn every_i16_mode_is_stored_without_interpretation() {
    let (mut app, mut session) = application_with_document();
    for mode in [0, 1, 2, 3, 4, 35, 64, 96, -1, i16::MIN, i16::MAX] {
        set_mode(&mut app, &mut session, mode).expect("an i16 is a legal $PDMODE");
        assert_eq!(drawing(&app).point_display_mode(), mode, "{mode}");
    }
}

/// `PDSIZE <n>` stores the raw (signed) size, bumps the revision and records no
/// undo entry; the sign keeps its documented meaning.
#[test]
fn pdsize_stores_the_raw_signed_size() {
    let (mut app, mut session) = application_with_document();
    assert_eq!(drawing(&app).point_display_size(), 0.0, "the DWG default");
    let before_undo = undo_depth(&app);

    for (size, label) in [
        (2.5, "absolute drawing-unit size"),
        (0.0, "viewport-relative"),
        (-20.0, "percentage of the viewport height"),
    ] {
        let before_revision = revision(&app);
        set_size(&mut app, &mut session, size).expect("a finite size is legal $PDSIZE");
        assert_eq!(
            drawing(&app).point_display_size(),
            size,
            "{label} is stored raw"
        );
        assert_eq!(
            revision(&app),
            Revision(before_revision.0 + 1),
            "{label} is one revision bump"
        );
    }
    assert_eq!(
        undo_depth(&app),
        before_undo,
        "a $PDSIZE write is not undoable (no header patch kind)"
    );
}

/// A write of the stored pair is an authoritative no-op: no revision bump, no
/// undo entry, and the diagnostic says so instead of claiming a change.
#[test]
fn rewriting_the_stored_value_is_a_reported_no_op() {
    let (mut app, mut session) = application_with_document();
    set_mode(&mut app, &mut session, 2).expect("seed");
    set_size(&mut app, &mut session, 1.5).expect("seed");
    let settled = revision(&app);
    let before_undo = undo_depth(&app);

    let outcome = set_mode(&mut app, &mut session, 2).expect("the stored mode is admitted");
    assert_eq!(
        revision(&app),
        settled,
        "nothing changed, so no new revision"
    );
    assert_eq!(
        outcome
            .changes
            .as_ref()
            .map(|changes| changes.before == changes.after),
        Some(true),
        "the change set reports an authoritative no-op"
    );
    assert!(
        outcome.diagnostics[0]
            .message
            .contains("already stored, no change"),
        "{:?}",
        outcome.diagnostics[0]
    );

    set_size(&mut app, &mut session, 1.5).expect("the stored size is admitted");
    assert_eq!(revision(&app), settled);
    assert_eq!(undo_depth(&app), before_undo);
}

/// One command writes only its own value: `PDMODE` leaves `$PDSIZE` alone and
/// vice versa (they are two separate sysvars, not one style blob).
#[test]
fn the_two_commands_write_one_value_each() {
    let (mut app, mut session) = application_with_document();
    set_mode(&mut app, &mut session, 3).expect("$PDMODE");
    set_size(&mut app, &mut session, -5.0).expect("$PDSIZE");
    assert_eq!(drawing(&app).point_display_mode(), 3);
    assert_eq!(drawing(&app).point_display_size(), -5.0);

    set_size(&mut app, &mut session, 0.0).expect("$PDSIZE back to the default");
    assert_eq!(
        drawing(&app).point_display_mode(),
        3,
        "a $PDSIZE write never rewrites $PDMODE"
    );
}

/// An absent payload (a bare `PDMODE` / `PDSIZE` on the command line) is refused
/// **by name**, and nothing is written.
#[test]
fn an_absent_payload_is_refused_by_name() {
    for (id, name) in [
        (CommandId::SetPointDisplayMode, "PDMODE"),
        (CommandId::SetPointDisplaySize, "PDSIZE"),
    ] {
        let (mut app, mut session) = application_with_document();
        let before_revision = revision(&app);
        let before_undo = undo_depth(&app);
        let error = execute(&mut app, &mut session, id, CommandPayload::None)
            .expect_err("a bare sysvar name carries no value");
        let text = format!("{error:?}");
        assert!(text.contains(name), "{text} must name {name}");
        assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
        assert_eq!(revision(&app), before_revision, "a refusal writes nothing");
        assert_eq!(undo_depth(&app), before_undo);
    }
}

/// A non-finite `PDSIZE` is a named `InvalidInput` at the command layer too (the
/// database refuses it as well), so a value the display layer would have to
/// sanitise can never be stored by a command.
#[test]
fn a_non_finite_size_is_refused_by_name() {
    let (mut app, mut session) = application_with_document();
    let before_revision = revision(&app);
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = set_size(&mut app, &mut session, bad).expect_err("not a size");
        let text = format!("{error:?}");
        assert!(text.contains("PDSIZE"), "{text} must name PDSIZE");
        assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    }
    assert_eq!(drawing(&app).point_display_size(), 0.0);
    assert_eq!(revision(&app), before_revision, "no partial state");
    assert_eq!(undo_depth(&app), 0);
}

/// Viewer mode refuses both commands through the existing document-mutation rule.
#[test]
fn viewer_mode_refuses_both_commands() {
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).expect("switch mode");
    let before_revision = revision(&app);

    let error = set_mode(&mut app, &mut session, 4).expect_err("Viewer cannot write the header");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    let error = set_size(&mut app, &mut session, 2.0).expect_err("Viewer cannot write the header");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");

    assert_eq!(drawing(&app).point_display_mode(), 0);
    assert_eq!(drawing(&app).point_display_size(), 0.0);
    assert_eq!(revision(&app), before_revision);
}

/// The rule the Viewer refusal uses is the shared `requires_work_mode` set, and
/// both commands are in it (a single rule for every document mutation).
#[test]
fn both_commands_are_document_mutations() {
    for id in [
        CommandId::SetPointDisplayMode,
        CommandId::SetPointDisplaySize,
    ] {
        assert!(id.requires_work_mode(), "{id:?} must be Work-only");
    }
}

/// Work mode admits both: the refusal above is a mode rule, not a missing feature.
#[test]
fn work_mode_admits_both_commands() {
    let (mut app, mut session) = application_with_document();
    assert_eq!(session.mode(), AppMode::Work);
    set_mode(&mut app, &mut session, 4).expect("Work admits $PDMODE");
    set_size(&mut app, &mut session, -10.0).expect("Work admits $PDSIZE");
    assert_eq!(drawing(&app).point_display_mode(), 4);
    assert_eq!(drawing(&app).point_display_size(), -10.0);
}
