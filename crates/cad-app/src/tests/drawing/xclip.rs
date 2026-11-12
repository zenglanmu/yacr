//! `XCLIP` contract tests: refusals, the three switch forms, the read-only
//! report, the block-local ↔ world mapping, and the one-undo-step write.
//!
//! The clip is a **display** attribute on the INSERT, so every assertion reads
//! it back from `entity_render_attributes` (never from the entity geometry). A
//! refusal must leave the revision, the undo stack and the stored clip
//! untouched; a write bumps the revision once and records exactly one undo step
//! that restores the previous display state.

use super::*;

use crate::command_payload::XClipCommand;
use cad_domain::BlockClip;

fn payload(command: XClipCommand) -> CommandPayload {
    CommandPayload::XClip(Box::new(command))
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(cad_history::History::undo_depth)
        .unwrap_or(0)
}

fn stored_clip(app: &Application, id: u128) -> Option<BlockClip> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_render_attributes(EntityId(id))
        .clip
}

fn insert_transform(app: &Application, id: u128) -> Transform3 {
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .expect("the insert")
        .geometry
    {
        SemanticGeometry::Insert { transform, .. } => *transform,
        other => panic!("expected an INSERT, got {other:?}"),
    }
}

/// Layer 0, block 0 (one line member 100 in block space), a model line (1) and
/// a model INSERT (3) of block 0 at `transform`.
fn insert_app(transform: Transform3) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(0),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    let mut member = ent(
        100,
        SemanticGeometry::Line {
            start: point(-5.0, 0.0),
            end: point(5.0, 0.0),
        },
        0,
    );
    member.space = SpaceId::Block(cad_domain::BlockId(0));
    b.insert_entity(member).unwrap();
    b.insert_entity(ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        1,
    ))
    .unwrap();
    b.insert_entity(ent(
        3,
        SemanticGeometry::Insert {
            block: cad_domain::BlockId(0),
            transform,
        },
        2,
    ))
    .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
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

fn xclip_app() -> (Application, SessionState) {
    insert_app(Transform3::identity())
}

fn rotation_z(degrees: f64) -> Transform3 {
    let theta = degrees.to_radians();
    let mut m = Transform3::identity().matrix;
    m[0][0] = theta.cos();
    m[0][1] = -theta.sin();
    m[1][0] = theta.sin();
    m[1][1] = theta.cos();
    Transform3 { matrix: m }
}

/// Rotate the block plane out of world XY (Y axis → world +Z).
fn tilt_out_of_plane() -> Transform3 {
    let mut m = Transform3::identity().matrix;
    m[1][1] = 0.0;
    m[2][1] = 1.0;
    m[1][2] = -1.0;
    m[2][2] = 0.0;
    Transform3 { matrix: m }
}

fn rect(minx: f64, miny: f64, maxx: f64, maxy: f64) -> XClipCommand {
    XClipCommand::Rect([minx, miny, maxx, maxy])
}

/// One refusal case: stable code, app factory, selection, payload.
type RefusalCase = (
    &'static str,
    fn() -> (Application, SessionState),
    Vec<u128>,
    CommandPayload,
);

