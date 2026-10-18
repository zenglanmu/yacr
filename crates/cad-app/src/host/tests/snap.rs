use super::*;

#[test]
fn snap_candidates_near_finds_endpoint_and_midpoint_of_the_demo_line() {
    let drawing = demo_database();
    let document = DocumentId(1);

    // The demo's first entity is a line (0,0)-(4000,0); a cursor 5 world
    // units from its end resolves an endpoint hint at (0,0).
    let near_end =
        snap_candidates_near(&drawing, document, p(0.0, 5.0), 1.0, SpaceId::Model).unwrap();
    let endpoint = near_end
        .iter()
        .find(|c| c.kind == cad_measure::SnapKind::Endpoint && c.point == p(0.0, 0.0))
        .expect("the line end is an endpoint hint");
    assert!((endpoint.logical_pixel_distance - 5.0).abs() < 1e-9);
    assert_eq!(endpoint.space, SpaceId::Model);

    // Near the segment middle the midpoint wins.
    let near_mid =
        snap_candidates_near(&drawing, document, p(2000.0, 5.0), 1.0, SpaceId::Model).unwrap();
    assert!(
        near_mid
            .iter()
            .any(|c| { c.kind == cad_measure::SnapKind::Midpoint && c.point == p(2000.0, 0.0) }),
        "{near_mid:?}"
    );
}

#[test]
fn snap_candidates_near_returns_nothing_far_from_geometry() {
    let drawing = demo_database();
    let far = snap_candidates_near(
        &drawing,
        DocumentId(1),
        p(1_000_000.0, 1_000_000.0),
        1.0,
        SpaceId::Model,
    )
    .unwrap();
    assert!(far.is_empty(), "{far:?}");
}

#[test]
fn snap_candidates_near_rejects_a_bad_cursor_or_tolerance() {
    let drawing = demo_database();
    let nan_cursor = Point3 {
        x: f64::NAN,
        y: 0.0,
        z: 0.0,
    };
    assert!(matches!(
        snap_candidates_near(&drawing, DocumentId(1), nan_cursor, 1.0, SpaceId::Model),
        Err(CadError::InvalidInput(_))
    ));
    assert!(matches!(
        snap_candidates_near(&drawing, DocumentId(1), p(0.0, 0.0), 0.0, SpaceId::Model),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn snap_candidates_near_only_offers_model_space_targets() {
    // Only model geometry is expanded, so a paper-space request honestly
    // yields no hints rather than snapping model geometry into a layout.
    let drawing = demo_database();
    let paper = snap_candidates_near(
        &drawing,
        DocumentId(1),
        p(0.0, 5.0),
        1.0,
        SpaceId::Paper(cad_domain::LayoutId(0)),
    )
    .unwrap();
    assert!(paper.is_empty(), "{paper:?}");
}

#[test]
fn controller_snap_hints_are_empty_without_a_live_cursor() {
    let controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    // No tool, no cursor: the explicit empty overlay, never a stale hint.
    assert!(controller.snap_hints_near_cursor().unwrap().is_empty());
}

#[test]
fn controller_snap_hints_follow_the_measurement_cursor() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller
        .execute(Command {
            schema_version: 1,
            id: crate::CommandId::Measure,
            document: controller.document_id,
            viewport: controller.viewport_id,
            payload: crate::CommandPayload::None,
        })
        .unwrap();
    // A running tool with no cursor yet still yields nothing.
    assert!(controller.snap_hints_near_cursor().unwrap().is_empty());

    // The demo line starts at (0,0); a cursor just off that end resolves a
    // real endpoint hint through the viewport's world_per_px.
    controller
        .session
        .set_measurement_cursor(Some(p(0.0, 5.0)))
        .unwrap();
    let hints = controller.snap_hints_near_cursor().unwrap();
    assert!(
        hints
            .iter()
            .any(|c| { c.kind == cad_measure::SnapKind::Endpoint && c.point == p(0.0, 0.0) }),
        "{hints:?}"
    );
}

fn controller_with_single_line() -> HostController {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Line {
                start: p(0.0, 0.0),
                end: p(100.0, 0.0),
            },
            draw_order: 0,
        })
        .unwrap();
    controller
        .application
        .workspace
        .documents
        .get_mut(&controller.document_id)
        .unwrap()
        .drawing = Arc::new(builder.finish().unwrap());
    controller
}

