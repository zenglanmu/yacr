//! TRIM/EXTEND capture-time entity pick contracts.
//!
//! `docs/drawing-edit.md` §2–§4: a TRIM/EXTEND click is an entity pick, not a
//! free point. These tests drive the real host path (`capture_tool_point`,
//! `move_tool_cursor`, `command_for_draw_intent`) against a small controlled
//! drawing whose geometry sits inside the default `[800, 600]` plan camera.

use super::*;
use crate::CommandPayload;

/// A controlled database for the pick tests.
///
/// Coordinates are chosen so every pick under the default top view (ortho
/// `scale = 1.0`, world `x ∈ [-400, 400]`, `y ∈ [-300, 300]`) maps to a distinct
/// screen pixel with the nearest other entity hundreds of world units away —
/// far outside the 3-logical-pixel pickbox.
///
/// * `101` — LINE at `y = 100`, the TRIM/EXTEND target.
/// * `102` — LINE at `x = 0`, the boundary.
/// * `103` — CIRCLE at `(250, -200)`, r `50`, a wrong-type target.
fn fixture_database() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(entity(101, line(-200.0, 100.0, 200.0, 100.0), 0))
        .unwrap();
    b.insert_entity(entity(102, line(0.0, -100.0, 0.0, 300.0), 1))
        .unwrap();
    b.insert_entity(entity(
        103,
        SemanticGeometry::Circle {
            center: p(250.0, -200.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 50.0,
        },
        2,
    ))
    .unwrap();
    b.finish().unwrap()
}

fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: p(x0, y0),
        end: p(x1, y1),
    }
}

fn entity(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: crate::app_drawing::class_name(&geometry).to_string(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: order,
    }
}

/// A host controller with the controlled drawing installed.
fn fixture() -> HostController {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller
        .application
        .workspace
        .documents
        .get_mut(&controller.document_id)
        .unwrap()
        .drawing = Arc::new(fixture_database());
    // Deterministic raw points: these contracts are about the entity pick, not
    // about snapping.
    controller.set_snap_enabled(false);
    controller
}

fn reference(document: DocumentId, entity: u128) -> SelectionRef {
    SelectionRef {
        document,
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn target_ref(controller: &HostController) -> SelectionRef {
    reference(controller.document_id, 101)
}

fn boundary_ref(controller: &HostController) -> SelectionRef {
    reference(controller.document_id, 102)
}

/// Empty space (all entities hundreds of units away) inside the viewport.
fn empty_point() -> Point3 {
    p(350.0, 250.0)
}

#[test]
fn trim_capture_refuses_empty_space_and_keeps_capture_intact() {
    let mut controller = fixture();
    let revision = controller.drawing().unwrap().revision();
    controller.begin_draw_tool(DrawToolKind::Trim).unwrap();

    // A click on empty space is refused immediately with the existing refusal
    // text, and the capture is untouched: no point, no consumed step, no
    // revision churn, so the user can simply click again.
    let miss = controller.capture_tool_point(empty_point()).unwrap_err();
    assert!(matches!(miss, CadError::InvalidInput(_)), "{miss:?}");
    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(preview.remaining, 2);
    assert!(preview.points.is_empty());
    assert!(preview.picks.is_empty());
    assert!(!preview.can_confirm());
    assert_eq!(controller.drawing().unwrap().revision(), revision);

    // A hit stores both the raw pick point (the command needs it) and the ref
    // resolved at capture time, then advances.
    controller.capture_tool_point(p(-150.0, 100.0)).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(preview.remaining, 1);
    assert_eq!(preview.points, vec![p(-150.0, 100.0)]);
    assert_eq!(preview.picks, vec![target_ref(&controller)]);

    // The second step obeys the same contract.
    let miss = controller.capture_tool_point(empty_point()).unwrap_err();
    assert!(matches!(miss, CadError::InvalidInput(_)), "{miss:?}");
    assert_eq!(controller.session.draw_preview().unwrap().remaining, 1);
    controller.capture_tool_point(p(0.0, 250.0)).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(preview.remaining, 0);
    assert!(preview.can_confirm());
    assert_eq!(
        preview.picks,
        vec![target_ref(&controller), boundary_ref(&controller)]
    );
    // Still no database change: capture is intent, not a transaction.
    assert_eq!(controller.drawing().unwrap().revision(), revision);
}

#[test]
fn extend_capture_advances_only_on_a_resolved_entity() {
    let mut controller = fixture();
    controller.begin_draw_tool(DrawToolKind::Extend).unwrap();

    let miss = controller.capture_tool_point(empty_point()).unwrap_err();
    assert!(matches!(miss, CadError::InvalidInput(_)), "{miss:?}");
    assert_eq!(controller.session.draw_preview().unwrap().remaining, 2);

    controller.capture_tool_point(p(-150.0, 100.0)).unwrap();
    controller.capture_tool_point(p(0.0, 250.0)).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert!(preview.can_confirm());
    assert_eq!(
        preview.picks,
        vec![target_ref(&controller), boundary_ref(&controller)]
    );
}

#[test]
fn cursor_move_publishes_the_entity_under_the_cursor() {
    let mut controller = fixture();
    controller.begin_draw_tool(DrawToolKind::Trim).unwrap();

    // No entity under the cursor: no candidate is fabricated.
    controller.move_tool_cursor(empty_point()).unwrap();
    assert_eq!(controller.session.draw_preview().unwrap().candidate, None);

    // The entity under the cursor is published, resolved by the same pick path.
    controller.move_tool_cursor(p(-150.0, 100.0)).unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().candidate,
        Some(target_ref(&controller))
    );

    // Capturing the target clears the stale candidate; the next move resolves
    // the pending boundary step instead.
    controller.capture_tool_point(p(-150.0, 100.0)).unwrap();
    assert_eq!(controller.session.draw_preview().unwrap().candidate, None);
    controller.move_tool_cursor(p(0.0, 250.0)).unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().candidate,
        Some(boundary_ref(&controller))
    );
}

