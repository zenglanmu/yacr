//! The graduated Wave 2 **paper-space write** contracts:
//! `VPLAYER`, `MVSETUP`, `PSBSCALE`, `PSTSCALE` and the real `VPMAX`/`VPMIN`
//! editing state.
//!
//! Every write is exactly one revision, one `ChangeSet` and one undo step, and
//! every refusal happens before the transaction opens (revision, undo depth and
//! the stored rows are unchanged). The per-viewport layer state is asserted to
//! be **per viewport** and never the global layer table; `VPMAX`/`VPMIN` are
//! asserted to be session-only and to restore the camera bit-for-bit.
//!
//! This module lives beside `tests/drawing/` for grouping but is declared from
//! `tests/extended_refusals.rs` (the batch that used to own these names) so the
//! 1000-line `tests/layout_commands.rs` does not have to grow.

use super::*;
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer, Layout, PaperViewport,
    Scale,
};

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

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn tokens(values: &[&str]) -> CommandPayload {
    CommandPayload::LayerWalk(values.iter().map(|value| (*value).to_string()).collect())
}

fn viewport(scale: f64) -> PaperViewport {
    PaperViewport {
        clip: vec![
            point(0.0, 0.0),
            point(100.0, 0.0),
            point(100.0, 50.0),
            point(0.0, 50.0),
        ],
        model_to_paper: Transform3::scale(scale),
        completeness: Completeness::Complete,
        layer_overrides: std::collections::BTreeMap::new(),
    }
}

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

fn entity(id: u128, geometry: SemanticGeometry, space: SpaceId, type_key: &str) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry,
        draw_order: id as i64,
    }
}

fn text_geometry(height: f64, wrap_width: Option<f64>) -> SemanticGeometry {
    SemanticGeometry::Text {
        text: "paper".into(),
        position: point(5.0, 5.0),
        style: StyleId(0),
        height,
        rotation: 0.0,
        font: Some("arial.ttf".into()),
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width,
        width_factor: 1.0,
    }
}

fn insert_geometry(scale: f64) -> SemanticGeometry {
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = scale;
    matrix[1][1] = scale;
    matrix[0][3] = 40.0;
    matrix[1][3] = 25.0;
    SemanticGeometry::Insert {
        block: cad_domain::BlockId(0),
        transform: Transform3 { matrix },
    }
}

/// The fixture: layer `0`/`WALLS`/`HIDDEN`/`2`, scale `1:100`, one paper layout
/// with `viewports` viewports of `viewport_scale`, plus the given entities and a
/// block definition so an `INSERT` validates.
fn fixture(
    viewports: usize,
    viewport_scale: f64,
    entities: Vec<DbEntity>,
) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [
        (LayerId(0), "0"),
        (LayerId(1), "WALLS"),
        (LayerId(2), "HIDDEN"),
        (LayerId(3), "2"),
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
    builder
        .insert_block(BlockDefinition {
            id: cad_domain::BlockId(0),
            entities: Vec::new(),
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet1".into(),
            viewports: (0..viewports).map(|_| viewport(viewport_scale)).collect(),
        })
        .unwrap();
    builder.set_plot_settings(plot_settings(1)).unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DOC,
        Document {
            id: DOC,
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DOC, [800.0, 600.0]),
    );
    let mut session = SessionState::new(DOC, AppMode::Work);
    session.active_space = SpaceId::Paper(LayoutId(1));
    (app, session)
}

