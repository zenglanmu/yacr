//! `XLINE`/`RAY` create-command contract tests.
//!
//! `cad-db` has no infinite-line geometry, so both commands store one finite
//! `SemanticGeometry::Line` clipped to the drawing's XY model bounds (or the
//! importer's default 1000-unit box around `base` for an empty drawing) and keep
//! the source class name in `type_key` (`AcDbXline`/`AcDbRay`). These tests pin
//! the reduction (exact clipped endpoints in both directions for XLINE, the
//! positive half only for RAY), the one-entity/one-transaction/one-undo shape on
//! the active layer, the honest `drawing.xline`/`drawing.ray` diagnostic and every
//! named refusal (nothing written, no revision bump, no undo entry).

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer};

/// The bounds fixture: a closed polyline spanning exactly `(0,0)-(10,10)`, so the
/// derived clip box is `(-1,-1)-(11,11)` (margin `(√200 · 0.05).max(1.0) == 1.0`).
fn app_with_bounds(mode: AppMode) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbPolyline".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Polyline {
            points: vec![
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(10.0, 10.0),
                point(0.0, 10.0),
            ],
            bulges: vec![0.0; 4],
            closed: true,
            width: 0.0,
        },
        draw_order: 0,
    })
    .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([61u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let session = SessionState::new(DocumentId(1), mode);
    (app, session)
}

/// A work-mode empty drawing (no model geometry, so `bounds()` is `None`).
fn empty_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let drawing = b.finish().unwrap();
    assert!(drawing.bounds().is_none(), "fixture must have no bounds");
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([62u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let session = SessionState::new(DocumentId(1), AppMode::Work);
    (app, session)
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {expected}, got {actual}"
    );
}

fn assert_point_close(actual: Point3, expected: Point3) {
    assert_close(actual.x, expected.x);
    assert_close(actual.y, expected.y);
    assert_close(actual.z, expected.z);
}

/// The stored entity of `outcome`, asserting it is a LINE on `layer` in model
/// space with `type_key`, and returning its endpoints.
fn stored_line(
    app: &Application,
    outcome: &CommandOutcome,
    layer: LayerId,
    type_key: &str,
) -> (Point3, Point3) {
    assert_eq!(outcome.objects.len(), 1, "one entity per construct line");
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(outcome.objects[0].0))
        .expect("the created construct-line entity");
    assert_eq!(entity.object.type_key, type_key);
    assert_eq!(entity.layer, layer);
    assert_eq!(entity.space, SpaceId::Model);
    match &entity.geometry {
        SemanticGeometry::Line { start, end } => (*start, *end),
        other => panic!("expected a finite LINE, got {other:?}"),
    }
}

fn diagnostic_codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

#[test]
fn xline_is_clipped_to_the_expanded_model_bounds_in_both_directions() {
    let (mut app, mut session) = app_with_bounds(AppMode::Work);
    // The active layer is not layer 0: the created entity must follow it.
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(7)),
    )
    .unwrap();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateXLine,
        CommandPayload::XLine {
            base: point(0.0, 5.0),
            through: point(1.0, 5.0),
        },
    )
    .unwrap();

    // The 10x10 bounds get a 1.0 margin, so the box is (-1,-1)-(11,11) and the
    // horizontal XLINE through y = 5 spans it in **both** directions from the
    // base point at x = 0.
    let (start, end) = stored_line(&app, &outcome, LayerId(7), "AcDbXline");
    assert_point_close(start, point(-1.0, 5.0));
    assert_point_close(end, point(11.0, 5.0));
    assert!(start.x < 0.0 && end.x > 10.0, "both directions extended");

    let changes = outcome.changes.as_ref().expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(changes.changes.len(), 1, "one transaction");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");

    // The honest loss statement is always attached to a success.
    assert!(diagnostic_codes(&outcome).contains(&"drawing.xline"));
    let message = &outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "drawing.xline")
        .unwrap()
        .message;
    assert!(message.contains("finite segment"), "{message}");
    assert!(message.contains("model bounds"), "{message}");
    assert!(message.contains("LINE"), "{message}");
}

#[test]
fn xline_through_a_diagonal_keeps_exact_clipped_endpoints() {
    let (mut app, mut session) = app_with_bounds(AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateXLine,
        CommandPayload::XLine {
            base: point(0.0, 0.0),
            through: point(1.0, 1.0),
        },
    )
    .unwrap();
    // A straight line enters and leaves the axis-aligned box once, so the result
    // is one segment: (-1,-1) to (11,11), not a stitched approximation.
    let (start, end) = stored_line(&app, &outcome, LayerId(0), "AcDbXline");
    assert_point_close(start, point(-1.0, -1.0));
    assert_point_close(end, point(11.0, 11.0));
    assert_eq!(
        outcome.changes.as_ref().unwrap().changes.len(),
        1,
        "one transaction"
    );
}

#[test]
fn ray_extends_only_the_positive_half_from_the_base_point() {
    let (mut app, mut session) = app_with_bounds(AppMode::Work);
    let base = point(0.0, 5.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateRay,
        CommandPayload::Ray {
            base,
            through: point(1.0, 5.0),
        },
    )
    .unwrap();
    let (start, end) = stored_line(&app, &outcome, LayerId(0), "AcDbRay");
    // The ray starts **at** the base and stops at the +x box edge; the negative
    // half (x < 0) is never part of the stored segment.
    assert_point_close(start, base);
    assert_point_close(end, point(11.0, 5.0));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");
    assert!(diagnostic_codes(&outcome).contains(&"drawing.ray"));
    let message = &outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "drawing.ray")
        .unwrap()
        .message;
    assert!(message.contains("finite segment"), "{message}");
}

