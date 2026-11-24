//! `PARAM` structural edits and the `param.<name>` value write (ADR-0043 D8/D9).
//!
//! These pin the cad-app half of the vertical slice: one command is one
//! transaction / one revision / one undo step; the parameter is the authority and
//! the element geometry is derived from it; a value edit goes through the
//! ordinary property command but never through the display-property grammar; and
//! every refusal (unknown name, unknown slot, no evaluator, mixed selection,
//! still-referenced delete) writes nothing.

use super::*;
use cad_db::BlockDefinition;
use cad_domain::param::{
    Expr, ParamBindings, ParamConfig, ParamConfigTable, ParamKind, ParamRow, ParamSeq, ParamTable,
};

/// A model-space circle with the given radius.
pub(super) fn circle(id: u128, radius: f64) -> (u128, SemanticGeometry) {
    (
        id,
        SemanticGeometry::Circle {
            center: point(id as f64, 0.0),
            normal: cad_domain::Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius,
        },
    )
}

/// A wide polyline at y = `id`.
fn wide_polyline(id: u128, width: f64) -> (u128, SemanticGeometry) {
    (
        id,
        SemanticGeometry::Polyline {
            points: vec![point(0.0, id as f64), point(10.0, id as f64)],
            bulges: vec![0.0],
            closed: false,
            width,
        },
    )
}

/// One text entity of height `height`.
fn text(id: u128, height: f64) -> (u128, SemanticGeometry) {
    (
        id,
        SemanticGeometry::Text {
            text: "hello".into(),
            position: point(0.0, id as f64),
            style: cad_domain::StyleId(0),
            height,
            rotation: 0.0,
            font: None,
            h_align: cad_domain::TextAlignH::Left,
            v_align: cad_domain::TextAlignV::Baseline,
            wrap_width: None,
            width_factor: 1.0,
            line_spacing: cad_domain::TextLineSpacing::default(),
        },
    )
}

/// A Work-mode application over the given entities (no parameters, no bindings).
pub(super) fn param_app(entities: Vec<(u128, SemanticGeometry)>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for (order, (id, geometry)) in entities.into_iter().enumerate() {
        builder
            .insert_entity(ent(id, geometry, order as i64))
            .unwrap();
    }
    let drawing = builder.finish().expect("a valid parameter document");
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
            compat: Vec::new(),
            sources: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

/// A Work-mode app over a builder that already holds the document.
fn app_with(builder: DrawingDatabaseBuilder) -> (Application, SessionState) {
    let drawing = builder.finish().expect("a valid parameter document");
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
            compat: Vec::new(),
            sources: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

/// Base rows `r=20`, `q=10`, `s=7`; one circle bound to `r`. Configurations:
/// `small` overrides `r=5`, `qonly` overrides `q=99`, `hide` suppresses `s`.
pub(super) fn configured_app() -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    let (id, geometry) = circle(1, 20.0);
    builder.insert_entity(ent(id, geometry, 0)).unwrap();
    let table = ParamTable::from_rows([
        ParamRow::new(ParamSeq(1), "r", ParamKind::Length, Expr::Num(20.0)).unwrap(),
        ParamRow::new(ParamSeq(2), "q", ParamKind::Length, Expr::Num(10.0)).unwrap(),
        ParamRow::new(ParamSeq(3), "s", ParamKind::Length, Expr::Num(7.0)).unwrap(),
    ])
    .unwrap();
    builder.set_parameters(table).unwrap();
    let mut configs = ParamConfigTable::new();
    let mut small = ParamConfig::new("small").unwrap();
    small.set_override(ParamSeq(1), Expr::Num(5.0)).unwrap();
    configs.insert(small).unwrap();
    let mut qonly = ParamConfig::new("qonly").unwrap();
    qonly.set_override(ParamSeq(2), Expr::Num(99.0)).unwrap();
    configs.insert(qonly).unwrap();
    let mut hide = ParamConfig::new("hide").unwrap();
    hide.suppress(ParamSeq(3));
    configs.insert(hide).unwrap();
    builder.set_param_configs(configs).unwrap();
    builder
        .set_param_bindings(
            EntityId(1),
            ParamBindings::from_entries([("radius".to_string(), Expr::Ref(ParamSeq(1)))]).unwrap(),
        )
        .unwrap();
    app_with(builder)
}

/// An app whose only entity is a block INSERT (P1 defers an `Insert` evaluator).
fn insert_app() -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
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
        .insert_entity(ent(
            1,
            SemanticGeometry::Insert {
                block: cad_domain::BlockId(0),
                transform: cad_domain::Transform3::identity(),
            },
            0,
        ))
        .unwrap();
    app_with(builder)
}

