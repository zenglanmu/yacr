//! Wave 13 contracts: page-setup/conversion reports and named refusals.
//!
//! Bare `PAGESETUP`/`-PAGESETUP` report stored `PLOTSETTINGS` records and
//! `CONVERTPOLY` reports the single polyline representation; both write nothing.
//! `PSETUPIN`/`-PSETUPIN` and the typed `PAGESETUP` form are writes covered by
//! `layout_page_setup`. The rest are stable named refusals.

use super::*;

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

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

#[test]
fn pagesetup_reports_stored_records_without_writing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    for ext in [
        ExtendedCommandId::PageSetup,
        ExtendedCommandId::DashPageSetup,
    ] {
        let outcome = execute(&mut app, &mut session, ext).expect("PAGESETUP is a report");
        assert!(outcome.changes.is_none());
        assert!(outcome
            .diagnostics
            .iter()
            .any(|row| row.code == "pagesetup.report_only"));
        assert_eq!(revision(&app), before);
        assert!(!app.can_undo(&DocumentId(1)));
    }
}

#[test]
fn convertpoly_reports_the_single_representation() {
    let (mut app, mut session) = application_with_document();
    let outcome = execute(&mut app, &mut session, ExtendedCommandId::ConvertPoly).expect("report");
    assert!(outcome.changes.is_none());
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "convertpoly.single_representation"));
}

const REFUSALS: [(ExtendedCommandId, &str); 15] = [
    (
        ExtendedCommandId::PlotStyleList,
        "plotstyle.unsupported: named plot styles are not implemented (no CTB/STB table)",
    ),
    (
        ExtendedCommandId::DashPlotStyleList,
        "-plotstyle.unsupported: named plot styles are not implemented (no CTB/STB table)",
    ),
    (
        ExtendedCommandId::ConvertPlotStyles,
        "convertpstyles.unsupported: plot-style conversion is not implemented (no CTB/STB model)",
    ),
    (
        ExtendedCommandId::ConvertCtb,
        "convertctb.unsupported: CTB conversion is not implemented (no CTB/STB model)",
    ),
    (
        ExtendedCommandId::ConvertGeneric,
        "convert.unsupported: generic object conversion is not implemented",
    ),
    (
        ExtendedCommandId::TableExport,
        "tableexport.unsupported: no CSV/file writer is implemented",
    ),
    (
        ExtendedCommandId::TableStyleList,
        "tablestyle.unsupported: no table-style table is implemented",
    ),
    (
        ExtendedCommandId::TableInsert,
        "tinsert.unsupported: no block-in-cell creation is implemented",
    ),
    (
        ExtendedCommandId::MatchCell,
        "matchcell.unsupported: no cell-style copy surface is implemented",
    ),
    (
        ExtendedCommandId::Field,
        "field.unsupported: fields are not implemented (no field model)",
    ),
    (
        ExtendedCommandId::RText,
        "rtext.unsupported: remote text is not implemented (no RTEXT model)",
    ),
    (
        ExtendedCommandId::RTextEdit,
        "rtedit.unsupported: remote text is not implemented (no RTEXT model)",
    ),
    (
        ExtendedCommandId::Spell,
        "spell.unsupported: no spell-check dictionary or model is implemented",
    ),
    (
        ExtendedCommandId::SuperHatch,
        "superhatch.unsupported: image/block-based super-hatching is not implemented",
    ),
    (
        ExtendedCommandId::LayTrans,
        "laytrans.unsupported: layer translation standards are not implemented",
    ),
];

#[test]
fn every_wave13_refusal_answers_with_its_exact_stable_reason() {
    for (ext, expected) in REFUSALS {
        let (mut app, mut session) = application_with_document();
        let before = revision(&app);
        let error = execute(&mut app, &mut session, ext).expect_err("a refusal never succeeds");
        match error {
            CadError::Unsupported(reason) => assert_eq!(reason, expected),
            other => panic!("{ext:?} must be Unsupported, got {other:?}"),
        }
        assert_eq!(revision(&app), before);
        assert!(!app.can_undo(&DocumentId(1)));
        assert!(!ext.requires_work_mode(), "{ext:?} stays Viewer-legal");
    }
}

