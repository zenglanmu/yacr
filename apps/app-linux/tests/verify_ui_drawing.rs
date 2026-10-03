#![cfg(target_os = "linux")]
//! Real-drawing operation replay for `scripts/verify-ui.sh` (layer 2).
//!
//! Opens a committed open-source fixture DXF with the production [`LinuxApp`]
//! and exercises model-space operations that the synthetic demo cannot cover:
//! multi-layer visibility, paper/model layout switching, pointer selection and
//! a real MOVE transaction that undo restores. Software Vulkan only.
use app_linux::{LinuxApp, LinuxOptions};
use cad_render_wgpu::headless::{encode_png, RgbaImage};
use slint::{ComponentHandle, Model as _};
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dxf/qcad-examples/entities.dxf")
}

fn frame_rgba(app: &LinuxApp) -> RgbaImage {
    let mut image = cad_ui_slint::offscreen::snapshot(app.adapter.window()).unwrap();
    for _ in 0..3 {
        std::thread::sleep(std::time::Duration::from_millis(3));
        image = cad_ui_slint::offscreen::snapshot(app.adapter.window()).unwrap();
    }
    image
}

/// Copy the CAD canvas region out of a full-window frame.
fn crop(image: &RgbaImage, rect: [f64; 4]) -> Vec<u8> {
    let x0 = rect[0].max(0.0) as u32;
    let y0 = rect[1].max(0.0) as u32;
    let x1 = (rect[0] + rect[2]).min(image.width as f64) as u32;
    let y1 = (rect[1] + rect[3]).min(image.height as f64) as u32;
    let mut out = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
    for y in y0..y1 {
        for x in x0..x1 {
            out.extend_from_slice(&image.pixel(x, y));
        }
    }
    out
}

/// The settled CAD canvas pixels (excluding chrome/status text).
///
/// A large real drawing rebuilds its scene incrementally, so a fixed number of
/// snapshots can still read the previous base; waiting for two identical canvas
/// frames is the honest settle condition (with a bounded retry). The status bar
/// is excluded because a command legitimately changes its text.
fn canvas_region(app: &LinuxApp) -> Vec<u8> {
    let (rect, _) = app.adapter.handle().shell_geometry().unwrap();
    let mut previous = crop(&frame_rgba(app), rect);
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(8));
        let next = crop(&frame_rgba(app), rect);
        if next == previous {
            return next;
        }
        previous = next;
    }
    previous
}

