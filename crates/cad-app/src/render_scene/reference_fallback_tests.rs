//! Reference axes/grid on a blank drawing (no model-space bounds).
//!
//! Split out of `tests.rs` because that file is at the 1000-line limit. The
//! behaviour under test lives in `prepare_inner`: a drawing with no model
//! bounds must fall back to the live 2D viewport rectangle for the reference
//! chrome instead of dropping grid/axes and reporting a user-facing partial
//! diagnostic.

use super::*;
use crate::viewer_config::OverlayVisibility;
use cad_db::DrawingDatabaseBuilder;
use cad_domain::DatabaseId;

#[test]
fn a_blank_drawing_falls_back_to_the_viewport_for_axes_and_grid() {
    // A blank drawing has no model-space bounds; the 2D viewport rectangle is a
    // real reference rectangle, so the grid/axes must still draw (a new drawing
    // is a normal state, not a `Partial` overlay with a user-facing diagnostic).
    let db = DrawingDatabaseBuilder::new(DatabaseId(9)).finish().unwrap();
    let mut controller = CadSceneController::default();
    let layers = LayerOverrideSet::new();
    let on = OverlayInputs {
        viewport_bounds: Some((
            cad_domain::Point3 {
                x: -10.0,
                y: -5.0,
                z: 0.0,
            },
            cad_domain::Point3 {
                x: 10.0,
                y: 5.0,
                z: 0.0,
            },
        )),
        visibility: OverlayVisibility {
            selection_highlight: false,
            grid: true,
            axes: true,
            ..OverlayVisibility::default()
        },
        ..OverlayInputs::default()
    };
    controller
        .prepare_with_overlays(
            Some(&db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &on,
        )
        .unwrap();
    let ready = controller.ready.as_ref().unwrap();
    assert!(
        !ready
            .highlight_diagnostics
            .iter()
            .any(|d| d.code == "overlay.bounds-unavailable"),
        "a blank drawing with a 2D viewport has a usable reference rectangle"
    );
    assert!(matches!(
        ready.highlight_completeness,
        cad_domain::Completeness::Complete
    ));
    let orders: Vec<i64> = ready
        .highlight
        .added
        .iter()
        .map(|batch| batch.draw_order)
        .collect();
    assert!(orders.contains(&AXES_GRID_DRAW_ORDER), "grid batch");
    assert!(orders.contains(&(AXES_GRID_DRAW_ORDER + 1)), "axes batch");
}
