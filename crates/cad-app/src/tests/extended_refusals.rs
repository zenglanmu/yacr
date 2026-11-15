//! Wave 2 named-refusal contracts: `HYPERLINKOPTIONS`, `INPUTSEARCHOPTIONS`,
//! `-INPUTSEARCHOPTIONS`, `SIGVALIDATE`, `VIEWPLOTDETAILS`.
//!
//! Each name is admitted by the vocabulary and refused with
//! `CadError::Unsupported` carrying a **stable** reason. A refusal is a
//! legitimate partial implementation in this build (same shape as
//! `LAYOUTMERGE`/`DIMBREAK`): it must write nothing, bump no revision and record
//! no history.
//!
//! # Graduated names (2026-11-11)
//!
//! `VPLAYER`, `MVSETUP`, `PSBSCALE` and `PSTSCALE` have left this list: they are
//! real paper-space writes now, dispatched through
//! [`Application::extended_paper_write`] and contract-tested in
//! [`paper_space_writes`] (`tests/drawing/layout_commands.rs`). Their old
//! `*.unsupported` strings are gone on purpose — a graduated command must not
//! keep a refusal string that claims the feature has no model.
//!
//! # Graduated names (2026-11-16)
//!
//! `BASE` and `LIMITS` have left this list too: `cad-db` now stores `$INSBASE`,
//! `$LIMMIN` and `$LIMMAX` (`cad_db::drawing::header_geometry`), so both are real
//! read/write commands and their `base.unsupported`/`limits.unsupported` strings
//! are gone. Their contracts are pinned at the bottom of this module.

use super::*;

// The graduated Wave 2 paper-space writes (`VPLAYER`, `MVSETUP`, `PSBSCALE`,
// `PSTSCALE`, and the real `VPMAX`/`VPMIN` state). Declared here with an
// explicit path so `tests/layout_commands.rs` (at the size ceiling) does not
// have to grow.
#[path = "drawing/layout_commands.rs"]
mod paper_space_writes;

const REFUSALS: [(ExtendedCommandId, &str); 5] = [
    (
        ExtendedCommandId::HyperlinkOptions,
        "hyperlinkoptions.unsupported: this build has no hyperlink storage model",
    ),
    (
        ExtendedCommandId::InputSearchOptions,
        "inputsearchoptions.unsupported: input-search settings are not modeled",
    ),
    (
        ExtendedCommandId::DashInputSearchOptions,
        "-inputsearchoptions.unsupported: input-search settings are not modeled",
    ),
    (
        ExtendedCommandId::SigValidate,
        "sigvalidate.unsupported: this build has no digital-signature model",
    ),
    (
        ExtendedCommandId::ViewPlotDetails,
        "viewplotdetails.unsupported: no plot-details model",
    ),
];

#[test]
fn every_wave2_refusal_returns_its_exact_stable_reason_and_changes_nothing() {
    let (mut app, mut session) = application_with_document();
    let before_revision = app.workspace.documents[&DocumentId(1)].drawing.revision();
    let before_entities = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();

    for (ext, expected) in REFUSALS {
        let refused = app.execute(
            &mut session,
            command(CommandId::Extended(ext), CommandPayload::None),
        );
        match refused {
            Err(CadError::Unsupported(reason)) => {
                assert_eq!(reason, expected, "{} reason must be stable", ext.selector());
            }
            other => panic!("{expected} expected Unsupported, got {other:?}"),
        }
        assert_eq!(
            app.workspace.documents[&DocumentId(1)].drawing.revision(),
            before_revision,
            "{} must not bump the revision",
            ext.selector()
        );
        assert_eq!(
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity_count(),
            before_entities,
            "{} must not write entities",
            ext.selector()
        );
        assert!(
            !app.can_undo(&DocumentId(1)),
            "{} must not record history",
            ext.selector()
        );
    }
}

#[test]
fn a_refusal_payload_is_rejected_before_the_feature_reason() {
    let (mut app, mut session) = application_with_document();
    let refused = app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::HyperlinkOptions),
            CommandPayload::Points(vec![point(1.0, 2.0)]),
        ),
    );
    match refused {
        Err(CadError::InvalidInput(message)) => {
            assert!(
                message.contains("HYPERLINKOPTIONS needs no payload"),
                "{message}"
            );
        }
        other => panic!("expected InvalidInput for a payload, got {other:?}"),
    }
    assert!(!app.can_undo(&DocumentId(1)));
}

// ---------------------------------------------------------------------------
// BASE / LIMITS: the graduated vector-header writes
// ---------------------------------------------------------------------------

