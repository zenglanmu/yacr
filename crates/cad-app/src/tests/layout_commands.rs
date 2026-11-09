//! Contract tests for the 2026-11-07 layout / viewport batch.
//!
//! The read commands (`LAYOUT LIST`, `LAYOUT SET`, `VPMAX`, `VPMIN`) and the
//! promoted **writes** (`LAYOUT NEW|RENAME|DELETE`, `LAYOUTMERGE`,
//! `LAYOUTWIZARD`, `VPORTS`, `VPCLIP RECT`, `VPSCALE`) are exercised end to end.
//! Every write is one revision, one `ChangeSet` and one undo step; the round
//! trips assert undo/redo and the status projection. `LAYVPI` freezes real
//! per-viewport layer overrides in every viewport of the active layout and
//! never writes the global layer table.

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer, Layout, PaperViewport, Scale};

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

fn viewport() -> PaperViewport {
    PaperViewport {
        clip: vec![
            point(0.0, 0.0),
            point(100.0, 0.0),
            point(100.0, 50.0),
            point(0.0, 50.0),
        ],
        model_to_paper: Transform3::scale(100.0),
        completeness: Completeness::Complete,
        layer_overrides: std::collections::BTreeMap::new(),
    }
}

/// A stored A4 page setup for one layout (210 x 297 mm, no rotation).
fn plot_settings(layout: u128) -> cad_db::PlotSettingsRecord {
    cad_db::PlotSettingsRecord {
        layout: LayoutId(layout),
        paper_size_name: "ISO_A4".into(),
        paper_width: 210.0,
        paper_height: 297.0,
        margins: cad_db::PlotMargins::uniform(5.0),
        rotation: cad_db::PlotRotation::None,
        scale_numerator: 1.0,
        scale_denominator: 1.0,
        plot_type: cad_db::PlotType::Layout,
        paper_units: cad_db::PlotPaperUnits::Millimeters,
        provenance: cad_db::PlotProvenance::Imported,
    }
}

fn paper_entity(id: u128, layout: u128) -> cad_db::DbEntity {
    cad_db::DbEntity {
        object: cad_db::DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Paper(LayoutId(layout)),
        geometry: SemanticGeometry::Polyline {
            points: vec![point(0.0, 0.0), point(10.0, 0.0)],
            bulges: Vec::new(),
            closed: false,
        },
        draw_order: id as i64,
    }
}

/// A document with layer `0`, `WALLS`, `HIDDEN`, a `1:100` scale, the given
/// paper layouts (each with one viewport and a stored A4 page setup unless
/// `viewports` is false), plus optional paper-space entities.
fn fixture_with_entities(
    layouts: &[(u128, &str, bool)],
    entities: &[(u128, u128)],
) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [
        (LayerId(0), "0"),
        (LayerId(1), "WALLS"),
        (LayerId(2), "HIDDEN"),
    ] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    builder
        .insert_scale(Scale {
            id: ScaleId(1),
            name: "1:100".into(),
            paper_units: 1.0,
            drawing_units: 100.0,
        })
        .unwrap();
    for (id, name, has_viewport) in layouts {
        builder
            .insert_layout(Layout {
                id: LayoutId(*id),
                name: (*name).into(),
                viewports: if *has_viewport {
                    vec![viewport()]
                } else {
                    Vec::new()
                },
            })
            .unwrap();
        builder.set_plot_settings(plot_settings(*id)).unwrap();
    }
    for (id, layout) in entities {
        builder.insert_entity(paper_entity(*id, *layout)).unwrap();
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

fn fixture(layouts: &[(u128, &str, bool)]) -> (Application, SessionState) {
    fixture_with_entities(layouts, &[])
}

fn standard_fixture() -> (Application, SessionState) {
    fixture(&[(1, "Sheet1", true), (2, "Sheet2", true)])
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DOC].drawing.revision()
}

fn undo_available(app: &Application) -> bool {
    app.can_undo(&DOC)
}

fn redo_available(app: &Application) -> bool {
    app.can_redo(&DOC)
}

fn drawing(app: &Application) -> &cad_db::DrawingDatabase {
    app.workspace.documents[&DOC].drawing.as_ref()
}

fn layout_names(app: &Application) -> Vec<String> {
    drawing(app)
        .layouts()
        .map(|layout| layout.name.clone())
        .collect()
}

