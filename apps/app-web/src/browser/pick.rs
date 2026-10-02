//! Canvas-local picking for the web host: logical pixels → world.
//!
//! Two consumers share this module:
//!
//! * the measurement/annotation `CanvasPickMapper` installed on the adapter,
//!   which must resolve a canvas-local logical point to a world point on the
//!   viewport work plane; and
//! * the selection gesture in [`super::input`], which hit-tests the same point
//!   against model-space geometry through `cad_app::pick_at_screen`.
//!
//! Both read the *authoritative* `cad_app::Viewport` and the canvas logical size
//! the shell reports, so a pick and a render of the same pixel agree (audit
//! U07/B17). Degenerate input returns `None`/an error rather than a fabricated
//! world point.

use std::cell::RefCell;
use std::rc::Rc;

use cad_app::host::HostController;
use cad_app::input::{CanvasMetrics, ViewMetrics};
use cad_app::picking::pick_at_screen;
use cad_app::{BackFacePolicy, PickReport, Viewport};
use cad_domain::{CadError, CadResult, Point3, SelectionRef, TolerancePolicy, ViewportId};
use cad_ui_slint::CanvasPickMapper;

use super::SharedHandle;

/// Map a canvas-local logical point to a world point on the viewport work plane.
///
/// `canvas_size` is the CAD content rectangle in logical pixels (not the whole
/// window); `logical` is measured from its top-left with `y` growing downwards.
/// A degenerate canvas or a non-finite point returns `None`, never a made-up
/// point on the axes.
pub(super) fn map_canvas_point(
    viewport: &Viewport,
    canvas_size: [f64; 2],
    logical: [f64; 2],
) -> Option<Point3> {
    if !logical[0].is_finite() || !logical[1].is_finite() {
        return None;
    }
    // The pointer already arrives canvas-local, so the canvas origin is zero;
    // going through `ViewMetrics` keeps the shared validity rules (finite,
    // positive size and DPI) in one place. The DPI scale only affects physical
    // conversion, not this logical→world mapping.
    let metrics = ViewMetrics::new(
        CanvasMetrics::new([0.0, 0.0], canvas_size, viewport.dpi_scale.max(1.0)),
        viewport,
    );
    metrics.surface_to_world(logical)
}

/// The selection refs a pick report resolves, or an empty list for a miss.
///
/// A miss is an explicit "clear the selection" (`Select` with an empty payload),
/// never a fabricated entity ref.
pub(super) fn selection_refs_from_report(report: &PickReport) -> Vec<SelectionRef> {
    report
        .hit
        .as_ref()
        .map(|hit| vec![hit.source.clone()])
        .unwrap_or_default()
}

/// The canvas logical size the mapper and selection picker should use.
///
/// Prefers the shell's real CAD rectangle; falls back to the viewport's cached
/// logical size when the handle is not available (for example before the shell
/// has laid out).
pub(super) fn canvas_size(handle: &SharedHandle, viewport: &Viewport) -> Option<[f64; 2]> {
    if let Some(handle) = handle.borrow().as_ref() {
        if let Some((size, _scale)) = handle.cad_surface_size() {
            if size[0].is_finite() && size[1].is_finite() && size[0] > 0.0 && size[1] > 0.0 {
                return Some(size);
            }
        }
    }
    let size = viewport.logical_size;
    (size[0].is_finite() && size[1].is_finite() && size[0] > 0.0 && size[1] > 0.0).then_some(size)
}

/// The mapper installed on `UiAdapter::set_canvas_pick_mapper` for the web host.
///
/// It holds the controller and viewport rather than a snapshot so every pick
/// sees the live camera/work plane; a stale snapshot would disagree with what is
/// on screen after a pan or zoom.
pub(super) struct WebCanvasPickMapper {
    pub(super) controller: Rc<RefCell<HostController>>,
    pub(super) handle: SharedHandle,
    pub(super) viewport: ViewportId,
}

impl CanvasPickMapper for WebCanvasPickMapper {
    fn to_world(&self, logical: [f64; 2]) -> Option<Point3> {
        let controller = self.controller.borrow();
        let viewport = controller
            .application
            .workspace
            .viewports
            .get(&self.viewport)?;
        let size = canvas_size(&self.handle, viewport)?;
        map_canvas_point(viewport, size, logical)
    }
}

