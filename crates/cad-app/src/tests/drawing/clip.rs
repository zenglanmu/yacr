//! `CLIP` / `CLIPIT` contract tests: the two boundary commands over the existing
//! image/block clip models.
//!
//! The commands do not own a clip model: `CLIP`/`CLIPIT` resolve the target from
//! the selection, build the boundary in the space that target stores it in and
//! write through `IMAGECLIP`'s / `XCLIP`'s own validated path. So every assertion
//! reads the clip back where the specific command stores it — the image's
//! `SemanticGeometry::Image` `clip` field, or the INSERT's display attribute
//! (`entity_render_attributes(id).clip`) — never from a third place.
//!
//! Each write is one transaction / one undo step; a refusal writes nothing, bumps
//! no revision and records no history. `CLIPIT` additionally proves the boundary
//! is built from the **other** selected entities (`LINE` / `ARC` / polyline), that
//! arcs are chord-approximated, and that an open / degenerate / self-intersecting
//! ring is refused by name before any write.

use super::*;

use crate::command_payload::ClipCommand;
use cad_domain::{BlockClip, ImageAdjust, ImageClip};

/// The image's pixel-space clip, read back from the stored entity.
fn stored_image_clip(app: &Application, entity: u128) -> Option<ImageClip> {
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(entity))
        .expect("the stored image")
        .geometry
    {
        SemanticGeometry::Image { clip, .. } => clip.clone(),
        other => panic!("entity {entity} is not an image: {other:?}"),
    }
}

/// The INSERT's display-only clip boundary.
fn stored_block_clip(app: &Application, entity: u128) -> Option<BlockClip> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_render_attributes(EntityId(entity))
        .clip
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(cad_history::History::undo_depth)
        .unwrap_or(0)
}

fn image_entity(id: u128, origin: Point3, order: i64) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Image {
            origin,
            u: p3(1.0, 0.0, 0.0),
            v: p3(0.0, 1.0, 0.0),
            pixels: [100.0, 100.0],
            file: Some("Textures/Clip.PNG".into()),
            clip: None,
            visible: true,
            adjust: ImageAdjust::default(),
        },
        order,
    )
}

fn closed_polyline(id: u128, points: Vec<Point3>, bulges: Vec<f64>, order: i64) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed: true,
            width: 0.0,
        },
        order,
    )
}

/// One model-space `LINE` edge.
fn edge(id: u128, start: Point3, end: Point3, order: i64) -> DbEntity {
    ent(id, SemanticGeometry::Line { start, end }, order)
}

