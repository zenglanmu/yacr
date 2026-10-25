//! `GRIPSTRETCH` command contract tests (`crate::app_drawing::grip`).
//!
//! Pins the write path the grip model's doc mandates: re-read the addressed
//! entity, re-validate the handle against the **current** geometry, and stage
//! exactly one `update_entity` in one transaction / one undo step. Every refusal
//! is named and happens **before** the transaction, so it changes no revision,
//! records no undo entry and writes no transaction — the reason a zero
//! displacement is an error rather than an empty `Ok`.
//!
//! No renderer marker, host arming or pointer routing is involved (none exists):
//! the payload is built directly here, exactly as a future gesture lane would.

use super::*;

use crate::grips::{
    GripId, GripKind, REASON_NON_FINITE_TARGET, REASON_STALE_HANDLE, REASON_TARGET_MISSING,
    REASON_UNSUPPORTED_KIND, REASON_ZERO_DISPLACEMENT,
};
use crate::GripStretchCommand;

const LINE: u128 = 1;
const POINT: u128 = 2;
const CIRCLE: u128 = 3;
const PAPER_LINE: u128 = 4;
/// A geometry with **no** grip model at all (`ELLIPSE` axis grips are NOT
/// IMPLEMENTED), used for the `grip.unsupported_kind` refusal now that `CIRCLE`
/// and `ARC` have handles.
const ELLIPSE: u128 = 5;

/// Non-round coordinates so an "untouched endpoint" assertion is a real bit
/// comparison rather than a happy accident of decimal literals.
const LINE_START: Point3 = Point3 {
    x: 0.1,
    y: 0.2,
    z: 1.0 / 3.0,
};
const LINE_END: Point3 = Point3 {
    x: -4.5,
    y: 6.25,
    z: 0.5,
};
const POINT_AT: Point3 = Point3 {
    x: 3.0,
    y: 4.0,
    z: 0.0,
};

