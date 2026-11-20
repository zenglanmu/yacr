//! Contract tests for the `LAYOUTWIZARD` create path.
//!
//! Split out of `layout_commands` so that module stays inside the 1000-line
//! code gate. `LAYOUTWIZARD` is the one layout command that creates the layout
//! row, its authored page setup and its bounded viewport arrangement in a
//! **single** revision and undo step, so the tests here pin that atomicity and
//! the two declared boundaries: an arrangement outside the typed set is refused
//! by name, and the printer/plot-device/plot-style/title-block steps are
//! reported as unavailable instead of being applied or silently dropped.

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer, Layout, PaperViewport};

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

/// Two paper layouts (ids 1 and 2), each with one viewport and a stored A4 page
/// setup: the state a wizard run appends a third layout to.
fn fixture() -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for (id, name) in [(1u128, "Sheet1"), (2, "Sheet2")] {
        builder
            .insert_layout(Layout {
                id: LayoutId(id),
                name: name.into(),
                viewports: vec![viewport()],
            })
            .unwrap();
        builder.set_plot_settings(plot_settings(id)).unwrap();
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
            compat: Vec::new(),
            sources: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DOC, [800.0, 600.0]),
    );
    (app, SessionState::new(DOC, AppMode::Work))
}

/// One `LAYOUTWIZARD` command with an optional page setup and arrangement.
fn wizard(name: &str, paper: Option<PaperSetup>, arrangement: Option<&str>) -> Command {
    command(
        CommandId::LayoutWizard,
        layout_payload(LayoutPayload::Wizard {
            name: name.into(),
            paper,
            arrangement: arrangement.map(|spelling| spelling.to_string()),
            unavailable: Vec::new(),
        }),
    )
}

fn a4() -> PaperSetup {
    PaperSetup {
        name: "A4".into(),
        width: 210.0,
        height: 297.0,
    }
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DOC].drawing.revision()
}

fn drawing(app: &Application) -> &cad_db::DrawingDatabase {
    app.workspace.documents[&DOC].drawing.as_ref()
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
        Ok(_) => panic!("the command was expected to be refused"),
        Err(error) => format!("{error}"),
    }
}

fn codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DOC)
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

#[test]
fn layoutwizard_creates_the_layout_and_applies_the_paper_setup() {
    let (mut app, mut session) = fixture();
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::LayoutWizard,
                layout_payload(LayoutPayload::Wizard {
                    name: "Sheet3".into(),
                    paper: Some(a4()),
                    arrangement: None,
                    unavailable: Vec::new(),
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
                    arrangement: None,
                    unavailable: Vec::new(),
                }),
            ),
        )
        .expect("LAYOUTWIZARD without paper");
    assert_eq!(codes(&no_paper), vec!["layout.created"]);
    let viewportless = drawing(&app)
        .layouts()
        .find(|layout| layout.name == "Sheet4")
        .expect("the second wizard run created its layout");
    assert!(
        viewportless.viewports.is_empty(),
        "a wizard run without ARRANGE creates no viewport"
    );
}