fn switch_to(app: &mut Application, session: &mut SessionState, name: &str) {
    app.execute(
        session,
        command(
            CommandId::Layout,
            layout_payload(LayoutPayload::Set { name: name.into() }),
        ),
    )
    .expect("a known layout switches");
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
        Err(CadError::Unsupported(message)) | Err(CadError::InvalidInput(message)) => message,
        Err(other) => panic!("expected a named refusal, got {other:?}"),
        Ok(outcome) => panic!("expected a refusal, got {outcome:?}"),
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
fn layout_list_reads_the_layout_table_without_writing() {
    let (mut app, mut session) = standard_fixture();
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::Layout, layout_payload(LayoutPayload::List)),
        )
        .expect("LAYOUT LIST is read-only");
    assert_eq!(codes(&outcome), vec!["layout.list", "layout.list"]);
    assert!(outcome.diagnostics[0].message.contains("Sheet1"));
    assert!(outcome.diagnostics[1].message.contains("Sheet2"));
    assert_eq!(revision(&app), before, "read-only");
    assert!(!undo_available(&app), "no undo entry");
}

#[test]
fn layout_set_switches_space_and_refuses_an_unknown_name() {
    let (mut app, mut session) = standard_fixture();
    let before = revision(&app);
    switch_to(&mut app, &mut session, "Sheet2");
    assert_eq!(session.active_space, SpaceId::Paper(LayoutId(2)));
    assert_eq!(revision(&app), before);
    assert!(!undo_available(&app));

    let error = refusal(app.execute(
        &mut session,
        command(
            CommandId::Layout,
            layout_payload(LayoutPayload::Set {
                name: "Nowhere".into(),
            }),
        ),
    ));
    assert!(error.contains("layout.unknown"), "{error}");
    assert_eq!(session.active_space, SpaceId::Paper(LayoutId(2)));
}

#[test]
fn layout_new_creates_one_undo_step_and_round_trips() {
    let (mut app, mut session) = standard_fixture();
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Layout,
                layout_payload(LayoutPayload::New {
                    name: "Sheet3".into(),
                }),
            ),
        )
        .expect("LAYOUT NEW creates the layout");
    assert_eq!(codes(&outcome), vec!["layout.created"]);
    let changes = outcome.changes.as_ref().expect("one ChangeSet");
    assert_eq!(changes.before, before);
    assert_eq!(
        changes.after,
        Revision(before.0 + 1),
        "exactly one revision"
    );
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert!(drawing(&app).layout(LayoutId(3)).is_some());
    assert!(undo_available(&app));
    assert!(
        crate::render_scene::layout_descriptors(drawing(&app))
            .iter()
            .any(|layout| layout.name == "Sheet3"),
        "the status projection includes the new layout"
    );

    undo(&mut app, &mut session);
    assert_eq!(
        revision(&app),
        Revision(before.0 + 2),
        "undo is one revision"
    );
    assert!(drawing(&app).layout(LayoutId(3)).is_none());
    assert!(redo_available(&app));
    assert!(
        !crate::render_scene::layout_descriptors(drawing(&app))
            .iter()
            .any(|layout| layout.name == "Sheet3"),
        "the status projection excludes the undone layout"
    );

    redo(&mut app, &mut session);
    assert!(drawing(&app).layout(LayoutId(3)).is_some());
    assert_eq!(
        drawing(&app)
            .layout(LayoutId(3))
            .map(|layout| layout.name.clone()),
        Some("Sheet3".to_string())
    );
}

#[test]
fn layout_new_refuses_duplicates_and_illegal_names() {
    let (mut app, mut session) = standard_fixture();
    let before = revision(&app);
    let new = |name: &str| layout_payload(LayoutPayload::New { name: name.into() });
    let duplicate = refusal(app.execute(&mut session, command(CommandId::Layout, new("sheet1"))));
    assert!(duplicate.contains("layout.duplicate_name"), "{duplicate}");
    let illegal = refusal(app.execute(&mut session, command(CommandId::Layout, new("Bad/Name"))));
    assert!(illegal.contains("layout.invalid_name"), "{illegal}");
    assert_eq!(revision(&app), before, "a refusal writes nothing");
    assert!(!undo_available(&app));
}