pub(super) fn stored_drawing(app: &Application) -> Arc<DrawingDatabase> {
    app.workspace.documents[&DocumentId(1)].drawing.clone()
}

pub(super) fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn geometry(app: &Application, id: u128) -> SemanticGeometry {
    stored_drawing(app)
        .entity(EntityId(id))
        .expect("the entity exists")
        .geometry
        .clone()
}

pub(super) fn radius(app: &Application, id: u128) -> f64 {
    match geometry(app, id) {
        SemanticGeometry::Circle { radius, .. } => radius,
        other => panic!("expected a circle, got {other:?}"),
    }
}

pub(super) fn seq_of(app: &Application, name: &str) -> ParamSeq {
    stored_drawing(app)
        .parameters()
        .get_by_name(name)
        .expect("the parameter exists")
        .seq
}

pub(super) fn create(
    app: &mut Application,
    session: &mut SessionState,
    name: &str,
    kind: ParamKind,
    value: f64,
) {
    execute(
        app,
        session,
        CommandId::Param,
        CommandPayload::Param(crate::ParamOp::Create {
            name: name.into(),
            kind,
            expr: Expr::Num(value),
        }),
    )
    .expect("create");
}

pub(super) fn bind(
    app: &mut Application,
    session: &mut SessionState,
    slots: Vec<(String, Expr)>,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::Param,
        CommandPayload::Param(crate::ParamOp::Bind { slots }),
    )
}

fn set_property(
    app: &mut Application,
    session: &mut SessionState,
    key: &str,
    value: &str,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::SetEntityProperty,
        CommandPayload::EntityProperty {
            key: key.into(),
            value: value.into(),
        },
    )
}

pub(super) fn undo(app: &mut Application, session: &mut SessionState) -> CommandOutcome {
    execute(app, session, CommandId::Undo, CommandPayload::None).expect("undo")
}

pub(super) fn redo(app: &mut Application, session: &mut SessionState) -> CommandOutcome {
    execute(app, session, CommandId::Redo, CommandPayload::None).expect("redo")
}

/// Lock one layer through the layer-table write (the only sanctioned path).
fn lock_layer(app: &mut Application, layer: LayerId) {
    let mut database = stored_drawing(app).as_ref().clone();
    let mut attributes = database.layer_attributes(layer);
    attributes.locked = true;
    database
        .set_layer_attributes(layer, attributes, TransactionId(900), "lock")
        .expect("a layer-table attribute write is always allowed");
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .expect("the document is open")
        .drawing = Arc::new(database);
}