fn empty_fixture(viewports: usize) -> (Application, SessionState) {
    fixture(viewports, 100.0, Vec::new())
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

fn viewports(app: &Application) -> Vec<PaperViewport> {
    drawing(app)
        .layout(LayoutId(1))
        .expect("the fixture layout exists")
        .viewports
        .clone()
}

fn frozen(app: &Application, viewport: usize) -> Vec<LayerId> {
    viewports(app)[viewport]
        .layer_overrides
        .iter()
        .filter(|(_, frozen)| **frozen)
        .map(|(id, _)| *id)
        .collect()
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

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection.replace(ids.iter().map(|id| SelectionRef {
        document: DOC,
        entity: EntityId(*id),
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn undo(app: &mut Application, session: &mut SessionState) {
    app.execute(session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo replays the recorded step");
}

fn redo(app: &mut Application, session: &mut SessionState) {
    app.execute(session, command(CommandId::Redo, CommandPayload::None))
        .expect("redo replays the recorded step");
}

fn vplayer(values: &[&str]) -> Command {
    command(
        CommandId::Extended(ExtendedCommandId::VPlayer),
        tokens(values),
    )
}

// ---------------------------------------------------------------- VPLAYER ---

#[test]
fn vplayer_freezes_one_viewport_and_round_trips_through_undo_redo() {
    let (mut app, mut session) = empty_fixture(2);
    let before = revision(&app);
    let depth = undo_depth(&app);
    let outcome = app
        .execute(&mut session, vplayer(&["FREEZE", "WALLS"]))
        .expect("VPLAYER FREEZE writes per-viewport state");
    assert_eq!(codes(&outcome), vec!["vplayer.frozen"]);
    assert!(outcome.diagnostics[0].message.contains("WALLS"));
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert_eq!(undo_depth(&app), depth + 1, "exactly one undo step");
    assert_eq!(frozen(&app, 0), vec![LayerId(1)]);
    assert!(
        frozen(&app, 1).is_empty(),
        "the other viewport is untouched"
    );
    assert!(
        drawing(&app).layer(LayerId(1)).unwrap().visible,
        "the global layer table is never written"
    );

    undo(&mut app, &mut session);
    assert!(frozen(&app, 0).is_empty(), "undo restores the override map");
    redo(&mut app, &mut session);
    assert_eq!(frozen(&app, 0), vec![LayerId(1)], "redo reapplies it");
}

#[test]
fn vplayer_thaw_removes_the_override_and_the_repeat_is_a_named_no_op() {
    let (mut app, mut session) = empty_fixture(1);
    app.execute(&mut session, vplayer(&["FREEZE", "WALLS", "HIDDEN"]))
        .expect("freeze two layers");
    assert_eq!(frozen(&app, 0), vec![LayerId(1), LayerId(2)]);

    let outcome = app
        .execute(&mut session, vplayer(&["THAW", "WALLS"]))
        .expect("THAW removes one override");
    assert_eq!(codes(&outcome), vec!["vplayer.thawed"]);
    assert_eq!(frozen(&app, 0), vec![LayerId(2)]);

    let written = revision(&app);
    let depth = undo_depth(&app);
    let again = app
        .execute(&mut session, vplayer(&["THAW", "WALLS"]))
        .expect("a repeated THAW is a declared no-op");
    assert_eq!(codes(&again), vec!["vplayer.unchanged"]);
    assert_eq!(revision(&app), written, "no revision bump");
    assert_eq!(undo_depth(&app), depth, "no new undo step");

    let repeated_freeze = app
        .execute(&mut session, vplayer(&["FREEZE", "HIDDEN"]))
        .expect("an already-frozen layer is a declared no-op");
    assert_eq!(codes(&repeated_freeze), vec!["vplayer.unchanged"]);
}

#[test]
fn vplayer_explicit_index_targets_exactly_one_viewport() {
    let (mut app, mut session) = empty_fixture(3);
    // The bracket form is unambiguously the index, so it works even though the
    // fixture also has a layer named "2".
    app.execute(&mut session, vplayer(&["FREEZE", "WALLS", "[2]"]))
        .expect("an explicit index targets viewport 2");
    assert!(frozen(&app, 0).is_empty());
    assert!(frozen(&app, 1).is_empty());
    assert_eq!(frozen(&app, 2), vec![LayerId(1)]);

    // The bare form is the index only when no layer carries that name; "1" is
    // not a layer here.
    app.execute(&mut session, vplayer(&["FREEZE", "HIDDEN", "1"]))
        .expect("a bare index targets viewport 1");
    assert_eq!(frozen(&app, 1), vec![LayerId(2)]);
    assert_eq!(frozen(&app, 2), vec![LayerId(1)], "viewport 2 is unchanged");
}

#[test]
fn vplayer_numeric_layer_name_is_a_layer_and_not_an_index() {
    // Layer `2` exists, so the trailing `2` must resolve as a layer name even
    // though it also parses as a viewport index.
    let (mut app, mut session) = empty_fixture(3);
    app.execute(&mut session, vplayer(&["FREEZE", "2"]))
        .expect("a numeric layer name freezes that layer");
    assert_eq!(frozen(&app, 0), vec![LayerId(3)], "layer `2` on viewport 0");
}

#[test]
fn vplayer_refusals_write_nothing() {
    let (mut app, mut session) = empty_fixture(2);
    let before = revision(&app);

    let unknown = refusal(app.execute(&mut session, vplayer(&["FREEZE", "NOPE"])));
    assert!(unknown.contains("vplayer.unknown_layer"), "{unknown}");

    let action = refusal(app.execute(&mut session, vplayer(&["FROZEN", "WALLS"])));
    assert!(action.contains("vplayer.invalid_argument"), "{action}");

    let empty = refusal(app.execute(&mut session, vplayer(&["FREEZE"])));
    assert!(empty.contains("vplayer.invalid_argument"), "{empty}");

    let index = refusal(app.execute(&mut session, vplayer(&["FREEZE", "WALLS", "9"])));
    assert!(index.contains("vplayer.invalid_viewport"), "{index}");

    let whole_payload = refusal(app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::VPlayer),
            CommandPayload::Points(vec![point(1.0, 2.0)]),
        ),
    ));
    assert!(
        whole_payload.contains("vplayer.invalid_argument"),
        "{whole_payload}"
    );

    assert_eq!(revision(&app), before, "a refusal writes nothing");
    assert_eq!(undo_depth(&app), 0);

    // Model space has no per-viewport state to write.
    session.active_space = SpaceId::Model;
    let model = refusal(app.execute(&mut session, vplayer(&["FREEZE", "WALLS"])));
    assert!(model.contains("vplayer.paper_space_required"), "{model}");

    // A layout with no viewport refuses by name instead of silently succeeding.
    let (mut app, mut session) = empty_fixture(0);
    let none = refusal(app.execute(&mut session, vplayer(&["FREEZE", "WALLS"])));
    assert!(none.contains("vplayer.no_viewport"), "{none}");
}

#[test]
fn vplayer_report_is_read_only_and_viewer_legal() {
    let (mut app, mut session) = empty_fixture(2);
    app.execute(&mut session, vplayer(&["FREEZE", "HIDDEN"]))
        .expect("seed one override");
    session
        .switch_mode(AppMode::Viewer)
        .expect("Viewer is legal");
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::VPlayer),
                CommandPayload::None,
            ),
        )
        .expect("the bare VPLAYER report is read-only");
    assert_eq!(
        codes(&outcome),
        vec!["vplayer.report", "vplayer.report", "vplayer.report.active"]
    );
    assert!(outcome.diagnostics[0].message.contains("HIDDEN"));
    assert_eq!(revision(&app), before);

    // The write form itself is Work-only even though the capability registry
    // still lists the selector as Viewer-legal (see the module docs).
    let denied = app
        .execute(&mut session, vplayer(&["FREEZE", "WALLS"]))
        .expect_err("a Viewer session must not write per-viewport state");
    assert!(matches!(denied, CadError::PermissionDenied), "{denied:?}");
}