#[test]
fn layout_rename_keeps_the_plot_settings_key_and_round_trips() {
    let (mut app, mut session) = standard_fixture();
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Layout,
                layout_payload(LayoutPayload::Rename {
                    old: "Sheet1".into(),
                    new: "Plan".into(),
                }),
            ),
        )
        .expect("LAYOUT RENAME renames the layout");
    assert_eq!(codes(&outcome), vec!["layout.renamed"]);
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(
        drawing(&app)
            .layout(LayoutId(1))
            .map(|layout| layout.name.clone()),
        Some("Plan".to_string())
    );
    assert_eq!(
        drawing(&app).plot_settings(LayoutId(1)),
        Some(&plot_settings(1)),
        "plot settings are keyed by id and survive the rename"
    );

    undo(&mut app, &mut session);
    assert_eq!(
        drawing(&app)
            .layout(LayoutId(1))
            .map(|layout| layout.name.clone()),
        Some("Sheet1".to_string()),
        "undo restores the exact old name"
    );
    redo(&mut app, &mut session);
    assert_eq!(
        drawing(&app)
            .layout(LayoutId(1))
            .map(|layout| layout.name.clone()),
        Some("Plan".to_string())
    );
}

#[test]
fn layout_rename_refuses_unknown_and_duplicate_names() {
    let (mut app, mut session) = standard_fixture();
    let before = revision(&app);
    let rename = |old: &str, new: &str| {
        layout_payload(LayoutPayload::Rename {
            old: old.into(),
            new: new.into(),
        })
    };
    let unknown = refusal(app.execute(
        &mut session,
        command(CommandId::Layout, rename("Nowhere", "Sheet3")),
    ));
    assert!(unknown.contains("layout.unknown"), "{unknown}");
    let duplicate = refusal(app.execute(
        &mut session,
        command(CommandId::Layout, rename("Sheet1", "Sheet2")),
    ));
    assert!(duplicate.contains("layout.duplicate_name"), "{duplicate}");
    assert_eq!(revision(&app), before);
    assert!(!undo_available(&app));
}

#[test]
fn layout_delete_removes_row_and_plot_settings_in_one_undo_step() {
    let (mut app, mut session) = standard_fixture();
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Layout,
                layout_payload(LayoutPayload::Delete {
                    name: "Sheet1".into(),
                }),
            ),
        )
        .expect("LAYOUT DELETE deletes the layout");
    assert_eq!(codes(&outcome), vec!["layout.deleted"]);
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert!(drawing(&app).layout(LayoutId(1)).is_none());
    assert!(
        drawing(&app).plot_settings(LayoutId(1)).is_none(),
        "the plot settings cascade with the layout"
    );
    assert_eq!(layout_names(&app), vec!["Sheet2".to_string()]);

    undo(&mut app, &mut session);
    assert!(drawing(&app).layout(LayoutId(1)).is_some());
    assert_eq!(
        drawing(&app).plot_settings(LayoutId(1)),
        Some(&plot_settings(1)),
        "undo restores the stored page setup"
    );
    redo(&mut app, &mut session);
    assert!(drawing(&app).layout(LayoutId(1)).is_none());
    assert!(drawing(&app).plot_settings(LayoutId(1)).is_none());
}

#[test]
fn layout_delete_of_the_active_layout_falls_back_and_clears_the_maximized_marker() {
    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    app.execute(
        &mut session,
        command(CommandId::VpMax, CommandPayload::None),
    )
    .expect("VPMAX in a paper layout with a viewport");
    assert!(session.maximized_viewport().is_some());

    app.execute(
        &mut session,
        command(
            CommandId::Layout,
            layout_payload(LayoutPayload::Delete {
                name: "Sheet1".into(),
            }),
        ),
    )
    .expect("deleting the active layout switches to a remaining one");
    assert_eq!(
        session.active_space,
        SpaceId::Paper(LayoutId(2)),
        "the first remaining paper layout becomes active"
    );
    assert_eq!(
        session.maximized_viewport(),
        None,
        "the maximized marker named the deleted layout"
    );
}

#[test]
fn layout_delete_refuses_unknown_and_the_last_layout() {
    let (mut app, mut session) = standard_fixture();
    let unknown = refusal(app.execute(
        &mut session,
        command(
            CommandId::Layout,
            layout_payload(LayoutPayload::Delete {
                name: "Nowhere".into(),
            }),
        ),
    ));
    assert!(unknown.contains("layout.unknown"), "{unknown}");

    let (mut app, mut session) = fixture(&[(1, "Only", true)]);
    let before = revision(&app);
    let last = refusal(app.execute(
        &mut session,
        command(
            CommandId::Layout,
            layout_payload(LayoutPayload::Delete {
                name: "Only".into(),
            }),
        ),
    ));
    assert!(last.contains("layout.last_layout"), "{last}");
    assert_eq!(revision(&app), before);
    assert!(!undo_available(&app));
}

