//! The camera arithmetic behind `open` and the `FIT` selector — **one** fit rule,
//! shared and unit-tested.
//!
//! Two things have to fit together here, and both are pure:
//!
//! 1. **The bounds an open retains.** [`retain_bounds`] copies the database's
//!    `(min, max)` extents while the imported `Database` is still alive. Only the
//!    two points survive the import's `drop` of the database — the document itself
//!    is still released (`budget::DB_RETAINED` is `false`), so a refit is not a
//!    second query path.
//! 2. **The fit itself.** [`fit_camera`] is the engine's own rule: bounding-box
//!    centre, the larger of the two axis ratios, a 1.05 margin, and the validated
//!    `Projection::orthographic` constructor. The open calls it once (that is the
//!    initial framing) and `FIT` calls it again with the retained bounds at the
//!    session's **current** CSS size. There is deliberately no second "refit"
//!    arithmetic that could disagree with the first.
//!
//! [`zoom_step`] is the `ZOOMIN`/`ZOOMOUT` step: the camera's own `zoom_at` anchored
//! at the viewport centre, because a bare selector supplies no cursor.
//!
//! Because this module has no wasm, DOM, GPU or session state, all of it is asserted
//! by `cargo test -p app-web` on the host — including that a refit restores exactly
//! the framing a zoom step moved away from.

use cad_app::Camera;
use cad_db::DrawingDatabase;
use cad_domain::Point3;

/// The drawing extents a fit is computed from, in world coordinates.
///
/// Retained instead of the imported database: two points the import already
/// measured, not a second copy of the document.
pub type DrawingBounds = (Point3, Point3);

/// Copy the extents of `database`, or `None` when it holds no finite geometry.
///
/// Called exactly where the database is still owned (the geometry half of an open),
/// so nothing has to be re-parsed to refit later.
pub fn retain_bounds(database: &DrawingDatabase) -> Option<DrawingBounds> {
    database.bounds()
}

/// The 2D camera that frames `bounds` at `logical` CSS pixels.
///
/// The arithmetic is the engine's `fit_camera` rule (bounding-box centre, the larger
/// of the two axis ratios, 1.05 margin), and the projection goes through the
/// validated `Projection::orthographic` constructor. Absent or non-finite bounds keep
/// the default top view rather than producing a NaN scale — which is why a session
/// without retained bounds reports `canFit: false` instead of a claimed refit.
pub fn fit_camera(bounds: Option<DrawingBounds>, logical: [f64; 2]) -> Camera {
    let mut camera = Camera::top_view_2d();
    let Some((min, max)) = bounds else {
        return camera;
    };
    let extent_x = (max.x - min.x).max(1e-6);
    let extent_y = (max.y - min.y).max(1e-6);
    let center = Point3 {
        x: (min.x + max.x) * 0.5,
        y: (min.y + max.y) * 0.5,
        z: 0.0,
    };
    let world_per_px = (extent_x / logical[0].max(1.0)).max(extent_y / logical[1].max(1.0)) * 1.05;
    if !(center.x.is_finite() && center.y.is_finite()) {
        return camera;
    }
    // Shift both eye and target, so the view direction and the eye distance of
    // the top view are preserved (the 2D path only reads the target centre and the
    // projection scale).
    let offset = Point3 {
        x: center.x - camera.target.x,
        y: center.y - camera.target.y,
        z: 0.0,
    };
    camera.target = Point3 {
        x: camera.target.x + offset.x,
        y: camera.target.y + offset.y,
        z: camera.target.z,
    };
    camera.eye = Point3 {
        x: camera.eye.x + offset.x,
        y: camera.eye.y + offset.y,
        z: camera.eye.z,
    };
    if let Ok(projection) = cad_app::Projection::orthographic(world_per_px) {
        camera.projection = projection;
    }
    camera
}

