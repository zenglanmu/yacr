#![cfg(target_os = "linux")]
//! Real Slint/wgpu software-Vulkan integration, with synthetic CAD content only.
use cad_app::{viewer_config::ViewerConfig, Command, CommandId};
use cad_domain::CadResult;
use cad_render_wgpu::headless::{encode_png, RgbaImage};
use cad_ui_slint::{offscreen, UiAdapter, UiCommandSink, UiConfiguration};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc, sync::Arc};

struct Sink(Rc<RefCell<Vec<CommandId>>>);
impl UiCommandSink for Sink {
    fn send(&mut self, command: Command) -> CadResult<()> {
        self.0.borrow_mut().push(command.id);
        Ok(())
    }
}
fn save(name: &str, frame: &RgbaImage) {
    if let Ok(directory) = std::env::var("YACR_UI_OUTPUT") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            format!("{directory}/{name}.png"),
            encode_png(frame).unwrap(),
        )
        .unwrap();
    }
}
fn pump(adapter: &UiAdapter) -> RgbaImage {
    let mut frame = offscreen::snapshot(adapter.window()).unwrap();
    for _ in 0..3 {
        std::thread::sleep(std::time::Duration::from_millis(2));
        frame = offscreen::snapshot(adapter.window()).unwrap();
    }
    frame
}

fn click(adapter: &UiAdapter, x: f32, y: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(x, y);
    adapter
        .window()
        .dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
    adapter
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
}

#[test]
fn concept_shell_renders_on_lavapipe_and_canvas_only_removes_hit_regions() {
    offscreen::install().unwrap();
    let commands = Rc::new(RefCell::new(Vec::new()));
    let mut adapter =
        UiAdapter::new(UiConfiguration::default(), Sink(commands.clone()), true).unwrap();
    let mut host = cad_app::host::HostController::with_demo_document([1040.0, 500.0]).unwrap();
    host.fit().unwrap();
    let drawing = host.drawing().unwrap();
    let incoming = Rc::new(RefCell::new(Some(drawing.clone())));
    let view =
        cad_ui_slint::install_cad_bridge(adapter.handle(), adapter.window(), incoming).unwrap();
    view.sync_session(
        &host.session.active_space,
        &host.application.workspace.viewports[&host.viewport_id],
    );
    let camera = view.camera();
    let rows = host.layer_rows().unwrap();
    adapter
        .handle()
        .set_layer_state(
            &cad_ui_slint::LayerPanelState::from_rows(&rows, ""),
            &rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        )
        .unwrap();
    adapter
        .handle()
        .set_status(
            cad_ui_slint::MessageSource::from_request("zh-CN").text("status.synthetic", &[]),
        )
        .unwrap();
    adapter.component().show().unwrap();
    let frame = pump(&adapter);
    save("desktop", &frame);
    assert_eq!((frame.width, frame.height), (1280, 800));
    assert!(frame.pixel(1100, 10)[0] > 180, "concept chrome is light");
    let (desktop_rect, _) = adapter.handle().shell_geometry().unwrap();
    assert_eq!(desktop_rect[0], 240.0, "desktop dock is beside the canvas");
    assert!(desktop_rect[1] > 100.0);
    assert!(
        view.frames_rendered() > 0,
        "the shared CAD renderer really ran"
    );
    assert_eq!(view.last_error(), None);
    click(&adapter, 28.0, 18.0);
    assert_eq!(&*commands.borrow(), &[CommandId::OpenDrawing]);
    commands.borrow_mut().clear();
    for (name, size) in [
        ("compact", [1000.0, 700.0]),
        ("mobile", [390.0, 844.0]),
        ("narrow", [320.0, 740.0]),
    ] {
        adapter.fit_window_to_logical(size, 1.0);
        let frame = pump(&adapter);
        save(name, &frame);
        let (rect, _) = adapter.handle().shell_geometry().unwrap();
        assert_eq!(rect[0], 0.0);
        assert!(rect[2] >= size[0] - 1.0);
        assert!(rect[3] > size[1] * 0.6);
        if name == "mobile" {
            assert!(!adapter.handle().canvas_hit_test([345.0, 92.0]).unwrap());
            assert!(!adapter.handle().canvas_hit_test([32.0, 80.0]).unwrap());
            assert!(adapter.handle().canvas_hit_test([180.0, 400.0]).unwrap());
            click(&adapter, 145.0, 810.0);
            let frame = pump(&adapter);
            save("mobile-tools", &frame);
            assert!(adapter.component().get_tools_open());
            click(&adapter, 52.0, 708.0);
            assert_eq!(&*commands.borrow(), &[CommandId::Measure]);
            commands.borrow_mut().clear();
            adapter.component().set_tools_open(false);
            adapter.component().set_side_panel_open(true);
            let frame = pump(&adapter);
            save("mobile-layers", &frame);
            assert_eq!(adapter.handle().shell_geometry().unwrap().0[2], 390.0);
            adapter.component().set_side_panel_open(false);
        }
    }
    let mut config = ViewerConfig::default();
    config.ui.preset = cad_app::viewer_config::Preset::CanvasOnly;
    adapter.handle().set_config(config).unwrap();
    let frame = pump(&adapter);
    save("canvas-only", &frame);
    assert_eq!(
        adapter.handle().shell_geometry().unwrap().0,
        [0.0, 0.0, 320.0, 740.0]
    );
    click(&adapter, 28.0, 18.0);
    assert!(adapter.handle().canvas_hit_test([28.0, 18.0]).unwrap());
    assert!(
        commands.borrow().is_empty(),
        "layout/config changes must not send CAD commands"
    );
    assert_eq!(
        view.camera(),
        camera,
        "layout/config must preserve the camera"
    );
    assert!(Arc::ptr_eq(&drawing, &host.drawing().unwrap()));
}
