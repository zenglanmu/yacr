use std::collections::BTreeMap;

use super::*;

// ---------------------------------------------------------------------------
// LAYTHW
// ---------------------------------------------------------------------------

#[test]
fn laythw_thaws_every_frozen_layer_and_leaves_the_viewport_flag_alone() {
    let frozen = LayerAttributes {
        frozen: true,
        ..LayerAttributes::default()
    };
    let frozen_new_viewport = LayerAttributes {
        frozen: true,
        frozen_in_new_viewport: true,
        ..LayerAttributes::default()
    };
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", frozen.clone()),
            (LayerId(3), "WALLS", frozen_new_viewport),
            (LayerId(4), "DETAIL", LayerAttributes::default()),
        ],
        vec![line(1, LayerId(3), 0.0)],
    );
    // LAYTHW takes no selection: an existing selection is ignored, never refused.
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayThw,
        CommandPayload::None,
    )
    .expect("LAYTHW thaws all frozen layers");

    assert!(
        !layer_attributes(&app, LayerId(0)).frozen,
        "the current layer is thawed too"
    );
    assert!(!layer_attributes(&app, LayerId(3)).frozen);
    assert!(!layer_attributes(&app, LayerId(4)).frozen);
    assert!(
        layer_attributes(&app, LayerId(3)).frozen_in_new_viewport,
        "the new-viewport flag is not touched"
    );
    assert_eq!(
        revision(&app),
        Revision(before.0 .0 + 2),
        "one bump per changed layer"
    );
    assert_eq!(
        undo_depth(&app),
        before.1 + 1,
        "one undo step for the whole thaw batch"
    );
    assert_eq!(diagnostic_codes(&outcome), vec!["laythw.applied"]);
    let message = &outcome.diagnostics[0].message;
    assert!(message.contains("2 layer(s) at drawing level"), "{message}");
    assert!(message.contains("in any viewport"), "{message}");
    assert!(message.contains("one transaction"), "{message}");

    // Undo re-freezes the two changed rows exactly; redo thaws them again.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYTHW");
    assert!(layer_attributes(&app, LayerId(0)).frozen);
    assert!(layer_attributes(&app, LayerId(3)).frozen);
    assert!(layer_attributes(&app, LayerId(3)).frozen_in_new_viewport);
    assert!(!layer_attributes(&app, LayerId(4)).frozen);
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo LAYTHW");
    assert!(!layer_attributes(&app, LayerId(0)).frozen);
    assert!(!layer_attributes(&app, LayerId(3)).frozen);
}

#[test]
fn laythw_refuses_a_payload_and_changes_nothing() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (
                LayerId(3),
                "WALLS",
                LayerAttributes {
                    frozen: true,
                    ..LayerAttributes::default()
                },
            ),
        ],
        vec![line(1, LayerId(0), 0.0)],
    );
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayThw,
        CommandPayload::Points(vec![point(0.0, 0.0)]),
    )
    .expect_err("LAYTHW takes no payload");
    assert!(refusal(error).starts_with("laythw.payload:"));
    assert!(layer_attributes(&app, LayerId(3)).frozen);
    assert_untouched(&app, before);
}

/// Layers `0`/`WALLS`/`DOORS` (with `WALLS` optionally frozen at drawing
/// level), entity 1 on `WALLS`, and one `SheetN` layout per entry of
/// `layouts`; each inner vector is that layout's viewports, each map that
/// viewport's per-layer override state (`true` = frozen in that viewport only).
fn layout_app(
    freeze_walls: bool,
    layouts: Vec<Vec<BTreeMap<LayerId, bool>>>,
) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [
        (LayerId(0), "0"),
        (LayerId(3), "WALLS"),
        (LayerId(5), "DOORS"),
    ] {
        let attributes = if id == LayerId(3) && freeze_walls {
            LayerAttributes {
                frozen: true,
                ..LayerAttributes::default()
            }
        } else {
            LayerAttributes::default()
        };
        let visible = attributes.is_visible();
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible,
            })
            .unwrap();
        builder.set_layer_attributes(id, attributes).unwrap();
    }
    for (index, viewports) in layouts.into_iter().enumerate() {
        builder
            .insert_layout(Layout {
                id: LayoutId(index as u128 + 1),
                name: format!("Sheet{}", index + 1),
                viewports: viewports
                    .into_iter()
                    .map(|layer_overrides| PaperViewport {
                        clip: vec![
                            point(0.0, 0.0),
                            point(100.0, 0.0),
                            point(100.0, 50.0),
                            point(0.0, 50.0),
                        ],
                        model_to_paper: Transform3::scale(100.0),
                        completeness: Completeness::Complete,
                        layer_overrides,
                    })
                    .collect(),
            })
            .unwrap();
    }
    builder.insert_entity(line(1, LayerId(3), 0.0)).unwrap();
    app_with_database(builder.finish().unwrap())
}