/// Layer 0, block 0 (one line member 100) and the clip fixtures:
///
/// * image 10 at the origin (100 x 100 px, one world unit per pixel, XY plane);
/// * image 11 at (200, 0) — the second clippable target;
/// * INSERT 3 of block 0 at the identity, INSERT 4 translated by (5, 5), INSERT 5
///   rotated 45 degrees about Z, INSERT 6 tilted out of the XY plane and INSERT 7
///   rotated 90 degrees about Z;
/// * closed square polyline 20 (`0..10`);
/// * the same square as four `LINE`s 21–24;
/// * the CRESCENT polyline 29 (one semicircular bulge on `0..10`);
/// * bow-tie polyline 26, two-point polyline 27, open polyline 28 and the
///   non-convex chevron polyline 31;
/// * CIRCLE 30 and a PDF-underlay-shaped `Opaque` row 40.
fn clip_app() -> (Application, SessionState) {
    let corners = [
        point(0.0, 0.0),
        point(10.0, 0.0),
        point(10.0, 10.0),
        point(0.0, 10.0),
    ];
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(cad_db::BlockDefinition {
        id: cad_domain::BlockId(0),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    let mut member = edge(100, point(-5.0, 0.0), point(5.0, 0.0), 0);
    member.space = SpaceId::Block(cad_domain::BlockId(0));
    b.insert_entity(member).unwrap();
    b.insert_entity(image_entity(10, point(0.0, 0.0), 1))
        .unwrap();
    b.insert_entity(image_entity(11, point(200.0, 0.0), 2))
        .unwrap();
    b.insert_entity(ent(
        3,
        SemanticGeometry::Insert {
            block: cad_domain::BlockId(0),
            transform: Transform3::identity(),
        },
        3,
    ))
    .unwrap();
    b.insert_entity(ent(
        4,
        SemanticGeometry::Insert {
            block: cad_domain::BlockId(0),
            transform: Transform3::translation(p3(5.0, 5.0, 0.0)),
        },
        4,
    ))
    .unwrap();
    b.insert_entity(ent(
        5,
        SemanticGeometry::Insert {
            block: cad_domain::BlockId(0),
            transform: rotation_z(45.0),
        },
        16,
    ))
    .unwrap();
    b.insert_entity(ent(
        6,
        SemanticGeometry::Insert {
            block: cad_domain::BlockId(0),
            transform: tilt_out_of_plane(),
        },
        17,
    ))
    .unwrap();
    b.insert_entity(ent(
        7,
        SemanticGeometry::Insert {
            block: cad_domain::BlockId(0),
            transform: rotation_z(90.0),
        },
        18,
    ))
    .unwrap();
    b.insert_entity(closed_polyline(20, corners.to_vec(), Vec::new(), 5))
        .unwrap();
    for (index, window) in corners.windows(2).enumerate() {
        b.insert_entity(edge(
            21 + index as u128,
            window[0],
            window[1],
            6 + index as i64,
        ))
        .unwrap();
    }
    b.insert_entity(edge(24, corners[3], corners[0], 9))
        .unwrap();
    b.insert_entity(closed_polyline(
        26,
        vec![
            point(0.0, 0.0),
            point(10.0, 10.0),
            point(10.0, 0.0),
            point(0.0, 10.0),
        ],
        Vec::new(),
        10,
    ))
    .unwrap();
    b.insert_entity(closed_polyline(
        27,
        vec![point(0.0, 0.0), point(10.0, 0.0)],
        Vec::new(),
        11,
    ))
    .unwrap();
    b.insert_entity(ent(
        28,
        SemanticGeometry::Polyline {
            points: vec![point(0.0, 0.0), point(10.0, 0.0), point(10.0, 10.0)],
            bulges: Vec::new(),
            closed: false,
            width: 0.0,
        },
        12,
    ))
    .unwrap();
    b.insert_entity(closed_polyline(
        29,
        corners.to_vec(),
        vec![1.0, 0.0, 0.0, 0.0],
        13,
    ))
    .unwrap();
    // A simple (non-self-intersecting) but **non-convex** ring: the fan
    // triangulation the image renderer uses cannot fill it.
    b.insert_entity(closed_polyline(
        31,
        vec![
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(10.0, 10.0),
            point(5.0, 5.0),
            point(0.0, 10.0),
        ],
        Vec::new(),
        19,
    ))
    .unwrap();
    b.insert_entity(ent(
        30,
        SemanticGeometry::Circle {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 4.0,
        },
        14,
    ))
    .unwrap();
    let mut underlay = ent(
        40,
        SemanticGeometry::Opaque {
            type_key: "AcDbPdfReference".into(),
            version: 1,
            payload: Vec::new(),
        },
        15,
    );
    underlay.object.type_key = "AcDbPdfReference".into();
    b.insert_entity(underlay).unwrap();

    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([21u8; 32]),
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
    let session = SessionState::new(DocumentId(1), AppMode::Work);
    (app, session)
}

/// Rotate about world Z by `degrees` (the block plane stays parallel to XY).
fn rotation_z(degrees: f64) -> Transform3 {
    let theta = degrees.to_radians();
    let mut m = Transform3::identity().matrix;
    m[0][0] = theta.cos();
    m[0][1] = -theta.sin();
    m[1][0] = theta.sin();
    m[1][1] = theta.cos();
    Transform3 { matrix: m }
}

/// Rotate the block plane out of world XY (the block's Y axis maps to world +Z).
fn tilt_out_of_plane() -> Transform3 {
    let mut m = Transform3::identity().matrix;
    m[1][1] = 0.0;
    m[2][1] = 1.0;
    m[1][2] = -1.0;
    m[2][2] = 0.0;
    Transform3 { matrix: m }
}

fn clip_command(payload: ClipCommand) -> CommandPayload {
    CommandPayload::Clip(Box::new(payload))
}

fn rect(minx: f64, miny: f64, maxx: f64, maxy: f64) -> CommandPayload {
    clip_command(ClipCommand::Rect([minx, miny, maxx, maxy]))
}

fn polygon(points: &[(f64, f64)]) -> CommandPayload {
    clip_command(ClipCommand::Polygon(
        points.iter().map(|(x, y)| point(*x, *y)).collect(),
    ))
}

fn select(session: &mut SessionState, entities: &[u128]) {
    session
        .selection
        .replace(entities.iter().copied().map(ref_of));
}

fn clip_id() -> CommandId {
    CommandId::Extended(ExtendedCommandId::Clip)
}

fn clipit_id() -> CommandId {
    CommandId::Extended(ExtendedCommandId::ClipIt)
}

/// The four `LINE`s of the `0..10` square, in selection order.
const SQUARE_EDGES: [u128; 4] = [21, 22, 23, 24];

#[test]
fn clip_rect_on_an_image_writes_the_pixel_clip_in_one_undo_step() {
    let (mut app, mut session) = clip_app();
    select(&mut session, &[10]);
    let base = revision(&app);
    assert_eq!(undo_depth(&app), 0);

    let outcome = execute(
        &mut app,
        &mut session,
        clip_id(),
        rect(10.0, 20.0, 60.0, 80.0),
    )
    .unwrap();
    assert_eq!(outcome.objects, vec![ObjectId(10)]);
    assert_eq!(revision(&app), Revision(base.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(
        stored_image_clip(&app, 10),
        Some(ImageClip {
            vertices: vec![[10.0, 20.0], [60.0, 80.0]],
            inside: true,
        })
    );
    // The write is reported as a CLIP step *and* by the model it wrote.
    let codes: Vec<&str> = outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert_eq!(codes, vec!["clip.applied", "imageclip.applied"]);

    // A second clip replaces the first, and one undo restores the *previous* clip
    // (not "no clip"): the undo patch carries the real before/after row.
    execute(
        &mut app,
        &mut session,
        clip_id(),
        rect(0.0, 0.0, 100.0, 50.0),
    )
    .unwrap();
    assert_eq!(
        stored_image_clip(&app, 10),
        Some(ImageClip {
            vertices: vec![[0.0, 0.0], [100.0, 50.0]],
            inside: true,
        })
    );
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        stored_image_clip(&app, 10),
        Some(ImageClip {
            vertices: vec![[10.0, 20.0], [60.0, 80.0]],
            inside: true,
        })
    );
    // Two clip writes recorded two steps; the single undo consumed the second.
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn clip_poly_on_an_image_maps_the_world_ring_into_pixels() {
    let (mut app, mut session) = clip_app();
    select(&mut session, &[10]);
    let outcome = execute(
        &mut app,
        &mut session,
        clip_id(),
        polygon(&[(0.0, 0.0), (40.0, 0.0), (40.0, 25.0), (0.0, 25.0)]),
    )
    .unwrap();
    assert_eq!(outcome.objects, vec![ObjectId(10)]);
    assert_eq!(
        stored_image_clip(&app, 10),
        Some(ImageClip {
            vertices: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 25.0], [0.0, 25.0],],
            inside: true,
        })
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "imageclip.applied"));
}

#[test]
fn clip_poly_on_an_image_refuses_a_non_convex_ring_by_name() {
    let (mut app, mut session) = clip_app();
    select(&mut session, &[10]);
    // Start from a stored rectangle so "unchanged" is a real previous state, not
    // just "still absent".
    execute(
        &mut app,
        &mut session,
        clip_id(),
        rect(0.0, 0.0, 20.0, 20.0),
    )
    .unwrap();
    let before_clip = stored_image_clip(&app, 10);
    assert!(before_clip.is_some(), "the rectangle was stored");
    let before = revision(&app);
    let depth = undo_depth(&app);

    // A simple but reflex ring (a chevron): the image renderer fan-triangulates
    // the clip polygon, so only a convex ring is drawable.
    let error = execute(
        &mut app,
        &mut session,
        clip_id(),
        polygon(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (5.0, 5.0),
            (0.0, 10.0),
        ]),
    )
    .expect_err("a non-convex image ring has no render path");
    assert!(
        matches!(&error, CadError::InvalidInput(message)
            if message.starts_with("clip.polygon_non_convex")),
        "got {error:?}"
    );
    assert!(
        error.to_string().contains("reflex"),
        "the refusal names the reason: {error}"
    );
    assert_eq!(revision(&app), before, "no revision");
    assert_eq!(undo_depth(&app), depth, "no undo entry");
    assert_eq!(
        stored_image_clip(&app, 10),
        before_clip,
        "the previous clip is unchanged"
    );
}

