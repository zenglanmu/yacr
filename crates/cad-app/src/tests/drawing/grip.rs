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
    GripKind, REASON_NON_FINITE_TARGET, REASON_STALE_HANDLE, REASON_TARGET_MISSING,
    REASON_UNSUPPORTED_KIND, REASON_ZERO_DISPLACEMENT,
};

use super::grip_fixture::*;
use crate::tests::grips::assert_bits_eq;
use cad_domain::DimensionPoints;

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
fn arc_endpoint_grip_drag_is_one_change_one_undo_step_and_undoes_exactly() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    let before = geometry_of(&app, ARC);
    let (center, normal, radius, start, sweep) = arc_parts(&app, ARC);
    // The start handle sits at angle 0; drag it to the in-plane direction of
    // angle 0.5 (off the plane in z, which the endpoint rule ignores).
    let target = p3(radius * 0.5f64.cos(), radius * 0.5f64.sin(), 42.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(ARC, GripKind::Endpoint, 0, target),
    )
    .unwrap();

    // One entity update, in one transaction, with one undo entry.
    let changes = outcome.changes.as_ref().expect("a ChangeSet is published");
    assert_eq!(changes.changes.len(), 1, "exactly one entity update");
    assert_eq!(outcome.objects, vec![ObjectId(ARC)]);
    assert_eq!(revision_of(&app), Revision(revision.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    let (new_center, new_normal, new_radius, new_start, new_sweep) = arc_parts(&app, ARC);
    assert_eq!(new_center, center, "the centre is bit-identical");
    assert_eq!(new_normal, normal, "the normal is bit-identical");
    assert_eq!(
        new_radius.to_bits(),
        radius.to_bits(),
        "the radius is bit-identical"
    );
    assert!(
        (new_start - 0.5).abs() < 1e-12,
        "the dragged start angle follows the target, got {new_start}"
    );
    assert!(
        (new_sweep - (sweep - 0.5)).abs() < 1e-12,
        "the sweep is recomputed in the same direction, got {new_sweep}"
    );
    assert!(
        ((new_start + new_sweep) - (start + sweep)).abs() < 1e-12,
        "the opposite endpoint keeps its angle"
    );

    // Undo restores the original geometry exactly (the stored struct, not a
    // re-derived approximation).
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(geometry_of(&app, ARC), before, "undo restores the arc");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn an_arc_endpoint_drag_that_collapses_the_arc_is_refused_before_any_write() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    let before = geometry_of(&app, ARC);
    // Dragging the end handle onto the start's stored position is a full turn.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(ARC, GripKind::Endpoint, 1, p3(ARC_RADIUS, 0.0, 0.0)),
    )
    .unwrap_err();
    refused(error, crate::grips::REASON_DEGENERATE_ARC, false);
    assert_untouched(&app, revision, 0);
    assert_eq!(geometry_of(&app, ARC), before);
}