#[test]
fn resolve_snap_finds_the_demo_line_endpoint_and_is_empty_far_away() {
    let controller = controller_with_single_line();
    // Snapping defaults on and is independent of a running tool.
    assert!(controller.snap_enabled());
    let candidate = controller.resolve_snap(p(0.0, 5.0)).unwrap();
    let candidate = candidate.expect("endpoint near the line start");
    assert_eq!(candidate.kind, cad_measure::SnapKind::Endpoint);
    assert_eq!(candidate.point, p(0.0, 0.0));

    assert!(controller
        .resolve_snap(p(100000.0, 100000.0))
        .unwrap()
        .is_none());
}

#[test]
fn resolve_snap_is_disabled_with_the_session_flag_and_rejects_non_finite_points() {
    let mut controller = controller_with_single_line();
    assert!(controller.resolve_snap(p(0.0, 5.0)).unwrap().is_some());

    assert!(!controller.toggle_snap());
    assert!(controller.resolve_snap(p(0.0, 5.0)).unwrap().is_none());
    controller.set_snap_enabled(true);
    assert!(controller.snap_enabled());
    assert!(controller.resolve_snap(p(0.0, 5.0)).unwrap().is_some());

    let error = controller
        .resolve_snap(p(f64::NAN, 0.0))
        .expect_err("non-finite points are invalid input");
    assert!(matches!(error, cad_domain::CadError::InvalidInput(_)));
}

#[test]
fn resolve_snap_needs_a_viewport_and_a_candidate_only() {
    let mut controller = controller_with_single_line();
    controller.application.workspace.viewports.clear();
    assert!(controller.resolve_snap(p(0.0, 5.0)).unwrap().is_none());
}

#[test]
fn tool_cursor_and_capture_store_the_snapped_point() {
    let mut controller = controller_with_single_line();
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    controller.move_tool_cursor(p(0.0, 5.0)).unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().cursor,
        Some(p(0.0, 0.0))
    );
    controller.capture_tool_point(p(0.0, 5.0)).unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().points,
        vec![p(0.0, 0.0)]
    );

    // The second capture snaps to the other end of the same line.
    controller.capture_tool_point(p(100.0, 5.0)).unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().points,
        vec![p(0.0, 0.0), p(100.0, 0.0)]
    );
}

#[test]
fn draw_tool_cursor_yields_snap_hints_and_the_flag_clears_them() {
    let mut controller = controller_with_single_line();
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    controller.move_tool_cursor(p(0.0, 5.0)).unwrap();
    let hints = controller.snap_hints_near_cursor().unwrap();
    assert!(
        hints
            .iter()
            .any(|c| c.kind == cad_measure::SnapKind::Endpoint && c.point == p(0.0, 0.0)),
        "{hints:?}"
    );

    controller.set_snap_enabled(false);
    assert!(controller.snap_hints_near_cursor().unwrap().is_empty());
    controller.set_snap_enabled(true);
    controller.move_tool_cursor(p(0.0, 5.0)).unwrap();
    assert!(!controller.snap_hints_near_cursor().unwrap().is_empty());
}

#[test]
fn snap_targets_are_built_once_per_scene_and_rebuilt_after_a_change() {
    let mut controller = controller_with_single_line();
    // The cache is thread-local, so a single-threaded test run may start with a
    // list already baked for a scene of the same identity; measure deltas from
    // the state after the first query rather than from zero.
    let builds = snap_target_build_count();

    // Two queries on the unchanged scene share one baked target list.
    assert!(controller.resolve_snap(p(0.0, 5.0)).unwrap().is_some());
    let after_first = snap_target_build_count();
    assert!(
        after_first <= builds + 1,
        "the first query builds at most one target list"
    );
    assert!(controller.resolve_snap(p(100.0, 5.0)).unwrap().is_some());
    assert_eq!(
        snap_target_build_count(),
        after_first,
        "a repeated query on the same scene must reuse the baked targets"
    );

    // An edit changes the scene identity: the next query rebuilds.
    controller
        .create_line(p(0.0, 200.0), p(100.0, 200.0))
        .unwrap();
    assert!(controller.resolve_snap(p(0.0, 5.0)).unwrap().is_some());
    assert_eq!(
        snap_target_build_count(),
        after_first + 1,
        "an edit must invalidate the baked targets"
    );

    // Undo changes the scene identity again: rebuild rather than reuse.
    controller
        .application
        .execute(
            &mut controller.session,
            Command {
                schema_version: 1,
                id: crate::CommandId::Undo,
                document: controller.document_id,
                viewport: controller.viewport_id,
                payload: crate::CommandPayload::None,
            },
        )
        .unwrap();
    assert!(controller.resolve_snap(p(0.0, 5.0)).unwrap().is_some());
    assert_eq!(
        snap_target_build_count(),
        after_first + 2,
        "undo must invalidate the baked targets"
    );

    // A new (blank) document is a different scene identity.
    controller.new_blank_document().unwrap();
    assert!(controller.resolve_snap(p(0.0, 5.0)).unwrap().is_none());
    assert_eq!(
        snap_target_build_count(),
        after_first + 3,
        "a new document must invalidate the baked targets"
    );

    // Disabled snapping returns nothing and builds nothing at all.
    controller.set_snap_enabled(false);
    let disabled_builds = snap_target_build_count();
    assert!(controller.resolve_snap(p(0.0, 5.0)).unwrap().is_none());
    assert_eq!(
        snap_target_build_count(),
        disabled_builds,
        "a disabled snap must not build targets"
    );
}