/// A drawing with a `LINE` (id 1), a `POINT` (id 2), a `CIRCLE` (id 3, the
/// unsupported-kind target) and a paper-space `LINE` (id 4, the
/// non-addressable target). Nothing is created through a command, so the undo
/// stack starts empty and a drag's undo depth is unambiguous.
fn grip_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        LINE,
        SemanticGeometry::Line {
            start: LINE_START,
            end: LINE_END,
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(POINT, SemanticGeometry::Point(POINT_AT), 1))
        .unwrap();
    b.insert_entity(ent(
        CIRCLE,
        SemanticGeometry::Circle {
            center: p3(0.0, 0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
        },
        2,
    ))
    .unwrap();
    let mut paper = ent(
        PAPER_LINE,
        SemanticGeometry::Line {
            start: p3(0.0, 0.0, 0.0),
            end: p3(1.0, 0.0, 0.0),
        },
        3,
    );
    paper.space = SpaceId::Paper(LayoutId(1));
    b.insert_entity(ent(
        ELLIPSE,
        SemanticGeometry::Ellipse {
            center: p3(0.0, 0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(5.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },
        4,
    ))
    .unwrap();
    // The drawing must carry the layout the paper-space entity references.
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Sheet1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.insert_entity(paper).unwrap();
    let drawing = b.finish().unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([11u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

fn grip_id(entity: u128, kind: GripKind, index: u8) -> GripId {
    GripId {
        entity: EntityId(entity),
        kind,
        index,
    }
}

/// The payload a grip gesture would produce: one handle id plus the world point.
fn drag(entity: u128, kind: GripKind, index: u8, to: Point3) -> CommandPayload {
    CommandPayload::GripStretch(Box::new(GripStretchCommand {
        id: grip_id(entity, kind, index),
        to,
    }))
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn geometry_of(app: &Application, entity: u128) -> SemanticGeometry {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(entity))
        .unwrap_or_else(|| panic!("entity {entity} is gone"))
        .geometry
        .clone()
}

fn endpoints_of(app: &Application, entity: u128) -> (Point3, Point3) {
    match geometry_of(app, entity) {
        SemanticGeometry::Line { start, end } => (start, end),
        other => panic!("entity {entity} is not a LINE: {other:?}"),
    }
}

/// The documented exact midpoint of two stored points (`(start + end) * 0.5`),
/// recomputed here independently of the model.
fn midpoint_of(start: Point3, end: Point3) -> Point3 {
    Point3 {
        x: (start.x + end.x) * 0.5,
        y: (start.y + end.y) * 0.5,
        z: (start.z + end.z) * 0.5,
    }
}

fn position_of(app: &Application, entity: u128) -> Point3 {
    match geometry_of(app, entity) {
        SemanticGeometry::Point(position) => position,
        other => panic!("entity {entity} is not a POINT: {other:?}"),
    }
}

/// Assert a refusal with the exact stable reason code, and that the refusal
/// variant is the documented one (`InvalidInput` for its family,
/// `Unsupported` for the kind/addressability refusals).
fn refused(error: CadError, code: &str, unsupported: bool) -> String {
    let message = match (error, unsupported) {
        (CadError::Unsupported(message), true) | (CadError::InvalidInput(message), false) => {
            message
        }
        (other, _) => panic!("unexpected refusal variant for {code}: {other:?}"),
    };
    assert!(
        message.starts_with(code),
        "reason must start with {code}, got {message}"
    );
    message
}

/// Every refusal must leave the drawing and the undo stack exactly as they were.
fn assert_untouched(app: &Application, revision: Revision, undo: usize) {
    assert_eq!(
        revision_of(app),
        revision,
        "a refusal must not bump revision"
    );
    assert_eq!(undo_depth(app), undo, "a refusal must not record history");
}

fn revision_of(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

#[test]
fn line_start_grip_drag_is_one_change_one_undo_step_and_undoes_exactly() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(LINE, GripKind::Endpoint, 0, p3(7.0, 8.0, 9.0)),
    )
    .unwrap();

    // One entity update, in one transaction, with one undo entry.
    let changes = outcome.changes.as_ref().expect("a ChangeSet is published");
    assert_eq!(changes.changes.len(), 1, "exactly one entity update");
    assert_eq!(outcome.objects, vec![ObjectId(LINE)]);
    assert_eq!(revision_of(&app), Revision(revision.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    let (start, end) = endpoints_of(&app, LINE);
    assert_eq!(start, p3(7.0, 8.0, 9.0), "the dragged endpoint takes `to`");
    assert_eq!(end, LINE_END, "the other endpoint keeps its exact bits");

    // Undo restores the original handle exactly (same `f64` bits, not a
    // re-derived approximation).
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let (start, end) = endpoints_of(&app, LINE);
    assert_eq!(start.x.to_bits(), LINE_START.x.to_bits());
    assert_eq!(start.y.to_bits(), LINE_START.y.to_bits());
    assert_eq!(start.z.to_bits(), LINE_START.z.to_bits());
    assert_eq!(end, LINE_END);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn line_end_grip_drag_leaves_the_start_endpoint_bit_identical() {
    let (mut app, mut session) = grip_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(LINE, GripKind::Endpoint, 1, p3(-1.0, 2.0, 3.0)),
    )
    .unwrap();
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 1);
    assert_eq!(undo_depth(&app), 1);

    let (start, end) = endpoints_of(&app, LINE);
    // Bit-identical, not approximately equal: the drag rule returns the stored
    // point instead of recomputing a pair.
    assert_eq!(start.x.to_bits(), LINE_START.x.to_bits());
    assert_eq!(start.y.to_bits(), LINE_START.y.to_bits());
    assert_eq!(start.z.to_bits(), LINE_START.z.to_bits());
    assert_eq!(end, p3(-1.0, 2.0, 3.0));

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let (start, end) = endpoints_of(&app, LINE);
    assert_eq!(start, LINE_START);
    assert_eq!(end.z.to_bits(), LINE_END.z.to_bits());
    assert_eq!(end, LINE_END);
}

#[test]
fn line_midpoint_grip_drag_translates_one_line_in_one_undo_step() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    let (start, end) = endpoints_of(&app, LINE);
    let mid = midpoint_of(start, end);
    let to = p3(mid.x + 2.5, mid.y - 1.25, mid.z + 0.75);
    // The delta the model sees is `to - midpoint` in `f64`, so compute the
    // expectation the same way instead of assuming `(mid + d) - mid == d`.
    let delta = p3(to.x - mid.x, to.y - mid.y, to.z - mid.z);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(LINE, GripKind::Midpoint, 2, to),
    )
    .unwrap();

    // One entity update, in one transaction, with one undo entry — the whole line
    // moved, but it is still one handle drag.
    assert_eq!(
        outcome.changes.as_ref().expect("a ChangeSet").changes.len(),
        1
    );
    assert_eq!(outcome.objects, vec![ObjectId(LINE)]);
    assert_eq!(revision_of(&app), Revision(revision.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    // Both endpoints moved by the *same* delta, so the line stays parallel and
    // keeps its length.
    let (new_start, new_end) = endpoints_of(&app, LINE);
    assert_eq!(
        new_start,
        p3(start.x + delta.x, start.y + delta.y, start.z + delta.z)
    );
    assert_eq!(
        new_end,
        p3(end.x + delta.x, end.y + delta.y, end.z + delta.z)
    );
    // Same direction and length. The difference of two translated points is
    // not bit-exact (cancellation), so this is the one tolerance in the test.
    for (axis, (before, after)) in [
        ("x", (end.x - start.x, new_end.x - new_start.x)),
        ("y", (end.y - start.y, new_end.y - new_start.y)),
        ("z", (end.z - start.z, new_end.z - new_start.z)),
    ] {
        assert!(
            (after - before).abs() < 1e-12,
            "{axis} direction must be preserved: {after} vs {before}"
        );
        assert!(
            before.abs() > 1e-3,
            "the fixture is not degenerate on {axis}"
        );
    }

    // Undo restores both endpoints exactly (same `f64` bits).
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let (start_after, end_after) = endpoints_of(&app, LINE);
    assert_eq!(start_after, start);
    assert_eq!(end_after, end);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn circle_quadrant_grip_drag_sets_the_radius_in_one_undo_step() {
    let (mut app, mut session) = grip_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(CIRCLE, GripKind::Quadrant, 0, p3(8.0, 0.0, 0.0)),
    )
    .unwrap();
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 1);
    assert_eq!(undo_depth(&app), 1);
    match geometry_of(&app, CIRCLE) {
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            assert_eq!(center, p3(0.0, 0.0, 0.0), "the centre stays put");
            assert_eq!(radius, 8.0, "the quadrant drag sets the radius");
            assert_eq!(normal, p3(0.0, 0.0, 1.0), "the normal stays put");
        }
        other => panic!("expected a CIRCLE, got {other:?}"),
    }

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    match geometry_of(&app, CIRCLE) {
        SemanticGeometry::Circle { radius, .. } => {
            assert_eq!(radius, 5.0, "undo restores the stored radius");
        }
        other => panic!("expected a CIRCLE, got {other:?}"),
    }
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_quadrant_drag_that_would_zero_the_radius_is_refused_without_a_write() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(CIRCLE, GripKind::Quadrant, 0, p3(0.0, 0.0, 0.0)),
    )
    .unwrap_err();
    refused(error, crate::grips::REASON_DEGENERATE_RADIUS, false);
    assert_untouched(&app, revision, 0);
    match geometry_of(&app, CIRCLE) {
        SemanticGeometry::Circle { radius, .. } => assert_eq!(radius, 5.0),
        other => panic!("expected a CIRCLE, got {other:?}"),
    }
}

#[test]
fn point_grip_drag_moves_the_position_in_one_undo_step() {
    let (mut app, mut session) = grip_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(POINT, GripKind::Point, 0, p3(-1.5, 2.25, -3.0)),
    )
    .unwrap();
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 1);
    assert_eq!(outcome.objects, vec![ObjectId(POINT)]);
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(position_of(&app, POINT), p3(-1.5, 2.25, -3.0));

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(position_of(&app, POINT), POINT_AT);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn zero_displacement_is_refused_with_no_revision_change_and_no_undo_entry() {
    // Dragging a handle onto its own stored position is an error, not a silent
    // no-op: accepting it would stage an unchanged entity and leave an empty undo
    // entry behind.
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(LINE, GripKind::Endpoint, 0, LINE_START),
    )
    .unwrap_err();
    refused(error, REASON_ZERO_DISPLACEMENT, false);
    assert_untouched(&app, revision, 0);
    assert_eq!(endpoints_of(&app, LINE), (LINE_START, LINE_END));

    // The same for a POINT, in all three coordinates (a Z-only equality counts).
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(POINT, GripKind::Point, 0, POINT_AT),
    )
    .unwrap_err();
    refused(error, REASON_ZERO_DISPLACEMENT, false);
    assert_untouched(&app, revision, 0);
    assert_eq!(position_of(&app, POINT), POINT_AT);
}