#[test]
fn wave13_conversion_report_is_viewer_legal() {
    let ext = ExtendedCommandId::ConvertPoly;
    let (mut app, mut session) = application_with_document();
    session
        .switch_mode(AppMode::Viewer)
        .expect("Viewer is legal");
    execute(&mut app, &mut session, ext).unwrap_or_else(|error| panic!("{ext:?}: {error:?}"));
}

// ---------------------------------------------------------------------------
// `ALIGNSPACE` / `VPSYNC`: real viewport-alignment writes.
// ---------------------------------------------------------------------------

fn paper_rect(min: (f64, f64), max: (f64, f64)) -> Vec<Point3> {
    vec![
        Point3 {
            x: min.0,
            y: min.1,
            z: 0.0,
        },
        Point3 {
            x: max.0,
            y: min.1,
            z: 0.0,
        },
        Point3 {
            x: max.0,
            y: max.1,
            z: 0.0,
        },
        Point3 {
            x: min.0,
            y: max.1,
            z: 0.0,
        },
    ]
}

fn viewport(mpp: f64, tx: f64, ty: f64) -> PaperViewport {
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = mpp;
    matrix[1][1] = mpp;
    matrix[0][3] = tx;
    matrix[1][3] = ty;
    PaperViewport {
        clip: paper_rect((0.0, 0.0), (100.0, 50.0)),
        model_to_paper: Transform3 { matrix },
        completeness: Completeness::Complete,
        layer_overrides: std::collections::BTreeMap::new(),
    }
}

/// A document with layer `0` and layout `Sheet1` holding `viewports`.
fn layout_fixture(viewports: Vec<PaperViewport>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet1".into(),
            viewports,
        })
        .unwrap();
    let drawing = builder.finish().unwrap();
    let (mut app, session) = application_with_document();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

fn switch_to_sheet(app: &mut Application, session: &mut SessionState) {
    app.execute(
        session,
        command(
            CommandId::SwitchSpace,
            CommandPayload::Space(SpaceId::Paper(LayoutId(1))),
        ),
    )
    .expect("the fixture layout is drawable");
}

fn stored_viewports(app: &Application) -> Vec<PaperViewport> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .layout(LayoutId(1))
        .expect("layout exists")
        .viewports
        .clone()
}

fn extended(
    app: &mut Application,
    session: &mut SessionState,
    ext: ExtendedCommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(CommandId::Extended(ext), payload))
}

fn refusal_text(result: CadResult<CommandOutcome>) -> String {
    match result {
        Err(CadError::Unsupported(message)) | Err(CadError::InvalidInput(message)) => message,
        Err(other) => panic!("expected a named refusal, got {other:?}"),
        Ok(outcome) => panic!("expected a refusal, got {outcome:?}"),
    }
}

#[test]
fn alignspace_pans_and_zooms_the_first_viewport_in_one_undo_step() {
    let (mut app, mut session) = layout_fixture(vec![viewport(100.0, 0.0, 0.0)]);
    switch_to_sheet(&mut app, &mut session);
    let before = revision(&app);
    // model (1000, 500) -> paper (10, 20); the second pair only fixes the scale:
    // |paper| = 20 over |model| = 100, so paper_per_model = 0.2 (mpp = 5).
    let outcome = extended(
        &mut app,
        &mut session,
        ExtendedCommandId::AlignSpace,
        CommandPayload::Points(vec![
            point(1000.0, 500.0),
            point(10.0, 20.0),
            point(1100.0, 500.0),
            point(30.0, 20.0),
        ]),
    )
    .expect("ALIGNSPACE aligns the first viewport");
    let codes: Vec<&str> = outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert_eq!(
        codes,
        vec!["alignspace.aligned"],
        "parallel pairs: no rotation"
    );
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert!(app.can_undo(&DocumentId(1)), "one undo step");

    let aligned = stored_viewports(&app)[0].clone();
    assert_eq!(aligned.model_to_paper.matrix[0][0], 5.0);
    assert_eq!(aligned.model_to_paper.matrix[1][1], 5.0);
    assert_eq!(aligned.model_to_paper.matrix[0][3], 950.0);
    assert_eq!(aligned.model_to_paper.matrix[1][3], 400.0);
    assert_eq!(
        aligned.clip,
        paper_rect((0.0, 0.0), (100.0, 50.0)),
        "the clip is untouched"
    );
    // paper = (model - t) / mpp: the aligned model point lands on its paper point.
    let to_paper = |x: f64, y: f64| {
        (
            (x - aligned.model_to_paper.matrix[0][3]) / aligned.model_to_paper.matrix[0][0],
            (y - aligned.model_to_paper.matrix[1][3]) / aligned.model_to_paper.matrix[1][1],
        )
    };
    assert_eq!(to_paper(1000.0, 500.0), (10.0, 20.0));

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo restores the previous alignment");
    assert_eq!(stored_viewports(&app)[0], viewport(100.0, 0.0, 0.0));
}

