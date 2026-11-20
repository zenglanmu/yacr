//! CHAMFER / FILLET two-line edit contracts (ADR 0016 decisions B/C).
//!
//! The scenario groups live in sibling submodules (`chamfer`, `fillet`,
//! `refusals`, `edit_policy`) to keep this file under the code-size limit; the
//! shared drawing fixtures stay here so every group sees them through
//! `use super::*`.

use super::*;

/// The two endpoints of a LINE entity, or a panic.
fn line_ends(app: &Application, entity: u128) -> (Point3, Point3) {
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(entity))
        .unwrap_or_else(|| panic!("entity {entity} must exist"))
        .geometry
    {
        SemanticGeometry::Line { start, end } => (*start, *end),
        other => panic!("expected a LINE, got {other:?}"),
    }
}

/// Whether a LINE entity's two endpoints are exactly `a`/`b` (in either order).
fn line_is(app: &Application, entity: u128, a: Point3, b: Point3) -> bool {
    let (start, end) = line_ends(app, entity);
    let same = |p: Point3, q: Point3| (p.x - q.x).abs() < 1e-9 && (p.y - q.y).abs() < 1e-9;
    (same(start, a) && same(end, b)) || (same(start, b) && same(end, a))
}

/// The stored entity of one id.
fn db_entity(app: &Application, entity: u128) -> DbEntity {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(entity))
        .unwrap()
        .clone()
}

/// The stored geometry of one entity.
fn stored_geometry(app: &Application, entity: u128) -> SemanticGeometry {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(entity))
        .unwrap()
        .geometry
        .clone()
}

/// Wrap a finished drawing in the standard application/session pair.
fn app_from(drawing: cad_db::DrawingDatabase) -> (Application, SessionState) {
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
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

/// A drawing with two straight lines (ids 1 and 2) on layer 0, a second layer
/// `5`, and an ARC (id 3) for the wrong-type refusal.
fn two_line_app(l1: (Point3, Point3), l2: (Point3, Point3)) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(5), "CORNER")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_entity(ent(
        1,
        SemanticGeometry::Line {
            start: l1.0,
            end: l1.1,
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        2,
        SemanticGeometry::Line {
            start: l2.0,
            end: l2.1,
        },
        1,
    ))
    .unwrap();
    b.insert_entity(ent(
        3,
        SemanticGeometry::Arc {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
            start: 0.0,
            sweep: 1.0,
        },
        2,
    ))
    .unwrap();
    app_from(b.finish().unwrap())
}

/// An L corner at the origin: L1 the +x axis, L2 the +y axis, both 10 long.
fn l_corner() -> (Application, SessionState) {
    two_line_app(
        (point(0.0, 0.0), point(10.0, 0.0)),
        (point(0.0, 0.0), point(0.0, 10.0)),
    )
}

fn set_number(session: &mut SessionState, parameter: ToolParameter, value: f64) {
    session
        .set_tool_parameter(parameter, ToolValue::Number(value))
        .unwrap_or_else(|error| panic!("{parameter:?} = {value} refused: {error}"));
}

fn chamfer(first: u128, second: u128, pick_first: Point3, pick_second: Point3) -> CommandPayload {
    CommandPayload::Chamfer(Box::new(TwoLineCommand {
        first: ref_of(first),
        second: ref_of(second),
        pick_first,
        pick_second,
    }))
}

fn fillet(first: u128, second: u128, pick_first: Point3, pick_second: Point3) -> CommandPayload {
    CommandPayload::Fillet(Box::new(TwoLineCommand {
        first: ref_of(first),
        second: ref_of(second),
        pick_first,
        pick_second,
    }))
}

/// Run a CHAMFER/FILLET payload that must be refused with no state change.
fn assert_refused(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
    expected_unsupported: bool,
) {
    let rev = revision(app);
    let depth = app
        .history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0);
    let error = execute(app, session, id, payload).unwrap_err();
    if expected_unsupported {
        assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    } else {
        assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    }
    assert_eq!(revision(app), rev, "a refused edit changes nothing");
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|history| history.undo_depth())
            .unwrap_or(0),
        depth,
        "a refused edit records no undo step"
    );
}

/// A drawing with a LINE (entity 1) and a CIRCLE (entity 2) on layer 0.
fn line_circle_app(
    line: (Point3, Point3),
    center: Point3,
    radius: f64,
) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        1,
        SemanticGeometry::Line {
            start: line.0,
            end: line.1,
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        2,
        SemanticGeometry::Circle {
            center,
            normal: p3(0.0, 0.0, 1.0),
            radius,
        },
        0,
    ))
    .unwrap();
    app_from(b.finish().unwrap())
}

mod chamfer;
mod edit_policy;
mod fillet;
mod refusals;