#[test]
fn stale_or_deleted_handles_are_refused_and_write_nothing() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);

    // Index outside the LINE's handle range (0 = start, 1 = end).
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(LINE, GripKind::Endpoint, 2, p3(1.0, 1.0, 1.0)),
    )
    .unwrap_err();
    refused(error, REASON_STALE_HANDLE, false);
    assert_untouched(&app, revision, 0);

    // The right entity with the wrong kind of handle.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(LINE, GripKind::Point, 0, p3(1.0, 1.0, 1.0)),
    )
    .unwrap_err();
    refused(error, REASON_STALE_HANDLE, false);
    assert_untouched(&app, revision, 0);

    // An id naming an entity that does not exist (never picked, or deleted since
    // the pick) is named as missing, not silently dropped.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(999, GripKind::Endpoint, 0, p3(1.0, 1.0, 1.0)),
    )
    .unwrap_err();
    refused(error, REASON_TARGET_MISSING, false);
    assert_untouched(&app, revision, 0);
    assert_eq!(endpoints_of(&app, LINE), (LINE_START, LINE_END));
}

#[test]
fn a_geometry_without_a_grip_model_is_refused_as_an_unsupported_kind() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    // An ELLIPSE exposes no grip at all (its axis/quadrant handles are NOT
    // IMPLEMENTED), so any id naming one is the kind refusal — even though the
    // kind/index would also be "stale" against it.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(ELLIPSE, GripKind::Endpoint, 0, p3(1.0, 0.0, 0.0)),
    )
    .unwrap_err();
    refused(error, REASON_UNSUPPORTED_KIND, true);
    assert_untouched(&app, revision, 0);

    // A CIRCLE *does* have handles, so the wrong kind for it is a stale handle,
    // not an unsupported kind.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(CIRCLE, GripKind::Endpoint, 0, p3(1.0, 0.0, 0.0)),
    )
    .unwrap_err();
    refused(error, REASON_STALE_HANDLE, false);
    assert_untouched(&app, revision, 0);
}