// ---------------------------------------------------------------- MVSETUP ---

#[test]
fn mvsetup_creates_a_scaled_grid_and_undo_restores_the_previous_viewports() {
    let (mut app, mut session) = empty_fixture(1);
    let before = revision(&app);
    let restore_point = viewports(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::MvSetup),
                tokens(&["2", "2", "1:100"]),
            ),
        )
        .expect("MVSETUP writes a real grid");
    assert_eq!(codes(&outcome), vec!["mvsetup.applied"]);
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    let stored = viewports(&app);
    assert_eq!(stored.len(), 4, "rows x cols viewports");
    let cell_width = 210.0 / 2.0;
    let cell_height = 297.0 / 2.0;
    assert_eq!(
        stored[0].clip,
        vec![
            point(0.0, 0.0),
            point(cell_width, 0.0),
            point(cell_width, cell_height),
            point(0.0, cell_height),
        ],
        "the cells are equal and cover the sheet from its origin"
    );
    // 1:100 is paper/drawing, so the stored model-per-paper diagonal is 100.
    assert_eq!(stored[0].model_to_paper.matrix[0][0], 100.0);
    assert_eq!(stored[0].model_to_paper.matrix[1][1], 100.0);
    // The model origin stays the view centre of each cell.
    let view = cad_representation::viewport_transform(&stored[0]);
    match view {
        cad_representation::ViewportState::Supported(view) => {
            assert_eq!(view.model_anchor, point(0.0, 0.0));
            assert!((view.paper_per_model - 0.01).abs() < 1e-12);
        }
        other => panic!("the scaled viewport must stay supported, got {other:?}"),
    }

    undo(&mut app, &mut session);
    assert_eq!(
        viewports(&app),
        restore_point,
        "undo restores the old vector"
    );
}

