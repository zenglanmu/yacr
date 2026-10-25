//! `SETLAYERPROPERTY` contracts: the layer attribute write path behind the layer
//! palette (`CommandId::SetLayerProperty` + `CommandPayload::SetLayerProperty`).
//!
//! The evidence these pin, against the real command funnel
//! (`Application::execute`):
//!
//! * every supported key succeeds and reads back from
//!   [`cad_db::DrawingDatabase::layer_attributes`];
//! * `off` / `frozen` flip the **derived** `Layer::visible` flag (the only
//!   behaviour a layer attribute has in this phase) and restoring both to `false`
//!   shows the row again — the `visible == !(off || frozen)` invariant;
//! * every refusal (unknown layer, unknown key, unimplemented key, bad value, a
//!   missing linetype) leaves the attributes, the revision and the undo depth
//!   untouched;
//! * one real change is one revision bump and **no** undo entry (layer edits are
//!   not undoable).

use super::*;
use crate::layers::{visible_model_entities, LayerOverrideSet};
use cad_db::{
    DbEntity, DbObject, DrawingDatabaseBuilder, Layer, LayerAttributes, LayerColor,
    LayerLineWeight, LineType, LinetypePattern,
};

/// A drawing with layers `0`/`WALLS`, a real linetype table
/// (`CONTINUOUS`/`DASHED`) and one model-space line on `WALLS`.
///
/// The linetype table matters: the `linetype` key resolves against the drawing's
/// own table, so a fixture without one could only ever prove the refusal.
fn layer_property_app() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    for (id, name, pattern) in [
        (LinetypeId(0), "CONTINUOUS", LinetypePattern::continuous()),
        (
            LinetypeId(1),
            "DASHED",
            LinetypePattern::from_elements([0.5, -0.25]),
        ),
    ] {
        b.insert_linetype(LineType {
            id,
            name: name.into(),
            pattern,
            complex: false,
        })
        .unwrap();
    }
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(3),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        draw_order: 0,
    })
    .unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    (app, session)
}

/// Run one command through the real dispatch funnel.
fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn drawing(app: &Application) -> Arc<DrawingDatabase> {
    app.workspace.documents[&DocumentId(1)].drawing.clone()
}

fn write(
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
            layer: layer.into(),
            key: key.into(),
            value: value.into(),
        },
    )
}

fn attributes(app: &Application, name: &str) -> LayerAttributes {
    let db = drawing(app);
    let id = db
        .layers()
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("no layer named {name}"))
        .id;
    db.layer_attributes(id)
}

fn visible(app: &Application, name: &str) -> bool {
    let db = drawing(app);
    let row = db
        .layers()
        .find(|row| row.name == name)
        .expect("the layer row")
        .visible;
    row
}

// ---------------------------------------------------------------------------
// Storage keys: they succeed and read back
// ---------------------------------------------------------------------------

#[test]
fn a_colour_write_stores_the_aci_identity_and_its_palette_rgb() {
    let (mut app, mut session) = layer_property_app();
    let before = revision(&app);

    let outcome = write(&mut app, &mut session, "WALLS", "color", "aci:1")
        .expect("a palette index is a legal layer colour");

    assert_eq!(
        attributes(&app, "WALLS").color,
        LayerColor {
            aci: Some(1),
            rgb: [255, 0, 0],
        }
    );
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(
        outcome.diagnostics[0].code,
        "drawing.layer_property_changed"
    );
    assert!(
        outcome.diagnostics[0].message.contains("WALLS")
            && outcome.diagnostics[0].message.contains("color"),
        "the diagnostic names the layer and the key: {}",
        outcome.diagnostics[0].message
    );

    // A true colour stores the triple only (no invented ACI index).
    write(&mut app, &mut session, "WALLS", "color", "rgb:#0a0B0c").expect("a true colour");
    assert_eq!(
        attributes(&app, "WALLS").color,
        LayerColor {
            aci: None,
            rgb: [0x0a, 0x0b, 0x0c],
        }
    );
    // `r,g,b` is the same value in another spelling.
    write(&mut app, &mut session, "WALLS", "color", "10,11,12").expect("channel triple");
    assert_eq!(attributes(&app, "WALLS").color.rgb, [10, 11, 12]);
}

#[test]
fn a_linetype_write_canonicalises_to_the_table_spelling() {
    let (mut app, mut session) = layer_property_app();

    write(&mut app, &mut session, "WALLS", "linetype", "dashed")
        .expect("DASHED exists; names are case-insensitive");

    assert_eq!(attributes(&app, "WALLS").linetype, "DASHED");
}

