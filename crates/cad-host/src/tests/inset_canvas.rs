//! Full-window carriers (Android `native-below`) declare the visible CAD rect
//! inside the native surface. Fit/zoom must frame that rect, not the whole
//! window, while input keeps mapping surface coordinates through the rect
//! origin. These are host-target contracts: no GPU, no device, no pixels.

use super::*;
use std::cell::Cell;

struct InsetSurface {
    metrics: Cell<SurfaceMetrics>,
}

impl InsetSurface {
    fn new(metrics: SurfaceMetrics) -> Self {
        Self {
            metrics: Cell::new(metrics),
        }
    }
}

impl SurfaceSource for InsetSurface {
    fn surface(&self) -> Option<SurfaceMetrics> {
        Some(self.metrics.get())
    }
}

fn inset_core(surface: SurfaceMetrics) -> (Rc<HostCore>, Rc<InsetSurface>) {
    let source = Rc::new(InsetSurface::new(surface));
    let platform = HostPlatform {
        files: Rc::new(FakeFiles::default()),
        config_store: Rc::new(FakeConfig),
        surface: source.clone(),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    let core = Rc::new(HostCore::new(options(), platform, RenderSessionHandle::default()).unwrap());
    (core, source)
}

fn fitted_scale(core: &HostCore) -> f64 {
    let c = core.controller.borrow();
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    viewport.world_per_px()
}

#[test]
fn a_full_window_host_fits_the_visible_rect_not_the_window() {
    // A small CAD rect inside a large window: a window fit would leave the
    // drawing far smaller than a fit into the visible rect.
    let (core, _source) = inset_core(SurfaceMetrics {
        logical: [200.0, 500.0],
        origin: [0.0, 100.0],
        scale: 2.625,
        safe_insets: [0.0; 4],
    });
    core.tick().unwrap();

    let metrics = core.canvas_metrics.get();
    assert_eq!(metrics.origin_logical, [0.0, 100.0]);
    assert_eq!(metrics.size_logical, [200.0, 500.0]);
    let view = core.render.view_snapshot();
    assert_eq!(view.canvas_origin, Some([0.0, 100.0]));
    assert_eq!(view.canvas_size, [200.0, 500.0]);

    core.controller.borrow_mut().fit().unwrap();
    let rect_fit = fitted_scale(&core);

    let (full, _full_source) = inset_core(SurfaceMetrics {
        logical: [800.0, 600.0],
        origin: [0.0, 0.0],
        scale: 2.625,
        safe_insets: [0.0; 4],
    });
    full.tick().unwrap();
    full.controller.borrow_mut().fit().unwrap();
    let window_fit = fitted_scale(&full);

    assert!(
        rect_fit > window_fit * 1.5,
        "fit must use the visible rect: rect={rect_fit} window={window_fit}"
    );
}

#[test]
fn input_positions_are_surface_coordinates_and_map_back_to_the_rect() {
    let (core, _source) = inset_core(SurfaceMetrics {
        logical: [411.428_57, 606.857_14],
        origin: [0.0, 92.952_38],
        scale: 2.625,
        safe_insets: [0.0; 4],
    });
    core.tick().unwrap();
    let metrics = core.canvas_metrics.get();
    let c = core.controller.borrow();
    let viewport = &c.application.workspace.viewports[&c.viewport_id];

    // The chrome reports rect-local positions; the host adds the rect origin
    // and `ViewMetrics` subtracts it exactly once.
    let local = [123.0, 210.0];
    let surface_position = [
        metrics.origin_logical[0] + local[0],
        metrics.origin_logical[1] + local[1],
    ];
    let mapped = cad_app::input::ViewMetrics::new(metrics, viewport)
        .surface_to_world(surface_position)
        .expect("a valid canvas maps");
    let expected = viewport
        .screen_to_world(local, viewport.logical_size)
        .expect("the rect-local mapping");
    assert!((mapped.x - expected.x).abs() < 1e-9);
    assert!((mapped.y - expected.y).abs() < 1e-9);
}

#[test]
fn an_origin_change_updates_the_view_placement_without_moving_the_camera() {
    let (core, source) = inset_core(SurfaceMetrics {
        logical: [411.428_57, 606.857_14],
        origin: [0.0, 92.952_38],
        scale: 2.625,
        safe_insets: [0.0; 4],
    });
    core.tick().unwrap();
    let before = core.render.view_snapshot();
    let camera_before = before.camera;

    // The command bar collapses: the rect moves up, same size.
    source.metrics.set(SurfaceMetrics {
        logical: [411.428_57, 606.857_14],
        origin: [0.0, 10.0],
        scale: 2.625,
        safe_insets: [0.0; 4],
    });
    core.tick().unwrap();
    let after = core.render.view_snapshot();
    assert_eq!(after.canvas_origin, Some([0.0, 10.0]));
    assert_eq!(after.canvas_size, [411.428_57, 606.857_14]);
    // Metrics never move the camera: the placement is a render-time offset.
    assert_eq!(after.camera, camera_before);
}
