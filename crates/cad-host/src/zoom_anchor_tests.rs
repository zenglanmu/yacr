use super::select_canvas_metrics;
use cad_app::input::CanvasMetrics;
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
fn platform_insets_are_resolved_but_metrics_remain_canvas_local() {
    let metrics =
        select_canvas_metrics(surface([900.0, 700.0], 2.0, [40.0, 20.0, 30.0, 10.0])).unwrap();
    assert_eq!(metrics.origin_logical, [0.0, 0.0]);
    assert_eq!(metrics.size_logical, [870.0, 630.0]);
    assert_eq!(metrics.dpi_scale, 2.0);
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
}