#[test]
fn a_lineweight_write_uses_hundredths_of_a_millimetre() {
    let (mut app, mut session) = layer_property_app();

    write(&mut app, &mut session, "WALLS", "lineweight", "35").expect("0.35 mm");
    assert_eq!(
        attributes(&app, "WALLS").lineweight,
        LayerLineWeight::Hundredths(35)
    );
    write(&mut app, &mut session, "WALLS", "lineweight", "Default").expect("the symbolic value");
    assert_eq!(
        attributes(&app, "WALLS").lineweight,
        LayerLineWeight::Default
    );
    write(&mut app, &mut session, "WALLS", "lineweight", "ByLayer").expect("ByLayer");
    assert_eq!(
        attributes(&app, "WALLS").lineweight,
        LayerLineWeight::ByLayer
    );
}

#[test]
fn a_transparency_write_stores_opacity_not_percent() {
    let (mut app, mut session) = layer_property_app();

    write(&mut app, &mut session, "WALLS", "transparency", "25").expect("a legal percentage");
    let stored = attributes(&app, "WALLS").opacity;
    assert!((stored - 0.75).abs() < 1e-6, "stored opacity was {stored}");
    // The projection round-trips: the percent a row shows is the one written.
    assert_eq!(
        crate::layer_property::transparency_value(stored),
        "25".to_string()
    );

    write(&mut app, &mut session, "WALLS", "transparency", "100").expect("fully transparent");
    assert_eq!(attributes(&app, "WALLS").opacity, 0.0);
}

#[test]
fn lock_and_plottable_are_stored_without_any_behaviour() {
    let (mut app, mut session) = layer_property_app();
    // A locked / non-plotting layer is still as visible as before: these two keys
    // are storage only in this phase.
    assert!(visible(&app, "WALLS"));

    write(&mut app, &mut session, "WALLS", "locked", "1").expect("a legal flag");
    write(&mut app, &mut session, "WALLS", "plottable", "false").expect("a legal flag");

    let stored = attributes(&app, "WALLS");
    assert!(stored.locked);
    assert!(!stored.plottable);
    assert!(visible(&app, "WALLS"), "locking hides nothing");
}

// ---------------------------------------------------------------------------
// off / frozen: the only keys with behaviour, through `visible`
// ---------------------------------------------------------------------------

#[test]
fn off_and_frozen_drive_visible_and_only_both_being_false_shows_the_layer() {
    let (mut app, mut session) = layer_property_app();

    write(&mut app, &mut session, "WALLS", "off", "true").expect("a legal flag");
    assert!(!visible(&app, "WALLS"), "an off layer is hidden");

    write(&mut app, &mut session, "WALLS", "frozen", "1").expect("a legal flag");
    assert!(!visible(&app, "WALLS"));
    let stored = attributes(&app, "WALLS");
    assert!(stored.off && stored.frozen);
    assert_eq!(visible(&app, "WALLS"), !(stored.off || stored.frozen));

    // Clearing the off flag alone keeps it hidden: `frozen` still holds.
    write(&mut app, &mut session, "WALLS", "off", "0").expect("a legal flag");
    assert!(!visible(&app, "WALLS"));

    // Clearing both shows it again.
    write(&mut app, &mut session, "WALLS", "frozen", "false").expect("a legal flag");
    assert!(visible(&app, "WALLS"));
    let stored = attributes(&app, "WALLS");
    assert_eq!(visible(&app, "WALLS"), !(stored.off || stored.frozen));
}

#[test]
fn the_hidden_layer_is_really_filtered_from_the_visible_entity_set() {
    // The documented behaviour of `off`/`frozen` is the existing visibility path,
    // so the entity on the layer must drop out of the scene's entity set.
    let (mut app, mut session) = layer_property_app();
    let db = drawing(&app);
    let overrides = LayerOverrideSet::new();
    assert_eq!(visible_model_entities(&db, &overrides).len(), 1);

    write(&mut app, &mut session, "WALLS", "frozen", "true").expect("a legal flag");

    let db = drawing(&app);
    assert!(
        visible_model_entities(&db, &overrides).is_empty(),
        "a frozen layer's entities are not drawn"
    );
}

// ---------------------------------------------------------------------------
// Refusals: nothing written, same revision, same undo depth
// ---------------------------------------------------------------------------

