//! `VPORTS` standard configurations and their refusals.
//!
//! Split out of `layout_commands.rs` so that file stays inside the code-size
//! gate. The documented Standard set is bounded to the **symmetric** equal-cell
//! arrangements this build's helper can express; every member writes the same
//! `set_layout_viewports` path as `VPORTS <rows> <cols>`, i.e. one revision and
//! one undo step. Named configurations and the asymmetric Standard arrangements
//! are refused by the parser (see the `cad-ui-core` argument tests) and never
//! reach this write path.

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

/// One drawing with a `Sheet1` paper layout that already has one viewport and
/// no stored page setup, so `VPORTS` builds on the documented ISO A4 default
/// (210 x 297 mm).
fn fixture() -> (Application, SessionState) {
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
            viewports: vec![viewport()],
        })
        .unwrap();
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

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DOC].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DOC)
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn drawing(app: &Application) -> &cad_db::DrawingDatabase {
    app.workspace.documents[&DOC].drawing.as_ref()
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

fn execute_configuration(
    app: &mut Application,
    session: &mut SessionState,
    configuration: StandardViewportConfiguration,
) -> CommandOutcome {
    app.execute(
        session,
        command(
            CommandId::Vports,
            layout_payload(LayoutPayload::ViewportsConfiguration { configuration }),
        ),
    )
    .expect("a bounded standard configuration writes its grid")
}

/// Every symmetric Standard configuration writes exactly its `(rows, cols)`
/// grid, tiles the A4 sheet without a gap, and is one revision / one undo step.
#[test]
fn vports_standard_configurations_write_their_equal_cell_grids() {
    let cases = [
        (StandardViewportConfiguration::Single, 1u32, 1u32),
        (StandardViewportConfiguration::TwoVertical, 1, 2),
        (StandardViewportConfiguration::TwoHorizontal, 2, 1),
        (StandardViewportConfiguration::ThreeVertical, 1, 3),
        (StandardViewportConfiguration::ThreeHorizontal, 3, 1),
        (StandardViewportConfiguration::FourEqual, 2, 2),
    ];
    for (configuration, rows, cols) in cases {
        let (mut app, mut session) = fixture();
        switch_to_sheet(&mut app, &mut session);
        let before_revision = revision(&app);
        let before_depth = undo_depth(&app);
        let outcome = execute_configuration(&mut app, &mut session, configuration);
        assert_eq!(codes(&outcome), vec!["vports.created"], "{configuration:?}");
        assert!(
            outcome.diagnostics[0]
                .message
                .contains(configuration.name()),
            "the diagnostic names the configuration: {}",
            outcome.diagnostics[0].message
        );
        assert_eq!(
            revision(&app),
            Revision(before_revision.0 + 1),
            "{configuration:?}: one revision"
        );
        assert_eq!(
            undo_depth(&app),
            before_depth + 1,
            "{configuration:?}: one undo step"
        );

        let layout = drawing(&app).layout(LayoutId(1)).expect("layout exists");
        let total = (rows * cols) as usize;
        assert_eq!(layout.viewports.len(), total, "{configuration:?}");
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
            "{configuration:?}: the first cell starts at the sheet origin"
        );
        let last = layout.viewports.last().expect("at least one cell");
        assert_eq!(
            last.clip[2],
            point(210.0, 297.0),
            "{configuration:?}: the last cell reaches the sheet corner"
        );
        for viewport in &layout.viewports {
            assert_eq!(viewport.completeness, Completeness::Complete);
            assert_eq!(viewport.model_to_paper.matrix[0][0], 1.0);
            assert_eq!(viewport.model_to_paper.matrix[1][1], 1.0);
        }

        undo(&mut app, &mut session);
        let reverted = drawing(&app).layout(LayoutId(1)).expect("layout exists");
        assert_eq!(
            reverted.viewports,
            vec![viewport()],
            "{configuration:?}: undo restores the previous viewport vector"
        );
        redo(&mut app, &mut session);
        let redone = drawing(&app).layout(LayoutId(1)).expect("layout exists");
        assert_eq!(
            redone.viewports.len(),
            total,
            "{configuration:?}: redo reapplies"
        );
    }
}

/// The historical explicit grid form is untouched by the Standard set: it still
/// takes arbitrary positive counts and writes the same declared equal-cell grid.
#[test]
fn vports_explicit_grid_form_is_preserved_alongside_standard_configurations() {
    let (mut app, mut session) = fixture();
    switch_to_sheet(&mut app, &mut session);
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Vports,
                layout_payload(LayoutPayload::Viewports { rows: 2, cols: 3 }),
            ),
        )
        .expect("the explicit grid still writes");
    assert_eq!(codes(&outcome), vec!["vports.created"]);
    assert!(
        outcome.diagnostics[0]
            .message
            .contains("explicit grid form"),
        "{}",
        outcome.diagnostics[0].message
    );
    assert_eq!(revision(&app), Revision(before.0 + 1));
    let layout = drawing(&app).layout(LayoutId(1)).expect("layout exists");
    assert_eq!(layout.viewports.len(), 6);
    assert_eq!(layout.viewports[0].clip[2], point(70.0, 148.5));
}

/// A standard configuration in model space is the same explicit, atomic
/// refusal as the `<rows> <cols>` form: no revision, no history, no write.
#[test]
fn vports_configuration_refuses_model_space_without_writing() {
    let (mut app, mut session) = fixture();
    let before_revision = revision(&app);
    let before_depth = undo_depth(&app);
    let before_viewports = drawing(&app)
        .layout(LayoutId(1))
        .expect("layout exists")
        .viewports
        .clone();
    let message = refusal(app.execute(
        &mut session,
        command(
            CommandId::Vports,
            layout_payload(LayoutPayload::ViewportsConfiguration {
                configuration: StandardViewportConfiguration::FourEqual,
            }),
        ),
    ));
    assert!(message.contains("vports.paper_space_required"), "{message}");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), before_depth);
    assert_eq!(
        drawing(&app)
            .layout(LayoutId(1))
            .expect("layout exists")
            .viewports,
        before_viewports
    );
}

/// The refusal for a non-positive explicit grid is unchanged and writes nothing.
#[test]
fn vports_invalid_explicit_grid_still_refuses_atomically() {
    let (mut app, mut session) = fixture();
    switch_to_sheet(&mut app, &mut session);
    let before_revision = revision(&app);
    let before_depth = undo_depth(&app);
    for (rows, cols) in [(0, 2), (2, 0), (0, 0)] {
        let message = refusal(app.execute(
            &mut session,
            command(
                CommandId::Vports,
                layout_payload(LayoutPayload::Viewports { rows, cols }),
            ),
        ));
        assert!(message.contains("vports.invalid_grid"), "{message}");
    }
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), before_depth);
    assert_eq!(
        drawing(&app)
            .layout(LayoutId(1))
            .expect("layout exists")
            .viewports,
        vec![viewport()]
    );
}