fn overrides(pairs: &[(LayerId, bool)]) -> BTreeMap<LayerId, bool> {
    pairs.iter().copied().collect()
}

fn viewport_overrides(app: &Application, layout: LayoutId) -> Vec<BTreeMap<LayerId, bool>> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .layout(layout)
        .unwrap_or_else(|| panic!("layout {} must exist", layout.0))
        .viewports
        .iter()
        .map(|viewport| viewport.layer_overrides.clone())
        .collect()
}

#[test]
fn laythw_thaws_a_layer_frozen_in_every_viewport_of_every_layout() {
    let (mut app, mut session) = layout_app(
        true,
        vec![
            vec![
                overrides(&[(LayerId(3), true), (LayerId(5), false)]),
                overrides(&[(LayerId(3), true)]),
            ],
            vec![overrides(&[(LayerId(3), true)])],
        ],
    );
    let before = state(&app);
    let clips_before: Vec<_> = app.workspace.documents[&DocumentId(1)]
        .drawing
        .layouts()
        .map(|layout| {
            layout
                .viewports
                .iter()
                .map(|viewport| (viewport.clip.clone(), viewport.model_to_paper))
                .collect::<Vec<_>>()
        })
        .collect();

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayThw,
        CommandPayload::None,
    )
    .expect("LAYTHW thaws every layer in every viewport");

    assert!(
        !layer_attributes(&app, LayerId(3)).frozen,
        "the drawing-level flag is thawed too"
    );
    assert_eq!(
        viewport_overrides(&app, LayoutId(1)),
        vec![overrides(&[(LayerId(5), false)]), BTreeMap::new()],
        "every frozen override is cleared; the explicit non-frozen override on \
         layer 5 survives"
    );
    assert_eq!(
        viewport_overrides(&app, LayoutId(2)),
        vec![BTreeMap::new()],
        "the second layout is thawed in the same step"
    );
    let clips_after: Vec<_> = app.workspace.documents[&DocumentId(1)]
        .drawing
        .layouts()
        .map(|layout| {
            layout
                .viewports
                .iter()
                .map(|viewport| (viewport.clip.clone(), viewport.model_to_paper))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        clips_after, clips_before,
        "only the frozen state changes: viewport geometry and views are preserved"
    );
    assert_eq!(
        revision(&app),
        Revision(before.0 .0 + 3),
        "one bump for the layer row and one per changed layout"
    );
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");
    assert_eq!(diagnostic_codes(&outcome), vec!["laythw.applied"]);
    let message = &outcome.diagnostics[0].message;
    assert!(message.contains("1 layer(s) at drawing level"), "{message}");
    assert!(
        message.contains("3 frozen viewport override(s) in 2 layout(s)"),
        "{message}"
    );
    assert!(message.contains("one transaction"), "{message}");
}

#[test]
fn laythw_undo_restores_the_viewport_overrides_and_redo_clears_them_again() {
    let (mut app, mut session) = layout_app(
        false,
        vec![vec![
            overrides(&[(LayerId(3), true)]),
            overrides(&[(LayerId(3), true), (LayerId(5), true)]),
        ]],
    );
    let before = state(&app);

    execute(
        &mut app,
        &mut session,
        CommandId::LayThw,
        CommandPayload::None,
    )
    .expect("a viewport-only freeze is thawed even when the layer table is thawed");
    assert!(
        !layer_attributes(&app, LayerId(3)).frozen,
        "the layer table was already thawed and stays thawed"
    );
    assert_eq!(
        viewport_overrides(&app, LayoutId(1)),
        vec![BTreeMap::new(), BTreeMap::new()]
    );
    assert_eq!(
        revision(&app),
        Revision(before.0 .0 + 1),
        "one bump for the one changed layout"
    );
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYTHW");
    assert_eq!(
        viewport_overrides(&app, LayoutId(1)),
        vec![
            overrides(&[(LayerId(3), true)]),
            overrides(&[(LayerId(3), true), (LayerId(5), true)]),
        ],
        "undo restores each viewport's exact override map"
    );
    assert!(!layer_attributes(&app, LayerId(3)).frozen);

    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo LAYTHW");
    assert_eq!(
        viewport_overrides(&app, LayoutId(1)),
        vec![BTreeMap::new(), BTreeMap::new()],
        "redo clears them again"
    );
}

#[test]
fn laythw_reports_unchanged_when_nothing_is_frozen() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", LayerAttributes::default()),
        ],
        Vec::new(),
    );
    let before = state(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayThw,
        CommandPayload::None,
    )
    .expect("nothing to thaw is a no-op");
    assert_eq!(state(&app), before);
    assert!(outcome.changes.is_none());
    assert_eq!(diagnostic_codes(&outcome), vec!["laythw.unchanged"]);
}
