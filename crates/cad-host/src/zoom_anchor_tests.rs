use super::select_canvas_metrics;
use cad_app::input::CanvasMetrics;
use cad_domain::CadError;
use cad_platform::SurfaceMetrics;

fn surface(logical: [f64; 2], scale: f64, safe_insets: [f64; 4]) -> SurfaceMetrics {
    SurfaceMetrics {
        logical,
        origin: [0.0, 0.0],
        scale,
        safe_insets,
    }
}

#[test]
fn platform_insets_become_the_canvas_rect_inside_the_surface() {
    let metrics =
        select_canvas_metrics(surface([900.0, 700.0], 2.0, [40.0, 20.0, 30.0, 10.0])).unwrap();
    // The canvas is the inset rect (fit/zoom) and keeps its offset inside the
    // native surface (the renderer places the camera on that rect's centre).
    assert_eq!(metrics.origin_logical, [10.0, 40.0]);
    assert_eq!(metrics.size_logical, [870.0, 630.0]);
    assert_eq!(metrics.dpi_scale, 2.0);
}

#[test]
fn a_surface_origin_places_the_canvas_rect() {
    let mut surface = surface([411.43, 914.29], 2.625, [0.0; 4]);
    surface.origin = [0.0, 92.95];
    let metrics = select_canvas_metrics(surface).unwrap();
    assert_eq!(metrics.origin_logical, [0.0, 92.95]);
    assert_eq!(metrics.size_logical, [411.43, 914.29]);
}

#[test]
fn invalid_platform_metrics_are_rejected() {
    for platform in [
        surface([0.0, 600.0], 1.0, [0.0; 4]),
        surface([f64::NAN, 600.0], 1.0, [0.0; 4]),
        surface([800.0, 600.0], 0.0, [0.0; 4]),
        surface([800.0, 600.0], f64::NAN, [0.0; 4]),
    ] {
        assert!(matches!(
            select_canvas_metrics(platform),
            Err(CadError::InvalidInput(_))
        ));
    }
    assert!(!CanvasMetrics::new([0.0; 2], [f64::NAN, 10.0], 1.0).is_valid());

    let mut nan_origin = surface([800.0, 600.0], 1.0, [0.0; 4]);
    nan_origin.origin = [f64::NAN, 0.0];
    assert!(matches!(
        select_canvas_metrics(nan_origin),
        Err(CadError::InvalidInput(_))
    ));
}
