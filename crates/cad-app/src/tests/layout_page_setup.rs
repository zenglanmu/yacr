//! Contract tests for the page-setup / `MVIEW` writes added on top of the
//! layout table: typed `PAGESETUP`, `PSETUPIN` and the `MVIEW` viewport append.
//!
//! Each write is exactly one revision, one `ChangeSet` and one undo step; undo
//! restores the previous plot row/viewport list exactly and redo re-applies it.

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer, Layout, PlotRotation};

const DOC: DocumentId = DocumentId(1);

fn command(id: CommandId, payload: CommandPayload) -> Command {
    Command {
        schema_version: 1,
        id,
        document: DOC,
        viewport: ViewportId(1),
        payload,
    }
}

fn layout_payload(payload: LayoutPayload) -> CommandPayload {
    CommandPayload::Layout(Box::new(payload))
}

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

/// A stored A4 page setup for one layout (210 x 297 mm, no rotation).
fn plot_settings(layout: u128) -> cad_db::PlotSettingsRecord {
    cad_db::PlotSettingsRecord {
        layout: LayoutId(layout),
        paper_size_name: "ISO_A4".into(),
        paper_width: 210.0,
        paper_height: 297.0,
        margins: cad_db::PlotMargins::uniform(5.0),
        rotation: PlotRotation::None,
        scale_numerator: 1.0,
        scale_denominator: 1.0,
        plot_type: cad_db::PlotType::Layout,
        paper_units: cad_db::PlotPaperUnits::Millimeters,
        provenance: cad_db::PlotProvenance::Imported,
    }
}

/// A document with layer `0` and the given paper layouts (each with one stored
/// A4 page setup when `store_plots`, otherwise no stored row).
fn fixture(layouts: &[(u128, &str)], store_plots: bool) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for (id, name) in layouts {
        builder
            .insert_layout(Layout {
                id: LayoutId(*id),
                name: (*name).into(),
                viewports: Vec::new(),
            })
            .unwrap();
        if store_plots {
            builder.set_plot_settings(plot_settings(*id)).unwrap();
        }
    }
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DOC,
        Document {
            id: DOC,
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([0u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DOC, [800.0, 600.0]),
    );
    (app, SessionState::new(DOC, AppMode::Work))
}

fn drawing(app: &Application) -> &cad_db::DrawingDatabase {
    app.workspace.documents[&DOC].drawing.as_ref()
}

fn revision(app: &Application) -> Revision {
    drawing(app).revision()
}

fn undo(app: &mut Application, session: &mut SessionState) {
    app.execute(session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo replays the recorded step");
}

fn redo(app: &mut Application, session: &mut SessionState) {
    app.execute(session, command(CommandId::Redo, CommandPayload::None))
        .expect("redo replays the recorded step");
}

fn refusal(result: CadResult<CommandOutcome>) -> String {
    match result {
        Err(error) => error.to_string(),
        Ok(outcome) => panic!("expected a refusal, got {:?}", outcome.diagnostics),
    }
}

fn codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

#[test]
fn pagesetup_payload_writes_one_revision_and_undo_restores_the_previous_row() {
    let (mut app, mut session) = fixture(&[(1, "Sheet1"), (2, "Sheet2")], true);
    let previous = drawing(&app).plot_settings(LayoutId(1)).cloned();
    let start = revision(&app);

    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::PageSetup),
                layout_payload(LayoutPayload::PageSetup {
                    layout: "Sheet1".into(),
                    paper: PaperSetup {
                        name: "A3".into(),
                        width: 420.0,
                        height: 297.0,
                    },
                    scale: Some((1.0, 50.0)),
                    rotation: PlotRotation::Degrees90,
                }),
            ),
        )
        .expect("PAGESETUP writes the typed page setup");
    assert_eq!(codes(&outcome), vec!["pagesetup.applied"]);
    assert!(outcome.changes.is_some());
    assert_eq!(revision(&app), Revision(start.0 + 1), "one revision");
    assert!(app.can_undo(&DOC), "one undo step");

    let stored = drawing(&app)
        .plot_settings(LayoutId(1))
        .expect("the row was written")
        .clone();
    assert_eq!(stored.layout, LayoutId(1));
    assert_eq!(stored.paper_size_name, "A3");
    assert_eq!((stored.paper_width, stored.paper_height), (420.0, 297.0));
    assert_eq!(stored.scale_numerator, 1.0);
    assert_eq!(stored.scale_denominator, 50.0);
    assert_eq!(stored.rotation, PlotRotation::Degrees90);
    assert_eq!(stored.plot_type, cad_db::PlotType::Layout);
    assert_eq!(stored.paper_units, cad_db::PlotPaperUnits::Millimeters);
    assert!(stored.margins.is_zero());
    assert_eq!(stored.provenance, cad_db::PlotProvenance::Authored);

    undo(&mut app, &mut session);
    assert_eq!(drawing(&app).plot_settings(LayoutId(1)).cloned(), previous);

    redo(&mut app, &mut session);
    assert_eq!(
        drawing(&app).plot_settings(LayoutId(1)).cloned(),
        Some(stored)
    );
}

