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

// ---------------------------------------------------------------------------
// Polygonal viewport writes: `MVIEW` with 3+ points and polygonal `VPCLIP`,
// plus the viewport-targeted `LAYVPI` form.
// ---------------------------------------------------------------------------

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn viewports(app: &Application) -> Vec<cad_db::PaperViewport> {
    drawing(app)
        .layout(LayoutId(1))
        .expect("layout exists")
        .viewports
        .clone()
}

fn mview(points: Vec<Point3>) -> Command {
    command(
        CommandId::Extended(ExtendedCommandId::MView),
        CommandPayload::Points(points),
    )
}

fn switch_to_sheet(app: &mut Application, session: &mut SessionState) {
    app.execute(
        session,
        command(
            CommandId::Layout,
            layout_payload(LayoutPayload::Set {
                name: "Sheet1".into(),
            }),
        ),
    )
    .expect("switch to the paper layout");
}

#[test]
fn mview_appends_a_polygonal_viewport_in_one_undo_step() {
    let (mut app, mut session) = fixture(&[(1, "Sheet1")], true);
    switch_to_sheet(&mut app, &mut session);
    let start = revision(&app);
    let polygon = vec![point(10.0, 20.0), point(70.0, 20.0), point(40.0, 90.0)];
    let outcome = app
        .execute(&mut session, mview(polygon.clone()))
        .expect("MVIEW accepts a polygonal boundary");
    assert_eq!(
        codes(&outcome),
        vec!["mview.created_polygon", "mview.polygon_not_renderable"]
    );
    assert_eq!(revision(&app), Revision(start.0 + 1), "one revision");
    assert!(app.can_undo(&DOC), "one undo step");
    let written = viewports(&app);
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].clip, polygon, "the boundary is stored verbatim");
    assert_eq!(written[0].completeness, Completeness::Complete);
    assert_eq!(written[0].model_to_paper.matrix[0][0], 1.0);
    // The paper centre of the polygon's bounding box is the model origin.
    assert_eq!(written[0].model_to_paper.matrix[0][3], -40.0);
    assert_eq!(written[0].model_to_paper.matrix[1][3], -55.0);

    undo(&mut app, &mut session);
    assert!(viewports(&app).is_empty(), "undo removes the viewport");
    redo(&mut app, &mut session);
    assert_eq!(viewports(&app)[0].clip, polygon);
}

#[test]
fn mview_refuses_degenerate_polygons_by_name() {
    let (mut app, mut session) = fixture(&[(1, "Sheet1")], true);
    switch_to_sheet(&mut app, &mut session);
    let start = revision(&app);
    for (points, expected) in [
        // Collinear vertices: zero-area boundary.
        (
            vec![point(0.0, 0.0), point(10.0, 10.0), point(20.0, 20.0)],
            "mview.degenerate_polygon",
        ),
        // Repeated vertex.
        (
            vec![
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(10.0, 0.0),
                point(0.0, 10.0),
            ],
            "mview.degenerate_polygon",
        ),
        // Bow-tie: edges 0-1 and 2-3 cross.
        (
            vec![
                point(0.0, 0.0),
                point(10.0, 10.0),
                point(10.0, 0.0),
                point(0.0, 10.0),
            ],
            "mview.self_intersecting",
        ),
        // A paper clip boundary cannot leave the paper plane.
        (
            vec![point(0.0, 0.0), point(10.0, 0.0), p3(0.0, 10.0, 5.0)],
            "mview.non_coplanar",
        ),
    ] {
        let error = refusal(app.execute(&mut session, mview(points)));
        assert!(error.contains(expected), "expected {expected}, got {error}");
    }
    assert_eq!(revision(&app), start, "refusals write nothing");
    assert!(!app.can_undo(&DOC));
}

#[test]
fn vpclip_polygon_replaces_the_clip_after_a_mview_and_round_trips() {
    let (mut app, mut session) = fixture(&[(1, "Sheet1")], true);
    switch_to_sheet(&mut app, &mut session);
    app.execute(
        &mut session,
        mview(vec![point(10.0, 20.0), point(60.0, 80.0)]),
    )
    .expect("one rectangular viewport to clip");
    let start = revision(&app);
    let polygon = vec![
        point(5.0, 5.0),
        point(50.0, 5.0),
        point(50.0, 40.0),
        point(20.0, 45.0),
    ];
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::VpClip,
                layout_payload(LayoutPayload::ClipPolygon {
                    points: polygon.clone(),
                }),
            ),
        )
        .expect("polygonal VPCLIP rewrites the first viewport clip");
    assert_eq!(
        codes(&outcome),
        vec!["vpclip.applied_polygon", "vpclip.polygon_not_renderable"]
    );
    assert_eq!(revision(&app), Revision(start.0 + 1), "one revision");
    let clipped = viewports(&app);
    assert_eq!(clipped[0].clip, polygon);
    assert_eq!(
        clipped[0].model_to_paper.matrix[0][0], 1.0,
        "the clip write preserves the transform"
    );

    undo(&mut app, &mut session);
    assert_eq!(
        viewports(&app)[0].clip.len(),
        4,
        "undo restores the rectangle"
    );
    assert_eq!(viewports(&app)[0].clip[0], point(10.0, 20.0));
}