#[test]
fn clip_poly_on_an_image_refuses_a_self_intersecting_ring_by_name() {
    let (mut app, mut session) = clip_app();
    select(&mut session, &[10]);
    let before = revision(&app);
    let depth = undo_depth(&app);
    let error = execute(
        &mut app,
        &mut session,
        clip_id(),
        // The bow-tie ring: two non-adjacent edges cross.
        polygon(&[(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)]),
    )
    .expect_err("a crossing image ring has no render path");
    assert!(
        matches!(&error, CadError::InvalidInput(message)
            if message.starts_with("imageclip.polygon_self_intersection")),
        "got {error:?}"
    );
    assert_eq!(revision(&app), before, "no revision");
    assert_eq!(undo_depth(&app), depth, "no undo entry");
    assert_eq!(stored_image_clip(&app, 10), None, "nothing was stored");
}

#[test]
fn clip_on_a_block_reference_wires_rect_on_off_and_delete() {
    let (mut app, mut session) = clip_app();
    select(&mut session, &[3]);
    let base = revision(&app);

    // RECT is block-local (the XCLIP space), one transaction.
    execute(&mut app, &mut session, clip_id(), rect(0.0, 0.0, 4.0, 4.0)).unwrap();
    assert_eq!(revision(&app), Revision(base.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(
        stored_block_clip(&app, 3),
        Some(BlockClip::new(vec![[0.0, 0.0], [4.0, 4.0]], true, true))
    );

    // A reference translated by (5, 5) reads the same block-local rectangle.
    select(&mut session, &[4]);
    execute(
        &mut app,
        &mut session,
        clip_id(),
        rect(0.0, 0.0, 10.0, 10.0),
    )
    .unwrap();
    assert_eq!(
        stored_block_clip(&app, 4),
        Some(BlockClip::new(vec![[0.0, 0.0], [10.0, 10.0]], true, true))
    );

    // A 90-degree reference still maps the rectangle to an axis-aligned world
    // rectangle, so the rectangle form is accepted there too.
    select(&mut session, &[7]);
    execute(
        &mut app,
        &mut session,
        clip_id(),
        rect(-2.0, -2.0, 2.0, 2.0),
    )
    .unwrap();
    assert_eq!(
        stored_block_clip(&app, 7),
        Some(BlockClip::new(vec![[-2.0, -2.0], [2.0, 2.0]], true, true))
    );

    // A 45-degree reference maps it to a rotated world quad, which is a
    // renderable region: the rectangle form is stored, not refused.
    select(&mut session, &[5]);
    execute(
        &mut app,
        &mut session,
        clip_id(),
        rect(-2.0, -2.0, 2.0, 2.0),
    )
    .unwrap();
    assert_eq!(
        stored_block_clip(&app, 5),
        Some(BlockClip::new(vec![[-2.0, -2.0], [2.0, 2.0]], true, true))
    );

    // ON/OFF switch the stored boundary; DELETE removes it — each one undo step.
    select(&mut session, &[3]);
    let depth = undo_depth(&app);
    execute(
        &mut app,
        &mut session,
        clip_id(),
        clip_command(ClipCommand::Off),
    )
    .unwrap();
    assert_eq!(
        stored_block_clip(&app, 3).map(|clip| clip.visible),
        Some(false)
    );
    execute(
        &mut app,
        &mut session,
        clip_id(),
        clip_command(ClipCommand::On),
    )
    .unwrap();
    assert_eq!(
        stored_block_clip(&app, 3).map(|clip| clip.visible),
        Some(true)
    );
    execute(
        &mut app,
        &mut session,
        clip_id(),
        clip_command(ClipCommand::Delete),
    )
    .unwrap();
    assert_eq!(stored_block_clip(&app, 3), None);
    assert_eq!(undo_depth(&app), depth + 3);
}

#[test]
fn every_clip_refusal_names_its_reason_and_writes_nothing() {
    // (code, selection, payload)
    let cases: Vec<(&'static str, Vec<u128>, CommandPayload)> = vec![
        ("clip.payload", vec![10], CommandPayload::None),
        ("clip.no_selection", Vec::new(), rect(0.0, 0.0, 1.0, 1.0)),
        ("clip.multiple", vec![10, 11], rect(0.0, 0.0, 1.0, 1.0)),
        (
            // A polyline is a boundary source, never a clip target.
            "clip.not_clippable",
            vec![20],
            rect(0.0, 0.0, 1.0, 1.0),
        ),
        (
            // A PDF underlay is an `Opaque` row: no underlay clip model exists.
            "clip.not_clippable",
            vec![40],
            rect(0.0, 0.0, 1.0, 1.0),
        ),
        ("clip.not_clippable", vec![30], rect(0.0, 0.0, 1.0, 1.0)),
        (
            // An image clip has no enabled/disabled flag in this build.
            "clip.on_unsupported",
            vec![10],
            clip_command(ClipCommand::On),
        ),
        (
            "clip.off_unsupported",
            vec![10],
            clip_command(ClipCommand::Off),
        ),
        // The target-specific validation keeps its own stable codes.
        (
            "imageclip.invalid_rect",
            vec![10],
            rect(10.0, 10.0, 0.0, 0.0),
        ),
        (
            // A block polygon would be stored with no render path, so it is
            // refused before the write.
            "clip.polygon_block_not_renderable",
            vec![3],
            polygon(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0)]),
        ),
        // A rectangle that maps to a rotated/sheared world quad is *rendered* as
        // that quad, so it is accepted (see the positive test above); only a
        // boundary with no world-XY region is refused here.
        // A reference tilted out of world XY has no world rectangle at all.
        ("xclip.non_xy_plane", vec![6], rect(0.0, 0.0, 4.0, 4.0)),
        ("xclip.degenerate_rect", vec![3], rect(4.0, 4.0, 0.0, 0.0)),
    ];

    for (code, selection, payload) in cases {
        let (mut app, mut session) = clip_app();
        select(&mut session, &selection);
        let before = revision(&app);
        let depth = undo_depth(&app);
        let error = execute(&mut app, &mut session, clip_id(), payload)
            .expect_err("the refusal must not write");
        assert!(
            matches!(&error, CadError::InvalidInput(message) if message.starts_with(code)),
            "expected {code}, got {error:?}"
        );
        assert_eq!(revision(&app), before, "{code} bumps no revision");
        assert_eq!(undo_depth(&app), depth, "{code} records no undo entry");
        assert_eq!(
            stored_image_clip(&app, 10),
            None,
            "{code} writes no image clip"
        );
        for insert in [3, 4, 5, 6, 7] {
            assert_eq!(
                stored_block_clip(&app, insert),
                None,
                "{code} writes no clip on INSERT {insert}"
            );
        }
    }

    // The PDF-underlay refusal names the stored kind, not a generic "unsupported".
    let (mut app, mut session) = clip_app();
    select(&mut session, &[40]);
    let error = execute(&mut app, &mut session, clip_id(), rect(0.0, 0.0, 1.0, 1.0))
        .expect_err("an underlay has no clip model");
    assert!(
        error.to_string().contains("AcDbPdfReference"),
        "the refusal names the kind: {error}"
    );
}

