//! `SETLAYERPROPERTY` against the session's **current** layer.
//!
//! The mirror image of the active-layer admission rule
//! (`app_drawing::drawing_create`'s `ensure_layer_can_be_current`, covered in
//! `tests::layers`): a layer that is already current must not be hidden
//! afterwards. The gap this closes is explicit — the shared create funnel does
//! **not** look at visibility, so turning the current layer off or freezing it
//! would happily store geometry the user can neither see nor pick.
//!
//! The rule lives at the command layer
//! (`app_drawing::layer_edit::ensure_current_layer_stays_visible`) because only
//! the session knows which layer is current; `cad-db` keeps owning
//! `Layer::visible == !(off || frozen)` and its API is unchanged. Contracts
//! pinned here, all against the real command funnel:
//!
//! * `off = true` / `frozen = true` on the current layer is refused with
//!   `layer.current_off` / `layer.current_frozen`, **before** any write: the
//!   attributes, `Layer::visible`, the revision, the session and the history are
//!   all unchanged;
//! * clearing either flag (`false`) on the current layer is accepted — it can
//!   only make the layer more visible;
//! * a **non**-current layer can still be turned off or frozen (the panel's
//!   chips keep working for every other row), and so can the previous layer once
//!   the session points elsewhere;
//! * `locked = true` on the current layer is still accepted: locking is a write
//!   permission, not visibility;
//! * the refusal is an ordinary [`CadError::InvalidInput`] whose message starts
//!   with the stable code and names the layer, so the panel path needs no
//!   special case.

use super::layers::{app_with_walls_attributes, drawing, execute, revision};
use super::*;
use cad_db::LayerAttributes;

/// `WALLS` (id 3) — its id and name are what every refusal must carry.
const WALLS: LayerId = LayerId(3);

/// The stored, **derived** visibility flag of `WALLS`.
fn visible(app: &Application) -> bool {
    drawing(app).layer(WALLS).expect("the WALLS row").visible
}

/// The stored attributes of `WALLS`.
fn attributes(app: &Application) -> LayerAttributes {
    drawing(app).layer_attributes(WALLS)
}

/// Make `layer` the session's current layer through the real command.
///
/// Only used for layers that are on, thawed and unlocked: an invisible or locked
/// layer cannot become current (that refusal is pinned in `tests::layers`), so a
/// test that needs an invisible current layer sets `session.active_layer`
/// directly instead.
fn make_current(app: &mut Application, session: &mut SessionState, layer: LayerId) {
    execute(
        app,
        session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(layer),
    )
    .expect("an on, thawed and unlocked layer may become current");
}

fn set_property(
    app: &mut Application,
    session: &mut SessionState,
    layer: &str,
    key: &str,
    value: &str,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::SetLayerProperty,
        CommandPayload::SetLayerProperty {
            layer: layer.to_string(),
            key: key.to_string(),
            value: value.to_string(),
        },
    )
}

/// A current-layer refusal: an ordinary command error whose
/// message starts with `code` and names the layer and the rule.
fn assert_current_layer_refusal(error: &CadError, code: &str) {
    let CadError::InvalidInput(message) = error else {
        panic!("expected an ordinary InvalidInput refusal, got {error:?}");
    };
    assert!(
        message.starts_with(code),
        "the message starts with the stable code '{code}': {message}"
    );
    assert!(
        message.contains("WALLS"),
        "the refusal names the layer: {message}"
    );
    assert!(
        message.contains("current layer"),
        "the refusal says the layer is current: {message}"
    );
}

#[test]
fn turning_the_current_layer_off_is_refused_before_any_write() {
    let (mut app, mut session) = app_with_walls_attributes(LayerAttributes::default());
    make_current(&mut app, &mut session, WALLS);
    let before_revision = revision(&app);
    let before_attributes = attributes(&app);
    let before_visible = visible(&app);
    let before_entities = drawing(&app).entity_count();

    let error = set_property(&mut app, &mut session, "WALLS", "off", "true")
        .expect_err("an off current layer would hide the geometry it exists to receive");
    assert_current_layer_refusal(&error, "layer.current_off");

    assert_eq!(
        attributes(&app),
        before_attributes,
        "the stored attributes are byte-identical"
    );
    assert_eq!(visible(&app), before_visible, "Layer::visible is unchanged");
    assert!(visible(&app), "the layer is still shown");
    assert_eq!(
        revision(&app),
        before_revision,
        "a refusal is not a database write"
    );
    assert_eq!(
        drawing(&app).entity_count(),
        before_entities,
        "no entity appeared or vanished"
    );
    assert_eq!(session.active_layer, WALLS, "the session is unchanged");
    assert!(
        !app.can_undo(&DocumentId(1)),
        "a refusal records no history entry"
    );
}

#[test]
fn freezing_the_current_layer_is_refused_before_any_write() {
    let (mut app, mut session) = app_with_walls_attributes(LayerAttributes::default());
    make_current(&mut app, &mut session, WALLS);
    let before_revision = revision(&app);
    let before_attributes = attributes(&app);
    let before_visible = visible(&app);

    let error = set_property(&mut app, &mut session, "WALLS", "frozen", "true")
        .expect_err("a frozen current layer would hide the geometry it exists to receive");
    assert_current_layer_refusal(&error, "layer.current_frozen");

    assert_eq!(
        attributes(&app),
        before_attributes,
        "the stored attributes are byte-identical"
    );
    assert_eq!(visible(&app), before_visible, "Layer::visible is unchanged");
    assert!(visible(&app), "the layer is still shown");
    assert_eq!(
        revision(&app),
        before_revision,
        "a refusal is not a database write"
    );
    assert_eq!(session.active_layer, WALLS, "the session is unchanged");
    assert!(
        !app.can_undo(&DocumentId(1)),
        "a refusal records no history entry"
    );
}

