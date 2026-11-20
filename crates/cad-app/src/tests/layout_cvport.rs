//! `CVPORT` and the three commands that target it (`LAYVPI`, `VPSCALE`,
//! `VPCLIP`).
//!
//! Split out of `layout_commands.rs` so that file stays inside the code-size
//! gate. The contract is in `crate::command_id::extras` (the marker) and
//! `crate::command_id::layout` (the three consumers):
//!
//! * the marker is **session state only** (no database write, revision,
//!   history entry or undo step), **per layout**, and **one-based** on the
//!   command line while the stored index is zero-based;
//! * a layout with no viewport has no current viewport (`cvport.no_viewports`);
//! * `LAYVPI` (no index) freezes everywhere **except** the current viewport;
//! * `VPSCALE` / `VPCLIP` (no index) target the current viewport;
//! * an explicit payload index always wins over the marker.

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

/// A document with layers `0`/`WALLS`/`HIDDEN`, a `1:100` scale and the given
/// paper layouts (each with one viewport unless `viewports` is false).
fn fixture(layouts: &[(u128, &str, bool)]) -> (Application, SessionState) {
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

fn standard_fixture() -> (Application, SessionState) {
    fixture(&[(1, "Sheet1", true), (2, "Sheet2", true)])
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DOC].drawing.revision()
}

fn undo_available(app: &Application) -> bool {
    app.can_undo(&DOC)
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

/// `VPORTS 1 <cols>`: replace the layout's viewport list with a `cols`-wide grid.
fn create_viewports(app: &mut Application, session: &mut SessionState, cols: u32) {
    app.execute(
        session,
        command(
            CommandId::Vports,
            layout_payload(LayoutPayload::Viewports { rows: 1, cols }),
        ),
    )
    .expect("VPORTS creates the grid");
}

fn cvport(app: &mut Application, session: &mut SessionState, position: u32) -> CommandOutcome {
    app.execute(
        session,
        command(CommandId::Cvport, CommandPayload::Cvport(position)),
    )
    .expect("a valid CVPORT position is stored")
}

fn cvport_report(app: &mut Application, session: &mut SessionState) -> CommandOutcome {
    app.execute(session, command(CommandId::Cvport, CommandPayload::None))
        .expect("the bare CVPORT report")
}

fn viewport_override(app: &Application, layout: u128, viewport: usize) -> Option<bool> {
    drawing(app)
        .layout(LayoutId(layout))
        .expect("layout exists")
        .viewports[viewport]
        .layer_overrides
        .get(&LayerId(1))
        .copied()
}

// --------------------------------------------------------------- CVPORT ---

#[test]
fn cvport_defaults_to_one_reports_and_sets_a_position_without_writing() {
    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    create_viewports(&mut app, &mut session, 3);
    let before = revision(&app);
    let depth = undo_depth(&app);

    let report = cvport_report(&mut app, &mut session);
    assert_eq!(codes(&report), vec!["cvport.value"]);
    assert!(
        report.diagnostics[0].message.contains("CVPORT = 1 of 3"),
        "the documented default is the layout's first viewport: {}",
        report.diagnostics[0].message
    );

    let set = cvport(&mut app, &mut session, 2);
    assert_eq!(codes(&set), vec!["cvport.value"]);
    assert!(
        set.diagnostics[0].message.contains("CVPORT = 2 of 3"),
        "{}",
        set.diagnostics[0].message
    );
    assert_eq!(session.viewport_marker(LayoutId(1)), 1);

    // Session state only: no revision, no history entry, no undo step.
    assert_eq!(revision(&app), before, "CVPORT writes no drawing");
    assert_eq!(undo_depth(&app), depth, "CVPORT records no undo step");
}

#[test]
fn cvport_refuses_out_of_range_model_space_and_a_layout_without_viewports() {
    let (mut app, mut session) = standard_fixture();

    // Model space: this build keeps no model-space viewport id and reports no
    // fabricated 1.
    let model = refusal(app.execute(
        &mut session,
        command(CommandId::Cvport, CommandPayload::Cvport(1)),
    ));
    assert!(model.contains("cvport.paper_space_required"), "{model}");

    switch_to(&mut app, &mut session, "Sheet1");
    create_viewports(&mut app, &mut session, 2);
    let before = revision(&app);
    let depth = undo_depth(&app);
    let high = refusal(app.execute(
        &mut session,
        command(CommandId::Cvport, CommandPayload::Cvport(3)),
    ));
    assert!(high.contains("cvport.out_of_range"), "{high}");
    let zero = refusal(app.execute(
        &mut session,
        command(CommandId::Cvport, CommandPayload::Cvport(0)),
    ));
    assert!(zero.contains("cvport.out_of_range"), "{zero}");
    // Never clamped: the marker still names the first viewport.
    assert_eq!(session.viewport_marker(LayoutId(1)), 0);

    let (mut empty, mut empty_session) = fixture(&[(1, "Empty", false)]);
    switch_to(&mut empty, &mut empty_session, "Empty");
    let empty_before = revision(&empty);
    let none = refusal(empty.execute(
        &mut empty_session,
        command(CommandId::Cvport, CommandPayload::None),
    ));
    assert!(none.contains("cvport.no_viewports"), "{none}");
    assert_eq!(revision(&empty), empty_before);

    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), depth);
}

