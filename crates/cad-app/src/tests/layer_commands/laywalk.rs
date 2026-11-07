//! `LAYWALK` (AutoCAD 2026): show only the named layers plus the active layer.
//!
//! Declared divergence: the layer names are typed on the command line
//! (`LAYWALK WALLS,DIMS`) instead of being picked in AutoCAD's LayerWalk dialog.
//! The result is **overrides only** — the same validated atomic session-override
//! batch as `LAYISO`, with no database write, revision bump or history entry.
//! `LAYUNISO` / `RESTORELAYERS` clears it.

use super::*;

fn layers() -> Vec<(LayerId, &'static str, LayerAttributes)> {
    vec![
        (LayerId(0), "0", LayerAttributes::default()),
        (LayerId(3), "WALLS", LayerAttributes::default()),
        (LayerId(5), "DOORS", LayerAttributes::default()),
        (LayerId(6), "ROOF", LayerAttributes::default()),
    ]
}

fn walk(
    app: &mut Application,
    session: &mut SessionState,
    names: &[&str],
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::LayWalk,
        CommandPayload::LayerWalk(names.iter().map(|name| name.to_string()).collect()),
    )
}

#[test]
fn laywalk_keeps_only_the_named_layers_plus_the_active_layer() {
    let (mut app, mut session) = app_with(layers(), Vec::new());
    // The default active layer is `0`; `WALLS`/`DOORS` are named, `ROOF` hides.
    let before = state(&app);
    let outcome = walk(&mut app, &mut session, &["WALLS", "DOORS"]).expect("LAYWALK");

    assert_eq!(session.layer_overrides.get(LayerId(6)), Some(false));
    assert_eq!(
        session.layer_overrides.get(LayerId(3)),
        None,
        "named layer is kept"
    );
    assert_eq!(
        session.layer_overrides.get(LayerId(0)),
        None,
        "active layer is kept"
    );
    assert_eq!(state(&app), before, "no database write and no history");
    assert!(diagnostic_codes(&outcome).contains(&"layers.walk.applied"));
}

#[test]
fn laywalk_adds_the_active_layer_even_when_it_is_not_named() {
    let (mut app, mut session) = app_with(layers(), Vec::new());
    session.active_layer = LayerId(5);
    walk(&mut app, &mut session, &["WALLS"]).expect("LAYWALK");

    assert_eq!(
        session.layer_overrides.get(LayerId(5)),
        None,
        "active layer is kept"
    );
    assert_eq!(session.layer_overrides.get(LayerId(3)), None);
    assert_eq!(session.layer_overrides.get(LayerId(0)), Some(false));
    assert_eq!(session.layer_overrides.get(LayerId(6)), Some(false));
}

#[test]
fn laywalk_refuses_an_empty_or_unknown_name_list_and_a_bad_payload() {
    let (mut app, mut session) = app_with(layers(), Vec::new());
    let before = state(&app);

    let error = walk(&mut app, &mut session, &[]).expect_err("an empty list is refused");
    assert!(refusal(error).starts_with("layers.walk.needs_argument:"));
    assert_untouched(&app, before);
    assert!(session.layer_overrides.is_empty());

    let error = walk(&mut app, &mut session, &["NOPE"]).expect_err("unknown layer");
    assert!(refusal(error).starts_with("layers.walk.unknown_layer:"));
    assert_untouched(&app, before);
    assert!(session.layer_overrides.is_empty());

    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayWalk,
        CommandPayload::None,
    )
    .expect_err("a bare LAYWALK is refused");
    assert!(refusal(error).starts_with("layers.walk.needs_argument:"));
    assert_untouched(&app, before);
}

#[test]
fn laywalk_reports_a_noop_when_every_layer_is_already_kept() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", LayerAttributes::default()),
        ],
        Vec::new(),
    );
    let before = state(&app);
    let outcome = walk(&mut app, &mut session, &["WALLS"]).expect("the no-op is reported");
    assert_eq!(state(&app), before);
    assert!(session.layer_overrides.is_empty());
    assert_eq!(diagnostic_codes(&outcome), vec!["layers.walk.noop"]);
}