/// One fixed zoom step (`ZOOMIN`/`ZOOMOUT`) about the viewport centre.
///
/// A selector supplies no cursor, so the anchor is the centre of the live CSS
/// viewport — the same rule the desktop session applies for the same two names, with
/// the same factors (`cad_ui_core::session::ZOOM_IN_FACTOR`/`ZOOM_OUT_FACTOR`). The
/// error is the camera's own text (`zoom factor must be positive and finite`,
/// `zoom focus point is degenerate`, a validation failure).
pub fn zoom_step(camera: &mut Camera, factor: f64, size: [f64; 2]) -> Result<(), String> {
    let anchor = [size[0] * 0.5, size[1] * 0.5];
    camera
        .zoom_at(factor, anchor, size)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
    use cad_domain::{
        DatabaseId, EntityId, LayerId, ObjectId, Revision, SemanticGeometry, SpaceId,
    };

    /// A 10 x 4 world-space line on layer `0`: enough for real, finite bounds.
    fn database_with_a_line() -> DrawingDatabase {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
        builder
            .insert_layer(Layer {
                id: LayerId(0),
                name: "0".into(),
                visible: true,
            })
            .expect("the default layer is insertable");
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(1),
                    type_key: "AcDbLine".into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: EntityId(1),
                layer: LayerId(0),
                space: SpaceId::Model,
                geometry: SemanticGeometry::Line {
                    start: Point3 {
                        x: -2.0,
                        y: 1.0,
                        z: 0.0,
                    },
                    end: Point3 {
                        x: 8.0,
                        y: 5.0,
                        z: 0.0,
                    },
                },
                draw_order: 0,
            })
            .expect("one line is insertable");
        builder.finish().expect("the database is valid")
    }

    fn world_per_px(camera: &Camera) -> f64 {
        camera
            .camera2d_params()
            .expect("a fitted top view is a valid 2D view")
            .world_per_px
    }

    fn centre(camera: &Camera) -> [f64; 2] {
        let params = camera.camera2d_params().expect("a valid 2D view");
        [params.center.x, params.center.y]
    }

    #[test]
    fn the_open_retains_the_bounds_it_fitted_to() {
        let database = database_with_a_line();
        let bounds = retain_bounds(&database).expect("the line has finite bounds");
        // The retained pair is the database's own extents, not a rounded or
        // recomputed value.
        assert_eq!(Some(bounds), database.bounds());
        assert_eq!(bounds.0.x, -2.0);
        assert_eq!(bounds.0.y, 1.0);
        assert_eq!(bounds.1.x, 8.0);
        assert_eq!(bounds.1.y, 5.0);

        // …and the initial framing is the fit of exactly those bounds.
        let fitted = fit_camera(Some(bounds), [800.0, 600.0]);
        let expected = (10.0f64 / 800.0).max(4.0 / 600.0) * 1.05;
        assert!((world_per_px(&fitted) - expected).abs() < 1e-12);
        assert_eq!(centre(&fitted), [3.0, 3.0]);

        // A drawing with no geometry retains nothing, so `FIT` is not advertised.
        let mut empty = DrawingDatabaseBuilder::new(DatabaseId(2));
        empty
            .insert_layer(Layer {
                id: LayerId(0),
                name: "0".into(),
                visible: true,
            })
            .expect("the default layer is insertable");
        let empty = empty.finish().expect("the database is valid");
        assert_eq!(retain_bounds(&empty), None);
    }

    #[test]
    fn a_refit_restores_the_framing_a_zoom_step_moved_away_from() {
        let bounds = retain_bounds(&database_with_a_line());
        let size = [1024.0, 768.0];
        let fitted = fit_camera(bounds, size);
        let fitted_scale = world_per_px(&fitted);
        let fitted_centre = centre(&fitted);

        // One ZOOMIN step: 1.25x closer, anchored at the viewport centre, so the
        // framing centre does not move.
        let mut zoomed = fitted;
        zoom_step(&mut zoomed, cad_ui_core::session::ZOOM_IN_FACTOR, size)
            .expect("a positive factor on a valid camera");
        assert!((world_per_px(&zoomed) - fitted_scale / 1.25).abs() < 1e-12);
        assert_eq!(centre(&zoomed), fitted_centre);

        // FIT re-runs the same rule from the same bounds: the camera is back.
        let refitted = fit_camera(bounds, size);
        assert_eq!(world_per_px(&refitted), fitted_scale);
        assert_eq!(centre(&refitted), fitted_centre);

        // ZOOMOUT is the exact reciprocal step, and a single step each way returns
        // to where it started (to floating-point precision).
        let mut out = zoomed;
        zoom_step(&mut out, cad_ui_core::session::ZOOM_OUT_FACTOR, size)
            .expect("a positive factor");
        assert!((world_per_px(&out) - fitted_scale).abs() < 1e-12);
    }

    #[test]
    fn a_fit_at_another_size_keeps_the_same_drawing_in_frame() {
        let bounds = retain_bounds(&database_with_a_line());
        let small = fit_camera(bounds, [400.0, 300.0]);
        let large = fit_camera(bounds, [800.0, 600.0]);
        // Twice the viewport, half the world-per-pixel: the same extents are framed.
        assert!((world_per_px(&large) * 2.0 - world_per_px(&small)).abs() < 1e-12);
        assert_eq!(centre(&small), centre(&large));
    }

    #[test]
    fn no_bounds_keep_the_default_view_and_zoom_steps_still_work() {
        let default = Camera::top_view_2d();
        let fitted = fit_camera(None, [800.0, 600.0]);
        assert_eq!(world_per_px(&fitted), world_per_px(&default));
        assert_eq!(centre(&fitted), centre(&default));

        // A session without bounds can still zoom (the camera is valid); it simply
        // has nothing to refit to, which is what `canFit: false` says.
        let mut camera = fitted;
        zoom_step(
            &mut camera,
            cad_ui_core::session::ZOOM_IN_FACTOR,
            [800.0, 600.0],
        )
        .expect("the default view is a valid 2D camera");
        assert!(world_per_px(&camera) < world_per_px(&default));
    }

    #[test]
    fn a_non_finite_bounds_never_becomes_a_nan_scale() {
        let nan = Point3 {
            x: f64::NAN,
            y: 0.0,
            z: 0.0,
        };
        let camera = fit_camera(Some((nan, nan)), [800.0, 600.0]);
        assert!(world_per_px(&camera).is_finite());
        assert!(world_per_px(&camera) > 0.0);
    }
}