/// Number of differing bytes between two crops.
fn differing(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

/// Assert two CAD scenes match exactly, without dumping the buffers.
fn assert_same_scene(label: &str, actual: &[u8], expected: &[u8]) {
    assert_eq!(actual.len(), expected.len(), "{label}: frame size mismatch");
    let diff = differing(actual, expected);
    assert_eq!(diff, 0, "{label}: {diff} bytes differ");
}

fn shoot(app: &LinuxApp, dir: &Option<PathBuf>, name: &str) {
    let Some(dir) = dir else {
        return;
    };
    let image = frame_rgba(app);
    std::fs::write(dir.join(format!("{name}.png")), encode_png(&image).unwrap()).unwrap();
}

/// Brightest drawn pixel inside the CAD rectangle, in window coordinates.
fn geometry_point(app: &LinuxApp) -> (f32, f32) {
    let image = frame_rgba(app);
    let (rect, _) = app.adapter.handle().shell_geometry().unwrap();
    let mut best = None;
    let mut best_sum = 0u32;
    for y in (rect[1] as u32)..((rect[1] + rect[3]) as u32) {
        for x in (rect[0] as u32)..((rect[0] + rect[2]) as u32) {
            let [r, g, b, _] = image.pixel(x, y);
            let sum = r as u32 + g as u32 + b as u32;
            if sum > best_sum {
                best_sum = sum;
                best = Some((x as f32, y as f32));
            }
        }
    }
    let (x, y) = best.expect("real drawing must contain drawn geometry");
    assert!(best_sum > 300, "brightest canvas pixel is not geometry");
    (x, y)
}

fn click(app: &LinuxApp, x: f32, y: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(x, y);
    let window = app.adapter.window();
    // A real pointer moves onto the target before pressing; dispatching the same
    // click twice without motion can leave the widget without a fresh position.
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

fn pick_pair(app: &LinuxApp) {
    let (rect, _) = app.adapter.handle().shell_geometry().unwrap();
    let cx = (rect[2] * 0.5) as f32;
    let cy = (rect[3] * 0.5) as f32;
    app.adapter.component().invoke_canvas_pick(cx, cy);
    app.adapter
        .component()
        .invoke_canvas_pick(cx + 50.0, cy + 30.0);
}

#[test]
fn verify_ui_real_drawing_operations() {
    cad_ui_slint::offscreen::install().unwrap();
    let dir = std::env::var("YACR_VERIFY_OUTPUT").ok().map(PathBuf::from);
    if let Some(dir) = &dir {
        std::fs::create_dir_all(dir).unwrap();
    }
    let drawing = fixture();
    assert!(drawing.exists(), "fixture missing: {}", drawing.display());
    let app = LinuxApp::new(LinuxOptions {
        headless: true,
        drawing: Some(drawing),
        ..LinuxOptions::default()
    })
    .unwrap();
    app.adapter.component().show().unwrap();
    let ui = app.adapter.component();
    shoot(&app, &dir, "20-real-drawing-initial");

    // --- a real drawing exposes several layers, and hiding one must change
    //     the rendered scene, with restore returning the exact pixels --------
    let layers = ui.get_layer_rows().row_count();
    assert!(layers >= 2, "entities.dxf must expose layers, got {layers}");
    let layer_baseline = canvas_region(&app);
    // Determinism guard: the same scene must read back identically twice.
    assert_same_scene("idle repeat", &canvas_region(&app), &layer_baseline);
    let mut changed = false;
    for index in 0..layers {
        ui.invoke_layer_visibility_toggled(index as i32, false);
        if canvas_region(&app) != layer_baseline {
            changed = true;
        }
        ui.invoke_layer_visibility_toggled(index as i32, true);
    }
    ui.invoke_restore_layers_requested();
    assert!(changed, "hiding a real layer must change the scene");
    shoot(&app, &dir, "20b-real-drawing-restored");
    assert_same_scene(
        "restoring real layers",
        &canvas_region(&app),
        &layer_baseline,
    );

    // --- paper/model layout switching ---------------------------------------
    let layouts = ui.get_layout_rows().row_count();
    if layouts > 0 {
        for index in 0..layouts {
            ui.invoke_layout_selected(index as i32);
            assert_eq!(
                ui.get_layout_active_index(),
                index as i32,
                "selecting layout {index} must become active"
            );
        }
        shoot(&app, &dir, "21-real-drawing-paper");
        ui.invoke_layout_selected(-1);
        assert_eq!(
            ui.get_layout_active_index(),
            -1,
            "model space must be active after switching back"
        );
    }

    // --- pointer selection: clicking real geometry selects it, clearing
    //     restores the exact baseline pixels ---------------------------------
    let baseline = canvas_region(&app);
    let (gx, gy) = geometry_point(&app);
    click(&app, gx, gy);
    assert!(
        ui.get_selection_count() > 0,
        "clicking drawn geometry must select it"
    );
    assert_ne!(
        canvas_region(&app),
        baseline,
        "selection highlight must change pixels"
    );
    shoot(&app, &dir, "22-real-drawing-selection");
    ui.invoke_clear_selection_requested();
    assert_eq!(ui.get_selection_count(), 0);
    assert_same_scene("clearing selection", &canvas_region(&app), &baseline);

    // --- MOVE is one real transaction that undo restores ---------------------
    click(&app, gx, gy);
    assert!(ui.get_selection_count() > 0, "MOVE needs a selection");
    ui.invoke_command_submitted("MOVE".into());
    assert!(
        ui.get_draw_tool_active(),
        "MOVE with a selection must start a capture"
    );
    pick_pair(&app);
    ui.invoke_confirm_draw_requested();
    assert!(
        !ui.get_draw_tool_active(),
        "confirmed MOVE must end capture"
    );
    let moved = canvas_region(&app);
    assert_ne!(moved, baseline, "MOVE must change the geometry");
    shoot(&app, &dir, "23-real-drawing-moved");
    ui.invoke_undo_requested();
    ui.invoke_clear_selection_requested();
    assert_same_scene("undoing MOVE", &canvas_region(&app), &baseline);

    // --- command aliases resolve like the full words ------------------------
    for alias in ["L", "C"] {
        ui.invoke_command_submitted(alias.into());
        assert!(
            ui.get_draw_tool_active(),
            "command alias {alias} must start a capture"
        );
        ui.invoke_command_submitted("ESC".into());
    }
    // TRIM may be refused (unsupported) or start; either way ESC clears it and
    // no command path panics.
    ui.invoke_command_submitted("TR".into());
    ui.invoke_command_submitted("ESC".into());
    assert!(
        !ui.get_draw_tool_active(),
        "ESC must clear any TRIM capture"
    );
}