#[test]
fn cvport_is_per_layout_and_survives_a_layout_switch() {
    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    create_viewports(&mut app, &mut session, 3);
    cvport(&mut app, &mut session, 3);
    assert_eq!(session.viewport_marker(LayoutId(1)), 2);

    switch_to(&mut app, &mut session, "Sheet2");
    let other = cvport_report(&mut app, &mut session);
    assert!(
        other.diagnostics[0].message.contains("CVPORT = 1 of 1"),
        "a never-pointed layout reports its documented default: {}",
        other.diagnostics[0].message
    );
    assert_eq!(session.viewport_marker(LayoutId(2)), 0);

    switch_to(&mut app, &mut session, "Sheet1");
    let back = cvport_report(&mut app, &mut session);
    assert!(
        back.diagnostics[0].message.contains("CVPORT = 3 of 3"),
        "the marker survives the switch: {}",
        back.diagnostics[0].message
    );
}

#[test]
fn a_stale_cvport_marker_refuses_instead_of_clamping() {
    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    create_viewports(&mut app, &mut session, 3);
    cvport(&mut app, &mut session, 3);
    // Shrink the layout to one viewport: the marker now names a viewport that
    // no longer exists.
    create_viewports(&mut app, &mut session, 1);
    let before = revision(&app);

    let report = refusal(app.execute(
        &mut session,
        command(CommandId::Cvport, CommandPayload::None),
    ));
    assert!(
        report.contains("cvport.out_of_range") && report.contains("1..=1"),
        "{report}"
    );
    let scale = refusal(app.execute(
        &mut session,
        command(
            CommandId::VpScale,
            layout_payload(LayoutPayload::Scale {
                name: "1:50".into(),
                viewport: None,
            }),
        ),
    ));
    assert!(
        scale.contains("vpscale.current_viewport_out_of_range"),
        "{scale}"
    );
    assert_eq!(revision(&app), before, "a stale marker writes nothing");
}

// ---------------------------------------------------------------- LAYVPI ---

#[test]
fn layvpi_without_an_index_excludes_the_current_viewport() {
    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    create_viewports(&mut app, &mut session, 3);
    cvport(&mut app, &mut session, 2);
    let before = revision(&app);

    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::LayVpi,
                layout_payload(LayoutPayload::FreezeViewportLayers {
                    layers: vec!["WALLS".into()],
                }),
            ),
        )
        .expect("LAYVPI freezes every viewport except the current one");
    assert_eq!(codes(&outcome), vec!["layvpi.frozen"]);
    assert!(
        outcome.diagnostics[0]
            .message
            .contains("except the current one"),
        "{}",
        outcome.diagnostics[0].message
    );
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert_eq!(viewport_override(&app, 1, 0), Some(true));
    assert_eq!(
        viewport_override(&app, 1, 1),
        None,
        "the current viewport (CVPORT 2) is excluded"
    );
    assert_eq!(viewport_override(&app, 1, 2), Some(true));
    assert_eq!(
        viewport_override(&app, 2, 0),
        None,
        "other layout untouched"
    );
    assert!(drawing(&app).layer(LayerId(1)).unwrap().visible);
}

// ---------------------------------------------------------------- VPSCALE ---

