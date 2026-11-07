//! Contract tests for the 2026-11-07 layout / viewport batch.
//!
//! The real read commands (`LAYOUT LIST`, `LAYOUT SET`, `VPMAX`, `VPMIN`) are
//! exercised end to end. Every layout/viewport write is asserted to be a
//! **named refusal** with no revision bump and no undo entry — the database has
//! no post-import layout/viewport write API, so a refusal is the honest result.

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
    }
}

/// A document with layer `0`, `WALLS`, `HIDDEN`, a `1:100` scale and the given
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
    app.execute(
        &mut session,
        command(
            CommandId::Layout,
            layout_payload(LayoutPayload::Set {
                name: "Sheet2".into(),
            }),
        ),
    )
    .expect("a known layout switches");
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
fn layout_new_refuses_duplicates_illegal_names_and_the_missing_write_api() {
    let (mut app, mut session) = standard_fixture();
    let before = revision(&app);
    let new = |name: &str| layout_payload(LayoutPayload::New { name: name.into() });
    let duplicate = refusal(app.execute(&mut session, command(CommandId::Layout, new("sheet1"))));
    assert!(duplicate.contains("layout.duplicate_name"), "{duplicate}");
    let illegal = refusal(app.execute(&mut session, command(CommandId::Layout, new("Bad/Name"))));
    assert!(illegal.contains("layout.invalid_name"), "{illegal}");
    let valid = refusal(app.execute(&mut session, command(CommandId::Layout, new("Sheet3"))));
    assert!(valid.contains("layout.write_unsupported"), "{valid}");
    assert_eq!(revision(&app), before, "a refusal writes nothing");
    assert!(!undo_available(&app));
}

#[test]
fn layout_rename_refuses_unknown_duplicate_and_missing_write_api() {
    let (mut app, mut session) = standard_fixture();
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
    let valid = refusal(app.execute(
        &mut session,
        command(CommandId::Layout, rename("Sheet1", "Sheet3")),
    ));
    assert!(valid.contains("layout.write_unsupported"), "{valid}");
}

#[test]
fn layout_delete_refuses_unknown_the_last_layout_and_missing_write_api() {
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
    let valid = refusal(app.execute(
        &mut session,
        command(
            CommandId::Layout,
            layout_payload(LayoutPayload::Delete {
                name: "Sheet1".into(),
            }),
        ),
    ));
    assert!(valid.contains("layout.write_unsupported"), "{valid}");

    // A one-layout drawing refuses the last-layout delete before the write
    // capability check.
    let (mut app, mut session) = fixture(&[(1, "Only", true)]);
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
}

#[test]
fn layoutmerge_refuses_missing_same_and_missing_write_api() {
    let (mut app, mut session) = standard_fixture();
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
    let valid = refusal(app.execute(
        &mut session,
        command(CommandId::LayoutMerge, merge("Sheet1", "Sheet2")),
    ));
    assert!(valid.contains("layoutmerge.write_unsupported"), "{valid}");
}

#[test]
fn layoutwizard_names_the_page_setup_boundary() {
    let (mut app, mut session) = standard_fixture();
    let wizard = |name: &str, paper: Option<PaperSetup>| {
        layout_payload(LayoutPayload::Wizard {
            name: name.into(),
            paper,
        })
    };
    let with_paper = refusal(app.execute(
        &mut session,
        command(
            CommandId::LayoutWizard,
            wizard(
                "Sheet3",
                Some(PaperSetup {
                    name: "A4".into(),
                    width: 210.0,
                    height: 297.0,
                }),
            ),
        ),
    ));
    assert!(
        with_paper.contains("layoutwizard.write_unsupported"),
        "{with_paper}"
    );
    assert!(with_paper.contains("A4"), "{with_paper}");
    assert!(with_paper.contains("not applied"), "{with_paper}");
    let no_paper = refusal(app.execute(
        &mut session,
        command(CommandId::LayoutWizard, wizard("Sheet3", None)),
    ));
    assert!(
        no_paper.contains("layoutwizard.write_unsupported"),
        "{no_paper}"
    );
}

#[test]
fn vports_refuses_model_space_and_names_the_missing_viewport_write() {
    let (mut app, mut session) = standard_fixture();
    let grid = LayoutPayload::Viewports { rows: 2, cols: 3 };
    let model = refusal(app.execute(
        &mut session,
        command(CommandId::Vports, layout_payload(grid.clone())),
    ));
    assert!(model.contains("vports.paper_space_required"), "{model}");

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
    let paper = refusal(app.execute(
        &mut session,
        command(CommandId::Vports, layout_payload(grid)),
    ));
    assert!(paper.contains("vports.write_unsupported"), "{paper}");
    assert!(paper.contains("6"), "the grid size is reported: {paper}");

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
fn vpclip_refuses_degenerate_rectangles_and_the_missing_viewport_selection() {
    let (mut app, mut session) = standard_fixture();
    app.execute(
        &mut session,
        command(
            CommandId::Layout,
            layout_payload(LayoutPayload::Set {
                name: "Sheet1".into(),
            }),
        ),
    )
    .unwrap();
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
    let rect = refusal(app.execute(
        &mut session,
        command(
            CommandId::VpClip,
            layout_payload(LayoutPayload::ClipRect {
                min: point(0.0, 0.0),
                max: point(100.0, 50.0),
            }),
        ),
    ));
    assert!(
        rect.contains("vpclip.viewport_selection_unsupported"),
        "{rect}"
    );
    assert!(rect.contains("NOT IMPLEMENTED"), "{rect}");
}

#[test]
fn vpscale_reports_the_resolved_scale_then_the_missing_write() {
    let (mut app, mut session) = standard_fixture();
    app.execute(
        &mut session,
        command(
            CommandId::Layout,
            layout_payload(LayoutPayload::Set {
                name: "Sheet1".into(),
            }),
        ),
    )
    .unwrap();
    let valid = refusal(app.execute(
        &mut session,
        command(
            CommandId::VpScale,
            layout_payload(LayoutPayload::Scale {
                name: "1:100".into(),
            }),
        ),
    ));
    assert!(valid.contains("vpscale.write_unsupported"), "{valid}");
    assert!(valid.contains("0.01"), "{valid}");
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
}

#[test]
fn layvpi_refuses_per_viewport_layer_state_and_never_freezes_globally() {
    let (mut app, mut session) = standard_fixture();
    app.execute(
        &mut session,
        command(
            CommandId::Layout,
            layout_payload(LayoutPayload::Set {
                name: "Sheet1".into(),
            }),
        ),
    )
    .unwrap();
    let refusal_text = refusal(app.execute(
        &mut session,
        command(
            CommandId::LayVpi,
            layout_payload(LayoutPayload::FreezeViewportLayers {
                layers: vec!["WALLS".into()],
            }),
        ),
    ));
    assert!(
        refusal_text.contains("layvpi.viewport_layer_state_unsupported"),
        "{refusal_text}"
    );
    assert!(
        refusal_text.contains("never freezes them globally"),
        "{refusal_text}"
    );
    let unknown = refusal(app.execute(
        &mut session,
        command(
            CommandId::LayVpi,
            layout_payload(LayoutPayload::FreezeViewportLayers {
                layers: vec!["NOPE".into()],
            }),
        ),
    ));
    assert!(unknown.contains("layvpi.unknown_layer"), "{unknown}");
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

    app.execute(
        &mut session,
        command(
            CommandId::Layout,
            layout_payload(LayoutPayload::Set {
                name: "Sheet1".into(),
            }),
        ),
    )
    .unwrap();
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
