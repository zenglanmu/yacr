//! The typed `-LAYER <option> …` write forms (`CommandId::SetLayerProperty` +
//! `CommandPayload::DashLayer`).
//!
//! Pinned here, against the real command funnel:
//!
//! * every option's write and its stable `layer.*` diagnostic;
//! * one transaction and **one** undo step per real line, with the whole line
//!   undone/redone in one revision;
//! * atomicity: a refusal anywhere in the line (an unknown name, a duplicate
//!   name, a forbidden delete, an invalid colour) leaves the drawing, its
//!   revision, the session and the history untouched — never a partially applied
//!   prefix;
//! * the active-layer rule for `MAKE`/`SET` (locked/frozen/off refused **before**
//!   mutation) and the "never hide the current layer" rule for `OFF`/`FREEZE`;
//! * `layer.unchanged` and no revision bump for a line that changes nothing;
//! * the Viewer refusal: the write carrier is Work-only, while the bare report
//!   (`ExtendedCommandId::DashLayer`) stays Viewer-legal.
//!
//! The parser itself (keywords, aliases, comma lists) is pinned in
//! `cad-ui-core`'s session tests; these tests start from the typed payload.

use super::*;
use crate::command_payload::DashLayerOps;
use cad_db::{LayerAttributes, LinetypePattern};

/// Execute one typed `-LAYER` write line.
fn dash(
    app: &mut Application,
    session: &mut SessionState,
    ops: DashLayerOps,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::SetLayerProperty, CommandPayload::DashLayer(ops)),
    )
}

/// Execute one ordinary command (undo/redo, `SETLAYERPROPERTY`, …).
fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn drawing(app: &Application) -> &cad_db::DrawingDatabase {
    app.workspace.documents[&DocumentId(1)].drawing.as_ref()
}

fn revision(app: &Application) -> Revision {
    drawing(app).revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn layer_id(app: &Application, name: &str) -> Option<LayerId> {
    drawing(app)
        .layers()
        .find(|layer| layer.name == name)
        .map(|layer| layer.id)
}

fn attributes(app: &Application, id: LayerId) -> LayerAttributes {
    drawing(app).layer_attributes(id)
}

fn row<'a>(outcome: &'a CommandOutcome, code: &str) -> &'a Diagnostic {
    outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("missing {code} diagnostic"))
}

fn codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn names(layers: &[&str]) -> Vec<String> {
    layers.iter().map(|name| (*name).to_string()).collect()
}

/// The shared fixture plus one `DASHED` linetype, so `LTYPE` has a real table
/// entry to resolve (the plain fixture's linetype table is empty).
fn app_with_linetype() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    builder
        .insert_linetype(cad_db::LineType {
            id: cad_domain::LinetypeId(1),
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([2.0, -1.0]),
            complex: false,
        })
        .unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(builder.finish().unwrap());
    (app, session)
}

// ---------------------------------------------------------------------------
// NEW / RENAME / DELETE
// ---------------------------------------------------------------------------