#[test]
fn mvsetup_applies_a_ratio_scale_and_refuses_outside_paper_space() {
    let (mut app, mut session) = empty_fixture(1);
    app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::MvSetup),
            tokens(&["1", "2", "1:50"]),
        ),
    )
    .expect("a ratio scale is accepted");
    let stored = viewports(&app);
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[1].model_to_paper.matrix[0][0], 50.0);

    let before = revision(&app);
    session.active_space = SpaceId::Model;
    let model = refusal(app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::MvSetup),
            tokens(&["2", "2", "1:100"]),
        ),
    ));
    assert!(model.contains("mvsetup.paper_space_required"), "{model}");
    assert_eq!(revision(&app), before);

    let zero = refusal(app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::MvSetup),
            tokens(&["0", "2", "1:100"]),
        ),
    ));
    assert!(zero.contains("mvsetup.invalid_grid"), "{zero}");

    // Back to paper space for the scale/arity refusals.
    session.active_space = SpaceId::Paper(LayoutId(1));
    let unknown_scale = refusal(app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::MvSetup),
            tokens(&["2", "2", "NOPE"]),
        ),
    ));
    assert!(
        unknown_scale.contains("mvsetup.invalid_scale"),
        "{unknown_scale}"
    );

    let too_many = refusal(app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::MvSetup),
            tokens(&["9", "9", "1:100"]),
        ),
    ));
    assert!(too_many.contains("mvsetup.invalid_grid"), "{too_many}");

    let arity = refusal(app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::MvSetup),
            tokens(&["2"]),
        ),
    ));
    assert!(arity.contains("mvsetup.invalid_argument"), "{arity}");

    let payload = refusal(app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::MvSetup),
            CommandPayload::Points(vec![point(2.0, 2.0)]),
        ),
    ));
    assert!(payload.contains("mvsetup.invalid_argument"), "{payload}");
    assert_eq!(revision(&app), before);
}

// --------------------------------------------------------------- PSBSCALE ---

#[test]
fn psbscale_scales_a_paper_block_by_the_viewport_factor() {
    let block = entity(
        20,
        insert_geometry(1.0),
        SpaceId::Paper(LayoutId(1)),
        "AcDbBlockReference",
    );
    let (mut app, mut session) = fixture(1, 100.0, vec![block]);
    select(&mut session, &[20]);
    let before = revision(&app);
    let depth = undo_depth(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::PsbScale),
                CommandPayload::None,
            ),
        )
        .expect("PSBSCALE writes the block transform");
    assert_eq!(codes(&outcome), vec!["psbscale.applied"]);
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert_eq!(undo_depth(&app), depth + 1, "one undo step");
    let stored = drawing(&app)
        .entity(EntityId(20))
        .expect("the block reference is still there")
        .geometry
        .clone();
    let SemanticGeometry::Insert { transform, .. } = stored else {
        panic!("the entity stays an INSERT");
    };
    // 1:100 ⇒ paper units per model unit = 0.01.
    assert!((transform.matrix[0][0] - 0.01).abs() < 1e-12);
    assert!((transform.matrix[1][1] - 0.01).abs() < 1e-12);
    assert_eq!(transform.matrix[0][3], 40.0, "the insertion point is fixed");
    assert_eq!(transform.matrix[1][3], 25.0);

    undo(&mut app, &mut session);
    let restored = drawing(&app).entity(EntityId(20)).unwrap().geometry.clone();
    assert!(
        matches!(&restored, SemanticGeometry::Insert { transform, .. }
            if transform.matrix[0][0] == 1.0 && transform.matrix[0][3] == 40.0),
        "undo restores the exact placement"
    );
}