/// The stored `$INSBASE` after a `BASE <x> <y> <z>` write.
#[test]
fn base_writes_the_insertion_base_point_in_one_revision_and_reports_it() {
    let (mut app, mut session) = application_with_document();
    let document = DocumentId(1);
    let before_revision = app.workspace.documents[&document].drawing.revision();

    // The bare form is the report and writes nothing; a fresh database stores no
    // value, so it is named rather than printed as an origin.
    let report = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::BasePoint),
                CommandPayload::None,
            ),
        )
        .expect("BASE without a point is the read-only report");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|row| row.code == "base.not_stored"),
        "{:?}",
        report.diagnostics
    );
    assert_eq!(
        app.workspace.documents[&document].drawing.revision(),
        before_revision
    );

    let written = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::BasePoint),
                CommandPayload::Points(vec![point(12.5, -3.0)]),
            ),
        )
        .expect("BASE writes one finite point");
    assert!(written
        .diagnostics
        .iter()
        .any(|row| row.code == "base.applied"));
    assert!(written
        .diagnostics
        .iter()
        .any(|row| row.code == "base.no_history"));
    let drawing = &app.workspace.documents[&document].drawing;
    assert_eq!(
        drawing.insertion_base(),
        Some(Point3 {
            x: 12.5,
            y: -3.0,
            z: 0.0
        })
    );
    assert_eq!(drawing.revision().0, before_revision.0 + 1);

    // `SETVAR INSBASE` reads the same stored value (one owner, one path).
    let setvar = app
        .execute(
            &mut session,
            command(CommandId::SetVar, CommandPayload::SetVar("INSBASE".into())),
        )
        .expect("SETVAR INSBASE reports the stored point");
    assert!(setvar.diagnostics[0].message.contains("12.5,-3,0"));
}

#[test]
fn base_refuses_a_payload_that_is_not_one_point_and_writes_nothing() {
    let (mut app, mut session) = application_with_document();
    let before = app.workspace.documents[&DocumentId(1)].drawing.revision();
    let refused = app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::BasePoint),
            CommandPayload::Points(vec![point(1.0, 2.0), point(3.0, 4.0)]),
        ),
    );
    match refused {
        Err(CadError::InvalidInput(message)) => {
            assert!(message.contains("base.invalid_point"), "{message}")
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        before
    );
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn limits_write_both_corners_in_one_revision_and_report_the_stored_pair() {
    let (mut app, mut session) = application_with_document();
    let document = DocumentId(1);
    let before_revision = app.workspace.documents[&document].drawing.revision();

    app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::DrawingLimits),
            CommandPayload::Points(vec![point(0.0, 0.0), point(420.0, 297.0)]),
        ),
    )
    .expect("LIMITS writes both corners");
    let drawing = &app.workspace.documents[&document].drawing;
    assert_eq!(drawing.model_limits(), Some(([0.0, 0.0], [420.0, 297.0])));
    assert_eq!(drawing.revision().0, before_revision.0 + 1);
    assert!(
        !app.can_undo(&document),
        "a header write records no undo step and says so"
    );

    // `SETVAR LIMMIN`/`LIMMAX` read the stored corners.
    for (name, expected) in [("LIMMIN", "0,0"), ("LIMMAX", "420,297")] {
        let report = app
            .execute(
                &mut session,
                command(CommandId::SetVar, CommandPayload::SetVar(name.into())),
            )
            .expect("the stored corner is reported");
        assert!(
            report.diagnostics[0].message.contains(expected),
            "{name}: {:?}",
            report.diagnostics
        );
    }
}

#[test]
fn limits_refuses_a_non_planar_corner_and_a_single_point() {
    let (mut app, mut session) = application_with_document();
    let before = app.workspace.documents[&DocumentId(1)].drawing.revision();
    for payload in [
        CommandPayload::Points(vec![point(0.0, 0.0)]),
        CommandPayload::Points(vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            point(1.0, 1.0),
        ]),
    ] {
        let refused = app.execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::DrawingLimits),
                payload,
            ),
        );
        assert!(
            matches!(refused, Err(CadError::InvalidInput(_))),
            "expected a named refusal, got {refused:?}"
        );
    }
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        before
    );
}

#[test]
fn base_and_limits_are_work_only() {
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).unwrap();
    for ext in [
        ExtendedCommandId::BasePoint,
        ExtendedCommandId::DrawingLimits,
    ] {
        let refused = app.execute(
            &mut session,
            command(CommandId::Extended(ext), CommandPayload::None),
        );
        assert!(
            matches!(refused, Err(CadError::PermissionDenied)),
            "{} must be Work-only, got {refused:?}",
            ext.selector()
        );
    }
}