#[test]
fn new_creates_every_named_layer_in_one_revision_and_one_undo_step() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = dash(
        &mut app,
        &mut session,
        DashLayerOps::New(names(&["DOORS", "DIMS"])),
    )
    .expect("-LAYER NEW creates both layers");

    assert!(layer_id(&app, "DOORS").is_some());
    assert!(layer_id(&app, "DIMS").is_some());
    // Each row's validated setter raises the revision (the database's own rule,
    // the same documented limitation `write_layer_flags` has); the whole line is
    // still **one** transaction and one undo step, and its reported change set
    // spans the line's revisions.
    assert!(revision(&app).0 > before.0, "the line is a real change");
    assert_eq!(undo_depth(&app), 1, "one undo step for the line");
    assert_eq!(codes(&outcome), ["layer.created", "layer.created"]);
    let changes = outcome.changes.expect("the line reports its change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after, revision(&app));

    // Undo removes both rows in one step, redo restores both.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert!(layer_id(&app, "DOORS").is_none());
    assert!(layer_id(&app, "DIMS").is_none());
    assert_eq!(drawing(&app).layers().count(), 2, "only 0 and WALLS stay");
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo");
    assert!(layer_id(&app, "DOORS").is_some());
    assert!(layer_id(&app, "DIMS").is_some());
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn a_duplicate_name_in_the_list_refuses_the_whole_line() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    // The first name would be legal on its own; the second is the stored WALLS
    // name case-insensitively, so the database refuses it and the already
    // staged DOORS row must not survive.
    let error = dash(
        &mut app,
        &mut session,
        DashLayerOps::New(names(&["DOORS", "walls"])),
    )
    .expect_err("a duplicate layer name is refused");
    assert!(
        format!("{error}").contains("layer.duplicate_name"),
        "{error}"
    );
    assert!(layer_id(&app, "DOORS").is_none(), "nothing is published");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn rename_is_one_step_and_a_same_name_rename_is_a_reported_no_op() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = dash(
        &mut app,
        &mut session,
        DashLayerOps::Rename {
            old: "WALLS".into(),
            new: "PARTITIONS".into(),
        },
    )
    .expect("the rename writes");
    assert!(layer_id(&app, "WALLS").is_none());
    assert!(layer_id(&app, "PARTITIONS").is_some());
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(codes(&outcome), ["layer.renamed"]);
    assert!(row(&outcome, "layer.renamed")
        .message
        .contains("PARTITIONS"));

    // Renaming to the stored name changes nothing: no revision, no patch, and the
    // honest `layer.unchanged` report (not a fabricated "renamed").
    let before = revision(&app);
    let outcome = dash(
        &mut app,
        &mut session,
        DashLayerOps::Rename {
            old: "PARTITIONS".into(),
            new: "PARTITIONS".into(),
        },
    )
    .expect("a same-name rename is admitted");
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 1, "no second undo step");
    assert_eq!(codes(&outcome), ["layer.unchanged"]);
}

#[test]
fn rename_of_an_unknown_layer_changes_nothing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let error = dash(
        &mut app,
        &mut session,
        DashLayerOps::Rename {
            old: "NOPE".into(),
            new: "OTHER".into(),
        },
    )
    .expect_err("an unknown old name is refused");
    assert!(format!("{error}").contains("layer.unknown"), "{error}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn delete_removes_the_row_and_falls_back_when_the_active_layer_is_deleted() {
    let (mut app, mut session) = application_with_document();
    // WALLS has no entities in this fixture, so it is deletable.
    let walls = layer_id(&app, "WALLS").expect("the WALLS layer");
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(walls),
    )
    .expect("WALLS is on, thawed and unlocked");

    let before = revision(&app);
    let outcome = dash(
        &mut app,
        &mut session,
        DashLayerOps::Delete(names(&["WALLS"])),
    )
    .expect("the delete writes");
    assert!(layer_id(&app, "WALLS").is_none());
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(
        codes(&outcome),
        ["layer.deleted", "drawing.active_layer_fallback"]
    );
    assert_eq!(
        session.active_layer,
        layer_id(&app, "0").expect("the default layer"),
        "the session falls back to layer 0"
    );
}