#[test]
fn create_and_bind_drive_the_circle_radius_in_one_revision_each() {
    let (mut app, mut session) = param_app(vec![circle(1, 10.0)]);
    session.selection.replace([ref_of(1)]);

    let before = revision(&app);
    create(&mut app, &mut session, "r", ParamKind::Length, 20.0);
    assert_eq!(
        revision(&app),
        Revision(before.0 + 1),
        "create is one revision"
    );
    assert_eq!(undo_depth(&app), 1, "create is one undo step");
    // An unreferenced parameter is a table row only: the element geometry is
    // untouched until a binding names it.
    assert_eq!(radius(&app, 1), 10.0);

    let seq = seq_of(&app, "r");
    let before = revision(&app);
    bind(
        &mut app,
        &mut session,
        vec![("radius".into(), Expr::Ref(seq))],
    )
    .expect("bind");
    assert_eq!(
        revision(&app),
        Revision(before.0 + 1),
        "bind is one revision"
    );
    assert_eq!(undo_depth(&app), 2, "bind is one undo step");
    assert_eq!(radius(&app, 1), 20.0, "the binding derives the geometry");

    // The read side lists exactly the referenced parameter, with its document
    // name as the source label and a number editor.
    let properties = crate::SelectionProperties::extract(&stored_drawing(&app), &session.selection);
    let row = properties
        .rows
        .iter()
        .find(|row| row.key == "param.r")
        .expect("the referenced parameter is listed");
    assert_eq!(row.value.as_deref(), Some("20"));
    assert_eq!(row.label.as_deref(), Some("r"));
    assert_eq!(row.group, crate::selection::PropertyGroup::Param);
    assert_eq!(row.kind, crate::selection::PropertyKind::Number);
    assert!(row.editable);

    // An element with no bindings has no parameter rows.
    let (unbound, unbound_session) = param_app(vec![circle(2, 5.0)]);
    let properties =
        crate::SelectionProperties::extract(&stored_drawing(&unbound), &unbound_session.selection);
    assert!(properties
        .rows
        .iter()
        .all(|row| !row.key.starts_with("param.")));
}

#[test]
fn value_edit_through_the_property_command_rewrites_the_radius() {
    let (mut app, mut session) = param_app(vec![circle(1, 10.0)]);
    session.selection.replace([ref_of(1)]);
    create(&mut app, &mut session, "r", ParamKind::Length, 20.0);
    let seq = seq_of(&app, "r");
    bind(
        &mut app,
        &mut session,
        vec![("radius".into(), Expr::Ref(seq))],
    )
    .expect("bind");

    let before = revision(&app);
    let depth = undo_depth(&app);
    let outcome = set_property(&mut app, &mut session, "param.r", "35").expect("value edit");
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert_eq!(undo_depth(&app), depth + 1, "one undo step");
    assert!(outcome.changes.is_some());
    assert_eq!(radius(&app, 1), 35.0);
    assert_eq!(
        stored_drawing(&app)
            .parameters()
            .get_by_name("r")
            .expect("the row exists")
            .expr,
        Expr::Num(35.0)
    );

    // Writing the same number again is a no-op: no transaction, no history.
    let before = revision(&app);
    let depth = undo_depth(&app);
    let outcome = set_property(&mut app, &mut session, "param.r", "35").expect("no-op");
    assert!(outcome.changes.is_none(), "a no-op publishes no ChangeSet");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "property.unchanged"));
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), depth);
}

#[test]
fn a_mixed_multi_selection_refuses_the_whole_write() {
    let (mut app, mut session) = param_app(vec![circle(1, 10.0), circle(2, 10.0)]);
    create(&mut app, &mut session, "r", ParamKind::Length, 20.0);
    let seq = seq_of(&app, "r");

    // Only the first circle is bound.
    session.selection.replace([ref_of(1)]);
    bind(
        &mut app,
        &mut session,
        vec![("radius".into(), Expr::Ref(seq))],
    )
    .expect("bind");
    assert_eq!(radius(&app, 1), 20.0);
    assert_eq!(radius(&app, 2), 10.0);

    session.selection.replace([ref_of(1), ref_of(2)]);
    // The read side intersects by key: one bound and one unbound element share no
    // `param.r` row, so the panel cannot offer a write that must be refused.
    let properties = crate::SelectionProperties::extract(&stored_drawing(&app), &session.selection);
    assert!(
        properties.rows.iter().all(|row| row.key != "param.r"),
        "a mixed bound/unbound selection has no shared parameter row"
    );
    let before = revision(&app);
    let depth = undo_depth(&app);
    let error = set_property(&mut app, &mut session, "param.r", "35").unwrap_err();
    assert!(
        format!("{error:?}").contains("param.mixed"),
        "got {error:?}"
    );
    assert_eq!(revision(&app), before, "no partial write");
    assert_eq!(undo_depth(&app), depth);
    assert_eq!(radius(&app, 1), 20.0);
    assert_eq!(radius(&app, 2), 10.0);
}