#[test]
fn clipit_builds_the_boundary_from_the_other_selected_entities() {
    let (mut app, mut session) = clip_app();
    // One target (image 10) plus the four LINE edges of the 0..10 square.
    let mut selection = vec![10];
    selection.extend(SQUARE_EDGES);
    select(&mut session, &selection);
    let base = revision(&app);

    let outcome = execute(&mut app, &mut session, clipit_id(), CommandPayload::None).unwrap();
    assert_eq!(outcome.objects, vec![ObjectId(10)]);
    assert_eq!(revision(&app), Revision(base.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(
        stored_image_clip(&app, 10),
        Some(ImageClip {
            vertices: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0],],
            inside: true,
        })
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "clipit.boundary"));

    // One undo restores the previous (absent) clip.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(stored_image_clip(&app, 10), None);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn clipit_refuses_a_block_reference_target_by_name() {
    // CLIPIT always builds a polygon, and a block polygon has no render path
    // (`block_clip_rect` refuses `BlockClipRejection::Polygon`), so the target
    // kind — not the boundary — is refused, and nothing is stored.
    for selection in [
        vec![4, 20],
        vec![3].into_iter().chain(SQUARE_EDGES).collect(),
    ] {
        let (mut app, mut session) = clip_app();
        select(&mut session, &selection);
        let before = revision(&app);
        let depth = undo_depth(&app);
        let error = execute(&mut app, &mut session, clipit_id(), CommandPayload::None)
            .expect_err("a block reference has no renderable polygon clip");
        assert!(
            matches!(&error, CadError::InvalidInput(message)
                if message.starts_with("clipit.polygon_block_not_renderable")),
            "selection {selection:?}: got {error:?}"
        );
        assert_eq!(revision(&app), before, "no revision");
        assert_eq!(undo_depth(&app), depth, "no undo entry");
        assert_eq!(stored_block_clip(&app, 3), None, "no clip on INSERT 3");
        assert_eq!(stored_block_clip(&app, 4), None, "no clip on INSERT 4");
        assert_eq!(stored_image_clip(&app, 10), None, "no image clip either");
    }
}