#[test]
fn trim_confirm_uses_the_refs_resolved_at_capture() {
    let mut controller = fixture();
    controller.begin_draw_tool(DrawToolKind::Trim).unwrap();
    controller.capture_tool_point(p(-150.0, 100.0)).unwrap();
    controller.capture_tool_point(p(0.0, 250.0)).unwrap();
    let captured = controller.session.draw_preview().unwrap().picks;
    assert_eq!(
        captured,
        vec![target_ref(&controller), boundary_ref(&controller)]
    );

    // Replace the drawing so a confirm-time re-pick of the stored points would
    // resolve to *different* entity ids (same geometry, ids 200/201). The refs
    // the user saw must still win: confirm must not pick a second time.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(entity(200, line(-200.0, 100.0, 200.0, 100.0), 0))
        .unwrap();
    b.insert_entity(entity(201, line(0.0, -100.0, 0.0, 300.0), 1))
        .unwrap();
    controller
        .application
        .workspace
        .documents
        .get_mut(&controller.document_id)
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());

    let intent = controller.session.draw_intent().unwrap();
    let selection = controller
        .session
        .draw_selection_snapshot()
        .unwrap()
        .to_vec();
    let command = controller
        .command_for_draw_intent(intent, &selection)
        .unwrap();
    match command.payload {
        CommandPayload::Trim {
            target,
            boundary,
            pick_point,
        } => {
            assert_eq!(target, target_ref(&controller), "captured target ref");
            assert_eq!(
                boundary,
                vec![boundary_ref(&controller)],
                "captured boundary ref"
            );
            assert_eq!(pick_point, p(-150.0, 100.0));
        }
        _ => panic!("expected a Trim payload"),
    }
}

#[test]
fn wrong_type_target_still_refused_by_the_command_layer() {
    let mut controller = fixture();
    let revision = controller.drawing().unwrap().revision();
    controller.begin_draw_tool(DrawToolKind::Trim).unwrap();
    // Target is the CIRCLE (a real pick), boundary is the vertical LINE.
    controller.capture_tool_point(p(300.0, -200.0)).unwrap();
    controller.capture_tool_point(p(0.0, 250.0)).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert_eq!(
        preview.picks[0],
        reference(controller.document_id, 103),
        "the circle was the picked target"
    );
    assert!(preview.can_confirm());

    // The command layer keeps its own target-type refusal untouched.
    let err = controller.confirm_draw().unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)), "{err:?}");
    assert_eq!(controller.drawing().unwrap().revision(), revision);
    // A failed confirm preserves the capture.
    assert!(controller.session.draw_preview().is_some());
}