#[test]
fn layoutmerge_moves_entities_and_deletes_the_source_in_one_undo_step() {
    let (mut app, mut session) =
        fixture_with_entities(&[(1, "Sheet1", true), (2, "Sheet2", true)], &[(10, 1)]);
    switch_to(&mut app, &mut session, "Sheet1");
    app.execute(
        &mut session,
        command(CommandId::VpMax, CommandPayload::None),
    )
    .expect("VPMAX in a paper layout with a viewport");
    let before = revision(&app);

    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::LayoutMerge,
                layout_payload(LayoutPayload::Merge {
                    source: "Sheet1".into(),
                    target: "Sheet2".into(),
                }),
            ),
        )
        .expect("LAYOUTMERGE moves and deletes in one batch");
    assert_eq!(codes(&outcome), vec!["layoutmerge.applied"]);
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert_eq!(
        drawing(&app).entity(EntityId(10)).map(|e| e.space.clone()),
        Some(SpaceId::Paper(LayoutId(2))),
        "the source entity moved into the target layout"
    );
    assert!(drawing(&app).layout(LayoutId(1)).is_none());
    assert_eq!(session.active_space, SpaceId::Paper(LayoutId(2)));
    assert_eq!(session.maximized_viewport(), None);

    undo(&mut app, &mut session);
    assert_eq!(
        drawing(&app).entity(EntityId(10)).map(|e| e.space.clone()),
        Some(SpaceId::Paper(LayoutId(1))),
        "undo restores the entity space"
    );
    assert!(drawing(&app).layout(LayoutId(1)).is_some());

    redo(&mut app, &mut session);
    assert_eq!(
        drawing(&app).entity(EntityId(10)).map(|e| e.space.clone()),
        Some(SpaceId::Paper(LayoutId(2)))
    );
    assert!(drawing(&app).layout(LayoutId(1)).is_none());
}

#[test]
fn layoutmerge_refuses_missing_names_and_the_same_layout() {
    let (mut app, mut session) = standard_fixture();
    let before = revision(&app);
    let merge = |source: &str, target: &str| {
        layout_payload(LayoutPayload::Merge {
            source: source.into(),
            target: target.into(),
        })
    };
    let missing = refusal(app.execute(
        &mut session,
        command(CommandId::LayoutMerge, merge("Nowhere", "Sheet1")),
    ));
    assert!(missing.contains("layout.unknown"), "{missing}");
    let same = refusal(app.execute(
        &mut session,
        command(CommandId::LayoutMerge, merge("Sheet1", "Sheet1")),
    ));
    assert!(same.contains("layoutmerge.same_layout"), "{same}");
    assert_eq!(revision(&app), before);
    assert!(!undo_available(&app));
}

#[test]
fn vports_creates_the_grid_and_undo_restores_the_previous_viewports() {
    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Vports,
                layout_payload(LayoutPayload::Viewports { rows: 2, cols: 3 }),
            ),
        )
        .expect("VPORTS writes a real grid");
    assert_eq!(codes(&outcome), vec!["vports.created"]);
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    let layout = drawing(&app).layout(LayoutId(1)).expect("layout exists");
    assert_eq!(layout.viewports.len(), 6, "rows * cols viewports");
    let cell_width = 210.0 / 3.0;
    let cell_height = 297.0 / 2.0;
    let first = &layout.viewports[0];
    assert_eq!(first.completeness, Completeness::Complete);
    assert_eq!(first.clip.len(), 4);
    assert_eq!(
        first.clip,
        vec![
            point(0.0, 0.0),
            point(cell_width, 0.0),
            point(cell_width, cell_height),
            point(0.0, cell_height),
        ],
        "the clip is four paper corners in the documented order"
    );
    assert_eq!(first.model_to_paper.matrix[0][0], 1.0);
    assert_eq!(first.model_to_paper.matrix[1][1], 1.0);
    // The model origin is the view centre of every cell.
    let view = cad_representation::viewport_transform(first);
    match view {
        cad_representation::ViewportState::Supported(view) => {
            assert_eq!(view.model_anchor, point(0.0, 0.0));
        }
        other => panic!("the generated viewport must be supported, got {other:?}"),
    }

    undo(&mut app, &mut session);
    let layout = drawing(&app).layout(LayoutId(1)).expect("layout exists");
    assert_eq!(layout.viewports.len(), 1, "undo restores the old vector");
    assert_eq!(layout.viewports[0], viewport());
}

