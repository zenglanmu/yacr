//! `VIEW` table import: acadrust named views → `tables/views.json` rows.
//!
//! acadrust models the drawing's named-view table (`CadDocument::views`), so this
//! build does not silently drop it. The mapping is explicit and approximate where
//! the source has no equivalent:
//!
//! * `target`/`direction` map directly; the eye is `target + direction * distance`.
//! * The DXF view stores a view **height** in world units, not a world-units-per-
//!   pixel scale (that depends on the viewport the view is later shown in), so an
//!   orthographic view is stored at `height / 1000` world units/pixel and the
//!   approximation is reported per row. A perspective view keeps `height` as the
//!   eye distance and derives the vertical FOV from `lens_length` (24 mm sensor).
//! * `ucs_*` maps to the work plane when `ucs_associated` is set; otherwise the
//!   world XY plane is used.
//! * `paper_space` rows cannot name which layout they belong to (the source field
//!   is only a boolean), so they are reported and skipped rather than attached to
//!   an arbitrary layout.
//!
//! Every skip is a named `import.view_*` diagnostic; a view is never dropped in
//! silence.

use cad_domain::view::{NamedView, NamedViewTable, ViewCamera, ViewProjection};
use cad_domain::{CadResult, Diagnostic, Point3, SpaceId, WorkPlane};

use crate::geometry::p3;
use crate::ImporterBuilder;

/// The world-units-per-1000-pixels convention used when a DXF view only carries
/// a world-space height.
const HEIGHT_TO_PIXEL_SCALE: f64 = 1000.0;

impl ImporterBuilder<'_> {
    /// Read the drawing's named views into the database builder.
    pub(crate) fn read_views(&mut self) -> CadResult<()> {
        let mut table = NamedViewTable::new();
        let views: Vec<acadrust::tables::View> = self.acad.views.iter().cloned().collect();
        for source in views {
            if source.xref_dependent || source.xref_reference {
                self.note_view(
                    "import.view_xref_dependent",
                    &source.name,
                    "the source row is an external-reference view; it is not this drawing's view",
                );
                continue;
            }
            if source.paper_space {
                self.note_view(
                    "import.view_paper_space_unresolved",
                    &source.name,
                    "the source row is a paper-space view but names no layout; the row was skipped \
                     instead of being attached to an arbitrary layout",
                );
                continue;
            }
            let Some(row) = row_from_source(&source, &mut |code, message| {
                self.note_view(code, &source.name, &message)
            }) else {
                continue;
            };
            match table.save(row) {
                Ok(_) => {}
                Err(error) => self.note_view(
                    "import.view_invalid",
                    &source.name,
                    &format!("the row was not stored: {error}"),
                ),
            }
        }
        if !table.is_empty() {
            self.builder.set_named_views(table)?;
        }
        Ok(())
    }

    fn note_view(&mut self, code: &str, name: &str, detail: &str) {
        self.diagnostics.push(Diagnostic {
            object: None,
            code: code.to_string(),
            message: format!("VIEW `{name}`: {detail}"),
        });
    }
}

/// Convert one source row, reporting the approximations through `note`.
fn row_from_source(
    source: &acadrust::tables::View,
    note: &mut dyn FnMut(&str, String),
) -> Option<NamedView> {
    let name = source.name.trim().to_string();
    if name.is_empty() {
        note(
            "import.view_name_empty",
            "the source row has no name and was skipped".into(),
        );
        return None;
    }
    let target = p3(source.target);
    let direction = p3(source.direction);
    if !finite(target) || !finite(direction) {
        note(
            "import.view_invalid_camera",
            "target or direction is not finite; the row was skipped".into(),
        );
        return None;
    }
    let length =
        (direction.x * direction.x + direction.y * direction.y + direction.z * direction.z).sqrt();
    if !length.is_finite() || length <= 1e-12 {
        note(
            "import.view_invalid_camera",
            "direction is the zero vector; the row was skipped".into(),
        );
        return None;
    }
    let direction = Point3 {
        x: direction.x / length,
        y: direction.y / length,
        z: direction.z / length,
    };
    let height = if source.height.is_finite() && source.height > 0.0 {
        source.height
    } else {
        1.0
    };
    let ortho_scale = height / HEIGHT_TO_PIXEL_SCALE;
    let (camera, is_3d) = if source.perspective {
        let distance = height.max(1.0);
        let fov = lens_to_fov(source.lens_length);
        (
            ViewCamera {
                eye: Point3 {
                    x: target.x + direction.x * distance,
                    y: target.y + direction.y * distance,
                    z: target.z + direction.z * distance,
                },
                target,
                up: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
                projection: ViewProjection::Perspective {
                    vertical_fov_radians: fov,
                },
            },
            true,
        )
    } else {
        (
            ViewCamera {
                eye: Point3 {
                    x: target.x,
                    y: target.y,
                    z: target.z + 1.0,
                },
                target,
                up: Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
                projection: ViewProjection::Orthographic { scale: ortho_scale },
            },
            false,
        )
    };
    let saved_2d_camera = ViewCamera {
        eye: Point3 {
            x: target.x,
            y: target.y,
            z: target.z + 1.0,
        },
        target,
        up: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        projection: ViewProjection::Orthographic { scale: ortho_scale },
    };
    let work_plane = if source.ucs_associated {
        WorkPlane {
            origin: p3(source.ucs_origin),
            u: p3(source.ucs_x_axis),
            v: p3(source.ucs_y_axis),
        }
    } else {
        WorkPlane {
            origin: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            u: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            v: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        }
    };
    let view = NamedView {
        name,
        camera,
        is_3d,
        saved_2d_camera,
        work_plane,
        space: SpaceId::Model,
    };
    if let Err(error) = view.validate() {
        note(
            "import.view_invalid_camera",
            format!("the converted row is not usable and was skipped: {error}"),
        );
        return None;
    }
    note(
        "import.view_camera_approximated",
        "the source stores a world-space height, not a per-viewport scale; the \
         orthographic scale was derived as height / 1000 and the 3D eye distance from the \
         same height"
            .into(),
    );
    Some(view)
}

fn lens_to_fov(lens_length: f64) -> f64 {
    let lens = if lens_length.is_finite() && lens_length > 0.0 {
        lens_length
    } else {
        50.0
    };
    // 24 mm is the vertical sensor dimension AutoCAD's lens length is quoted
    // against; the result is clamped into the renderer's accepted FOV range.
    let fov = 2.0 * (12.0 / lens).atan();
    fov.clamp(1e-3, std::f64::consts::PI - 1e-3)
}

fn finite(point: Point3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}