/// The wizard with `ARRANGE` and an authored paper setup: the viewports cover
/// the **authored** sheet (not the A4 default) and the whole run is one revision.
#[test]
fn layoutwizard_creates_the_requested_arrangement_on_the_authored_sheet() {
    let (mut app, mut session) = fixture();
    let before = revision(&app);
    let depth = undo_depth(&app);
    let outcome = app
        .execute(
            &mut session,
            wizard(
                "Sheet5",
                Some(PaperSetup {
                    name: "A5".into(),
                    width: 148.0,
                    height: 210.0,
                }),
                Some("TwoVertical"),
            ),
        )
        .expect("the wizard creates the layout and its arrangement");
    assert_eq!(
        codes(&outcome),
        vec![
            "layout.created",
            "layoutwizard.paper_applied",
            "layoutwizard.arrangement_applied",
        ]
    );
    assert!(outcome.diagnostics[2].message.contains("TwoVertical"));
    assert_eq!(
        revision(&app),
        Revision(before.0 + 1),
        "create + paper setup + viewports are one revision"
    );
    assert_eq!(undo_depth(&app), depth + 1, "exactly one undo step");

    let layout = drawing(&app)
        .layout(LayoutId(3))
        .expect("the wizard created the layout");
    assert_eq!(layout.viewports.len(), 2, "TwoVertical is a 1 x 2 grid");
    let cell_width = 148.0 / 2.0;
    assert_eq!(
        layout.viewports[0].clip,
        vec![
            point(0.0, 0.0),
            point(cell_width, 0.0),
            point(cell_width, 210.0),
            point(0.0, 210.0),
        ],
        "the first cell is half of the authored A5 sheet"
    );
    assert_eq!(
        layout.viewports[1].clip,
        vec![
            point(cell_width, 0.0),
            point(148.0, 0.0),
            point(148.0, 210.0),
            point(cell_width, 210.0),
        ]
    );
    for viewport in &layout.viewports {
        assert_eq!(viewport.completeness, Completeness::Complete);
        assert_eq!(viewport.model_to_paper.matrix[0][0], 1.0);
    }
    let stored = drawing(&app)
        .plot_settings(LayoutId(3))
        .expect("the authored page setup is still written");
    assert_eq!((stored.paper_width, stored.paper_height), (148.0, 210.0));
    assert_eq!(stored.provenance, cad_db::PlotProvenance::Authored);
}

/// The arrangement spelling is case-insensitive and every member of the set
/// writes the cells its `VPORTS` grid would write.
#[test]
fn layoutwizard_arrangements_map_to_their_vports_grids() {
    let (mut app, mut session) = fixture();
    for (name, expected, rows, cols) in [
        ("single", 1usize, 1u32, 1u32),
        ("two-horizontal", 2, 2, 1),
        ("ThreeVertical", 3, 1, 3),
        ("three_horizontal", 3, 3, 1),
        ("four", 4, 2, 2),
    ] {
        let layout_name = format!("Arrangement{expected}x{rows}x{cols}");
        let outcome = app
            .execute(&mut session, wizard(&layout_name, Some(a4()), Some(name)))
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            codes(&outcome),
            vec![
                "layout.created",
                "layoutwizard.paper_applied",
                "layoutwizard.arrangement_applied"
            ]
        );
        let layout = drawing(&app)
            .layouts()
            .find(|layout| layout.name == layout_name)
            .expect("the wizard created the layout");
        assert_eq!(layout.viewports.len(), expected, "{name}");
        // Every cell is `cols` wide and `rows` tall on the A4 sheet, and the
        // cells tile the sheet without a gap.
        let cell_width = 210.0 / f64::from(cols);
        let cell_height = 297.0 / f64::from(rows);
        assert_eq!(
            layout.viewports[0].clip,
            vec![
                point(0.0, 0.0),
                point(cell_width, 0.0),
                point(cell_width, cell_height),
                point(0.0, cell_height),
            ],
            "{name}"
        );
        let last = &layout.viewports[expected - 1];
        assert_eq!(
            last.clip[2],
            point(210.0, 297.0),
            "{name}: the last cell ends at the sheet corner"
        );
    }
}

/// One undo removes the layout and its viewports together; redo restores both.
#[test]
fn layoutwizard_undo_removes_the_layout_and_its_viewports_and_redo_restores_them() {
    let (mut app, mut session) = fixture();
    let depth = undo_depth(&app);
    app.execute(&mut session, wizard("Sheet6", Some(a4()), Some("four")))
        .expect("the wizard creates a 2 x 2 arrangement");
    let layout = drawing(&app)
        .layout(LayoutId(3))
        .expect("the wizard created the layout");
    assert_eq!(layout.viewports.len(), 4, "Four is a 2 x 2 grid");
    let created = layout.viewports.clone();
    let stored = drawing(&app).plot_settings(LayoutId(3)).cloned();

    undo(&mut app, &mut session);
    assert_eq!(undo_depth(&app), depth, "undo consumed the one step");
    assert!(
        drawing(&app).layout(LayoutId(3)).is_none(),
        "the layout and its viewports are gone together"
    );
    assert!(drawing(&app).plot_settings(LayoutId(3)).is_none());

    redo(&mut app, &mut session);
    let restored = drawing(&app)
        .layout(LayoutId(3))
        .expect("redo re-creates the layout");
    assert_eq!(restored.viewports, created, "redo restores the same cells");
    assert_eq!(
        drawing(&app).plot_settings(LayoutId(3)).cloned(),
        stored,
        "redo restores the authored page setup"
    );
}

