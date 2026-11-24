//! `VIEW` table export: `tables/views.json` rows → acadrust named views.
//!
//! The reverse map is explicit about the one approximation the DXF view model
//! forces: the source stores a world-space **height**, not the viewport's
//! world-units-per-pixel scale. An orthographic view is therefore written with
//! `height = width = scale * 1000` and a `export.view_camera_approximated` note;
//! the round trip through our own importer applies the same convention.
//!
//! A view is never dropped silently: a degenerate camera is reported as a dropped
//! row with a stable `export.view_*` code.

use acadrust::tables::View as DxfView;
use acadrust::{CadDocument, Vector3};
use cad_db::DrawingDatabase;
use cad_domain::view::{NamedView, ViewProjection};
use cad_domain::{Point3, SpaceId};

use super::report::{ExportDiagnostic, ExportOutcome, ExportReport};

/// Mirror of the import convention: `height = scale * 1000`.
const HEIGHT_PER_PIXEL_SCALE: f64 = 1000.0;

/// Write every persisted named view into the export document.
pub(crate) fn write_views(
    doc: &mut CadDocument,
    database: &DrawingDatabase,
    report: &mut ExportReport,
) -> (usize, usize) {
    let mut converted = 0usize;
    let mut dropped = 0usize;
    for view in database.named_views().views() {
        let Some(entry) = entry_from_view(view) else {
            dropped += 1;
            report.entries.push(ExportDiagnostic {
                object: None,
                code: "export.view_degenerate_camera".into(),
                outcome: ExportOutcome::Dropped,
                message: format!(
                    "named view '{}': the eye and target are the same point, so no viewing \
                     direction can be written",
                    view.name
                ),
            });
            continue;
        };
        doc.views.add_or_replace(entry);
        // The height/scale convention is a real conversion, not a faithful copy;
        // count it so the report's completeness is `Partial`, not `Complete`.
        converted += 1;
        report.notes.push(ExportDiagnostic {
            object: None,
            code: "export.view_camera_approximated".into(),
            outcome: ExportOutcome::Note,
            message: format!(
                "named view '{}': the DXF VIEW row stores a world-space height; an orthographic \
                 scale is written as height = scale * {HEIGHT_PER_PIXEL_SCALE} and a 3D eye \
                 distance as the same height",
                view.name
            ),
        });
    }
    (converted, dropped)
}

fn entry_from_view(view: &NamedView) -> Option<DxfView> {
    let eye = view.camera.eye;
    let target = view.camera.target;
    let direction = Point3 {
        x: eye.x - target.x,
        y: eye.y - target.y,
        z: eye.z - target.z,
    };
    let length =
        (direction.x * direction.x + direction.y * direction.y + direction.z * direction.z).sqrt();
    if !length.is_finite() || length <= 1e-12 {
        return None;
    }
    let direction = Vector3::new(
        direction.x / length,
        direction.y / length,
        direction.z / length,
    );
    let (height, perspective) = match view.camera.projection {
        ViewProjection::Orthographic { scale } => (scale * HEIGHT_PER_PIXEL_SCALE, view.is_3d),
        ViewProjection::Perspective { .. } => (length, true),
    };
    let lens_length = match view.camera.projection {
        ViewProjection::Orthographic { .. } => 50.0,
        ViewProjection::Perspective {
            vertical_fov_radians,
        } => 12.0 / (vertical_fov_radians / 2.0).tan(),
    };
    let mut entry = DxfView::new(view.name.clone());
    entry.center = Vector3::new(target.x, target.y, target.z);
    entry.target = Vector3::new(target.x, target.y, target.z);
    entry.direction = direction;
    entry.height = height;
    entry.width = height;
    entry.perspective = perspective;
    entry.lens_length = if lens_length.is_finite() && lens_length > 0.0 {
        lens_length
    } else {
        50.0
    };
    entry.paper_space = matches!(view.space, SpaceId::Paper(_));
    entry.ucs_associated = true;
    entry.ucs_origin = Vector3::new(
        view.work_plane.origin.x,
        view.work_plane.origin.y,
        view.work_plane.origin.z,
    );
    entry.ucs_x_axis = Vector3::new(
        view.work_plane.u.x,
        view.work_plane.u.y,
        view.work_plane.u.z,
    );
    entry.ucs_y_axis = Vector3::new(
        view.work_plane.v.x,
        view.work_plane.v.y,
        view.work_plane.v.z,
    );
    Some(entry)
}