/// The controller's real undo depth for the open document (0 before any edit).
fn undo_depth(controller: &HostController) -> usize {
    controller
        .application
        .history
        .get(&controller.document_id)
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

#[test]
fn trim_confirm_commits_the_real_geometry_change_through_confirm_draw() {
    // `fixture()` is a non-parallel pair: target LINE `101` at y = 100 and
    // boundary LINE `102` at x = 0, crossing at (0, 100). This pins the
    // **interactive** path (`capture_tool_point` -> `confirm_draw` -> command ->
    // DB), not just the command unit tests.
    let mut controller = fixture();
    let revision_before = controller.drawing().unwrap().revision();
    let undo_before = undo_depth(&controller);

    controller.begin_draw_tool(DrawToolKind::Trim).unwrap();
    controller.capture_tool_point(p(-150.0, 100.0)).unwrap();
    controller.capture_tool_point(p(0.0, 250.0)).unwrap();
    assert!(controller.session.draw_preview().unwrap().can_confirm());

    let outcome = controller
        .confirm_draw()
        .expect("TRIM confirm must succeed on a crossing boundary");
    assert_eq!(outcome.objects, vec![ObjectId(101)]);

    let (start, end) = match &controller
        .drawing()
        .unwrap()
        .entity(EntityId(101))
        .expect("the trimmed target is modified, not deleted")
        .geometry
    {
        SemanticGeometry::Line { start, end } => (*start, *end),
        other => panic!("expected a LINE, got {other:?}"),
    };
    // The target originally ran (-200, 100) -> (200, 100); the pick was on the
    // left half, so TRIM keeps that half and removes x > 0.
    assert!((start.x - -200.0).abs() < 1e-9, "start.x={}", start.x);
    assert!((start.y - 100.0).abs() < 1e-9, "start.y={}", start.y);
    assert!((end.x - 0.0).abs() < 1e-9, "end.x={} (the cut)", end.x);
    assert!((end.y - 100.0).abs() < 1e-9, "end.y={}", end.y);
    assert!(
        end.x < 200.0,
        "the removed portion beyond the cut is gone: end={end:?}"
    );

    assert_eq!(
        controller.drawing().unwrap().revision(),
        Revision(revision_before.0 + 1),
        "exactly one database transaction"
    );
    assert_eq!(
        undo_depth(&controller),
        undo_before + 1,
        "exactly one undo step"
    );
    assert!(
        controller.session.draw_preview().is_none(),
        "a successful confirmation clears the capture"
    );
}

/// A controlled drawing with a target LINE at y = 100 and a **parallel**
/// boundary LINE at y = 200 (no crossing anywhere in the span).
fn parallel_boundary_fixture() -> HostController {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(entity(101, line(-200.0, 100.0, 200.0, 100.0), 0))
        .unwrap();
    b.insert_entity(entity(104, line(-200.0, 200.0, 200.0, 200.0), 1))
        .unwrap();
    controller
        .application
        .workspace
        .documents
        .get_mut(&controller.document_id)
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    controller.set_snap_enabled(false);
    controller
}

#[test]
fn trim_confirm_with_a_parallel_boundary_is_refused_and_changes_nothing() {
    let mut controller = parallel_boundary_fixture();
    let geometry_before = controller
        .drawing()
        .unwrap()
        .entity(EntityId(101))
        .unwrap()
        .geometry
        .clone();
    let revision_before = controller.drawing().unwrap().revision();
    let undo_before = undo_depth(&controller);

    controller.begin_draw_tool(DrawToolKind::Trim).unwrap();
    controller.capture_tool_point(p(-150.0, 100.0)).unwrap();
    controller.capture_tool_point(p(-150.0, 200.0)).unwrap();
    assert!(controller.session.draw_preview().unwrap().can_confirm());

    let error = controller
        .confirm_draw()
        .expect_err("a parallel boundary has no intersection to cut");
    match &error {
        CadError::InvalidInput(reason) => assert!(
            reason.contains("TRIM found no intersection"),
            "unexpected refusal: {error:?}"
        ),
        other => panic!("expected InvalidInput, got {other:?}"),
    }

    assert_eq!(
        controller
            .drawing()
            .unwrap()
            .entity(EntityId(101))
            .unwrap()
            .geometry,
        geometry_before,
        "a refused TRIM leaves the stored geometry byte-identical"
    );
    assert_eq!(
        controller.drawing().unwrap().revision(),
        revision_before,
        "a refused TRIM does not advance the document revision"
    );
    assert_eq!(
        undo_depth(&controller),
        undo_before,
        "a refused TRIM records no undo step"
    );
    assert!(
        controller.session.draw_preview().is_some(),
        "a failed confirmation preserves the capture"
    );
}