/// An arrangement outside the bounded set is refused by name, nothing is
/// created, and no default arrangement is substituted.
#[test]
fn layoutwizard_refuses_an_unknown_arrangement_by_name_and_writes_nothing() {
    let (mut app, mut session) = fixture();
    let before = revision(&app);
    let refused = refusal(app.execute(
        &mut session,
        wizard("Sheet7", Some(a4()), Some("ThreeRight")),
    ));
    assert!(
        refused.contains("layoutwizard.unknown_arrangement"),
        "{refused}"
    );
    assert!(
        refused.contains("ThreeRight"),
        "the refused value is named: {refused}"
    );
    assert!(
        refused.contains("Single"),
        "the supported set is named: {refused}"
    );
    assert_eq!(revision(&app), before, "a refusal writes nothing");
    assert!(
        drawing(&app).layout(LayoutId(3)).is_none(),
        "no layout is created"
    );
    assert!(!app.can_undo(&DOC), "a refusal records nothing");
}

/// Printers/plot devices and title blocks stay unavailable: the layout is still
/// created with its authored paper settings, and the step is refused by name.
#[test]
fn layoutwizard_reports_the_plot_device_and_title_block_steps_as_unavailable() {
    let (mut app, mut session) = fixture();
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::LayoutWizard,
                layout_payload(LayoutPayload::Wizard {
                    name: "Sheet8".into(),
                    paper: Some(a4()),
                    arrangement: None,
                    unavailable: vec![
                        UnavailableWizardStep::PlotDevice("DWG To PDF.pc3".into()),
                        UnavailableWizardStep::TitleBlock("ISO_A3_TITLEBLOCK".into()),
                    ],
                }),
            ),
        )
        .expect("the layout is still created");
    assert_eq!(
        codes(&outcome),
        vec![
            "layout.created",
            "layoutwizard.paper_applied",
            "layoutwizard.plot_device_not_implemented",
            "layoutwizard.title_block_not_implemented",
        ]
    );
    assert!(
        outcome.diagnostics[2].message.contains("DWG To PDF.pc3")
            && outcome.diagnostics[2].message.contains("NOT applied"),
        "the plot-device refusal names the request and states it was not applied: {}",
        outcome.diagnostics[2].message
    );
    assert!(
        outcome.diagnostics[3].message.contains("ISO_A3_TITLEBLOCK"),
        "{}",
        outcome.diagnostics[3].message
    );
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    let layout = drawing(&app)
        .layout(LayoutId(3))
        .expect("the layout was created with its paper setup");
    assert!(
        layout.viewports.is_empty(),
        "no arrangement was requested, so no viewport is invented"
    );
    let stored = drawing(&app)
        .plot_settings(LayoutId(3))
        .expect("the authored paper setup is applied");
    assert_eq!(stored.paper_size_name, "A4");
    assert_eq!(stored.provenance, cad_db::PlotProvenance::Authored);
}

/// A plot-style-table request is reported with its own code as well.
#[test]
fn layoutwizard_reports_the_plot_style_step_as_unavailable() {
    let (mut app, mut session) = fixture();
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::LayoutWizard,
                layout_payload(LayoutPayload::Wizard {
                    name: "Sheet9".into(),
                    paper: Some(a4()),
                    arrangement: Some("Single".into()),
                    unavailable: vec![UnavailableWizardStep::PlotStyle("monochrome.ctb".into())],
                }),
            ),
        )
        .expect("the layout and its arrangement are still created");
    assert_eq!(
        codes(&outcome),
        vec![
            "layout.created",
            "layoutwizard.paper_applied",
            "layoutwizard.arrangement_applied",
            "layoutwizard.plot_style_not_implemented",
        ]
    );
    let layout = drawing(&app)
        .layout(LayoutId(3))
        .expect("the layout was created");
    assert_eq!(
        layout.viewports.len(),
        1,
        "the arrangement is still applied"
    );
}