#[test]
fn psetupin_copies_a_stored_record_and_refuses_a_source_without_one() {
    let (mut app, mut session) = fixture(&[(1, "Sheet1"), (2, "Sheet2")], true);
    let source = drawing(&app).plot_settings(LayoutId(1)).cloned().unwrap();
    let target_before = drawing(&app).plot_settings(LayoutId(2)).cloned();
    let start = revision(&app);

    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::PageSetupIn),
                layout_payload(LayoutPayload::PageSetupIn {
                    source: "Sheet1".into(),
                    target: "Sheet2".into(),
                }),
            ),
        )
        .expect("PSETUPIN copies a stored page setup");
    assert_eq!(codes(&outcome), vec!["psetupin.applied"]);
    assert!(outcome.diagnostics[0].message.contains("Sheet1"));
    assert!(outcome.diagnostics[0].message.contains("Sheet2"));
    assert_eq!(revision(&app), Revision(start.0 + 1));
    let copied = drawing(&app)
        .plot_settings(LayoutId(2))
        .expect("the target row was written")
        .clone();
    assert_eq!(copied.layout, LayoutId(2), "re-keyed to the target");
    assert_eq!(copied.paper_size_name, source.paper_size_name);
    assert_eq!(copied.paper_width, source.paper_width);
    assert_eq!(copied.provenance, cad_db::PlotProvenance::Authored);

    undo(&mut app, &mut session);
    assert_eq!(
        drawing(&app).plot_settings(LayoutId(2)).cloned(),
        target_before
    );

    // A layout with no stored row cannot be an import source: the A4 fallback
    // is never synthesized into a real record.
    let (mut app, mut session) = fixture(&[(1, "Sheet1"), (2, "Sheet2")], false);
    let start = revision(&app);
    let error = refusal(app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::PageSetupIn),
            layout_payload(LayoutPayload::PageSetupIn {
                source: "Sheet1".into(),
                target: "Sheet2".into(),
            }),
        ),
    ));
    assert!(error.contains("psetupin.no_source"), "{error}");
    assert_eq!(revision(&app), start);
    assert!(!app.can_undo(&DOC));
    assert!(drawing(&app).plot_settings(LayoutId(2)).is_none());
}

#[test]
fn mview_appends_a_four_corner_viewport_and_one_undo_step() {
    let (mut app, mut session) = fixture(&[(1, "Sheet1")], true);

    // No active paper layout: named refusal before any geometry check.
    let model = refusal(app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::MView),
            CommandPayload::Points(vec![point(10.0, 20.0), point(60.0, 80.0)]),
        ),
    ));
    assert!(model.contains("mview.paper_space_required"), "{model}");

    app.execute(
        &mut session,
        command(
            CommandId::Layout,
            layout_payload(LayoutPayload::Set {
                name: "Sheet1".into(),
            }),
        ),
    )
    .expect("switch to the paper layout");

    let start = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::MView),
                CommandPayload::Points(vec![point(10.0, 20.0), point(60.0, 80.0)]),
            ),
        )
        .expect("MVIEW appends a viewport");
    assert!(
        codes(&outcome).contains(&"mview.created"),
        "{:?}",
        codes(&outcome)
    );
    assert_eq!(revision(&app), Revision(start.0 + 1), "one revision");

    let viewports = drawing(&app)
        .layout(LayoutId(1))
        .expect("layout exists")
        .viewports
        .clone();
    assert_eq!(viewports.len(), 1, "appended to the empty list");
    let viewport = &viewports[0];
    assert_eq!(viewport.clip.len(), 4, "four paper corners");
    assert_eq!(viewport.clip[0], point(10.0, 20.0));
    assert_eq!(viewport.clip[1], point(60.0, 20.0));
    assert_eq!(viewport.clip[2], point(60.0, 80.0));
    assert_eq!(viewport.clip[3], point(10.0, 80.0));
    assert_eq!(viewport.completeness, Completeness::Complete);
    assert_eq!(viewport.model_to_paper.matrix[0][0], 1.0);
    assert_eq!(viewport.model_to_paper.matrix[1][1], 1.0);
    assert_eq!(
        viewport.model_to_paper.matrix[0][3], -35.0,
        "translation is -paper_center.x"
    );
    assert_eq!(viewport.model_to_paper.matrix[1][3], -50.0);

    undo(&mut app, &mut session);
    assert!(
        drawing(&app)
            .layout(LayoutId(1))
            .unwrap()
            .viewports
            .is_empty(),
        "undo restores the previous viewport list"
    );
    redo(&mut app, &mut session);
    assert_eq!(
        drawing(&app).layout(LayoutId(1)).unwrap().viewports.len(),
        1
    );

    // Degenerate / reversed / wrong-count rectangles are refused without a
    // revision bump or an undo step.
    let after_redo = revision(&app);
    for points in [
        vec![point(60.0, 80.0), point(10.0, 20.0)],
        vec![point(10.0, 20.0), point(10.0, 20.0)],
        vec![point(10.0, 20.0)],
    ] {
        let error = refusal(app.execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::MView),
                CommandPayload::Points(points),
            ),
        ));
        assert!(error.contains("mview.invalid_rect"), "{error}");
        assert_eq!(revision(&app), after_redo);
    }
}