#[test]
fn psbscale_refusals_write_nothing() {
    let line = entity(
        21,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        SpaceId::Paper(LayoutId(1)),
        "AcDbLine",
    );
    let model_block = entity(
        22,
        insert_geometry(1.0),
        SpaceId::Model,
        "AcDbBlockReference",
    );
    let (mut app, mut session) = fixture(1, 100.0, vec![line, model_block]);
    let before = revision(&app);
    let psbscale = || {
        command(
            CommandId::Extended(ExtendedCommandId::PsbScale),
            CommandPayload::None,
        )
    };

    let none = refusal(app.execute(&mut session, psbscale()));
    assert!(none.contains("psbscale.needs_selection"), "{none}");

    select(&mut session, &[21]);
    let not_block = refusal(app.execute(&mut session, psbscale()));
    assert!(not_block.contains("psbscale.not_block"), "{not_block}");

    select(&mut session, &[22]);
    let model = refusal(app.execute(&mut session, psbscale()));
    assert!(model.contains("psbscale.model_space"), "{model}");

    assert_eq!(revision(&app), before, "a refusal writes nothing");
    assert_eq!(undo_depth(&app), 0);

    // A paper layout with no viewport has no paper-space factor.
    let block = entity(
        20,
        insert_geometry(1.0),
        SpaceId::Paper(LayoutId(1)),
        "AcDbBlockReference",
    );
    let (mut app, mut session) = fixture(0, 100.0, vec![block]);
    select(&mut session, &[20]);
    let no_viewport = refusal(app.execute(&mut session, psbscale()));
    assert!(
        no_viewport.contains("psbscale.no_viewport"),
        "{no_viewport}"
    );
}

#[test]
fn psbscale_with_a_one_to_one_viewport_is_a_named_no_op() {
    let block = entity(
        20,
        insert_geometry(1.0),
        SpaceId::Paper(LayoutId(1)),
        "AcDbBlockReference",
    );
    let (mut app, mut session) = fixture(1, 1.0, vec![block]);
    select(&mut session, &[20]);
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::PsbScale),
                CommandPayload::None,
            ),
        )
        .expect("a 1:1 viewport is a declared no-op");
    assert_eq!(codes(&outcome), vec!["psbscale.unchanged"]);
    assert_eq!(revision(&app), before, "no revision bump");
    assert_eq!(undo_depth(&app), 0, "no undo entry");
}

// --------------------------------------------------------------- PSTSCALE ---

#[test]
fn pstscale_scales_paper_text_heights_by_the_viewport_factor() {
    let text = entity(
        30,
        text_geometry(250.0, Some(1000.0)),
        SpaceId::Paper(LayoutId(1)),
        "AcDbMText",
    );
    let (mut app, mut session) = fixture(1, 100.0, vec![text]);
    select(&mut session, &[30]);
    let before = revision(&app);
    let depth = undo_depth(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::PstScale),
                CommandPayload::None,
            ),
        )
        .expect("PSTSCALE writes the stored height");
    assert_eq!(codes(&outcome), vec!["pstscale.applied"]);
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), depth + 1);
    match &drawing(&app).entity(EntityId(30)).unwrap().geometry {
        SemanticGeometry::Text {
            height, wrap_width, ..
        } => {
            assert!((height - 2.5).abs() < 1e-12, "250 × 0.01 = 2.5");
            assert!(
                (wrap_width.expect("the wrap width is kept") - 10.0).abs() < 1e-12,
                "the MTEXT wrap width scales with the height"
            );
        }
        other => panic!("the entity stays TEXT, got {other:?}"),
    }

    undo(&mut app, &mut session);
    match &drawing(&app).entity(EntityId(30)).unwrap().geometry {
        SemanticGeometry::Text {
            height, wrap_width, ..
        } => {
            assert_eq!(*height, 250.0);
            assert_eq!(*wrap_width, Some(1000.0));
        }
        other => panic!("the entity stays TEXT, got {other:?}"),
    }
}