#[test]
fn vports_refuses_model_space_and_invalid_grids() {
    let (mut app, mut session) = standard_fixture();
    let grid = LayoutPayload::Viewports { rows: 2, cols: 3 };
    let model = refusal(app.execute(
        &mut session,
        command(CommandId::Vports, layout_payload(grid.clone())),
    ));
    assert!(model.contains("vports.paper_space_required"), "{model}");

    switch_to(&mut app, &mut session, "Sheet1");
    let zero = refusal(app.execute(
        &mut session,
        command(
            CommandId::Vports,
            layout_payload(LayoutPayload::Viewports { rows: 0, cols: 2 }),
        ),
    ));
    assert!(zero.contains("vports.invalid_grid"), "{zero}");
}

#[test]
fn vpclip_reclips_the_first_viewport_and_undo_restores_it() {
    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::VpClip,
                layout_payload(LayoutPayload::ClipRect {
                    min: point(10.0, 10.0),
                    max: point(60.0, 40.0),
                }),
            ),
        )
        .expect("VPCLIP RECT rewrites the first viewport clip");
    assert_eq!(codes(&outcome), vec!["vpclip.applied"]);
    assert_eq!(revision(&app), Revision(before.0 + 1));
    let layout = drawing(&app).layout(LayoutId(1)).expect("layout exists");
    assert_eq!(
        layout.viewports[0].clip,
        vec![
            point(10.0, 10.0),
            point(60.0, 10.0),
            point(60.0, 40.0),
            point(10.0, 40.0),
        ]
    );
    assert_eq!(
        layout.viewports[0].model_to_paper,
        Transform3::scale(100.0),
        "the clip write preserves the transform and completeness"
    );

    undo(&mut app, &mut session);
    let layout = drawing(&app).layout(LayoutId(1)).expect("layout exists");
    assert_eq!(layout.viewports[0], viewport());
}

#[test]
fn vpclip_refuses_degenerate_rectangles_and_a_layout_without_viewports() {
    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    let degenerate = refusal(app.execute(
        &mut session,
        command(
            CommandId::VpClip,
            layout_payload(LayoutPayload::ClipRect {
                min: point(10.0, 10.0),
                max: point(10.0, 20.0),
            }),
        ),
    ));
    assert!(
        degenerate.contains("vpclip.degenerate_rect"),
        "{degenerate}"
    );

    let (mut app, mut session) = fixture(&[(1, "Empty", false)]);
    switch_to(&mut app, &mut session, "Empty");
    let before = revision(&app);
    let no_viewport = refusal(app.execute(
        &mut session,
        command(
            CommandId::VpClip,
            layout_payload(LayoutPayload::ClipRect {
                min: point(0.0, 0.0),
                max: point(100.0, 50.0),
            }),
        ),
    ));
    assert!(no_viewport.contains("vpclip.no_viewport"), "{no_viewport}");
    assert_eq!(revision(&app), before);
}

#[test]
fn vpscale_sets_the_stored_diagonal_and_preserves_the_model_anchor() {
    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::VpScale,
                layout_payload(LayoutPayload::Scale {
                    name: "1:50".into(),
                }),
            ),
        )
        .expect("VPSCALE rewrites the first viewport scale");
    assert_eq!(codes(&outcome), vec!["vpscale.applied"]);
    assert_eq!(revision(&app), Revision(before.0 + 1));
    let scaled = &drawing(&app).layout(LayoutId(1)).unwrap().viewports[0];
    // paper/drawing = 1/50, so the stored paper→model diagonal is 50.
    assert_eq!(scaled.model_to_paper.matrix[0][0], 50.0);
    assert_eq!(scaled.model_to_paper.matrix[1][1], 50.0);
    // Old anchor = 100 * (50, 25) = (5000, 2500); new translation =
    // anchor - 50 * (50, 25) = (2500, 1250).
    assert_eq!(scaled.model_to_paper.matrix[0][3], 2500.0);
    assert_eq!(scaled.model_to_paper.matrix[1][3], 1250.0);
    let view = cad_representation::viewport_transform(scaled);
    match view {
        cad_representation::ViewportState::Supported(view) => {
            assert_eq!(view.model_anchor, point(5000.0, 2500.0));
            assert!((view.paper_per_model - 0.02).abs() < 1e-12);
        }
        other => panic!("the scaled viewport must stay supported, got {other:?}"),
    }

    undo(&mut app, &mut session);
    assert_eq!(
        drawing(&app).layout(LayoutId(1)).unwrap().viewports[0],
        viewport()
    );
}

