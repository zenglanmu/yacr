#![cfg(target_os = "linux")]
//! Fixed UI scenario replay for `scripts/verify-ui.sh` on the real Linux host.
//!
//! This is not a test double: it builds the production [`LinuxApp`] (real
//! `HostController` + shared Slint component + shared wgpu/lavapipe CAD bridge)
//! and drives the real Slint callbacks and application commands. Screenshots are
//! the composited Slint frame read back from the GPU, written to
//! `YACR_VERIFY_OUTPUT` when set.
//!
//! Scope (layer 2): software-Vulkan, no window server, synthetic CAD content.
//! It can never stand in for a real GPU, a real drawing, or a device.
use app_linux::{LinuxApp, LinuxOptions};
use cad_render_wgpu::headless::encode_png;
use slint::{ComponentHandle, Model as _};
use std::path::{Path, PathBuf};

fn output_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var("YACR_VERIFY_OUTPUT").ok()?);
    std::fs::create_dir_all(&dir).expect("create YACR_VERIFY_OUTPUT");
    Some(dir)
}

fn shoot(app: &LinuxApp, dir: &Option<PathBuf>, name: &str) {
    let Some(dir) = dir else {
        return;
    };
    // Pump the shared renderer until a stable composited frame exists.
    let mut frame = None;
    for _ in 0..4 {
        frame = Some(cad_ui_slint::offscreen::snapshot(app.adapter.window()).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    let frame = frame.unwrap();
    std::fs::write(dir.join(format!("{name}.png")), encode_png(&frame).unwrap()).unwrap();
}

/// Pump the shared renderer and return the composited frame.
fn frame_rgba(app: &LinuxApp) -> cad_render_wgpu::headless::RgbaImage {
    let mut image = cad_ui_slint::offscreen::snapshot(app.adapter.window()).unwrap();
    for _ in 0..3 {
        std::thread::sleep(std::time::Duration::from_millis(3));
        image = cad_ui_slint::offscreen::snapshot(app.adapter.window()).unwrap();
    }
    image
}

/// Pump the shared renderer and return the composited pixels without writing.
fn frame(app: &LinuxApp) -> Vec<u8> {
    frame_rgba(app).pixels
}

/// Mean perceptual luminance of a logical-pixel region, sampled sparsely.
fn region_luma(image: &cad_render_wgpu::headless::RgbaImage, rect: [f64; 4]) -> f64 {
    let x0 = rect[0].max(0.0) as u32;
    let y0 = rect[1].max(0.0) as u32;
    let x1 = (rect[0] + rect[2]).min(image.width as f64) as u32;
    let y1 = (rect[1] + rect[3]).min(image.height as f64) as u32;
    let mut total = 0.0;
    let mut count = 0.0;
    let mut y = y0;
    while y < y1 {
        let mut x = x0;
        while x < x1 {
            let [r, g, b, _] = image.pixel(x, y);
            total += 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64;
            count += 1.0;
            x += 2;
        }
        y += 2;
    }
    if count == 0.0 {
        0.0
    } else {
        total / count
    }
}

/// AutoCAD-style dark frame: chrome, ribbon, panels and canvas are all dark.
fn assert_autocad_dark(image: &cad_render_wgpu::headless::RgbaImage) {
    for (region, rect) in [
        ("title strip", [400.0, 4.0, 400.0, 26.0]),
        ("ribbon tabs", [260.0, 44.0, 640.0, 30.0]),
        ("side panel", [20.0, 300.0, 180.0, 260.0]),
        ("canvas", [260.0, 200.0, 140.0, 300.0]),
    ] {
        let luma = region_luma(image, rect);
        assert!(
            luma < 120.0,
            "{region} must be AutoCAD-dark, got mean luma {luma:.1}"
        );
    }
}

/// A pick pair inside the canvas, in canvas-local logical coordinates.
fn pick_pair(app: &LinuxApp) {
    let (rect, _) = app.adapter.handle().shell_geometry().unwrap();
    let cx = (rect[2] * 0.5) as f32;
    let cy = (rect[3] * 0.5) as f32;
    app.adapter.component().invoke_canvas_pick(cx - 40.0, cy);
    app.adapter.component().invoke_canvas_pick(cx + 40.0, cy);
}

fn text(key: &str) -> String {
    cad_ui_slint::MessageSource::from_request("zh-CN").text(key, &[])
}

#[test]
fn verify_ui_fixed_scenario_replay() {
    cad_ui_slint::offscreen::install().unwrap();
    let dir = output_dir();
    let workspace = PathBuf::from(format!(
        "/tmp/opencode/yacr-verify-ui-scenario-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let export = workspace.join("annotations.json");
    let app = LinuxApp::new(LinuxOptions {
        headless: true,
        annotation_export: Some(export.clone()),
        annotation_import: Some(export.clone()),
        ..LinuxOptions::default()
    })
    .unwrap();
    app.adapter.component().show().unwrap();
    let ui = app.adapter.component();
    shoot(&app, &dir, "00-desktop-initial");

    // --- AutoCAD-style dark application frame (chrome/panels/canvas) --------
    assert_autocad_dark(&frame_rgba(&app));

    // --- measurement: open-ended tool is explicitly confirmable, and cancel
    //     never writes an annotation -----------------------------------------
    let annotations_before = ui.get_annotation_rows().row_count();
    ui.invoke_measure_kind_selected(text("measure.kind.polyline").into());
    assert!(ui.get_measurement_active(), "measure start must activate");
    pick_pair(&app);
    assert!(
        ui.get_measurement_can_confirm(),
        "an open-ended polyline is confirmable after two picks"
    );
    ui.invoke_cancel_measurement_requested();
    assert!(!ui.get_measurement_active(), "cancel must clear the tool");
    assert_eq!(
        ui.get_annotation_rows().row_count(),
        annotations_before,
        "a cancelled measurement must not write an annotation"
    );
    shoot(&app, &dir, "01-measure-cancelled");

    // --- measurement: distance auto-completes after two picks, then saves ---
    ui.invoke_measure_kind_selected(text("measure.kind.distance").into());
    assert!(ui.get_measurement_active());
    pick_pair(&app);
    assert!(
        !ui.get_measurement_active(),
        "distance auto-completes once both points are captured"
    );
    assert!(
        ui.get_measurement_can_save_annotation(),
        "an evaluated measurement must enable save-as-annotation"
    );
    ui.invoke_save_measurement_requested();
    assert_eq!(
        ui.get_annotation_rows().row_count(),
        annotations_before + 1,
        "save-as-annotation must add one annotation"
    );
    shoot(&app, &dir, "02-measure-saved");

    // --- every measurement kind can start and be cancelled without a panic --
    for index in 0..ui.get_measurement_kind_labels().row_count() {
        if let Some(label) = ui.get_measurement_kind_labels().row_data(index) {
            ui.invoke_measure_kind_selected(label);
        }
        ui.invoke_cancel_measurement_requested();
    }

    // --- undo / redo must move the annotation count both ways ---------------
    assert!(ui.get_can_undo());
    ui.invoke_undo_requested();
    assert!(ui.get_can_redo());
    assert_eq!(ui.get_annotation_rows().row_count(), annotations_before);
    ui.invoke_redo_requested();
    assert_eq!(ui.get_annotation_rows().row_count(), annotations_before + 1);

    // --- draw: LINE start -> two picks -> confirm ---------------------------
    assert!(ui.get_work_mode(), "the host starts in Work mode");
    ui.invoke_begin_draw_tool("line".into());
    assert!(ui.get_draw_tool_active(), "LINE capture must start");
    pick_pair(&app);
    assert!(ui.get_draw_can_confirm(), "LINE must be confirmable");
    ui.invoke_confirm_draw_requested();
    assert!(
        !ui.get_draw_tool_active(),
        "a committed LINE must end capture"
    );
    shoot(&app, &dir, "03-line-committed");
    ui.invoke_undo_requested();

    // --- annotation: text kind, one point, text, confirm --------------------
    ui.invoke_annotation_kind_selected(text("annotation.kind.text").into());
    assert!(ui.get_annotation_tool_active(), "TEXT tool must start");
    let (rect, _) = app.adapter.handle().shell_geometry().unwrap();
    ui.invoke_canvas_pick((rect[2] * 0.5) as f32, (rect[3] * 0.5) as f32);
    assert!(
        !ui.get_annotation_tool_can_confirm(),
        "TEXT must refuse to confirm before a text payload exists"
    );
    ui.invoke_annotation_text_edited("检查批注".into());
    assert!(
        ui.get_annotation_tool_can_confirm(),
        "TEXT with text can confirm"
    );
    ui.invoke_confirm_annotation_requested();
    assert!(
        !ui.get_annotation_tool_active(),
        "committed TEXT ends capture"
    );
    assert_eq!(ui.get_annotation_rows().row_count(), annotations_before + 2);
    shoot(&app, &dir, "04-text-annotation");

    // Text needs a host font engine to draw, so add a font-independent
    // drawable annotation (rectangle auto-commits after two picks) to prove the
    // committed-annotation overlay really reacts to visibility.
    ui.invoke_annotation_kind_selected(text("annotation.kind.rectangle").into());
    assert!(ui.get_annotation_tool_active(), "RECTANGLE tool must start");
    pick_pair(&app);
    assert!(
        !ui.get_annotation_tool_active(),
        "a rectangle auto-commits once both corners are captured"
    );
    assert_eq!(ui.get_annotation_rows().row_count(), annotations_before + 3);
    shoot(&app, &dir, "04b-rectangle-annotation");

    // --- annotation visibility must change the overlay, and re-showing must
    //     restore the exact pixels; delete must remove the row ---------------
    let annotation_count = ui.get_annotation_rows().row_count();
    let mut overlay_changed = false;
    for index in 0..annotation_count {
        let visible = frame(&app);
        ui.invoke_annotation_visibility_toggled(index as i32, false);
        if frame(&app) != visible {
            overlay_changed = true;
        }
        ui.invoke_annotation_visibility_toggled(index as i32, true);
        assert_eq!(
            frame(&app),
            visible,
            "re-showing annotation {index} must restore the exact overlay"
        );
    }
    assert!(
        overlay_changed,
        "hiding at least one drawn annotation must change the overlay pixels"
    );
    ui.invoke_annotation_delete_requested((annotation_count - 1) as i32);
    assert_eq!(
        ui.get_annotation_rows().row_count(),
        annotation_count - 1,
        "deleting a row must remove exactly one annotation"
    );

    // --- layers: hiding a layer must change the composited scene, and
    //     restoring must bring the exact pixels back -------------------------
    let layer_count = ui.get_layer_rows().row_count();
    assert!(layer_count >= 1, "the demo must expose at least one layer");
    let layer_baseline = frame(&app);
    let mut layer_changed_pixels = false;
    for index in 0..layer_count {
        ui.invoke_layer_visibility_toggled(index as i32, false);
        assert!(
            ui.get_layer_override_count() >= 1,
            "toggling a layer must record an override"
        );
        if frame(&app) != layer_baseline {
            layer_changed_pixels = true;
        }
        ui.invoke_layer_visibility_toggled(index as i32, true);
    }
    assert!(
        layer_changed_pixels,
        "hiding a layer must change the rendered scene, not only the panel state"
    );
    ui.invoke_restore_layers_requested();
    assert_eq!(
        ui.get_layer_override_count(),
        0,
        "restore must clear overrides"
    );
    assert_eq!(
        frame(&app),
        layer_baseline,
        "restoring layer visibility must restore the exact composited scene"
    );

    // --- layouts: model space is always selectable --------------------------
    ui.invoke_layout_selected(-1);
    for index in 0..ui.get_layout_rows().row_count() {
        ui.invoke_layout_selected(index as i32);
    }
    ui.invoke_layout_selected(-1);

    // --- real pointer scroll and drag go through the host navigation path --
    let (canvas, _) = app.adapter.handle().shell_geometry().unwrap();
    let center = slint::LogicalPosition::new(
        (canvas[0] + canvas[2] * 0.5) as f32,
        (canvas[1] + canvas[3] * 0.5) as f32,
    );
    let before_zoom = frame(&app);
    app.adapter
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position: center,
            delta_x: 0.0,
            delta_y: -120.0,
        });
    let after_zoom = frame(&app);
    assert_ne!(before_zoom, after_zoom, "scroll must zoom the CAD scene");
    let to = slint::LogicalPosition::new(center.x + 40.0, center.y + 20.0);
    {
        let window = app.adapter.window();
        window.dispatch_event(slint::platform::WindowEvent::PointerPressed {
            position: center,
            button: slint::platform::PointerEventButton::Left,
        });
        window.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: to });
        window.dispatch_event(slint::platform::WindowEvent::PointerReleased {
            position: to,
            button: slint::platform::PointerEventButton::Left,
        });
    }
    assert_ne!(
        after_zoom,
        frame(&app),
        "a left-button drag must pan the CAD scene"
    );

    // --- view: a 2D -> 3D -> 2D toggle must round-trip losslessly ----------
    // (documented Switch2d3d invariant: the saved 2D camera is restored).
    let before_3d = frame(&app);
    let was_3d = ui.get_view_3d();
    ui.invoke_toggle_view_mode_requested();
    let entered = ui.get_view_3d() != was_3d;
    ui.invoke_toggle_view_mode_requested();
    let after_3d = frame(&app);
    if entered {
        assert_eq!(
            before_3d, after_3d,
            "2D -> 3D -> 2D must restore the exact composited camera view"
        );
    }
    // Projection toggle and every standard view are reachable without a panic.
    ui.invoke_toggle_projection_requested();
    ui.invoke_toggle_projection_requested();
    let standard_views = ui.get_standard_view_labels();
    for index in 0..standard_views.row_count() {
        if let Some(label) = standard_views.row_data(index) {
            ui.invoke_standard_view_selected(label);
        }
    }
    // Re-fit so the later layout frames show content rather than a drifted view.
    ui.invoke_fit_requested();
    shoot(&app, &dir, "05-view-traversal");

    // --- minimal preset: keep canvas/panels/navigation, drop ribbon, command
    //     bar and status bar (defined minimal semantics, not a layout subset) --
    let mut minimal = cad_app::viewer_config::ViewerConfig::default();
    minimal.ui.preset = cad_app::viewer_config::Preset::Minimal;
    app.adapter.handle().set_config(minimal).unwrap();
    assert!(
        ui.get_application_ui(),
        "minimal keeps the application frame"
    );
    assert!(!ui.get_ribbon_visible(), "minimal hides the ribbon");
    assert!(!ui.get_command_visible(), "minimal hides the command bar");
    assert!(!ui.get_status_visible(), "minimal hides the status bar");
    assert!(ui.get_layouts_visible(), "minimal keeps the layout tabs");
    assert!(ui.get_navigation_visible(), "minimal keeps navigation");
    assert!(
        ui.get_layer_panel_visible(),
        "minimal keeps the layer panel"
    );
    shoot(&app, &dir, "05b-minimal");
    app.adapter
        .handle()
        .set_config(cad_app::viewer_config::ViewerConfig::default())
        .unwrap();

    // --- config presets: canvas-only hides all application chrome -----------
    let mut canvas_only = cad_app::viewer_config::ViewerConfig::default();
    canvas_only.ui.preset = cad_app::viewer_config::Preset::CanvasOnly;
    app.adapter.handle().set_config(canvas_only).unwrap();
    assert!(!ui.get_application_ui(), "canvas-only hides application UI");
    shoot(&app, &dir, "06-canvas-only");
    app.adapter
        .handle()
        .set_config(cad_app::viewer_config::ViewerConfig::default())
        .unwrap();
    assert!(ui.get_application_ui(), "restoring full config shows UI");

    // --- locale switch re-derives catalog labels without a command ----------
    app.adapter.handle().set_locale("en").unwrap();
    assert_eq!(ui.get_open_label().to_string(), "Open drawing");
    shoot(&app, &dir, "07-locale-en");
    app.adapter.handle().set_locale("zh-CN").unwrap();
    assert_eq!(ui.get_open_label().to_string(), "打开图纸");

    // --- annotation sidecar round-trips through the host ---------------------
    ui.invoke_export_requested();
    let exported = std::fs::read_to_string(&export).expect("export writes a sidecar");
    assert!(
        serde_json::from_str::<serde_json::Value>(&exported).is_ok(),
        "exported annotation sidecar must be valid JSON"
    );
    ui.invoke_import_requested();
    assert!(
        !ui.get_status_label().contains("失败"),
        "importing the sidecar we just wrote must not fail: {}",
        ui.get_status_label()
    );

    // --- command line drives the same real operations as the ribbon ---------
    // AutoCAD convention: the command area is a first-class control surface.
    let ribbon_before = ui.get_ribbon_expanded();
    ui.invoke_command_submitted("TOOLS".into());
    assert_ne!(
        ribbon_before,
        ui.get_ribbon_expanded(),
        "TOOLS must toggle the ribbon"
    );
    ui.invoke_command_submitted("TOOLS".into());

    let panel_before = ui.get_side_panel_open();
    ui.invoke_command_submitted("PANELS".into());
    assert_ne!(
        panel_before,
        ui.get_side_panel_open(),
        "PANELS must toggle the side panel"
    );
    ui.invoke_command_submitted("PANELS".into());

    ui.invoke_command_submitted("LINE".into());
    assert!(
        ui.get_draw_tool_active(),
        "the LINE command must start a capture"
    );
    ui.invoke_command_submitted("ESC".into());
    assert!(
        !ui.get_draw_tool_active(),
        "ESC must cancel the active capture"
    );

    ui.invoke_command_submitted("CIRCLE".into());
    assert!(ui.get_draw_tool_active(), "CIRCLE must start a capture");
    ui.invoke_command_submitted("CONFIRM".into());
    assert!(
        ui.get_draw_tool_active() && ui.get_status_label().contains("参数无效"),
        "an incomplete CIRCLE must be refused explicitly and keep the capture: {}",
        ui.get_status_label()
    );
    ui.invoke_command_submitted("ESC".into());
    assert!(!ui.get_draw_tool_active());

    ui.invoke_command_submitted("MOVE".into());
    assert!(
        !ui.get_draw_tool_active(),
        "MOVE without a selection must not start"
    );
    assert!(
        ui.get_status_label().contains("选择"),
        "the MOVE refusal must be explicit: {}",
        ui.get_status_label()
    );

    ui.invoke_command_submitted("NOPE".into());
    assert!(
        ui.get_status_label().contains("NOPE"),
        "an unknown command must be reported, not silently ignored: {}",
        ui.get_status_label()
    );
    ui.invoke_command_submitted("FIT".into());
    assert!(!frame(&app).is_empty(), "FIT must leave a rendered frame");

    // --- AutoCAD convention: ESC ends whatever command is active -----------
    ui.invoke_measure_kind_selected(text("measure.kind.polyline").into());
    assert!(ui.get_measurement_active());
    ui.invoke_command_submitted("ESC".into());
    assert!(
        !ui.get_measurement_active(),
        "ESC must cancel an active measurement"
    );
    ui.invoke_annotation_kind_selected(text("annotation.kind.rectangle").into());
    assert!(ui.get_annotation_tool_active());
    ui.invoke_command_submitted("ESC".into());
    assert!(
        !ui.get_annotation_tool_active(),
        "ESC must cancel an active annotation tool"
    );
    ui.invoke_pan_requested();
    assert!(ui.get_pan_active());
    ui.invoke_command_submitted("ESC".into());
    assert!(!ui.get_pan_active(), "ESC must leave pan mode");

    // --- resize matrix: canvas stays inside the window ----------------------
    for (name, size) in [
        ("08-compact-1000x700", [1000.0, 700.0]),
        ("09-mobile-390x844", [390.0, 844.0]),
        ("10-narrow-320x740", [320.0, 740.0]),
    ] {
        app.adapter
            .window()
            .set_size(slint::PhysicalSize::new(size[0] as u32, size[1] as u32));
        app.adapter.handle().refresh_window_layout().unwrap();
        shoot(&app, &dir, name);
        let (rect, _) = app.adapter.handle().shell_geometry().unwrap();
        assert!(
            rect[2] > 0.0 && rect[3] > 0.0,
            "canvas must be non-degenerate"
        );
        assert!(
            rect[0] + rect[2] <= size[0] + 1.0 && rect[1] + rect[3] <= size[1] + 1.0,
            "canvas {rect:?} must stay inside {size:?}"
        );
    }
    assert!(Path::new(&export).exists(), "sidecar evidence must exist");
}