#[test]
fn pstscale_refusals_write_nothing() {
    let line = entity(
        31,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        SpaceId::Paper(LayoutId(1)),
        "AcDbLine",
    );
    let model_text = entity(32, text_geometry(2.5, None), SpaceId::Model, "AcDbText");
    let (mut app, mut session) = fixture(1, 100.0, vec![line, model_text]);
    let before = revision(&app);
    let pstscale = || {
        command(
            CommandId::Extended(ExtendedCommandId::PstScale),
            CommandPayload::None,
        )
    };

    let none = refusal(app.execute(&mut session, pstscale()));
    assert!(none.contains("pstscale.needs_selection"), "{none}");

    select(&mut session, &[31]);
    let not_text = refusal(app.execute(&mut session, pstscale()));
    assert!(not_text.contains("pstscale.not_text"), "{not_text}");

    select(&mut session, &[32]);
    let model = refusal(app.execute(&mut session, pstscale()));
    assert!(model.contains("pstscale.model_space"), "{model}");

    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);

    let text = entity(
        30,
        text_geometry(2.5, None),
        SpaceId::Paper(LayoutId(1)),
        "AcDbText",
    );
    let (mut app, mut session) = fixture(0, 100.0, vec![text]);
    select(&mut session, &[30]);
    let no_viewport = refusal(app.execute(&mut session, pstscale()));
    assert!(
        no_viewport.contains("pstscale.no_viewport"),
        "{no_viewport}"
    );
}

// ---------------------------------------------------------- VPMAX / VPMIN ---

fn camera(app: &Application) -> Camera {
    app.workspace.viewports[&ViewportId(1)].camera
}

fn view_state(app: &Application) -> (Camera, ViewMode2d3d, WorkPlane) {
    let viewport = &app.workspace.viewports[&ViewportId(1)];
    (viewport.camera, viewport.view_mode, viewport.work_plane)
}

#[test]
fn vpmax_enters_a_real_maximized_state_and_vpmin_restores_the_camera_exactly() {
    let (mut app, mut session) = empty_fixture(1);
    let before = revision(&app);
    let generation = session.generation;
    let saved_view = view_state(&app);
    session.active_space = SpaceId::Model;
    // A model-space start is refused by name.
    let model = refusal(app.execute(
        &mut session,
        command(CommandId::VpMax, CommandPayload::None),
    ));
    assert!(model.contains("vpmax.paper_space_required"), "{model}");

    session.active_space = SpaceId::Paper(LayoutId(1));
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::VpMax, CommandPayload::None),
        )
        .expect("VPMAX enters the editing state");
    assert_eq!(codes(&outcome), vec!["vpmax.maximized"]);
    assert_eq!(
        session.maximized_viewport(),
        Some(LayoutViewportRef {
            layout: LayoutId(1),
            index: 0
        })
    );
    assert_ne!(camera(&app), saved_view.0, "the camera really moved");
    // The maximized camera frames the viewport's own model window: the paper
    // clip 100 x 50 at 1:100 is a 10000 x 5000 model rectangle, centred on
    // (5000, 2500), filling an 800 x 600 display ⇒ 12.5 world units per pixel.
    assert_eq!(camera(&app).eye.x, 5000.0);
    assert_eq!(camera(&app).eye.y, 2500.0);
    assert_eq!(camera(&app).target.x, 5000.0);
    assert_eq!(camera(&app).target.y, 2500.0);
    match camera(&app).projection {
        Projection::Orthographic { scale } => assert!((scale - 12.5).abs() < 1e-12),
        other => panic!("the maximized view is a 2D orthographic camera, got {other:?}"),
    }
    assert!(
        matches!(
            app.workspace.viewports[&ViewportId(1)].view_mode,
            ViewMode2d3d::TwoD { .. }
        ),
        "maximizing shows the plan view"
    );
    assert_eq!(session.generation, generation + 1, "the view changed");
    assert_eq!(revision(&app), before, "no drawing write");
    assert_eq!(undo_depth(&app), 0, "no undo entry");
    assert!(
        session
            .view_states
            .resolve(crate::app_layout_session::VPMAX_RESTORE_VIEW)
            .is_some(),
        "the pre-maximize view snapshot is stored"
    );

    let restored = app
        .execute(
            &mut session,
            command(CommandId::VpMin, CommandPayload::None),
        )
        .expect("VPMIN restores the captured view");
    assert_eq!(codes(&restored), vec!["vpmin.restored"]);
    assert_eq!(
        view_state(&app),
        saved_view,
        "the camera/mode/plane are exact"
    );
    assert_eq!(session.active_space, SpaceId::Paper(LayoutId(1)));
    assert_eq!(session.maximized_viewport(), None);
    assert!(
        session
            .view_states
            .resolve(crate::app_layout_session::VPMAX_RESTORE_VIEW)
            .is_none(),
        "the restore snapshot is consumed"
    );
    assert_eq!(revision(&app), before);
}