#[test]
fn a_non_finite_target_is_refused_before_any_write() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    for to in [
        p3(f64::NAN, 0.0, 0.0),
        p3(0.0, f64::INFINITY, 0.0),
        p3(0.0, 0.0, f64::NEG_INFINITY),
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::GripStretch,
            drag(LINE, GripKind::Endpoint, 0, to),
        )
        .unwrap_err();
        refused(error, REASON_NON_FINITE_TARGET, false);
        assert_untouched(&app, revision, 0);
    }
    assert_eq!(endpoints_of(&app, LINE), (LINE_START, LINE_END));
}

#[test]
fn a_non_model_space_target_reuses_the_addressability_refusal() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    // A `GripId` carries no instance path and no sub-element, so the reachable
    // "not directly addressable" case is an entity outside model space (here a
    // paper-space line, the same shape a block-definition member has).
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(PAPER_LINE, GripKind::Endpoint, 0, p3(2.0, 0.0, 0.0)),
    )
    .unwrap_err();
    refused(error, "property.not_addressable", true);
    assert_untouched(&app, revision, 0);
    assert_eq!(
        endpoints_of(&app, PAPER_LINE),
        (p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0))
    );
}

#[test]
fn a_bare_payload_is_refused_instead_of_fabricating_a_drag() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    // The typed `GRIPSTRETCH` selector carries no handle; the command must say so
    // rather than invent one.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("GripStretch"),
        "the refusal names the required payload: {error:?}"
    );
    assert_untouched(&app, revision, 0);
}