#[test]
fn cached_snap_matches_the_uncached_reference_on_probe_points() {
    let controller = controller_with_single_line();
    let drawing = controller.drawing().unwrap();
    let document = controller.document_id;
    let world_per_px = controller
        .application
        .workspace
        .viewports
        .get(&controller.viewport_id)
        .unwrap()
        .world_per_px();
    for probe in [
        p(0.0, 0.0),
        p(0.0, 5.0),
        p(50.0, 5.0),
        p(100.0, 5.0),
        p(50.0, 0.0),
        p(-10.0, 2.0),
        p(1000.0, 1000.0),
    ] {
        let cached =
            snap_candidates_near(&drawing, document, probe, world_per_px, SpaceId::Model).unwrap();
        let uncached =
            snap_candidates_near_uncached(&drawing, document, probe, world_per_px, SpaceId::Model)
                .unwrap();
        assert_eq!(
            cached, uncached,
            "cache differs from the reference at {probe:?}"
        );
        assert_eq!(
            controller.resolve_snap(probe).unwrap(),
            uncached.into_iter().next(),
            "resolve_snap differs from the reference at {probe:?}"
        );
    }
}

#[test]
fn snap_hint_kind_reports_the_best_candidate_or_none_without_rescanning() {
    let mut controller = controller_with_single_line();
    // No tool, no cursor: an honest `None`, never a fabricated kind.
    assert_eq!(controller.snap_hint_kind().unwrap(), None);

    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    // A live prompt with no cursor yet still has no candidate.
    assert_eq!(controller.snap_hint_kind().unwrap(), None);

    controller.move_tool_cursor(p(0.0, 5.0)).unwrap();
    let after_move = snap_target_build_count();
    assert_eq!(controller.snap_hint_kind().unwrap(), Some("endpoint"));
    assert_eq!(
        snap_target_build_count(),
        after_move,
        "the status kind must reuse the move's cached hint resolution, not rescan"
    );

    // Far from any geometry: no candidate, so `None`.
    controller.move_tool_cursor(p(100000.0, 100000.0)).unwrap();
    assert_eq!(controller.snap_hint_kind().unwrap(), None);

    // Snapping disabled: no candidate is reported either.
    controller.set_snap_enabled(false);
    controller.move_tool_cursor(p(0.0, 5.0)).unwrap();
    assert_eq!(controller.snap_hint_kind().unwrap(), None);
}

#[test]
fn hint_overlay_reuses_the_move_query_and_agrees_with_the_committed_point() {
    let mut controller = controller_with_single_line();
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();

    controller.move_tool_cursor(p(0.0, 5.0)).unwrap();
    let after_move = snap_target_build_count();
    let hints = controller.snap_hints_near_cursor().unwrap();
    assert!(
        hints
            .iter()
            .any(|c| c.kind == cad_measure::SnapKind::Endpoint && c.point == p(0.0, 0.0)),
        "{hints:?}"
    );
    assert_eq!(
        snap_target_build_count(),
        after_move,
        "the move and the hint overlay must share one baked target list"
    );

    controller.capture_tool_point(p(0.0, 5.0)).unwrap();
    let committed = controller.session.draw_preview().unwrap().points[0];
    assert_eq!(committed, p(0.0, 0.0));
    assert!(
        hints.iter().any(|hint| hint.point == committed),
        "the committed point must be one of the hints reported for the same cursor"
    );
    assert_eq!(
        snap_target_build_count(),
        after_move,
        "the hint/capture path must not rebuild the targets"
    );
}