#[test]
fn a_delete_list_that_hits_a_forbidden_row_deletes_nothing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    // `0` is the required default row: the second delete is refused, so the first
    // (legal) one must not be published.
    let error = dash(
        &mut app,
        &mut session,
        DashLayerOps::Delete(names(&["WALLS", "0"])),
    )
    .expect_err("deleting the default layer is refused");
    assert!(
        format!("{error}").contains("layer.default_undeletable"),
        "{error}"
    );
    assert!(
        layer_id(&app, "WALLS").is_some(),
        "the first delete is undone"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

// ---------------------------------------------------------------------------
// ON / OFF / FREEZE / THAW / LOCK / UNLOCK
// ---------------------------------------------------------------------------

#[test]
fn on_and_off_write_the_flag_and_the_derived_visibility() {
    let (mut app, mut session) = application_with_document();
    let walls = layer_id(&app, "WALLS").unwrap();
    let before = revision(&app);

    let outcome = dash(&mut app, &mut session, DashLayerOps::Off(names(&["WALLS"])))
        .expect("OFF writes a non-current layer");
    assert!(attributes(&app, walls).off);
    assert!(
        !drawing(&app).layer(walls).unwrap().visible,
        "visible is derived from off/frozen"
    );
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(codes(&outcome), ["layer.property_changed"]);

    dash(&mut app, &mut session, DashLayerOps::On(names(&["WALLS"]))).expect("ON writes");
    assert!(!attributes(&app, walls).off);
    assert!(drawing(&app).layer(walls).unwrap().visible);
    assert_eq!(undo_depth(&app), 2);

    // OFF on the session's current layer would hide the geometry it receives.
    let before = revision(&app);
    let error = dash(&mut app, &mut session, DashLayerOps::Off(names(&["0"])))
        .expect_err("the current layer must stay visible");
    assert!(format!("{error}").contains("layer.current_off"), "{error}");
    assert!(!attributes(&app, LayerId(0)).off);
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 2);

    // ON for an already-on layer is a reported no-op: no revision, no patch.
    let outcome =
        dash(&mut app, &mut session, DashLayerOps::On(names(&["WALLS"]))).expect("ON is admitted");
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 2);
    assert_eq!(codes(&outcome), ["layer.unchanged"]);
}

#[test]
fn freeze_and_thaw_write_the_flag_and_refuse_the_current_layer() {
    let (mut app, mut session) = application_with_document();
    let walls = layer_id(&app, "WALLS").unwrap();

    dash(
        &mut app,
        &mut session,
        DashLayerOps::Freeze(names(&["WALLS"])),
    )
    .expect("FREEZE writes a non-current layer");
    assert!(attributes(&app, walls).frozen);
    assert!(!drawing(&app).layer(walls).unwrap().visible);

    dash(
        &mut app,
        &mut session,
        DashLayerOps::Thaw(names(&["WALLS"])),
    )
    .expect("THAW writes");
    assert!(!attributes(&app, walls).frozen);
    assert!(drawing(&app).layer(walls).unwrap().visible);

    let before = revision(&app);
    let error = dash(&mut app, &mut session, DashLayerOps::Freeze(names(&["0"])))
        .expect_err("freezing the current layer is refused");
    assert!(
        format!("{error}").contains("layer.current_frozen"),
        "{error}"
    );
    assert!(!attributes(&app, LayerId(0)).frozen);
    assert_eq!(revision(&app), before);
}

#[test]
fn lock_and_unlock_write_the_flag_even_on_the_current_layer() {
    let (mut app, mut session) = application_with_document();
    let walls = layer_id(&app, "WALLS").unwrap();

    dash(
        &mut app,
        &mut session,
        DashLayerOps::Lock(names(&["WALLS"])),
    )
    .expect("LOCK writes");
    assert!(attributes(&app, walls).locked);
    dash(
        &mut app,
        &mut session,
        DashLayerOps::Unlock(names(&["WALLS"])),
    )
    .expect("UNLOCK writes");
    assert!(!attributes(&app, walls).locked);

    // Locking is a write permission, not visibility, so the current layer may be
    // locked (the same rule `SETLAYERPROPERTY locked` has).
    dash(&mut app, &mut session, DashLayerOps::Lock(names(&["0"])))
        .expect("locking the current layer is allowed");
    assert!(attributes(&app, LayerId(0)).locked);
}