#[test]
fn vpclip_polygon_refuses_bad_boundaries_by_name() {
    let (mut app, mut session) = fixture(&[(1, "Sheet1")], true);
    switch_to_sheet(&mut app, &mut session);
    let start = revision(&app);
    // No viewport yet: the boundary is validated first, then the viewport.
    let no_viewport = refusal(app.execute(
        &mut session,
        command(
            CommandId::VpClip,
            layout_payload(LayoutPayload::ClipPolygon {
                points: vec![point(0.0, 0.0), point(10.0, 0.0), point(0.0, 10.0)],
            }),
        ),
    ));
    assert!(no_viewport.contains("vpclip.no_viewport"), "{no_viewport}");

    app.execute(
        &mut session,
        mview(vec![point(10.0, 20.0), point(60.0, 80.0)]),
    )
    .expect("one viewport to clip");
    let with_viewport = revision(&app);
    for (points, expected) in [
        (
            vec![point(0.0, 0.0), point(10.0, 0.0)],
            "vpclip.degenerate_polygon",
        ),
        (
            vec![point(0.0, 0.0), point(10.0, 10.0), point(20.0, 20.0)],
            "vpclip.degenerate_polygon",
        ),
        (
            vec![
                point(0.0, 0.0),
                point(10.0, 10.0),
                point(10.0, 0.0),
                point(0.0, 10.0),
            ],
            "vpclip.self_intersecting",
        ),
        (
            vec![point(0.0, 0.0), point(10.0, 0.0), p3(0.0, 10.0, 3.0)],
            "vpclip.non_coplanar",
        ),
    ] {
        let error = refusal(app.execute(
            &mut session,
            command(
                CommandId::VpClip,
                layout_payload(LayoutPayload::ClipPolygon { points }),
            ),
        ));
        assert!(error.contains(expected), "expected {expected}, got {error}");
    }
    assert_eq!(revision(&app), with_viewport, "refusals write nothing");
    assert_ne!(revision(&app), start);
}

#[test]
fn layvpi_targeted_form_freezes_only_the_named_viewport() {
    let (mut app, mut session) = fixture(&[(1, "Sheet1")], true);
    switch_to_sheet(&mut app, &mut session);
    app.execute(
        &mut session,
        mview(vec![point(0.0, 0.0), point(50.0, 50.0)]),
    )
    .expect("first viewport");
    app.execute(
        &mut session,
        mview(vec![point(60.0, 0.0), point(100.0, 50.0)]),
    )
    .expect("second viewport");
    let start = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::LayVpi,
                layout_payload(LayoutPayload::FreezeViewportLayersIn {
                    layers: vec!["0".into()],
                    viewport: 1,
                }),
            ),
        )
        .expect("the targeted LAYVPI form freezes one viewport");
    assert_eq!(codes(&outcome), vec!["layvpi.frozen"]);
    assert!(outcome.diagnostics[0].message.contains("viewport 1"));
    assert_eq!(revision(&app), Revision(start.0 + 1), "one revision");
    let frozen = viewports(&app);
    assert_eq!(frozen[0].layer_overrides.get(&LayerId(0)), None);
    assert_eq!(frozen[1].layer_overrides.get(&LayerId(0)), Some(&true));
    assert!(
        drawing(&app).layer(LayerId(0)).unwrap().visible,
        "the global layer table is untouched"
    );

    // The targeted form is how a caller expresses "all except the current one":
    // freezing the other viewport leaves the first one alone.
    app.execute(
        &mut session,
        command(
            CommandId::LayVpi,
            layout_payload(LayoutPayload::FreezeViewportLayersIn {
                layers: vec!["0".into()],
                viewport: 0,
            }),
        ),
    )
    .expect("the other viewport");
    assert_eq!(
        viewports(&app)[0].layer_overrides.get(&LayerId(0)),
        Some(&true)
    );

    let before = revision(&app);
    let unknown = refusal(app.execute(
        &mut session,
        command(
            CommandId::LayVpi,
            layout_payload(LayoutPayload::FreezeViewportLayersIn {
                layers: vec!["0".into()],
                viewport: 9,
            }),
        ),
    ));
    assert!(unknown.contains("layvpi.unknown_viewport"), "{unknown}");
    assert_eq!(revision(&app), before);
}