#[test]
fn undo_and_redo_swap_the_parameter_row_and_rederive_the_geometry() {
    let (mut app, mut session) = param_app(vec![circle(1, 10.0)]);
    session.selection.replace([ref_of(1)]);
    create(&mut app, &mut session, "r", ParamKind::Length, 20.0);
    let seq = seq_of(&app, "r");
    bind(
        &mut app,
        &mut session,
        vec![("radius".into(), Expr::Ref(seq))],
    )
    .expect("bind");
    set_property(&mut app, &mut session, "param.r", "35").expect("value edit");
    assert_eq!(radius(&app, 1), 35.0);

    let undo_outcome = undo(&mut app, &mut session);
    assert_eq!(radius(&app, 1), 20.0, "undo restores the derived geometry");
    assert_eq!(
        stored_drawing(&app)
            .parameters()
            .get_by_name("r")
            .expect("the row exists")
            .expr,
        Expr::Num(20.0),
        "undo restores the parameter row"
    );
    assert!(undo_outcome.changes.is_some());

    let redo_outcome = redo(&mut app, &mut session);
    assert_eq!(radius(&app, 1), 35.0, "redo restores the edit");
    assert_eq!(
        stored_drawing(&app)
            .parameters()
            .get_by_name("r")
            .expect("the row exists")
            .expr,
        Expr::Num(35.0)
    );
    assert!(redo_outcome.changes.is_some());
}

#[test]
fn undo_and_redo_of_a_bind_restore_the_exact_pre_bind_geometry() {
    let (mut app, mut session) = param_app(vec![circle(1, 10.0)]);
    session.selection.replace([ref_of(1)]);
    create(&mut app, &mut session, "r", ParamKind::Length, 20.0);
    let seq = seq_of(&app, "r");
    bind(
        &mut app,
        &mut session,
        vec![("radius".into(), Expr::Ref(seq))],
    )
    .expect("bind");
    assert_eq!(radius(&app, 1), 20.0);
    assert!(stored_drawing(&app).param_bindings(EntityId(1)).is_some());

    // The top step is the bind, not the create: undo removes the binding and
    // restores the geometry it replaced exactly (ADR §4).
    undo(&mut app, &mut session);
    assert_eq!(radius(&app, 1), 10.0, "undo restores the pre-bind radius");
    assert!(
        stored_drawing(&app).param_bindings(EntityId(1)).is_none(),
        "undo removes the binding"
    );

    redo(&mut app, &mut session);
    assert_eq!(radius(&app, 1), 20.0, "redo restores the bound geometry");
    assert!(
        stored_drawing(&app).param_bindings(EntityId(1)).is_some(),
        "redo restores the binding"
    );
}

#[test]
fn a_parameter_edit_rewrites_exactly_the_bound_closure_once() {
    let (mut app, mut session) = param_app(vec![circle(1, 10.0), circle(2, 10.0), circle(3, 7.0)]);
    create(&mut app, &mut session, "r", ParamKind::Length, 20.0);
    let seq = seq_of(&app, "r");

    // Two of three circles bind the same parameter; the third stays independent.
    session.selection.replace([ref_of(1), ref_of(2)]);
    bind(
        &mut app,
        &mut session,
        vec![("radius".into(), Expr::Ref(seq))],
    )
    .expect("bind");
    assert_eq!(radius(&app, 1), 20.0);
    assert_eq!(radius(&app, 2), 20.0);
    assert_eq!(radius(&app, 3), 7.0);
    let independent = geometry(&app, 3);

    let outcome = set_property(&mut app, &mut session, "param.r", "30").expect("value edit");
    assert_eq!(
        outcome.objects,
        vec![ObjectId(1), ObjectId(2)],
        "exactly the bound elements are reported, once each, ascending"
    );
    assert_eq!(radius(&app, 1), 30.0);
    assert_eq!(radius(&app, 2), 30.0);
    assert_eq!(
        geometry(&app, 3),
        independent,
        "an unbound element is not touched"
    );
}