/// Hit-test a canvas-local point against the open drawing's model space.
///
/// Returns the closest hit's `SelectionRef`, or an empty list for a miss. The
/// tolerance comes from the shared default policy and back faces are culled, so
/// a click on empty space clears the selection instead of grabbing a far object.
/// A degenerate point/viewport is an error, not an empty success.
pub(super) fn pick_selection(
    controller: &HostController,
    canvas_size: [f64; 2],
    logical: [f64; 2],
) -> CadResult<Vec<SelectionRef>> {
    let viewport = controller
        .application
        .workspace
        .viewports
        .get(&controller.viewport_id)
        .ok_or_else(|| CadError::InvalidInput("viewport not found".into()))?;
    let drawing = controller
        .drawing()
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    let report = pick_at_screen(
        drawing.as_ref(),
        controller.document_id,
        &viewport.camera,
        logical,
        canvas_size,
        &TolerancePolicy::default(),
        BackFacePolicy::Cull,
    )?;
    Ok(selection_refs_from_report(&report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_app::Viewport;
    use cad_domain::{DocumentId, EntityId, GeometrySource, InstancePath, Precision};
    use cad_spatial::PickHit;

    fn viewport() -> Viewport {
        let mut vp = Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]);
        vp.camera.projection = cad_app::Projection::Orthographic { scale: 1.0 };
        vp.camera.target = Point3 {
            x: 10.0,
            y: 0.0,
            z: 0.0,
        };
        vp
    }

    #[test]
    fn canvas_centre_maps_to_the_camera_target() {
        let vp = viewport();
        let world = map_canvas_point(&vp, [800.0, 600.0], [400.0, 300.0]).expect("centre maps");
        assert!((world.x - 10.0).abs() < 1e-9);
        assert!((world.y - 0.0).abs() < 1e-9);
        assert!((world.z - 0.0).abs() < 1e-9);
    }

    #[test]
    fn canvas_point_y_is_flipped_relative_to_world_y() {
        let vp = viewport();
        // A pixel above the centre (smaller y) is +world y.
        let above = map_canvas_point(&vp, [800.0, 600.0], [400.0, 100.0]).unwrap();
        assert!(above.y > 0.0);
        let below = map_canvas_point(&vp, [800.0, 600.0], [400.0, 500.0]).unwrap();
        assert!(below.y < 0.0);
    }

    #[test]
    fn degenerate_canvas_or_point_maps_to_none() {
        let vp = viewport();
        assert!(map_canvas_point(&vp, [0.0, 600.0], [0.0, 0.0]).is_none());
        assert!(map_canvas_point(&vp, [800.0, 600.0], [f64::NAN, 0.0]).is_none());
        assert!(map_canvas_point(&vp, [800.0, f64::INFINITY], [0.0, 0.0]).is_none());
    }

    fn zero_hit(entity: u128) -> PickReport {
        PickReport {
            hit: Some(PickHit {
                source: SelectionRef {
                    document: DocumentId(1),
                    entity: EntityId(entity),
                    instance: InstancePath::default(),
                    sub_element: None,
                },
                point: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                distance: 1.0,
                offset: 0.0,
                precision: Precision::Analytic,
                geometry_source: GeometrySource::Analytic,
                sub_element_reason: None,
            }),
            skipped: Vec::new(),
        }
    }

    #[test]
    fn a_hit_becomes_one_ref_and_a_miss_becomes_none_not_a_fabricated_ref() {
        let refs = selection_refs_from_report(&zero_hit(7));
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].entity, EntityId(7));

        let miss = PickReport::default();
        assert!(selection_refs_from_report(&miss).is_empty());
    }

    #[test]
    fn pick_selection_finds_a_demo_line_and_misses_empty_space() {
        let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let _ = controller.fit();
        let size = [800.0, 600.0];
        // Project the midpoint of the bottom wall (0,0)-(4000,0) to its pixel,
        // then pick there: the hit must be a real selection ref.
        let pixel = {
            let viewport = controller
                .application
                .workspace
                .viewports
                .get(&controller.viewport_id)
                .unwrap();
            viewport
                .camera
                .world_to_screen(
                    Point3 {
                        x: 2000.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    size,
                )
                .expect("the wall midpoint projects")
        };
        let refs = pick_selection(&controller, size, pixel).unwrap();
        assert_eq!(refs.len(), 1, "the wall midpoint must pick the wall line");

        let miss = pick_selection(&controller, size, [1.0, 1.0]).unwrap();
        assert!(miss.is_empty(), "empty space must clear, not guess");
    }
}