#[test]
fn vpscale_targets_the_current_viewport_then_an_explicit_index() {
    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    create_viewports(&mut app, &mut session, 3);
    cvport(&mut app, &mut session, 2);
    let before = revision(&app);

    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::VpScale,
                layout_payload(LayoutPayload::Scale {
                    name: "1:50".into(),
                    viewport: None,
                }),
            ),
        )
        .expect("VPSCALE rescales the current viewport");
    assert_eq!(codes(&outcome), vec!["vpscale.applied"]);
    assert!(
        outcome.diagnostics[0].message.contains("viewport 1"),
        "{}",
        outcome.diagnostics[0].message
    );
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    let layout = drawing(&app).layout(LayoutId(1)).expect("layout exists");
    // paper/drawing = 1/50, so the stored paper→model diagonal is 50.
    assert_eq!(layout.viewports[1].model_to_paper.matrix[0][0], 50.0);
    assert_eq!(layout.viewports[1].model_to_paper.matrix[1][1], 50.0);
    assert_eq!(
        layout.viewports[0].model_to_paper.matrix[0][0], 1.0,
        "the other viewports keep their scale"
    );
    assert_eq!(
        layout.viewports[2].model_to_paper.matrix[0][0], 1.0,
        "the other viewports keep their scale"
    );
    // The model anchor of the target is the same one the pre-scale transform
    // showed at its clip centre (a grid cell centred on the model origin).
    let view = cad_representation::viewport_transform(&layout.viewports[1]);
    match view {
        cad_representation::ViewportState::Supported(view) => {
            assert_eq!(view.model_anchor, point(0.0, 0.0));
            assert!((view.paper_per_model - 0.02).abs() < 1e-12);
        }
        other => panic!("the scaled viewport must stay supported, got {other:?}"),
    }

    // An explicit index wins over the marker.
    app.execute(
        &mut session,
        command(
            CommandId::VpScale,
            layout_payload(LayoutPayload::Scale {
                name: "1:100".into(),
                viewport: Some(2),
            }),
        ),
    )
    .expect("VPSCALE with an explicit index targets that viewport");
    let layout = drawing(&app).layout(LayoutId(1)).expect("layout exists");
    assert_eq!(layout.viewports[2].model_to_paper.matrix[0][0], 100.0);
    assert_eq!(
        layout.viewports[1].model_to_paper.matrix[0][0], 50.0,
        "the current viewport was not retargeted"
    );

    // Out of range refuses by name and writes nothing.
    let after = revision(&app);
    let out_of_range = refusal(app.execute(
        &mut session,
        command(
            CommandId::VpScale,
            layout_payload(LayoutPayload::Scale {
                name: "1:100".into(),
                viewport: Some(3),
            }),
        ),
    ));
    assert!(
        out_of_range.contains("vpscale.unknown_viewport"),
        "{out_of_range}"
    );
    assert_eq!(revision(&app), after);
}

// ----------------------------------------------------------------- VPCLIP ---

#[test]
fn vpclip_targets_the_current_viewport_and_an_explicit_index_wins() {
    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    create_viewports(&mut app, &mut session, 3);
    cvport(&mut app, &mut session, 2);
    let before = revision(&app);

    let polygon = vec![
        point(-5.0, -5.0),
        point(5.0, -5.0),
        point(5.0, 5.0),
        point(-5.0, 5.0),
    ];
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::VpClip,
                layout_payload(LayoutPayload::ClipPolygon {
                    points: polygon.clone(),
                    viewport: None,
                }),
            ),
        )
        .expect("VPCLIP clips the current viewport");
    assert_eq!(codes(&outcome), vec!["vpclip.applied_polygon"]);
    assert!(
        outcome.diagnostics[0].message.contains("viewport 1"),
        "{}",
        outcome.diagnostics[0].message
    );
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    let layout = drawing(&app).layout(LayoutId(1)).expect("layout exists");
    assert_eq!(
        layout.viewports[1].clip, polygon,
        "the clip landed on the current viewport only"
    );
    assert_eq!(
        layout.viewports[0].clip.len(),
        4,
        "the other viewports keep their clip"
    );
    assert_eq!(layout.viewports[2].clip.len(), 4);

    // Explicit index overrides the marker.
    let rect_min = point(1.0, 1.0);
    let rect_max = point(3.0, 3.0);
    app.execute(
        &mut session,
        command(
            CommandId::VpClip,
            layout_payload(LayoutPayload::ClipRect {
                min: rect_min,
                max: rect_max,
                viewport: Some(2),
            }),
        ),
    )
    .expect("VPCLIP RECT with an explicit index targets that viewport");
    let layout = drawing(&app).layout(LayoutId(1)).expect("layout exists");
    assert_eq!(
        layout.viewports[2].clip,
        vec![
            point(1.0, 1.0),
            point(3.0, 1.0),
            point(3.0, 3.0),
            point(1.0, 3.0),
        ]
    );
    assert_eq!(
        layout.viewports[1].clip, polygon,
        "current viewport untouched"
    );

    let after = revision(&app);
    let out_of_range = refusal(app.execute(
        &mut session,
        command(
            CommandId::VpClip,
            layout_payload(LayoutPayload::ClipRect {
                min: rect_min,
                max: rect_max,
                viewport: Some(9),
            }),
        ),
    ));
    assert!(
        out_of_range.contains("vpclip.unknown_viewport"),
        "{out_of_range}"
    );
    assert_eq!(revision(&app), after, "a refusal writes nothing");

    undo(&mut app, &mut session);
    let layout = drawing(&app).layout(LayoutId(1)).expect("layout exists");
    assert_eq!(
        layout.viewports[2].clip.len(),
        4,
        "undo restores the explicit-index write"
    );
    assert_eq!(layout.viewports[1].clip, polygon, "the first write stands");
}