#[test]
fn a_locked_layer_refuses_bind_and_parameter_value_writes() {
    let (mut app, mut session) = param_app(vec![circle(1, 10.0)]);
    session.selection.replace([ref_of(1)]);
    create(&mut app, &mut session, "r", ParamKind::Length, 20.0);
    let r = seq_of(&app, "r");
    bind(
        &mut app,
        &mut session,
        vec![("radius".into(), Expr::Ref(r))],
    )
    .expect("bind");
    assert_eq!(radius(&app, 1), 20.0);
    // A second parameter lets the locked `Bind` be a real change.
    create(&mut app, &mut session, "q", ParamKind::Length, 5.0);
    let q = seq_of(&app, "q");

    lock_layer(&mut app, LayerId(0));

    // The parameter value cannot be committed because its derived geometry is a
    // write to a locked layer.
    let before = revision(&app);
    let depth = undo_depth(&app);
    let error = set_property(&mut app, &mut session, "param.r", "30").unwrap_err();
    assert!(
        format!("{error:?}").contains("layer.locked"),
        "got {error:?}"
    );
    assert_eq!(revision(&app), before, "nothing is written");
    assert_eq!(undo_depth(&app), depth, "no undo entry");
    assert_eq!(radius(&app, 1), 20.0);
    assert_eq!(
        stored_drawing(&app)
            .parameters()
            .get_by_name("r")
            .expect("the row exists")
            .expr,
        Expr::Num(20.0),
        "the table row is not partially written"
    );

    // Replacing the binding is a row write plus a derived geometry write; both
    // are refused while the layer is locked.
    let before = revision(&app);
    let depth = undo_depth(&app);
    let error = bind(
        &mut app,
        &mut session,
        vec![("radius".into(), Expr::Ref(q))],
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("layer.locked"),
        "got {error:?}"
    );
    assert_eq!(revision(&app), before, "nothing is written");
    assert_eq!(undo_depth(&app), depth, "no undo entry");
    assert_eq!(radius(&app, 1), 20.0);
    let drawing = stored_drawing(&app);
    let bindings = drawing
        .param_bindings(EntityId(1))
        .expect("the original binding is intact");
    assert!(
        bindings
            .get("radius")
            .is_some_and(|expr| *expr == Expr::Ref(r)),
        "the binding still names the original parameter"
    );
}

#[test]
fn deleting_a_referenced_parameter_is_refused() {
    let (mut app, mut session) = param_app(vec![circle(1, 10.0)]);
    session.selection.replace([ref_of(1)]);
    create(&mut app, &mut session, "r", ParamKind::Length, 20.0);
    let seq = seq_of(&app, "r");
    bind(
        &mut app,
        &mut session,
        vec![("radius".into(), Expr::Ref(seq))],
    )
    .expect("bind");

    let before = revision(&app);
    let depth = undo_depth(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Param,
        CommandPayload::Param(crate::ParamOp::Delete { seq }),
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("param.referenced"),
        "got {error:?}"
    );
    assert_eq!(revision(&app), before, "nothing is deleted");
    assert_eq!(undo_depth(&app), depth);
    assert!(stored_drawing(&app).parameters().get(seq).is_some());
}

#[test]
fn rename_keeps_the_reference_working() {
    let (mut app, mut session) = param_app(vec![circle(1, 10.0)]);
    session.selection.replace([ref_of(1)]);
    create(&mut app, &mut session, "r", ParamKind::Length, 20.0);
    let seq = seq_of(&app, "r");
    bind(
        &mut app,
        &mut session,
        vec![("radius".into(), Expr::Ref(seq))],
    )
    .expect("bind");

    execute(
        &mut app,
        &mut session,
        CommandId::Param,
        CommandPayload::Param(crate::ParamOp::Rename {
            seq,
            name: "radius_param".into(),
        }),
    )
    .expect("rename");
    assert!(
        stored_drawing(&app).parameters().get_by_name("r").is_none(),
        "the old name is gone"
    );
    assert_eq!(seq_of(&app, "radius_param"), seq, "the sequence is stable");
    assert_eq!(radius(&app, 1), 20.0, "references address the sequence");
    // The property key follows the new name.
    set_property(&mut app, &mut session, "param.radius_param", "25").expect("edit by new name");
    assert_eq!(radius(&app, 1), 25.0);
}

