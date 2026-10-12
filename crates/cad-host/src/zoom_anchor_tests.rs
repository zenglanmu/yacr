use super::select_canvas_metrics;
use cad_app::{
    host::HostController,
    input::{apply_canvas_metrics, ViewMetrics},
};
use cad_domain::CadError;
use cad_platform::SurfaceMetrics;

fn surface(logical: [f64; 2], scale: f64, safe_insets: [f64; 4]) -> SurfaceMetrics {
    SurfaceMetrics {
        logical,
        scale,
        safe_insets,
    }
}

#[test]
fn live_cad_metrics_ignore_window_origin_and_chrome_and_match_render_size() {
    // Synthetic window geometry: non-zero CAD origin in the window, high-DPI,
    // and substantial phone chrome/insets. CAD input coordinates are canvas-local.
    let window_cad_origin = [24.0, 148.0];
    assert!(window_cad_origin[0] > 0.0 && window_cad_origin[1] > 0.0);
    let metrics = select_canvas_metrics(
        surface([1080.0, 2400.0], 3.0, [84.0, 0.0, 72.0, 0.0]),
        Some(([360.0, 620.0], 3.0)),
    )
    .unwrap();
    assert_eq!(metrics.origin_logical, [0.0, 0.0]);
    assert_eq!(metrics.size_logical, [360.0, 620.0]);
    assert_eq!(metrics.dpi_scale, 3.0);
    assert_eq!(metrics.size_physical(), Some([1080.0, 1860.0]));

    let mut controller = HostController::with_demo_document([360.0, 620.0]).unwrap();
    let viewport = controller
        .application
        .workspace
        .viewports
        .get_mut(&controller.viewport_id)
        .unwrap();
    apply_canvas_metrics(viewport, &metrics).unwrap();
    assert_eq!(viewport.logical_size, metrics.size_logical);
    assert_eq!(viewport.dpi_scale, metrics.dpi_scale);

    // Pinch ZoomAt is canvas-local: anchor a distinctly off-center world point
    // under the local cursor, zoom, and prove that it remains under that point.
    let cursor = [73.0, 491.0];
    let before = ViewMetrics::new(metrics, viewport)
        .surface_to_world(cursor)
        .unwrap();
    viewport
        .camera
        .zoom_at(1.7, cursor, metrics.size_logical)
        .unwrap();
    let after = ViewMetrics::new(metrics, viewport)
        .surface_to_world(cursor)
        .unwrap();
    assert!((before.x - after.x).abs() < 1e-8);
    assert!((before.y - after.y).abs() < 1e-8);
}

#[test]
fn no_handle_uses_surface_insets_and_validates_metrics() {
    let fallback =
        select_canvas_metrics(surface([900.0, 700.0], 2.0, [40.0, 20.0, 30.0, 10.0]), None)
            .unwrap();
    assert_eq!(fallback.origin_logical, [10.0, 40.0]);
    assert_eq!(fallback.size_logical, [870.0, 630.0]);
    assert_eq!(fallback.dpi_scale, 2.0);

    let invalid_live = select_canvas_metrics(
        surface([800.0, 600.0], 1.0, [0.0; 4]),
        Some(([0.0, 600.0], 2.0)),
    )
    .unwrap();
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let viewport = controller
        .application
        .workspace
        .viewports
        .get_mut(&controller.viewport_id)
        .unwrap();
    assert!(matches!(
        apply_canvas_metrics(viewport, &invalid_live),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn live_canvas_still_rejects_invalid_platform_surface_metrics() {
    let invalid_platform_surfaces = [
        surface([0.0, 600.0], 1.0, [0.0; 4]),
        surface([f64::NAN, 600.0], 1.0, [0.0; 4]),
        surface([800.0, 600.0], 0.0, [0.0; 4]),
        surface([800.0, 600.0], f64::NAN, [0.0; 4]),
    ];

    for platform in invalid_platform_surfaces {
        assert!(matches!(
            select_canvas_metrics(platform, Some(([360.0, 620.0], 3.0))),
            Err(CadError::InvalidInput(_))
        ));
    }

    // A valid platform contract must not make a degenerate CAD region valid.
    let platform = surface([800.0, 600.0], 1.0, [20.0, 10.0, 20.0, 10.0]);
    let canvas = select_canvas_metrics(platform, Some(([f64::NAN, 620.0], 3.0))).unwrap();
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let viewport = controller
        .application
        .workspace
        .viewports
        .get_mut(&controller.viewport_id)
        .unwrap();
    assert!(matches!(
        apply_canvas_metrics(viewport, &canvas),
        Err(CadError::InvalidInput(_))
    ));
}