#[test]
fn every_xclip_refusal_writes_nothing() {
    let cases: Vec<RefusalCase> = vec![
        ("xclip.payload", xclip_app, vec![3], CommandPayload::None),
        (
            "xclip.no_selection",
            xclip_app,
            Vec::new(),
            payload(rect(0.0, 0.0, 1.0, 1.0)),
        ),
        (
            "xclip.multiple",
            xclip_app,
            vec![3, 1],
            payload(rect(0.0, 0.0, 1.0, 1.0)),
        ),
        (
            "xclip.not_insert",
            xclip_app,
            vec![1],
            payload(rect(0.0, 0.0, 1.0, 1.0)),
        ),
        (
            // A block member has no clip of its own.
            "xclip.not_insert",
            xclip_app,
            vec![100],
            payload(rect(0.0, 0.0, 1.0, 1.0)),
        ),
        (
            "xclip.no_boundary",
            xclip_app,
            vec![3],
            payload(XClipCommand::On),
        ),
        (
            "xclip.no_boundary",
            xclip_app,
            vec![3],
            payload(XClipCommand::Off),
        ),
        (
            "xclip.degenerate_rect",
            xclip_app,
            vec![3],
            payload(rect(1.0, 1.0, 0.0, 0.0)),
        ),
        (
            "xclip.degenerate_polygon",
            xclip_app,
            vec![3],
            payload(XClipCommand::Polygon(vec![[0.0, 0.0], [1.0, 1.0]])),
        ),
        (
            "xclip.self_intersecting",
            xclip_app,
            vec![3],
            payload(XClipCommand::Polygon(vec![
                [0.0, 0.0],
                [1.0, 1.0],
                [1.0, 0.0],
                [0.0, 1.0],
            ])),
        ),
        (
            "xclip.degenerate_transform",
            || insert_app(Transform3::scale(0.0)),
            vec![3],
            payload(rect(0.0, 0.0, 1.0, 1.0)),
        ),
        (
            "xclip.non_xy_plane",
            || insert_app(tilt_out_of_plane()),
            vec![3],
            payload(rect(0.0, 0.0, 1.0, 1.0)),
        ),
        (
            "xclip.rotation_not_renderable",
            || insert_app(rotation_z(45.0)),
            vec![3],
            payload(rect(0.0, 0.0, 1.0, 1.0)),
        ),
        (
            "xclip.invert_not_renderable",
            xclip_app,
            vec![3],
            payload(XClipCommand::Invert),
        ),
    ];
    for (code, build, selection, command_payload) in cases {
        let (mut app, mut session) = build();
        session
            .selection
            .replace(selection.iter().copied().map(ref_of));
        let before = revision(&app);
        let depth = undo_depth(&app);
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::Xclip),
            command_payload,
        )
        .expect_err("the refusal must not write");
        assert!(
            matches!(&error, CadError::InvalidInput(message) if message.starts_with(code)),
            "expected {code}, got {error:?}"
        );
        assert_eq!(revision(&app), before, "{code} bumps no revision");
        assert_eq!(undo_depth(&app), depth, "{code} records no undo entry");
        assert_eq!(stored_clip(&app, 3), None, "{code} stores nothing");
    }
}

#[test]
fn viewer_mode_is_refused_without_a_write() {
    let (mut app, mut session) = xclip_app();
    session.switch_mode(AppMode::Viewer).unwrap();
    session.selection.replace([ref_of(3)]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Xclip),
        payload(rect(0.0, 0.0, 1.0, 1.0)),
    )
    .expect_err("XCLIP is Work-only");
    assert!(matches!(error, CadError::PermissionDenied), "got {error:?}");
    assert_eq!(revision(&app), before);
    assert_eq!(stored_clip(&app, 3), None);
}

#[test]
fn rect_on_off_and_delete_are_one_undo_step_each() {
    let (mut app, mut session) = xclip_app();
    session.selection.replace([ref_of(3)]);
    let base = revision(&app);
    assert_eq!(undo_depth(&app), 0);

    // RECT: one revision, one undo step, block-local corners stored verbatim.
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Xclip),
        payload(rect(0.0, 0.0, 1.0, 1.0)),
    )
    .unwrap();
    assert_eq!(outcome.objects, vec![ObjectId(3)]);
    assert_eq!(revision(&app), Revision(base.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(
        stored_clip(&app, 3),
        Some(BlockClip {
            vertices: vec![[0.0, 0.0], [1.0, 1.0]],
            inside: true,
            visible: true,
        })
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "xclip.applied"));

    // UNDO restores the previous (no) clip through the display state.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(stored_clip(&app, 3), None);
    assert_eq!(undo_depth(&app), 0);

    // Re-apply, then switch OFF: the boundary is kept, the clip stops applying.
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Xclip),
        payload(rect(0.0, 0.0, 1.0, 1.0)),
    )
    .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Xclip),
        payload(XClipCommand::Off),
    )
    .unwrap();
    assert_eq!(stored_clip(&app, 3).map(|clip| clip.visible), Some(false));

    // ON switches it back; DELETE removes it entirely.
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Xclip),
        payload(XClipCommand::On),
    )
    .unwrap();
    assert_eq!(stored_clip(&app, 3).map(|clip| clip.visible), Some(true));
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Xclip),
        payload(XClipCommand::Delete),
    )
    .unwrap();
    assert_eq!(stored_clip(&app, 3), None);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "xclip.removed"));

    // The undo popped the first step; the four later writes are all recorded.
    assert_eq!(undo_depth(&app), 4);
}