#[test]
fn an_unknown_name_in_a_list_refuses_the_whole_line() {
    let (mut app, mut session) = application_with_document();
    let walls = layer_id(&app, "WALLS").unwrap();
    let before = revision(&app);
    let error = dash(
        &mut app,
        &mut session,
        DashLayerOps::Lock(names(&["WALLS", "NOPE"])),
    )
    .expect_err("an unknown layer name is refused");
    assert!(format!("{error}").contains("layer.unknown"), "{error}");
    assert!(
        !attributes(&app, walls).locked,
        "the legal name is not written"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

// ---------------------------------------------------------------------------
// SET / MAKE
// ---------------------------------------------------------------------------

#[test]
fn set_makes_an_existing_layer_current_without_touching_the_table() {
    let (mut app, mut session) = application_with_document();
    let walls = layer_id(&app, "WALLS").unwrap();
    let before = revision(&app);

    let outcome =
        dash(&mut app, &mut session, DashLayerOps::Set("WALLS".into())).expect("SET is admitted");
    assert_eq!(session.active_layer, walls);
    assert!(outcome.changes.is_none(), "session-only");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(codes(&outcome), ["layer.current"]);

    // SET to the layer that is already current changes nothing and says so.
    let outcome = dash(&mut app, &mut session, DashLayerOps::Set("walls".into()))
        .expect("SET resolves case-insensitively");
    assert_eq!(session.active_layer, walls);
    assert_eq!(codes(&outcome), ["layer.unchanged"]);
    assert_eq!(revision(&app), before);
}

#[test]
fn set_refuses_a_locked_frozen_or_off_layer_before_any_mutation() {
    let (mut app, mut session) = application_with_document();
    for (key, value, code) in [
        ("locked", "true", "layer.current_locked"),
        ("frozen", "true", "layer.current_frozen"),
        ("off", "true", "layer.current_off"),
    ] {
        // Write the flag with the ordinary panel command (WALLS is not current).
        execute(
            &mut app,
            &mut session,
            CommandId::SetLayerProperty,
            CommandPayload::SetLayerProperty {
                layer: "WALLS".into(),
                key: key.into(),
                value: value.into(),
            },
        )
        .expect("the panel write is admitted on a non-current layer");
        let error = dash(&mut app, &mut session, DashLayerOps::Set("WALLS".into()))
            .expect_err("an unusable layer cannot become current");
        assert!(format!("{error}").contains(code), "{key}: {error}");
        assert_eq!(session.active_layer, LayerId(0), "the session is untouched");
        // Restore the flag for the next round.
        execute(
            &mut app,
            &mut session,
            CommandId::SetLayerProperty,
            CommandPayload::SetLayerProperty {
                layer: "WALLS".into(),
                key: key.into(),
                value: "false".into(),
            },
        )
        .expect("the flag clears");
    }
}

#[test]
fn make_creates_a_missing_layer_then_makes_it_current_in_one_step() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = dash(&mut app, &mut session, DashLayerOps::Make("DOORS".into()))
        .expect("MAKE creates and selects");
    let doors = layer_id(&app, "DOORS").expect("the created layer");
    assert_eq!(session.active_layer, doors);
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1, "create + select is one table step");
    assert_eq!(codes(&outcome), ["layer.created", "layer.current"]);

    // Undo restores the table; the session's active layer choice is not history,
    // exactly like DELETELAYER's fallback (documented, not undone).
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert!(layer_id(&app, "DOORS").is_none());
    assert_eq!(session.active_layer, doors, "the session choice stays");
}

#[test]
fn make_on_an_existing_layer_is_session_only() {
    let (mut app, mut session) = application_with_document();
    let walls = layer_id(&app, "WALLS").unwrap();
    let before = revision(&app);
    let outcome = dash(&mut app, &mut session, DashLayerOps::Make("WALLS".into()))
        .expect("MAKE selects the existing layer");
    assert_eq!(session.active_layer, walls);
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(codes(&outcome), ["layer.current"]);
}

// ---------------------------------------------------------------------------
// COLOR / LTYPE / LWEIGHT / PLOT
// ---------------------------------------------------------------------------