#[test]
fn vpmax_restores_the_space_a_paper_viewport_was_maximized_from() {
    let (mut app, mut session) = empty_fixture(2);
    app.execute(
        &mut session,
        command(CommandId::VpMax, CommandPayload::None),
    )
    .expect("VPMAX in paper space");
    // Leaving paper space while maximized must not lose the restore space.
    session.active_space = SpaceId::Model;
    app.execute(
        &mut session,
        command(CommandId::VpMin, CommandPayload::None),
    )
    .expect("VPMIN restores");
    assert_eq!(
        session.active_space,
        SpaceId::Paper(LayoutId(1)),
        "VPMIN restores the captured space"
    );
}

#[test]
fn vpmax_and_vpmin_refusals_leave_the_state_untouched() {
    let (mut app, mut session) = empty_fixture(1);
    let empty = app
        .execute(
            &mut session,
            command(CommandId::VpMin, CommandPayload::None),
        )
        .expect("VPMIN with no maximized viewport is a no-op");
    assert_eq!(codes(&empty), vec!["vpmin.nothing_to_restore"]);

    let (mut app, mut session) = empty_fixture(0);
    let none = refusal(app.execute(
        &mut session,
        command(CommandId::VpMax, CommandPayload::None),
    ));
    assert!(none.contains("vpmax.no_viewport"), "{none}");

    let (mut app, mut session) = empty_fixture(1);
    let payload = refusal(app.execute(
        &mut session,
        command(
            CommandId::VpMax,
            CommandPayload::Points(vec![point(1.0, 1.0)]),
        ),
    ));
    assert!(payload.contains("vpmax.invalid_payload"), "{payload}");
}

#[test]
fn vpmin_refuses_by_name_when_the_restore_snapshot_was_deleted() {
    let (mut app, mut session) = empty_fixture(1);
    let saved_view = view_state(&app);
    app.execute(
        &mut session,
        command(CommandId::VpMax, CommandPayload::None),
    )
    .expect("VPMAX in paper space");
    let maximized = camera(&app);
    // The restore state lives in the session view store; deleting it destroys
    // the exact camera, and VPMIN must say so instead of guessing one.
    assert!(session
        .view_states
        .remove(crate::app_layout_session::VPMAX_RESTORE_VIEW));
    let missing = refusal(app.execute(
        &mut session,
        command(CommandId::VpMin, CommandPayload::None),
    ));
    assert!(missing.contains("vpmin.restore_state_missing"), "{missing}");
    assert_eq!(camera(&app), maximized, "the camera is left unchanged");
    assert!(
        session.maximized_viewport().is_some(),
        "the marker survives"
    );
    assert_ne!(camera(&app), saved_view.0);
}

#[test]
fn vpmax_while_another_layout_viewport_is_maximized_is_refused() {
    let (mut app, mut session) = empty_fixture(1);
    app.execute(
        &mut session,
        command(CommandId::VpMax, CommandPayload::None),
    )
    .expect("VPMAX in paper space");
    let maximized = camera(&app);
    // The marker is the (layout, index) pair; a second VPMAX on the same pair is
    // the named no-op, so retarget the marker to a different layout to prove the
    // "one at a time" rule.
    session.set_maximized_viewport(LayoutViewportRef {
        layout: LayoutId(9),
        index: 0,
    });
    let refused = refusal(app.execute(
        &mut session,
        command(CommandId::VpMax, CommandPayload::None),
    ));
    assert!(refused.contains("vpmax.already_maximized"), "{refused}");
    assert_eq!(camera(&app), maximized, "the camera is left unchanged");
}