#[test]
fn clipit_refuses_a_non_convex_image_boundary_by_name() {
    let (mut app, mut session) = clip_app();
    // Image 10 plus the simple but reflex chevron ring (polyline 31). The
    // self-intersecting half of the same rule is pinned by
    // `every_clipit_refusal_names_its_reason_and_writes_nothing`
    // (`clipit.self_intersecting`).
    select(&mut session, &[10, 31]);
    let before = revision(&app);
    let depth = undo_depth(&app);
    let error = execute(&mut app, &mut session, clipit_id(), CommandPayload::None)
        .expect_err("a non-convex image boundary has no render path");
    assert!(
        matches!(&error, CadError::InvalidInput(message)
            if message.starts_with("clipit.polygon_non_convex")),
        "got {error:?}"
    );
    assert!(
        error.to_string().contains("reflex"),
        "the refusal names the reason: {error}"
    );
    assert_eq!(revision(&app), before, "no revision");
    assert_eq!(undo_depth(&app), depth, "no undo entry");
    assert_eq!(stored_image_clip(&app, 10), None, "nothing was stored");
}

#[test]
fn clipit_chord_approximates_a_bulged_boundary_and_says_so() {
    let (mut app, mut session) = clip_app();
    // The crescent polyline 29: one semicircular bulge on the first edge.
    select(&mut session, &[10, 29]);
    let outcome = execute(&mut app, &mut session, clipit_id(), CommandPayload::None).unwrap();
    let clip = stored_image_clip(&app, 10).expect("the crescent clip");
    assert!(
        clip.vertices.len() > 4,
        "the arc must expand into chords, got {:?}",
        clip.vertices
    );
    let boundary = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "clipit.boundary")
        .expect("the boundary diagnostic");
    assert!(
        boundary.message.contains("32-chord"),
        "the approximation is declared: {}",
        boundary.message
    );
}

