//! `IMAGECLIP` polygonal clip contract tests.
//!
//! `ImageClip::vertices` is a pixel-space polygon, and the representation layer
//! already maps an N-gon (three or more vertices) to the visible region, so the
//! polygonal mode is a real stored N-gon — not a second clip model. The typed
//! `IMAGECLIP POLY` form supplies **world-space** points; the command checks
//! coplanarity, distinctness and self-intersection, then maps each point into
//! pixel space with the image's own `u`/`v` basis.
//!
//! The rectangle form stays exactly as it was (`tests::drawing::image`).

use super::image::{attach_default, image_app};
use super::*;
use crate::command_payload::ImageClipCommand;

fn polygon_payload(points: Vec<Point3>) -> CommandPayload {
    CommandPayload::ImageClip(Box::new(ImageClipCommand::Polygon(points)))
}

fn clip_payload() -> CommandPayload {
    CommandPayload::ImageClip(Box::new(ImageClipCommand::Off))
}

/// A world point at pixel `(px, py)` of an image at `origin` with a uniform
/// `units_per_pixel` basis (the `IMAGEATTACH` form the fixture builds).
fn pixel_point(px: f64, py: f64) -> Point3 {
    p3(10.0 + px * 0.05, 20.0 + py * 0.05, 0.0)
}

fn stored_clip(app: &Application, id: EntityId) -> Option<cad_domain::ImageClip> {
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(id)
        .expect("the image")
        .geometry
    {
        SemanticGeometry::Image { clip, .. } => clip.clone(),
        other => panic!("expected an IMAGE, got {other:?}"),
    }
}

fn select(session: &mut SessionState, id: EntityId) {
    session.selection = SelectionSet::new();
    session.selection.insert(SelectionRef {
        document: DocumentId(1),
        entity: id,
        instance: InstancePath::default(),
        sub_element: None,
    });
}

#[test]
fn imageclip_polygon_is_stored_as_its_exact_pixel_space_ngon() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let id = attach_default(&mut app, &mut session, "logo.png", [640.0, 480.0], 0.05);
    select(&mut session, id);
    let before = revision(&app);
    let undo_depth = app.history[&DocumentId(1)].undo_depth();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageClip),
        polygon_payload(vec![
            pixel_point(0.0, 0.0),
            pixel_point(320.0, 0.0),
            pixel_point(320.0, 240.0),
            pixel_point(0.0, 240.0),
        ]),
    )
    .expect("the polygon clip is stored");
    let changes = outcome.changes.expect("change set");
    assert_eq!(changes.after.0, before.0 + 1, "one revision bump");
    assert_eq!(changes.changes.len(), 1, "one transaction");
    let clip = stored_clip(&app, id).expect("a stored clip");
    assert!(clip.inside);
    assert_eq!(clip.vertices.len(), 4);
    for (actual, expected) in
        clip.vertices
            .iter()
            .zip([[0.0, 0.0], [320.0, 0.0], [320.0, 240.0], [0.0, 240.0]])
    {
        assert!((actual[0] - expected[0]).abs() <= 1e-9, "{actual:?}");
        assert!((actual[1] - expected[1]).abs() <= 1e-9, "{actual:?}");
    }
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), undo_depth + 1);
}

#[test]
fn imageclip_triangle_clip_and_off_round_trip() {
    let (mut app, mut session) = image_app(AppMode::Work);
    let id = attach_default(&mut app, &mut session, "logo.png", [640.0, 480.0], 0.05);
    select(&mut session, id);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageClip),
        polygon_payload(vec![
            pixel_point(0.0, 0.0),
            pixel_point(640.0, 0.0),
            pixel_point(0.0, 480.0),
        ]),
    )
    .expect("triangle clip");
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(stored_clip(&app, id).unwrap().vertices.len(), 3);

    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ImageClip),
        clip_payload(),
    )
    .expect("OFF");
    assert_eq!(stored_clip(&app, id), None, "OFF clears the clip");
}

#[test]
fn imageclip_polygon_refusals_write_nothing() {
    let expect_refusal =
        |app: &mut Application, session: &mut SessionState, payload: CommandPayload, code: &str| {
            let before = revision(app);
            let undo_depth = app.history[&DocumentId(1)].undo_depth();
            let error = execute(
                app,
                session,
                CommandId::Extended(ExtendedCommandId::ImageClip),
                payload,
            )
            .expect_err("refused");
            assert!(error.to_string().contains(code), "{error}");
            assert_eq!(revision(app), before, "{code} writes nothing");
            assert_eq!(
                app.history[&DocumentId(1)].undo_depth(),
                undo_depth,
                "{code} records no history"
            );
        };

    let (mut app, mut session) = image_app(AppMode::Work);
    let id = attach_default(&mut app, &mut session, "logo.png", [640.0, 480.0], 0.05);
    select(&mut session, id);

    // Fewer than three distinct points (two points, and a repeated point).
    expect_refusal(
        &mut app,
        &mut session,
        polygon_payload(vec![pixel_point(0.0, 0.0), pixel_point(10.0, 10.0)]),
        "imageclip.polygon_points",
    );
    expect_refusal(
        &mut app,
        &mut session,
        polygon_payload(vec![
            pixel_point(0.0, 0.0),
            pixel_point(0.0, 0.0),
            pixel_point(10.0, 0.0),
        ]),
        "imageclip.polygon_points",
    );

    // One point off the image plane (the image lies on z = 0).
    expect_refusal(
        &mut app,
        &mut session,
        polygon_payload(vec![
            pixel_point(0.0, 0.0),
            pixel_point(10.0, 0.0),
            p3(5.0, 5.0, 1.0),
        ]),
        "imageclip.polygon_non_coplanar",
    );

    // A bow-tie ring crosses itself.
    expect_refusal(
        &mut app,
        &mut session,
        polygon_payload(vec![
            pixel_point(0.0, 0.0),
            pixel_point(320.0, 240.0),
            pixel_point(320.0, 0.0),
            pixel_point(0.0, 240.0),
        ]),
        "imageclip.polygon_self_intersection",
    );

    // A non-finite point is not a coordinate.
    expect_refusal(
        &mut app,
        &mut session,
        polygon_payload(vec![
            pixel_point(0.0, 0.0),
            pixel_point(10.0, 0.0),
            p3(f64::NAN, 0.0, 0.0),
        ]),
        "imageclip.polygon_points",
    );

    // No selection.
    session.selection = SelectionSet::new();
    expect_refusal(
        &mut app,
        &mut session,
        polygon_payload(vec![
            pixel_point(0.0, 0.0),
            pixel_point(10.0, 0.0),
            pixel_point(0.0, 10.0),
        ]),
        "imageclip.no_selection",
    );
}
