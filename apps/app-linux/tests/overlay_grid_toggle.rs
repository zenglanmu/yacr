#![cfg(target_os = "linux")]
//! End-to-end regression: the desktop status-bar overlay toggles must reach the
//! CAD renderer, not only the config store / button state.
//!
//! This drives the production [`LinuxApp`] (real host + shared Slint component +
//! shared wgpu/lavapipe bridge) and reads back the composited frame, exactly like
//! `verify_ui`. The comparison is restricted to the CAD canvas rectangle, so a
//! status-bar button highlight cannot mask a canvas that never changed.
//!
//! The offscreen Slint platform installs a process-global rendering notifier, so
//! only **one** `LinuxApp` may be created and shown per test process. The grid
//! and axes toggles therefore share a single app in one test rather than living
//! in two tests that would collide on the second `show()`.
use app_linux::{LinuxApp, LinuxOptions};
use slint::ComponentHandle;
use std::sync::OnceLock;

fn install_offscreen() {
    static PLATFORM: OnceLock<Result<(), String>> = OnceLock::new();
    let result = PLATFORM
        .get_or_init(|| cad_ui_slint::offscreen::install().map_err(|error| error.to_string()));
    if let Err(error) = result {
        panic!("offscreen platform install failed: {error}");
    }
}

/// Pump the shared renderer until a stable composited frame exists.
fn frame(app: &LinuxApp) -> cad_render_wgpu::headless::RgbaImage {
    let mut image = cad_ui_slint::offscreen::snapshot(app.adapter.window()).unwrap();
    for _ in 0..4 {
        std::thread::sleep(std::time::Duration::from_millis(3));
        image = cad_ui_slint::offscreen::snapshot(app.adapter.window()).unwrap();
    }
    image
}

/// Count pixels that differ inside the CAD canvas rectangle only.
fn changed_canvas_pixels(
    before: &cad_render_wgpu::headless::RgbaImage,
    after: &cad_render_wgpu::headless::RgbaImage,
    rect: [f64; 4],
    scale: f64,
) -> usize {
    if before.width != after.width || before.height != after.height {
        return 0;
    }
    let x0 = (rect[0] * scale).ceil().max(0.0) as u32;
    let y0 = (rect[1] * scale).ceil().max(0.0) as u32;
    let x1 = ((rect[0] + rect[2]) * scale)
        .floor()
        .min(before.width as f64) as u32;
    let y1 = ((rect[1] + rect[3]) * scale)
        .floor()
        .min(before.height as f64) as u32;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| before.pixel(x, y) != after.pixel(x, y))
        .count()
}

#[test]
fn status_bar_overlay_toggles_reach_the_renderer() {
    install_offscreen();
    let app = LinuxApp::new(LinuxOptions {
        headless: true,
        ..LinuxOptions::default()
    })
    .unwrap();
    app.adapter.component().show().unwrap();
    let ui = app.adapter.component();
    let scale = app.adapter.window().scale_factor() as f64;
    let (rect, _) = app.adapter.handle().shell_geometry().unwrap();

    let baseline = frame(&app);
    assert!(
        !ui.get_overlay_grid(),
        "the grid starts disabled (config default)"
    );
    assert!(
        ui.get_overlay_axes(),
        "the axes start enabled (config default)"
    );

    // Grid: off -> on must change the canvas; on -> off must restore it exactly,
    // proving the overlay is a real derived layer and not a one-way mutation.
    ui.invoke_overlay_toggled("grid".into(), true);
    assert!(
        ui.get_overlay_grid(),
        "the grid toggle must update the config-backed button state"
    );
    let grid_on = frame(&app);
    assert!(
        changed_canvas_pixels(&baseline, &grid_on, rect, scale) > 0,
        "enabling the grid must change the rendered canvas, not only the button"
    );
    ui.invoke_overlay_toggled("grid".into(), false);
    assert!(!ui.get_overlay_grid());
    let grid_off = frame(&app);
    assert_eq!(
        changed_canvas_pixels(&baseline, &grid_off, rect, scale),
        0,
        "disabling the grid must restore the exact composited canvas"
    );

    // Axes: on -> off must change the canvas (the mirror of the grid direction);
    // off -> on must restore it exactly.
    ui.invoke_overlay_toggled("axes".into(), false);
    assert!(
        !ui.get_overlay_axes(),
        "the axes toggle must update the config-backed button state"
    );
    let axes_off = frame(&app);
    assert!(
        changed_canvas_pixels(&baseline, &axes_off, rect, scale) > 0,
        "disabling the axes must change the rendered canvas, not only the button"
    );
    ui.invoke_overlay_toggled("axes".into(), true);
    assert!(ui.get_overlay_axes());
    let axes_on = frame(&app);
    assert_eq!(
        changed_canvas_pixels(&baseline, &axes_on, rect, scale),
        0,
        "enabling the axes must restore the exact composited canvas"
    );
}