#[test]
fn alignspace_reports_a_rotation_it_cannot_apply() {
    let (mut app, mut session) = layout_fixture(vec![viewport(100.0, 0.0, 0.0)]);
    switch_to_sheet(&mut app, &mut session);
    // The paper pair runs along +y while the model pair runs along +x: a plan
    // viewport can only pan and zoom, so the rotation is reported, not faked.
    let outcome = extended(
        &mut app,
        &mut session,
        ExtendedCommandId::AlignSpace,
        CommandPayload::Points(vec![
            point(1000.0, 500.0),
            point(10.0, 20.0),
            point(1100.0, 500.0),
            point(10.0, 40.0),
        ]),
    )
    .expect("the pan+zoom part is still applied");
    let codes: Vec<&str> = outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert_eq!(
        codes,
        vec!["alignspace.aligned", "alignspace.rotation_ignored"]
    );
    assert!(outcome.diagnostics[1].message.contains("90"));
}

#[test]
fn alignspace_refuses_bad_input_before_it_writes() {
    let (mut app, mut session) = layout_fixture(vec![viewport(100.0, 0.0, 0.0)]);
    let before = revision(&app);

    // No paper layout is active yet.
    let model = refusal_text(extended(
        &mut app,
        &mut session,
        ExtendedCommandId::AlignSpace,
        CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 1.0)]),
    ));
    assert!(model.contains("alignspace.paper_space_required"), "{model}");

    switch_to_sheet(&mut app, &mut session);
    for payload in [
        CommandPayload::None,
        CommandPayload::Points(vec![point(0.0, 0.0)]),
        CommandPayload::Points(vec![point(0.0, 0.0), point(10.0, 10.0)]),
    ] {
        let error = refusal_text(extended(
            &mut app,
            &mut session,
            ExtendedCommandId::AlignSpace,
            payload,
        ));
        assert!(error.contains("alignspace.points_required"), "{error}");
    }
    let degenerate = refusal_text(extended(
        &mut app,
        &mut session,
        ExtendedCommandId::AlignSpace,
        CommandPayload::Points(vec![
            point(1000.0, 500.0),
            point(10.0, 20.0),
            point(1000.0, 500.0),
            point(30.0, 20.0),
        ]),
    ));
    assert!(
        degenerate.contains("alignspace.degenerate_alignment"),
        "{degenerate}"
    );
    assert_eq!(revision(&app), before, "refusals write nothing");
    assert!(
        !app.can_undo(&DocumentId(1)),
        "refusals record no undo step"
    );

    // A layout with no viewport has nothing to align.
    let (mut empty, mut empty_session) = layout_fixture(Vec::new());
    switch_to_sheet(&mut empty, &mut empty_session);
    let no_viewport = refusal_text(extended(
        &mut empty,
        &mut empty_session,
        ExtendedCommandId::AlignSpace,
        CommandPayload::Points(vec![
            point(1000.0, 500.0),
            point(10.0, 20.0),
            point(1100.0, 500.0),
            point(30.0, 20.0),
        ]),
    ));
    assert!(
        no_viewport.contains("alignspace.no_viewport"),
        "{no_viewport}"
    );
}