#[test]
fn polyline_width_and_text_height_slots_update() {
    let (mut app, mut session) = param_app(vec![wide_polyline(1, 1.0), text(2, 2.0)]);
    create(&mut app, &mut session, "w", ParamKind::Length, 4.0);
    create(&mut app, &mut session, "h", ParamKind::Length, 3.0);
    let w = seq_of(&app, "w");
    let h = seq_of(&app, "h");

    session.selection.replace([ref_of(1)]);
    bind(&mut app, &mut session, vec![("width".into(), Expr::Ref(w))]).expect("polyline bind");
    match geometry(&app, 1) {
        SemanticGeometry::Polyline { width, .. } => assert_eq!(width, 4.0),
        other => panic!("expected a polyline, got {other:?}"),
    }

    session.selection.replace([ref_of(2)]);
    bind(
        &mut app,
        &mut session,
        vec![("height".into(), Expr::Ref(h))],
    )
    .expect("text bind");
    match geometry(&app, 2) {
        SemanticGeometry::Text { height, .. } => assert_eq!(height, 3.0),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn binding_a_type_without_an_evaluator_is_refused() {
    let (mut app, mut session) = param_app(vec![(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
    )]);
    create(&mut app, &mut session, "r", ParamKind::Length, 20.0);
    let seq = seq_of(&app, "r");
    session.selection.replace([ref_of(1)]);

    let before = revision(&app);
    let depth = undo_depth(&app);
    let error = bind(
        &mut app,
        &mut session,
        vec![("radius".into(), Expr::Ref(seq))],
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("param.no_evaluator"),
        "got {error:?}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), depth);
    assert!(stored_drawing(&app).param_bindings(EntityId(1)).is_none());
}

#[test]
fn binding_an_unknown_slot_is_refused() {
    let (mut app, mut session) = param_app(vec![circle(1, 10.0)]);
    create(&mut app, &mut session, "r", ParamKind::Length, 20.0);
    let seq = seq_of(&app, "r");
    session.selection.replace([ref_of(1)]);

    let before = revision(&app);
    let depth = undo_depth(&app);
    let error = bind(
        &mut app,
        &mut session,
        vec![("diameter".into(), Expr::Ref(seq))],
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("param.unknown_slot"),
        "got {error:?}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), depth);
    assert!(stored_drawing(&app).param_bindings(EntityId(1)).is_none());
}

#[test]
fn an_unknown_parameter_name_is_refused() {
    let (mut app, mut session) = param_app(vec![circle(1, 10.0)]);
    session.selection.replace([ref_of(1)]);
    let before = revision(&app);

    let error = set_property(&mut app, &mut session, "param.nope", "1").unwrap_err();
    assert!(
        format!("{error:?}").contains("param.unknown_name"),
        "got {error:?}"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn param_config_switches_the_overlay_without_an_undo_step() {
    let (mut app, mut session) = configured_app();
    assert_eq!(radius(&app, 1), 20.0, "the base table is in force");
    assert_eq!(undo_depth(&app), 0);

    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Param,
        CommandPayload::Param(crate::ParamOp::Config {
            name: Some("small".into()),
        }),
    )
    .expect("config small");
    assert_eq!(radius(&app, 1), 5.0, "the overlay re-derives the geometry");
    assert_eq!(
        revision(&app),
        Revision(before.0 + 1),
        "the derived geometry is one revision"
    );
    assert!(
        outcome.changes.is_some(),
        "the revision publishes a ChangeSet"
    );
    assert_eq!(undo_depth(&app), 0, "config selection is not undoable");
    assert!(!app.can_undo(&DocumentId(1)));
    assert_eq!(
        stored_drawing(&app).active_configuration(),
        Some("small"),
        "the runtime overlay is active"
    );

    // Same name is a no-op: no revision, no ChangeSet.
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Param,
        CommandPayload::Param(crate::ParamOp::Config {
            name: Some("small".into()),
        }),
    )
    .expect("same config");
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app), before);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "param.unchanged"));

    // An unknown name is refused before anything mutates.
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Param,
        CommandPayload::Param(crate::ParamOp::Config {
            name: Some("nope".into()),
        }),
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("param.unknown_config"),
        "got {error:?}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(radius(&app, 1), 5.0, "the active overlay is unchanged");

    // Clearing the overlay returns to the base table.
    execute(
        &mut app,
        &mut session,
        CommandId::Param,
        CommandPayload::Param(crate::ParamOp::Config { name: None }),
    )
    .expect("base config");
    assert_eq!(radius(&app, 1), 20.0);
    assert_eq!(stored_drawing(&app).active_configuration(), None);
}