#[test]
fn refusals_leave_the_attributes_the_revision_and_the_undo_depth_untouched() {
    let (mut app, mut session) = layer_property_app();
    let before_revision = revision(&app);
    let before_attributes = attributes(&app, "WALLS");
    let before_undo = app.can_undo(&DocumentId(1));

    let cases: [(&str, &str, &str); 9] = [
        ("ROOF", "color", "aci:1"),
        ("WALLS", "colour", "aci:1"),
        ("WALLS", "plotstyle", "Normal"),
        ("WALLS", "color", "bylayer"),
        ("WALLS", "color", "aci:0"),
        ("WALLS", "color", "#GGGGGG"),
        ("WALLS", "linetype", "PHANTOM"),
        ("WALLS", "lineweight", "212"),
        ("WALLS", "transparency", "101"),
    ];
    for (layer, key, value) in cases {
        let error = write(&mut app, &mut session, layer, key, value)
            .expect_err("this combination must be refused: {layer}/{key}/{value}");
        let text = error.to_string();
        assert!(
            text.contains("layer."),
            "a refusal carries a stable layer code: {text}"
        );
    }

    assert_eq!(revision(&app), before_revision);
    assert_eq!(attributes(&app, "WALLS"), before_attributes);
    assert_eq!(app.can_undo(&DocumentId(1)), before_undo);
}

#[test]
fn an_unknown_layer_is_refused_by_name() {
    let (mut app, mut session) = layer_property_app();
    let error =
        write(&mut app, &mut session, "ROOF", "color", "aci:1").expect_err("ROOF does not exist");
    let text = error.to_string();
    assert!(text.contains("layer.unknown"), "{text}");
    assert!(text.contains("ROOF"), "the refusal names the layer: {text}");
}

#[test]
fn an_unknown_key_and_an_unimplemented_key_are_refused_by_name() {
    let (mut app, mut session) = layer_property_app();

    let error = write(&mut app, &mut session, "WALLS", "hatch", "x").expect_err("not a layer key");
    let text = error.to_string();
    assert!(text.contains("layer.property.unsupported"), "{text}");
    assert!(text.contains("hatch"), "the refusal names the key: {text}");

    // `plotstyle` is a key the grammar *knows* and refuses as not implemented:
    // the writer only emits a style handle, so a stored name could not survive an
    // export.
    let error =
        write(&mut app, &mut session, "WALLS", "plotstyle", "Normal").expect_err("not implemented");
    let text = error.to_string();
    assert!(text.contains("layer.property.unsupported"), "{text}");
    assert!(text.contains("plotstyle"), "{text}");
}

#[test]
fn a_symbolic_colour_or_linetype_is_refused_on_a_layer() {
    let (mut app, mut session) = layer_property_app();

    for (key, value) in [("color", "ByLayer"), ("linetype", "ByBlock")] {
        let error = write(&mut app, &mut session, "WALLS", key, value)
            .expect_err("a layer has nothing to inherit from");
        assert!(
            error.to_string().contains("layer.property.value"),
            "{key}={value}: {error}"
        );
    }
}

#[test]
fn a_visible_write_is_refused_because_visible_is_derived() {
    let (mut app, mut session) = layer_property_app();
    let error = write(&mut app, &mut session, "WALLS", "visible", "false")
        .expect_err("visible is derived from off/frozen");
    let text = error.to_string();
    assert!(text.contains("layer.property.unsupported"), "{text}");
    assert!(text.contains("off/frozen"), "{text}");
}

// ---------------------------------------------------------------------------
// Change semantics
// ---------------------------------------------------------------------------

#[test]
fn rewriting_the_stored_value_is_a_no_op_without_a_revision_bump() {
    let (mut app, mut session) = layer_property_app();
    write(&mut app, &mut session, "WALLS", "color", "aci:1").expect("first write");
    let before = revision(&app);

    let outcome = write(&mut app, &mut session, "WALLS", "color", "aci:1").expect("same value");

    assert_eq!(revision(&app), before, "nothing changed");
    assert_eq!(
        outcome.diagnostics[0].code,
        "drawing.layer_property_unchanged"
    );
}

#[test]
fn a_layer_attribute_write_records_no_undo_entry() {
    let (mut app, mut session) = layer_property_app();
    assert!(!app.can_undo(&DocumentId(1)));

    write(&mut app, &mut session, "WALLS", "color", "aci:3").expect("a legal colour");

    assert!(
        !app.can_undo(&DocumentId(1)),
        "layer edits are not undoable: history has no layer patch kind"
    );
}

#[test]
fn the_layer_property_command_is_work_only() {
    assert!(CommandId::SetLayerProperty.requires_work_mode());

    let (mut app, mut session) = layer_property_app();
    session.switch_mode(AppMode::Viewer).expect("switch mode");
    let before = revision(&app);
    let error = write(&mut app, &mut session, "WALLS", "color", "aci:1")
        .expect_err("Viewer mode refuses a database write");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(revision(&app), before);
}