#[test]
fn alignspace_and_vpsync_refuse_a_viewer_session() {
    let (mut app, mut session) =
        layout_fixture(vec![viewport(100.0, 0.0, 0.0), viewport(1.0, 0.0, 0.0)]);
    switch_to_sheet(&mut app, &mut session);
    session
        .switch_mode(AppMode::Viewer)
        .expect("Viewer is legal");
    let before = revision(&app);
    // Both selectors are declared Work-only, so the session authorization gate
    // refuses a Viewer before the handler runs: `PermissionDenied`, never a
    // handler-level `*.work_mode_required` string.
    let align = extended(
        &mut app,
        &mut session,
        ExtendedCommandId::AlignSpace,
        CommandPayload::Points(vec![
            point(1000.0, 500.0),
            point(10.0, 20.0),
            point(1100.0, 500.0),
            point(30.0, 20.0),
        ]),
    );
    assert!(matches!(align, Err(CadError::PermissionDenied)), "{align:?}");
    let sync = extended(
        &mut app,
        &mut session,
        ExtendedCommandId::VpSync,
        CommandPayload::None,
    );
    assert!(matches!(sync, Err(CadError::PermissionDenied)), "{sync:?}");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn vpsync_copies_the_master_alignment_in_one_undo_step() {
    let (mut app, mut session) =
        layout_fixture(vec![viewport(100.0, 70.0, 30.0), viewport(1.0, 0.0, 0.0)]);
    switch_to_sheet(&mut app, &mut session);
    let before = revision(&app);
    let outcome = extended(
        &mut app,
        &mut session,
        ExtendedCommandId::VpSync,
        CommandPayload::None,
    )
    .expect("VPSYNC copies viewport 0 onto viewport 1");
    let codes: Vec<&str> = outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert_eq!(codes, vec!["vpsync.applied"]);
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    let viewports = stored_viewports(&app);
    assert_eq!(viewports[1].model_to_paper, viewports[0].model_to_paper);
    assert_eq!(
        viewports[1].clip,
        paper_rect((0.0, 0.0), (100.0, 50.0)),
        "the clip of the synchronized viewport is untouched"
    );

    // A second run has nothing left to copy: named no-op, no revision.
    let written = revision(&app);
    let again = extended(
        &mut app,
        &mut session,
        ExtendedCommandId::VpSync,
        CommandPayload::None,
    )
    .expect("an already-synchronized layout is a named no-op");
    assert_eq!(
        again
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>(),
        vec!["vpsync.unchanged"]
    );
    assert!(again.changes.is_none());
    assert_eq!(revision(&app), written);

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo restores the synchronized viewport");
    assert_eq!(stored_viewports(&app)[1], viewport(1.0, 0.0, 0.0));
}

#[test]
fn vpsync_refuses_a_single_viewport_and_a_non_uniform_master() {
    let (mut single, mut single_session) = layout_fixture(vec![viewport(100.0, 0.0, 0.0)]);
    switch_to_sheet(&mut single, &mut single_session);
    let before = revision(&single);
    let insufficient = refusal_text(extended(
        &mut single,
        &mut single_session,
        ExtendedCommandId::VpSync,
        CommandPayload::None,
    ));
    assert!(
        insufficient.contains("vpsync.insufficient_viewports"),
        "{insufficient}"
    );
    assert_eq!(revision(&single), before);
    assert!(!single.can_undo(&DocumentId(1)));

    // Model space is not a layout the command can synchronize.
    let (mut model, mut model_session) = application_with_document();
    let no_paper = refusal_text(extended(
        &mut model,
        &mut model_session,
        ExtendedCommandId::VpSync,
        CommandPayload::None,
    ));
    assert!(
        no_paper.contains("vpsync.paper_space_required"),
        "{no_paper}"
    );

    // A mirrored master is not a pan+zoom alignment: refuse by name.
    //
    // The fixture stores a viewport the representation layer refuses to draw,
    // so the validated space switch would refuse first; the session is placed on
    // the layout directly to exercise the handler's own defensive guard (an
    // imported or undone viewport can carry such a matrix).
    let mut mirrored = viewport(1.0, 0.0, 0.0);
    mirrored.model_to_paper.matrix[0][0] = -2.0;
    mirrored.model_to_paper.matrix[1][1] = -2.0;
    let (mut app, mut session) = layout_fixture(vec![mirrored, viewport(1.0, 0.0, 0.0)]);
    session.active_space = SpaceId::Paper(LayoutId(1));
    let invalid_master = refusal_text(extended(
        &mut app,
        &mut session,
        ExtendedCommandId::VpSync,
        CommandPayload::None,
    ));
    assert!(
        invalid_master.contains("vpsync.invalid_master"),
        "{invalid_master}"
    );
}