#[test]
fn every_clipit_refusal_names_its_reason_and_writes_nothing() {
    let cases: Vec<(&'static str, Vec<u128>)> = vec![
        // No clippable target at all.
        ("clipit.not_clippable", Vec::new()),
        ("clipit.not_clippable", SQUARE_EDGES.to_vec()),
        // An underlay alone: no clip model, named by kind.
        ("clipit.not_clippable", vec![40]),
        // Two clippable targets.
        ("clipit.multiple_targets", vec![10, 11, 21, 22, 23, 24]),
        // The target alone: no boundary.
        ("clipit.no_boundary", vec![10]),
        // A CIRCLE is not a boundary entity kind.
        ("clipit.unsupported_boundary", vec![10, 30]),
        // Two of four edges: the chain never closes.
        ("clipit.open_boundary", vec![10, 21, 22]),
        // A single open polyline.
        ("clipit.open_boundary", vec![10, 28]),
        // A closed polyline mixed with open edges.
        ("clipit.boundary_closed", vec![10, 20, 21]),
        // A bow-tie ring.
        ("clipit.self_intersecting", vec![10, 26]),
        // A two-vertex ring.
        ("clipit.degenerate_polygon", vec![10, 27]),
    ];

    for (code, selection) in cases {
        let (mut app, mut session) = clip_app();
        select(&mut session, &selection);
        let before = revision(&app);
        let depth = undo_depth(&app);
        let error = execute(&mut app, &mut session, clipit_id(), CommandPayload::None)
            .expect_err("the refusal must not write");
        assert!(
            matches!(&error, CadError::InvalidInput(message) if message.starts_with(code)),
            "selection {selection:?}: expected {code}, got {error:?}"
        );
        assert_eq!(revision(&app), before, "{code} bumps no revision");
        assert_eq!(undo_depth(&app), depth, "{code} records no undo entry");
        assert_eq!(
            stored_image_clip(&app, 10),
            None,
            "{code} writes no image clip"
        );
        assert_eq!(
            stored_block_clip(&app, 4),
            None,
            "{code} writes no block clip"
        );
    }
}