#[test]
fn a_value_edit_refuses_a_parameter_the_active_config_overrides_or_suppresses() {
    let (mut app, mut session) = configured_app();
    session.selection.replace([ref_of(1)]);

    // `qonly` overrides `q`, not `r`: editing `r` still changes the base row and
    // the derived geometry.
    execute(
        &mut app,
        &mut session,
        CommandId::Param,
        CommandPayload::Param(crate::ParamOp::Config {
            name: Some("qonly".into()),
        }),
    )
    .expect("qonly");
    set_property(&mut app, &mut session, "param.r", "30").expect("r is inherited");
    assert_eq!(radius(&app, 1), 30.0);
    assert_eq!(
        stored_drawing(&app)
            .parameters()
            .get_by_name("r")
            .expect("the base row exists")
            .expr,
        Expr::Num(30.0)
    );

    // `small` overrides `r`: the base row edit would not change the derived
    // value, so it is refused by name and writes nothing.
    execute(
        &mut app,
        &mut session,
        CommandId::Param,
        CommandPayload::Param(crate::ParamOp::Config {
            name: Some("small".into()),
        }),
    )
    .expect("small");
    let before = revision(&app);
    let depth = undo_depth(&app);
    let error = set_property(&mut app, &mut session, "param.r", "35").unwrap_err();
    let text = format!("{error:?}");
    assert!(text.contains("param.config_override"), "got {text}");
    assert!(
        text.contains("overridden by configuration small"),
        "got {text}"
    );
    assert_eq!(revision(&app), before, "nothing is written");
    assert_eq!(undo_depth(&app), depth);
    assert_eq!(radius(&app, 1), 5.0, "the overlay value is unchanged");
    assert_eq!(
        stored_drawing(&app)
            .parameters()
            .get_by_name("r")
            .expect("the base row exists")
            .expr,
        Expr::Num(30.0),
        "the base row is not rewritten"
    );

    // A suppressed parameter is the same named refusal with its own word.
    execute(
        &mut app,
        &mut session,
        CommandId::Param,
        CommandPayload::Param(crate::ParamOp::Config {
            name: Some("hide".into()),
        }),
    )
    .expect("hide");
    let error = set_property(&mut app, &mut session, "param.s", "9").unwrap_err();
    let text = format!("{error:?}");
    assert!(text.contains("param.config_override"), "got {text}");
    assert!(
        text.contains("suppressed by configuration hide"),
        "got {text}"
    );
}

#[test]
fn binding_an_insert_refuses_with_no_evaluator() {
    let (mut app, mut session) = insert_app();
    create(&mut app, &mut session, "k", ParamKind::Length, 3.0);
    let seq = seq_of(&app, "k");
    session.selection.replace([ref_of(1)]);

    let before = revision(&app);
    let depth = undo_depth(&app);
    let error = bind(
        &mut app,
        &mut session,
        vec![("scale".into(), Expr::Ref(seq))],
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("param.no_evaluator"),
        "P1 defers the Insert evaluator, got {error:?}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), depth);
    assert!(stored_drawing(&app).param_bindings(EntityId(1)).is_none());
}