#[test]
fn polyline_vertex_grip_drag_is_one_change_one_undo_step_and_undoes_exactly() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    let before = geometry_of(&app, POLYLINE);
    let (points, bulges, closed) = polyline_parts(&app, POLYLINE);
    // The target is lifted in z: a vertex drag keeps the vertex's own z.
    let target = p3(12.0, -3.0, 9.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(POLYLINE, GripKind::Vertex, 1, target),
    )
    .unwrap();

    let changes = outcome.changes.as_ref().expect("a ChangeSet is published");
    assert_eq!(changes.changes.len(), 1, "exactly one entity update");
    assert_eq!(outcome.objects, vec![ObjectId(POLYLINE)]);
    assert_eq!(revision_of(&app), Revision(revision.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    let (new_points, new_bulges, new_closed) = polyline_parts(&app, POLYLINE);
    assert_eq!(
        new_points.len(),
        points.len(),
        "no vertex is added or dropped"
    );
    assert_eq!(
        new_points[1],
        p3(target.x, target.y, points[1].z),
        "the dragged vertex takes x/y and keeps its own z"
    );
    for index in [0usize, 2] {
        assert_eq!(
            new_points[index], points[index],
            "vertex {index} is untouched"
        );
    }
    assert_eq!(new_bulges, bulges, "every stored bulge is preserved");
    assert_eq!(new_closed, closed, "the closed flag is untouched");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        geometry_of(&app, POLYLINE),
        before,
        "undo restores the polyline"
    );
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn spline_control_point_grip_drag_is_one_change_one_undo_step_and_undoes_exactly() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    let before = geometry_of(&app, SPLINE);
    let target = p3(-2.5, 9.75, 0.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(SPLINE, GripKind::ControlPoint, 2, target),
    )
    .unwrap();

    // One entity update, in one transaction, with one undo entry.
    let changes = outcome.changes.as_ref().expect("a ChangeSet is published");
    assert_eq!(changes.changes.len(), 1, "exactly one entity update");
    assert_eq!(outcome.objects, vec![ObjectId(SPLINE)]);
    assert_eq!(revision_of(&app), Revision(revision.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    // One definition point moved; `degree`/`knots`/`weights` are untouched, so the
    // stored NURBS is a different curve on the same parameterisation (no re-fit).
    match geometry_of(&app, SPLINE) {
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            assert_eq!(degree, 3, "the degree is untouched");
            assert_eq!(
                knots,
                vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                "the knot vector is bit-identical"
            );
            assert_eq!(weights, vec![1.0; 4], "the weight vector is bit-identical");
            assert_eq!(control_points.len(), 4, "no control point is added");
            assert_eq!(control_points[2], target, "the dragged control point");
            assert_eq!(
                control_points[0],
                p3(0.0, 0.0, 1.0),
                "control point 0 is untouched"
            );
        }
        other => panic!("expected a SPLINE, got {other:?}"),
    }

    // Undo restores the original definition exactly (the stored NURBS struct).
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        geometry_of(&app, SPLINE),
        before,
        "undo restores the spline"
    );
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn text_insertion_grip_drag_is_one_change_one_undo_step_and_undoes_exactly() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    let before = geometry_of(&app, TEXT);
    let (content, position, style, height, rotation, font, h_align, v_align, wrap_width) =
        text_parts(&app, TEXT);
    assert_eq!(position, TEXT_AT, "the fixture stores the documented point");

    let target = p3(position.x + 7.5, position.y - 4.25, -1.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(TEXT, GripKind::Insertion, 0, target),
    )
    .unwrap();

    // One entity update, in one transaction, with one undo entry.
    let changes = outcome.changes.as_ref().expect("a ChangeSet is published");
    assert_eq!(changes.changes.len(), 1, "exactly one entity update");
    assert_eq!(outcome.objects, vec![ObjectId(TEXT)]);
    assert_eq!(revision_of(&app), Revision(revision.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    // Only the stored insertion point moved: the run's text, style, height,
    // rotation, font, attachment and wrap boundary are bit-identical (including
    // `wrap_width: Some(..)`, which is copied, never recomputed).
    let (
        content_after,
        position_after,
        style_after,
        height_after,
        rotation_after,
        font_after,
        h_after,
        v_after,
        wrap_after,
    ) = text_parts(&app, TEXT);
    assert_eq!(
        position_after, target,
        "the insertion point takes the target"
    );
    assert_eq!(content_after, content, "the string is untouched");
    assert_eq!(style_after, style, "the style id is untouched");
    assert_eq!(height_after.to_bits(), height.to_bits(), "the height");
    assert_eq!(rotation_after.to_bits(), rotation.to_bits(), "the rotation");
    assert_eq!(font_after, font, "the font key");
    assert_eq!(h_after, h_align, "the horizontal attachment");
    assert_eq!(v_after, v_align, "the vertical attachment");
    assert_eq!(
        wrap_after.map(f64::to_bits),
        wrap_width.map(f64::to_bits),
        "the wrap boundary (Some here, so it must survive)"
    );

    // Undo restores the stored geometry exactly (same `f64` bits, not a
    // re-derived approximation).
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(geometry_of(&app, TEXT), before, "undo restores the text");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn insert_insertion_grip_drag_is_one_change_one_undo_step_and_undoes_exactly() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    let before = geometry_of(&app, INSERT);
    let (block, transform) = insert_parts(&app, INSERT);
    let origin = transform.apply_point(p3(0.0, 0.0, 0.0));
    assert_eq!(origin, INSERT_AT, "the fixture stores the documented point");

    let target = p3(origin.x + 7.5, origin.y - 4.25, origin.z + 2.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(INSERT, GripKind::Insertion, 0, target),
    )
    .unwrap();

    // One entity update, in one transaction, with one undo entry.
    let changes = outcome.changes.as_ref().expect("a ChangeSet is published");
    assert_eq!(changes.changes.len(), 1, "exactly one entity update");
    assert_eq!(outcome.objects, vec![ObjectId(INSERT)]);
    assert_eq!(revision_of(&app), Revision(revision.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    // Only the translation column moved: the whole 3x3 linear part (rotation and
    // the non-uniform scale), row 3 and the block reference are bit-identical, and
    // the insertion point is the target exactly.
    let (block_after, transform_after) = insert_parts(&app, INSERT);
    assert_eq!(block_after, block, "the block reference is untouched");
    assert_eq!(
        linear_bits(&transform_after),
        linear_bits(&transform),
        "the rotation/scale part is bit-identical"
    );
    assert_eq!(
        transform_after.matrix[3], transform.matrix[3],
        "row 3 of the placement is untouched"
    );
    assert_eq!(
        transform_after.apply_point(p3(0.0, 0.0, 0.0)),
        target,
        "the insertion point is the drag target"
    );

    // Undo restores the stored geometry exactly (same `f64` bits, not a
    // re-derived approximation).
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        geometry_of(&app, INSERT),
        before,
        "undo restores the insert"
    );
    assert_eq!(undo_depth(&app), 0);
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
fn ellipse_quadrant_grip_drag_is_one_change_one_undo_step_and_undoes_exactly() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    let before = geometry_of(&app, ELLIPSE);
    let (center, normal, _, ratio, start, sweep) = ellipse_parts(&app, ELLIPSE);
    // Drag the +major handle to an off-axis point: the major radius becomes the
    // in-plane distance (5, not the x offset 5 alone ... it is the same here, so
    // the target is deliberately off both axes: (3, 4) -> distance 5, x 3).
    let target = p3(center.x + 3.0, center.y + 4.0, center.z + 9.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(ELLIPSE, GripKind::Quadrant, 0, target),
    )
    .unwrap();

    // One entity update, in one transaction, with one undo entry.
    let changes = outcome.changes.as_ref().expect("a ChangeSet is published");
    assert_eq!(changes.changes.len(), 1, "exactly one entity update");
    assert_eq!(outcome.objects, vec![ObjectId(ELLIPSE)]);
    assert_eq!(revision_of(&app), Revision(revision.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    let (new_center, new_normal, new_major, new_ratio, new_start, new_sweep) =
        ellipse_parts(&app, ELLIPSE);
    assert_eq!(new_center, center, "the centre is bit-identical");
    assert_eq!(new_normal, normal, "the normal is bit-identical");
    assert_eq!(new_start, start, "the start parameter is bit-identical");
    assert_eq!(new_sweep, sweep, "the sweep is bit-identical");
    assert!(
        (length_of(new_major) - 5.0).abs() < 1e-12,
        "the major radius is the in-plane distance, got {new_major:?}"
    );
    assert!(
        new_major.y.abs() < 1e-12 && new_major.z.abs() < 1e-12,
        "the major axis direction never rotates, got {new_major:?}"
    );
    // The stored minor radius (|major| * ratio = 2) is preserved, so the ratio is
    // recomputed to 2 / 5.
    assert!(
        (new_ratio - 2.0 / 5.0).abs() < 1e-12,
        "the minor radius is preserved: ratio {new_ratio} vs {}",
        2.0 / 5.0
    );
    assert_ne!(new_ratio, ratio, "the fixture really changes the ratio");

    // Undo restores the original geometry exactly (bit-identical struct).
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        geometry_of(&app, ELLIPSE),
        before,
        "undo restores the ellipse"
    );
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_geometry_without_a_grip_model_is_refused_as_an_unsupported_kind() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    // A MULTILEADER exposes no grip at all (its annotation text is not addressed
    // through a text insertion-point handle here), so any id naming one is the kind
    // refusal — even though the kind/index would also be "stale" against it.
    // (`SPLINE` used to be this fixture, then `INSERT`; both now have handles.)
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(MULTILEADER, GripKind::Endpoint, 0, p3(1.0, 0.0, 0.0)),
    )
    .unwrap_err();
    refused(error, REASON_UNSUPPORTED_KIND, true);
    assert_untouched(&app, revision, 0);

    // An ELLIPSE *does* have handles, so the wrong kind for it is a stale handle,
    // not an unsupported kind.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(ELLIPSE, GripKind::Endpoint, 0, p3(1.0, 0.0, 0.0)),
    )
    .unwrap_err();
    refused(error, REASON_STALE_HANDLE, false);
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

#[test]
fn dimension_definition_point_drag_re_synthesizes_in_one_undo_step_and_undoes_exactly() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    let before_definition = dimension_definition_of(&app, DIMENSION);
    let before_measurement = dimension_measurement_of(&app, DIMENSION);
    let before_display = dimension_display_of(&app, DIMENSION);
    // `Linear { rotation: 0.0 }` measures along +X: `|second.x - first.x|`.
    assert_eq!(before_measurement, 11.75);

    // Drag `second` (index 1) inward, in one gesture.
    let to = p3(2.5, 3.5, 0.25);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(DIMENSION, GripKind::DefinitionPoint, 1, to),
    )
    .unwrap();

    // One entity update, in one transaction, with one undo entry — a drag that
    // re-synthesizes the whole display is still one handle drag.
    let changes = outcome.changes.as_ref().expect("a ChangeSet is published");
    assert_eq!(changes.changes.len(), 1, "exactly one entity update");
    assert_eq!(outcome.objects, vec![ObjectId(DIMENSION)]);
    assert_eq!(revision_of(&app), Revision(revision.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    // The dragged point took the target; every other stored field is untouched.
    let after_definition = dimension_definition_of(&app, DIMENSION);
    match (&before_definition.points, &after_definition.points) {
        (
            DimensionPoints::Linear {
                first: before_first,
                dimension_line: before_line,
                ..
            },
            DimensionPoints::Linear {
                first: after_first,
                second: after_second,
                dimension_line: after_line,
            },
        ) => {
            assert_bits_eq(*after_second, to, "the dragged definition point");
            assert_bits_eq(*after_first, *before_first, "the untoched first point");
            assert_bits_eq(*after_line, *before_line, "the dimension line");
        }
        other => panic!("expected linear points, got {other:?}"),
    }
    assert_eq!(after_definition.kind, before_definition.kind);
    assert_eq!(
        after_definition.text_rotation.to_bits(),
        DIMENSION_TEXT_ROTATION.to_bits(),
        "group 53 is not a handle"
    );
    assert_eq!(after_definition.normal, before_definition.normal);
    assert_bits_eq(
        after_definition.text_middle,
        before_definition.text_middle,
        "the text middle point",
    );
    let after = dimension_geometry_of(&app, DIMENSION);
    assert_eq!(after.style_name(), DIMENSION_STYLE_NAME);
    assert_eq!(after.style(), &dimension_style());
    assert_eq!(after.text_override(), None);

    // The display is our **re-synthesis** of the mutated definition, with the
    // recomputed measurement: the dimension line shrank from 11.75 to 2.0.
    assert_eq!(after.measurement(), 2.0);
    assert_ne!(
        dimension_display_of(&app, DIMENSION),
        before_display,
        "the display follows the mutated definition"
    );
    assert!(
        !dimension_display_of(&app, DIMENSION).is_empty(),
        "the re-synthesized display is never empty"
    );

    // Undo restores the entity exactly, including the synthesized display.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let restored = dimension_definition_of(&app, DIMENSION);
    assert_eq!(
        restored.kind, before_definition.kind,
        "the subtype is restored"
    );
    assert_eq!(
        restored.text_rotation.to_bits(),
        before_definition.text_rotation.to_bits()
    );
    assert_eq!(restored.normal, before_definition.normal);
    match (&before_definition.points, &restored.points) {
        (
            DimensionPoints::Linear {
                first: before_first,
                second: before_second,
                dimension_line: before_line,
            },
            DimensionPoints::Linear {
                first,
                second,
                dimension_line,
            },
        ) => {
            assert_bits_eq(*first, *before_first, "restored first");
            assert_bits_eq(*second, *before_second, "restored second");
            assert_bits_eq(*dimension_line, *before_line, "restored dimension line");
        }
        other => panic!("expected linear points, got {other:?}"),
    }
    assert_bits_eq(
        restored.text_middle,
        before_definition.text_middle,
        "restored text middle point",
    );
    let restored_geometry = dimension_geometry_of(&app, DIMENSION);
    assert_eq!(restored_geometry.measurement(), before_measurement);
    assert_eq!(
        restored_geometry.display(),
        before_display.as_slice(),
        "undo restores the exact synthesized display, not a re-derivation"
    );
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_dimension_drag_onto_its_own_point_records_nothing() {
    let (mut app, mut session) = grip_app();
    let revision = revision_of(&app);
    // `DefinitionPoint` 0 is `first`; dragging it onto itself is a zero
    // displacement, refused before the transaction (no empty undo entry).
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GripStretch,
        drag(
            DIMENSION,
            GripKind::DefinitionPoint,
            0,
            p3(0.5, -1.75, 0.25),
        ),
    )
    .unwrap_err();
    refused(error, REASON_ZERO_DISPLACEMENT, false);
    assert_untouched(&app, revision, 0);
    assert_eq!(dimension_measurement_of(&app, DIMENSION), 11.75);
}