#[test]
fn color_accepts_a_bare_aci_and_refuses_bylayer() {
    let (mut app, mut session) = application_with_document();
    let walls = layer_id(&app, "WALLS").unwrap();
    let before = revision(&app);

    dash(
        &mut app,
        &mut session,
        DashLayerOps::Color {
            value: "3".into(),
            layers: names(&["WALLS"]),
        },
    )
    .expect("a bare ACI number is the command-line spelling");
    let color = attributes(&app, walls).color;
    assert_eq!(color.aci, Some(3), "bare 3 is normalised to aci:3");
    assert_eq!(color.rgb, crate::property_value::rgb_for_aci(3).unwrap());
    assert_eq!(revision(&app), Revision(before.0 + 1));

    let error = dash(
        &mut app,
        &mut session,
        DashLayerOps::Color {
            value: "BYLAYER".into(),
            layers: names(&["WALLS"]),
        },
    )
    .expect_err("a layer colour cannot be ByLayer");
    assert!(
        format!("{error}").contains("layer.property.value"),
        "{error}"
    );
    assert_eq!(attributes(&app, walls).color.aci, Some(3), "unchanged");
}

#[test]
fn ltype_requires_an_existing_table_entry() {
    let (mut app, mut session) = app_with_linetype();
    let walls = layer_id(&app, "WALLS").unwrap();

    dash(
        &mut app,
        &mut session,
        DashLayerOps::Linetype {
            name: "DASHED".into(),
            layers: names(&["WALLS"]),
        },
    )
    .expect("DASHED exists in this drawing");
    assert_eq!(attributes(&app, walls).linetype, "DASHED");

    let before = revision(&app);
    let error = dash(
        &mut app,
        &mut session,
        DashLayerOps::Linetype {
            name: "PHANTOM".into(),
            layers: names(&["WALLS"]),
        },
    )
    .expect_err("an unknown linetype is refused");
    assert!(
        format!("{error}").contains("layer.linetype_missing"),
        "{error}"
    );
    assert_eq!(attributes(&app, walls).linetype, "DASHED");
    assert_eq!(revision(&app), before);
}

#[test]
fn lweight_uses_the_layer_grammar_and_plot_sets_plottable() {
    let (mut app, mut session) = application_with_document();
    let walls = layer_id(&app, "WALLS").unwrap();

    dash(
        &mut app,
        &mut session,
        DashLayerOps::Lineweight {
            value: "35".into(),
            layers: names(&["WALLS"]),
        },
    )
    .expect("35 is 0.35 mm in the layer grammar");
    assert_eq!(
        attributes(&app, walls).lineweight,
        cad_db::LayerLineWeight::Hundredths(35)
    );

    dash(
        &mut app,
        &mut session,
        DashLayerOps::Plot {
            plottable: false,
            layers: names(&["WALLS"]),
        },
    )
    .expect("PLOT N writes");
    assert!(!attributes(&app, walls).plottable);

    let error = dash(
        &mut app,
        &mut session,
        DashLayerOps::Lineweight {
            value: "999".into(),
            layers: names(&["WALLS"]),
        },
    )
    .expect_err("an out-of-range lineweight is refused");
    assert!(
        format!("{error}").contains("layer.property.value"),
        "{error}"
    );
    assert_eq!(
        attributes(&app, walls).lineweight,
        cad_db::LayerLineWeight::Hundredths(35)
    );
}

// ---------------------------------------------------------------------------
// Authorization and the read-only report
// ---------------------------------------------------------------------------

#[test]
fn the_typed_writes_are_refused_in_viewer_mode() {
    let (mut app, _) = application_with_document();
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    let before = revision(&app);

    let error = dash(&mut app, &mut viewer, DashLayerOps::New(names(&["DOORS"])))
        .expect_err("Viewer mode refuses a drawing write");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert!(layer_id(&app, "DOORS").is_none());
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);

    // The read-only report stays Viewer-legal: `-LAYER` and `-LAYER ?` never go
    // through the write carrier.
    let report = execute(
        &mut app,
        &mut viewer,
        CommandId::Extended(ExtendedCommandId::DashLayer),
        CommandPayload::None,
    )
    .expect("-LAYER reports in Viewer mode");
    assert!(report.changes.is_none());
    assert!(report
        .diagnostics
        .iter()
        .any(|row| row.code == "layer.entry" && row.message.contains("'WALLS'")));
}