#[test]
fn vpscale_refuses_a_layout_without_viewports() {
    let (mut app, mut session) = fixture(&[(1, "Empty", false)]);
    switch_to(&mut app, &mut session, "Empty");
    let before = revision(&app);
    let no_viewport = refusal(app.execute(
        &mut session,
        command(
            CommandId::VpScale,
            layout_payload(LayoutPayload::Scale {
                name: "1:100".into(),
            }),
        ),
    ));
    assert!(no_viewport.contains("vpscale.no_viewport"), "{no_viewport}");
    assert_eq!(revision(&app), before);
}

#[test]
fn vpscale_refuses_invalid_scales_before_writing() {
    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    let before = revision(&app);
    let unknown = refusal(app.execute(
        &mut session,
        command(
            CommandId::VpScale,
            layout_payload(LayoutPayload::Scale {
                name: "NOPE".into(),
            }),
        ),
    ));
    assert!(unknown.contains("vpscale.unknown_scale"), "{unknown}");
    let zero = refusal(app.execute(
        &mut session,
        command(
            CommandId::VpScale,
            layout_payload(LayoutPayload::Scale { name: "0".into() }),
        ),
    ));
    assert!(zero.contains("vpscale.zero_scale"), "{zero}");
    let non_finite = refusal(app.execute(
        &mut session,
        command(
            CommandId::VpScale,
            layout_payload(LayoutPayload::Scale { name: "NaN".into() }),
        ),
    ));
    assert!(
        non_finite.contains("vpscale.non_finite_scale"),
        "{non_finite}"
    );
    assert_eq!(revision(&app), before);
    assert!(!undo_available(&app));
}

#[test]
fn layoutwizard_creates_the_layout_and_applies_the_paper_setup() {
    let (mut app, mut session) = standard_fixture();
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::LayoutWizard,
                layout_payload(LayoutPayload::Wizard {
                    name: "Sheet3".into(),
                    paper: Some(PaperSetup {
                        name: "A4".into(),
                        width: 210.0,
                        height: 297.0,
                    }),
                }),
            ),
        )
        .expect("LAYOUTWIZARD creates the layout");
    assert_eq!(
        codes(&outcome),
        vec!["layout.created", "layoutwizard.paper_applied"]
    );
    assert!(outcome.diagnostics[1].message.contains("A4"));
    assert_eq!(revision(&app), Revision(before.0 + 1), "one undo step");
    assert!(drawing(&app).layout(LayoutId(3)).is_some());
    let stored = drawing(&app)
        .plot_settings(LayoutId(3))
        .expect("the wizard wrote the authored page setup");
    assert_eq!(stored.layout, LayoutId(3));
    assert_eq!(stored.paper_size_name, "A4");
    assert_eq!((stored.paper_width, stored.paper_height), (210.0, 297.0));
    assert_eq!(stored.provenance, cad_db::PlotProvenance::Authored);

    undo(&mut app, &mut session);
    assert!(drawing(&app).layout(LayoutId(3)).is_none());

    let no_paper = app
        .execute(
            &mut session,
            command(
                CommandId::LayoutWizard,
                layout_payload(LayoutPayload::Wizard {
                    name: "Sheet4".into(),
                    paper: None,
                }),
            ),
        )
        .expect("LAYOUTWIZARD without paper");
    assert_eq!(codes(&no_paper), vec!["layout.created"]);
}

fn layvpi(layers: &[&str]) -> Command {
    command(
        CommandId::LayVpi,
        layout_payload(LayoutPayload::FreezeViewportLayers {
            layers: layers.iter().map(|name| (*name).to_string()).collect(),
        }),
    )
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DOC)
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn override_of(app: &Application, layout: u128, viewport: usize) -> Option<bool> {
    drawing(app)
        .layout(LayoutId(layout))
        .expect("layout exists")
        .viewports[viewport]
        .layer_overrides
        .get(&LayerId(1))
        .copied()
}

