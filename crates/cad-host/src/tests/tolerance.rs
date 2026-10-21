use super::*;

// --- configured pickbox / aperture (Options ▸ Selection / Drafting) ----------

/// The world point at a canvas pixel in the active view.
fn world_of(core: &HostCore, pixel: [f64; 2]) -> Point3 {
    let c = core.controller.borrow();
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    viewport
        .screen_to_world(pixel, viewport.logical_size)
        .expect("the point is on the canvas")
}

#[test]
fn a_configured_pickbox_widens_the_pick_and_hover_radius_live() {
    let core = hover_core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let rim_pixel = screen_of(&core, demo_circle_rim(&core));
    // 5 logical pixels outside the rim: beyond the default 3 px pickbox, inside a
    // 6 px one.
    let cursor_pixel = [rim_pixel[0] + 5.0, rim_pixel[1]];
    assert!(
        picks_nothing(&core, cursor_pixel),
        "a pick at 5 px must miss the default 3 px pickbox"
    );

    // The setting takes effect for the very next interaction, with no reopen.
    core.apply_setting(CadSettingKey::PickboxSize, CadSettingValue::Number(6))
        .unwrap();
    assert!(
        !picks_nothing(&core, cursor_pixel),
        "a 6 px pickbox must hit the same 5 px offset"
    );
    // The hover acquires the tint through the same widened policy.
    dispatch_pointer(&core, hover_cursor(&core, cursor_pixel));
    assert!(
        !hovered_refs(&core).is_empty(),
        "the hover highlight uses the configured pickbox too"
    );
}

#[test]
fn a_configured_aperture_widens_the_snap_aperture_live() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let rim_pixel = screen_of(&core, demo_circle_rim(&core));
    // 20 logical pixels outside the rim's quadrant: outside the 12 px default
    // aperture, inside a 24 px one.
    let cursor_world = world_of(&core, [rim_pixel[0] + 20.0, rim_pixel[1]]);
    assert!(
        core.controller
            .borrow()
            .resolve_snap(cursor_world)
            .expect("snap resolution succeeds")
            .is_none(),
        "nothing may snap at 20 px through the default 12 px aperture"
    );

    core.apply_setting(CadSettingKey::ApertureSize, CadSettingValue::Number(24))
        .unwrap();
    assert!(
        core.controller
            .borrow()
            .resolve_snap(cursor_world)
            .expect("snap resolution succeeds")
            .is_some(),
        "a 24 px aperture must reach the same 20 px offset"
    );
    // The pickbox is independent: it is still the default 3 px, so the same point
    // still cannot be picked.
    assert!(
        picks_nothing(&core, [rim_pixel[0] + 20.0, rim_pixel[1]]),
        "the aperture must never widen the pickbox"
    );
}

#[test]
fn the_pickbox_and_aperture_are_not_coupled() {
    let core = core();
    core.apply_setting(CadSettingKey::PickboxSize, CadSettingValue::Number(12))
        .unwrap();
    assert_eq!(core.tolerance_policy().pickbox_logical_pixels, 12.0);
    assert_eq!(
        core.tolerance_policy().interaction_logical_pixels,
        12.0,
        "writing the pickbox leaves the default aperture alone"
    );
    core.apply_setting(CadSettingKey::ApertureSize, CadSettingValue::Number(40))
        .unwrap();
    assert_eq!(core.tolerance_policy().pickbox_logical_pixels, 12.0);
    assert_eq!(core.tolerance_policy().interaction_logical_pixels, 40.0);
}