#[test]
fn clipit_refuses_a_payload_it_does_not_take() {
    let (mut app, mut session) = clip_app();
    select(&mut session, &[10, 21, 22, 23, 24]);
    let error = execute(
        &mut app,
        &mut session,
        clipit_id(),
        rect(0.0, 0.0, 1.0, 1.0),
    )
    .expect_err("CLIPIT takes no payload");
    assert!(
        error.to_string().contains("CLIPIT needs no payload"),
        "{error}"
    );
    assert_eq!(stored_image_clip(&app, 10), None);
}

#[test]
fn viewer_mode_refuses_both_commands_without_a_write() {
    for (id, selection, payload) in [
        (clip_id(), vec![10], rect(0.0, 0.0, 10.0, 10.0)),
        (clipit_id(), vec![10, 21, 22, 23, 24], CommandPayload::None),
    ] {
        let (mut app, mut session) = clip_app();
        session.switch_mode(AppMode::Viewer).unwrap();
        select(&mut session, &selection);
        let before = revision(&app);
        let error = execute(&mut app, &mut session, id, payload)
            .expect_err("both clipping commands are Work-only");
        assert!(matches!(error, CadError::PermissionDenied), "got {error:?}");
        assert_eq!(revision(&app), before);
        assert_eq!(stored_image_clip(&app, 10), None);
        assert_eq!(undo_depth(&app), 0);
    }
}