#[test]
fn an_empty_drawing_uses_the_default_box_around_the_base_point() {
    let (mut app, mut session) = empty_app();
    let base = point(100.0, 200.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateXLine,
        CommandPayload::XLine {
            base,
            through: point(101.0, 200.0),
        },
    )
    .unwrap();
    // No bounds: the importer's default 1000-unit box around `base`, i.e.
    // (-900, -800) to (1100, 1200) in XY.
    let (start, end) = stored_line(&app, &outcome, LayerId(0), "AcDbXline");
    assert_point_close(start, point(-900.0, 200.0));
    assert_point_close(end, point(1100.0, 200.0));
    let message = &outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "drawing.xline")
        .unwrap()
        .message;
    assert!(message.contains("default 1000-unit box"), "{message}");
    assert!(message.contains("LINE"), "{message}");
}

#[test]
fn degenerate_and_non_finite_inputs_are_named_refusals_that_write_nothing() {
    let (mut app, mut session) = app_with_bounds(AppMode::Work);
    let before = revision(&app);
    let entities_before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();

    for (id, payload, expected) in [
        (
            CommandId::CreateXLine,
            CommandPayload::XLine {
                base: point(1.0, 1.0),
                through: point(1.0, 1.0),
            },
            "xline.degenerate_direction",
        ),
        (
            CommandId::CreateRay,
            CommandPayload::Ray {
                base: point(1.0, 1.0),
                through: point(1.0, 1.0),
            },
            "ray.degenerate_direction",
        ),
        (
            CommandId::CreateXLine,
            CommandPayload::XLine {
                base: point(f64::NAN, 0.0),
                through: point(1.0, 1.0),
            },
            "xline.invalid_point",
        ),
        (
            CommandId::CreateRay,
            CommandPayload::Ray {
                base: point(0.0, 0.0),
                through: point(f64::INFINITY, 1.0),
            },
            "ray.invalid_point",
        ),
        // The positive half never enters the box: point the RAY away from it.
        (
            CommandId::CreateRay,
            CommandPayload::Ray {
                base: point(100.0, 100.0),
                through: point(101.0, 101.0),
            },
            "ray.outside_bounds",
        ),
    ] {
        let error =
            execute(&mut app, &mut session, id, payload).expect_err("the refusal must not succeed");
        let CadError::InvalidInput(message) = error else {
            panic!("expected InvalidInput, got {error:?}");
        };
        assert!(message.contains(expected), "{message} must name {expected}");
    }

    assert_eq!(revision(&app), before, "a refused input must not write");
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        entities_before,
        "no entity was created"
    );
    assert!(!app.can_undo(&DocumentId(1)), "no undo entry was recorded");
}

#[test]
fn the_wrong_payload_shape_is_refused_by_name() {
    let (mut app, mut session) = app_with_bounds(AppMode::Work);
    let before = revision(&app);
    // An XLINE payload sent to RAY (and vice versa) is a payload mismatch, never
    // a silently converted construct line.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::CreateRay,
        CommandPayload::XLine {
            base: point(0.0, 0.0),
            through: point(1.0, 1.0),
        },
    )
    .expect_err("a mismatched payload is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn undo_removes_the_construct_line_and_both_commands_share_one_session() {
    let (mut app, mut session) = app_with_bounds(AppMode::Work);
    let xline = execute(
        &mut app,
        &mut session,
        CommandId::CreateXLine,
        CommandPayload::XLine {
            base: point(0.0, 5.0),
            through: point(1.0, 5.0),
        },
    )
    .unwrap();
    let xline_id = EntityId(xline.objects[0].0);

    // The RAY command works in the same session, on the same runtime state.
    let ray = execute(
        &mut app,
        &mut session,
        CommandId::CreateRay,
        CommandPayload::Ray {
            base: point(5.0, 0.0),
            through: point(5.0, 1.0),
        },
    )
    .unwrap();
    let ray_id = EntityId(ray.objects[0].0);
    assert_ne!(xline_id, ray_id);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 2);
    let (ray_start, ray_end) = stored_line(&app, &ray, LayerId(0), "AcDbRay");
    assert_point_close(ray_start, point(5.0, 0.0));
    assert_point_close(ray_end, point(5.0, 11.0));

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(ray_id)
        .is_none());
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(xline_id)
        .is_some());
}

#[test]
fn viewer_mode_refuses_both_commands_before_any_write() {
    let (mut app, mut session) = app_with_bounds(AppMode::Viewer);
    let before = revision(&app);
    for (id, payload) in [
        (
            CommandId::CreateXLine,
            CommandPayload::XLine {
                base: point(0.0, 5.0),
                through: point(1.0, 5.0),
            },
        ),
        (
            CommandId::CreateRay,
            CommandPayload::Ray {
                base: point(0.0, 5.0),
                through: point(1.0, 5.0),
            },
        ),
    ] {
        let error = execute(&mut app, &mut session, id, payload)
            .expect_err("Work-only commands must refuse Viewer mode");
        assert!(
            matches!(error, CadError::PermissionDenied),
            "{id:?}: {error:?}"
        );
    }
    assert_eq!(revision(&app), before, "a refusal must not write");
    assert!(!app.can_undo(&DocumentId(1)));
}