#[test]
fn clearing_off_and_frozen_on_the_current_layer_is_allowed() {
    // An invisible layer cannot *become* current through the command, so the
    // fixture session is pointed at it directly: the rule under test is the write
    // (does it hide the current layer?), not the admission.
    let (mut app, mut session) = app_with_walls_attributes(LayerAttributes {
        off: true,
        frozen: true,
        ..LayerAttributes::default()
    });
    session.active_layer = WALLS;
    assert!(!visible(&app), "the fixture really starts hidden");
    let before = revision(&app);

    set_property(&mut app, &mut session, "WALLS", "off", "false")
        .expect("turning the current layer on only makes it more visible");
    assert!(
        !visible(&app),
        "the other flag still holds: `frozen` keeps the row hidden"
    );
    assert_eq!(revision(&app), Revision(before.0 + 1), "a real change");

    set_property(&mut app, &mut session, "WALLS", "frozen", "false")
        .expect("thawing the current layer only makes it more visible");
    assert!(visible(&app), "both flags cleared: the row is shown again");
    assert_eq!(revision(&app), Revision(before.0 + 2), "two real changes");
    assert_eq!(
        session.active_layer, WALLS,
        "a layer-table write never moves the current layer"
    );
}

#[test]
fn a_non_current_layer_can_still_be_turned_off_and_frozen() {
    // Regression for the panel: deciding/hiding another row must keep working.
    let (mut app, mut session) = app_with_walls_attributes(LayerAttributes::default());
    assert_eq!(
        session.active_layer,
        LayerId(0),
        "the fixture's current layer is 0"
    );
    let before = revision(&app);

    for (key, value) in [("off", "true"), ("frozen", "true")] {
        set_property(&mut app, &mut session, "WALLS", key, value).unwrap_or_else(|error| {
            panic!("'{key}' must stay writable on a non-current layer: {error}")
        });
    }

    let stored = attributes(&app);
    assert!(stored.off && stored.frozen);
    assert!(
        !visible(&app),
        "the hidden layer is hidden through the existing visibility path"
    );
    assert_eq!(revision(&app), Revision(before.0 + 2), "two real changes");
    assert_eq!(
        session.active_layer,
        LayerId(0),
        "a layer-table write never moves the current layer"
    );
}

#[test]
fn locking_the_current_layer_is_still_allowed() {
    // Locking is a write permission (cad-db's `drawing::lock`), not visibility:
    // it hides nothing, so it is not this rule's business.
    let (mut app, mut session) = app_with_walls_attributes(LayerAttributes::default());
    make_current(&mut app, &mut session, WALLS);
    let before = revision(&app);

    set_property(&mut app, &mut session, "WALLS", "locked", "true")
        .expect("locking the current layer stays allowed");

    assert!(attributes(&app).locked, "the lock was stored");
    assert!(visible(&app), "locking hides nothing");
    assert_eq!(revision(&app), Revision(before.0 + 1), "one real change");
    assert_eq!(session.active_layer, WALLS, "the session is unchanged");
}

#[test]
fn after_switching_the_current_layer_the_previous_layer_can_be_hidden() {
    let (mut app, mut session) = app_with_walls_attributes(LayerAttributes::default());
    make_current(&mut app, &mut session, WALLS);

    // Refused while WALLS is current...
    set_property(&mut app, &mut session, "WALLS", "off", "true")
        .expect_err("WALLS is the current layer right now");

    // ...and accepted as soon as the session points elsewhere, which is the
    // documented remedy in the refusal message.
    make_current(&mut app, &mut session, LayerId(0));
    let before = revision(&app);
    for (key, value) in [("off", "true"), ("frozen", "true")] {
        set_property(&mut app, &mut session, "WALLS", key, value).unwrap_or_else(|error| {
            panic!("'{key}' must be allowed once WALLS is no longer current: {error}")
        });
    }

    let stored = attributes(&app);
    assert!(stored.off && stored.frozen);
    assert!(!visible(&app), "the no-longer-current layer is hidden");
    assert_eq!(revision(&app), Revision(before.0 + 2), "two real changes");
    assert_eq!(session.active_layer, LayerId(0));
}

#[test]
fn the_refusal_surfaces_through_the_ordinary_command_error() {
    // The panel path (`CadSession::set_layer_property`) only dispatches, so the
    // contract is that the *ordinary* command error carries a code the panel can
    // branch on, first in the message — no special case on either side.
    for (key, code) in [
        ("off", "layer.current_off"),
        ("frozen", "layer.current_frozen"),
    ] {
        let (mut app, mut session) = app_with_walls_attributes(LayerAttributes::default());
        make_current(&mut app, &mut session, WALLS);

        let error = set_property(&mut app, &mut session, "WALLS", key, "true")
            .expect_err("the current layer cannot be hidden by a property write");
        assert_current_layer_refusal(&error, code);
    }
}