/// `LAYVPI <layer>[,...]`: the no-index (every viewport except the current) form.
fn layvpi_command(layers: &[&str]) -> Command {
    command(
        CommandId::LayVpi,
        layout_payload(LayoutPayload::FreezeViewportLayers {
            layers: layers.iter().map(|name| (*name).to_string()).collect(),
        }),
    )
}

#[test]
fn layvpi_without_an_index_excludes_the_current_viewport_and_round_trips() {
    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    create_viewports(&mut app, &mut session, 3);
    // The middle viewport is the current one: it must be the only one left out.
    cvport(&mut app, &mut session, 2);
    let before = revision(&app);
    let depth = undo_depth(&app);

    let outcome = app
        .execute(&mut session, layvpi_command(&["WALLS"]))
        .expect("LAYVPI freezes every viewport except the current one");
    assert_eq!(codes(&outcome), vec!["layvpi.frozen"]);
    assert!(outcome.diagnostics[0].message.contains("WALLS"));
    assert!(
        outcome.diagnostics[0]
            .message
            .contains("except the current one"),
        "{}",
        outcome.diagnostics[0].message
    );
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert_eq!(undo_depth(&app), depth + 1, "exactly one undo step");
    assert_eq!(viewport_override(&app, 1, 0), Some(true));
    assert_eq!(
        viewport_override(&app, 1, 1),
        None,
        "the current viewport (CVPORT 2) is excluded"
    );
    assert_eq!(viewport_override(&app, 1, 2), Some(true));
    assert_eq!(
        viewport_override(&app, 2, 0),
        None,
        "other layout untouched"
    );
    assert!(drawing(&app).layer(LayerId(1)).unwrap().visible);

    undo(&mut app, &mut session);
    assert_eq!(viewport_override(&app, 1, 0), None, "undo restores the map");
    redo(&mut app, &mut session);
    assert_eq!(
        viewport_override(&app, 1, 0),
        Some(true),
        "redo reapplies it"
    );
}

#[test]
fn layvpi_refusals_and_the_unchanged_case_write_nothing() {
    let (mut empty, mut empty_session) = fixture(&[(1, "Sheet1", false)]);
    switch_to(&mut empty, &mut empty_session, "Sheet1");
    let empty_before = revision(&empty);
    let no_viewport = refusal(empty.execute(&mut empty_session, layvpi_command(&["WALLS"])));
    assert!(no_viewport.contains("layvpi.no_viewport"), "{no_viewport}");
    assert_eq!(revision(&empty), empty_before);
    assert!(!undo_available(&empty), "a refusal records nothing");

    // A single-viewport layout cannot exclude its own only viewport: the
    // no-argument form refuses by name instead of freezing everywhere.
    let (mut single, mut single_session) = standard_fixture();
    switch_to(&mut single, &mut single_session, "Sheet1");
    let single_before = revision(&single);
    let only = refusal(single.execute(&mut single_session, layvpi_command(&["WALLS"])));
    assert!(only.contains("layvpi.no_other_viewport"), "{only}");
    assert_eq!(revision(&single), single_before);
    assert!(!undo_available(&single), "a refusal records nothing");
    // The explicit-index form is the documented way to freeze that viewport.
    let targeted = single
        .execute(
            &mut single_session,
            command(
                CommandId::LayVpi,
                layout_payload(LayoutPayload::FreezeViewportLayersIn {
                    layers: vec!["WALLS".into()],
                    viewport: 0,
                }),
            ),
        )
        .expect("the explicit-index form still freezes one viewport");
    assert_eq!(codes(&targeted), vec!["layvpi.frozen"]);
    assert_eq!(viewport_override(&single, 1, 0), Some(true));

    let (mut app, mut session) = standard_fixture();
    switch_to(&mut app, &mut session, "Sheet1");
    create_viewports(&mut app, &mut session, 3);
    let before = revision(&app);
    let depth = undo_depth(&app);
    let unknown = refusal(app.execute(&mut session, layvpi_command(&["NOPE"])));
    assert!(unknown.contains("layvpi.unknown_layer"), "{unknown}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), depth, "a refusal records nothing");

    app.execute(&mut session, layvpi_command(&["WALLS"]))
        .expect("the first LAYVPI writes");
    let written = revision(&app);
    let depth = undo_depth(&app);
    let outcome = app
        .execute(&mut session, layvpi_command(&["WALLS"]))
        .expect("a repeated LAYVPI is a declared no-op");
    assert_eq!(codes(&outcome), vec!["layvpi.unchanged"]);
    assert_eq!(revision(&app), written, "no revision bump");
    assert_eq!(undo_depth(&app), depth, "no new undo step");
}