#[test]
fn an_identical_clip_is_a_no_op() {
    let (mut app, mut session) = xclip_app();
    session.selection.replace([ref_of(3)]);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Xclip),
        payload(rect(0.0, 0.0, 1.0, 1.0)),
    )
    .unwrap();
    let before = revision(&app);
    let depth = undo_depth(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Xclip),
        payload(rect(0.0, 0.0, 1.0, 1.0)),
    )
    .unwrap();
    assert!(outcome.changes.is_none());
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "xclip.unchanged"));
    assert_eq!(revision(&app), before, "a no-op bumps no revision");
    assert_eq!(undo_depth(&app), depth, "a no-op records no undo entry");
}

#[test]
fn the_report_reads_the_stored_boundary_without_writing() {
    let (mut app, mut session) = xclip_app();
    session.selection.replace([ref_of(3)]);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Xclip),
        payload(rect(0.0, 0.0, 1.0, 1.0)),
    )
    .unwrap();
    let before = revision(&app);
    let depth = undo_depth(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Xclip),
        payload(XClipCommand::Report),
    )
    .unwrap();
    assert!(outcome.changes.is_none(), "the report never writes");
    let report = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "xclip.report")
        .expect("xclip.report");
    assert!(
        report.message.contains("world rect (0, 0)..(1, 1)"),
        "{}",
        report.message
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), depth);
}

#[test]
fn a_polygon_is_stored_and_reported_not_renderable() {
    let (mut app, mut session) = xclip_app();
    session.selection.replace([ref_of(3)]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Xclip),
        payload(XClipCommand::Polygon(vec![
            [0.0, 0.0],
            [1.0, 0.0],
            [0.5, 1.0],
        ])),
    )
    .unwrap();
    assert_eq!(
        stored_clip(&app, 3).map(|clip| clip.vertices.len()),
        Some(3)
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "xclip.polygon_not_renderable"));
}

#[test]
fn block_local_corners_map_to_world_for_a_translated_and_scaled_insert() {
    let transform = Transform3::translation(point(5.0, 5.0)).matrix_mul(&Transform3::scale(2.0));
    let (mut app, mut session) = insert_app(transform);
    session.selection.replace([ref_of(3)]);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Xclip),
        payload(rect(0.0, 0.0, 1.0, 1.0)),
    )
    .unwrap();
    let stored = stored_clip(&app, 3).expect("the stored boundary");
    // Stored in block space, not world space...
    assert_eq!(stored.vertices, vec![[0.0, 0.0], [1.0, 1.0]]);
    // ...and the same authoritative mapper the renderer uses resolves it to the
    // translated+scaled world rectangle.
    let world = cad_representation::block_clip_rect(&stored, &insert_transform(&app, 3))
        .unwrap()
        .expect("an active clip");
    assert!((world.min[0] - 5.0).abs() < 1e-9);
    assert!((world.min[1] - 5.0).abs() < 1e-9);
    assert!((world.max[0] - 7.0).abs() < 1e-9);
    assert!((world.max[1] - 7.0).abs() < 1e-9);
}

#[test]
fn the_boundary_follows_a_translated_insert() {
    let (mut app, mut session) = xclip_app();
    session.selection.replace([ref_of(3)]);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Xclip),
        payload(rect(0.0, 0.0, 1.0, 1.0)),
    )
    .unwrap();
    let stored = stored_clip(&app, 3).expect("the stored boundary");
    // The stored corners are block-local, so mapping them through a moved
    // reference yields the moved world rectangle with no second write.
    let world =
        cad_representation::block_clip_rect(&stored, &Transform3::translation(point(100.0, 0.0)))
            .unwrap()
            .unwrap();
    assert!((world.min[0] - 100.0).abs() < 1e-9);
    assert!((world.min[1]).abs() < 1e-9);
    assert!((world.max[0] - 101.0).abs() < 1e-9);
    assert!((world.max[1] - 1.0).abs() < 1e-9);
}