#[test]
fn layvpi_freezes_layers_per_viewport_and_round_trips_through_undo_redo() {
    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    let before = revision(&app);
    let depth = undo_depth(&app);

    let outcome = app
        .execute(&mut session, layvpi(&["WALLS"]))
        .expect("LAYVPI freezes real per-viewport state");
    assert_eq!(codes(&outcome), vec!["layvpi.frozen"]);
    assert!(outcome.diagnostics[0].message.contains("WALLS"));
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert_eq!(undo_depth(&app), depth + 1, "exactly one undo step");
    assert_eq!(override_of(&app, 1, 0), Some(true));
    assert_eq!(override_of(&app, 2, 0), None, "other layout untouched");
    assert!(drawing(&app).layer(LayerId(1)).unwrap().visible);

    undo(&mut app, &mut session);
    assert_eq!(override_of(&app, 1, 0), None, "undo restores the map");
    redo(&mut app, &mut session);
    assert_eq!(override_of(&app, 1, 0), Some(true), "redo reapplies it");
}

#[test]
fn layvpi_refusals_and_the_unchanged_case_write_nothing() {
    let (mut empty, mut empty_session) = fixture(&[(1, "Sheet1", false)]);
    switch_to(&mut empty, &mut empty_session, "Sheet1");
    let empty_before = revision(&empty);
    let no_viewport = refusal(empty.execute(&mut empty_session, layvpi(&["WALLS"])));
    assert!(no_viewport.contains("layvpi.no_viewport"), "{no_viewport}");
    assert_eq!(revision(&empty), empty_before);
    assert!(!undo_available(&empty), "a refusal records nothing");
    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    let before = revision(&app);
    let unknown = refusal(app.execute(&mut session, layvpi(&["NOPE"])));
    assert!(unknown.contains("layvpi.unknown_layer"), "{unknown}");
    assert_eq!(revision(&app), before);
    assert!(!undo_available(&app), "a refusal records nothing");

    app.execute(&mut session, layvpi(&["WALLS"]))
        .expect("the first LAYVPI writes");
    let written = revision(&app);
    let depth = undo_depth(&app);
    let outcome = app
        .execute(&mut session, layvpi(&["WALLS"]))
        .expect("a repeated LAYVPI is a declared no-op");
    assert_eq!(codes(&outcome), vec!["layvpi.unchanged"]);
    assert_eq!(revision(&app), written, "no revision bump");
    assert_eq!(undo_depth(&app), depth, "no new undo step");
}
#[test]
fn vpmax_and_vpmin_round_trip_session_state_only() {
    let (mut app, mut session) = standard_fixture();
    let before = revision(&app);
    let model = refusal(app.execute(
        &mut session,
        command(CommandId::VpMax, CommandPayload::None),
    ));
    assert!(model.contains("vpmax.paper_space_required"), "{model}");

    switch_to(&mut app, &mut session, "Sheet1");
    let maximized = app
        .execute(
            &mut session,
            command(CommandId::VpMax, CommandPayload::None),
        )
        .expect("VPMAX in a paper layout with a viewport");
    assert_eq!(codes(&maximized), vec!["vpmax.maximized"]);
    assert_eq!(
        session.maximized_viewport(),
        Some(LayoutViewportRef {
            layout: LayoutId(1),
            index: 0
        })
    );
    assert_eq!(revision(&app), before);
    assert!(!undo_available(&app));

    let again = app
        .execute(
            &mut session,
            command(CommandId::VpMax, CommandPayload::None),
        )
        .expect("a repeated VPMAX is a named no-op");
    assert_eq!(codes(&again), vec!["vpmax.already_maximized"]);

    let restored = app
        .execute(
            &mut session,
            command(CommandId::VpMin, CommandPayload::None),
        )
        .expect("VPMIN restores");
    assert_eq!(codes(&restored), vec!["vpmin.restored"]);
    assert_eq!(session.maximized_viewport(), None);
    let empty = app
        .execute(
            &mut session,
            command(CommandId::VpMin, CommandPayload::None),
        )
        .expect("VPMIN with an empty state is a named no-op");
    assert_eq!(codes(&empty), vec!["vpmin.nothing_to_restore"]);
}
