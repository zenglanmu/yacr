//! Units, error construction, geometry helpers and the actual `skpwright` call.
//!
//! Kept apart from the report/model assembly so the conversion arithmetic and the
//! one `Result` boundary into a third-party crate are readable on their own.

use cad_domain::{CadError, CadResult, Unit, UnitContext};
use cad_connector_core::ExportJob;

/// The exporter was handed a registry key it does not write.
pub const MISMATCHED_DESCRIPTOR: &str = "export.skp.mismatched_descriptor";
/// The job carries no usable source unit, so a metre scale cannot be derived.
pub const UNITS_UNKNOWN: &str = "export.skp.units_unknown";
/// A source geometry has no SketchUp mesh representation.
pub const UNSUPPORTED_GEOMETRY: &str = "export.skp.unsupported_geometry";
/// A triangle has zero area and cannot become a SketchUp face.
pub const DEGENERATE_TRIANGLE: &str = "export.skp.degenerate_triangle";
/// Layers/materials/colours were flattened onto the default tag.
pub const LAYER_MATERIAL_FLATTENED: &str = "export.skp.layer_material_flattened";
/// Per-vertex source colours were dropped; SketchUp faces carry no vertex colours.
pub const COLORS_DROPPED: &str = "export.skp.colors_dropped";
/// Per-vertex source normals were dropped; every face gets its own Newell normal.
pub const NORMALS_DROPPED: &str = "export.skp.normals_dropped";
/// A written mesh references a base-colour texture; the writer carries no texture.
pub const TEXTURE_NOT_WRITTEN: &str = "export.skp.texture_not_written";
/// A non-model-space entity was left out (the writer has no paper space).
pub const PAPER_SPACE_DROPPED: &str = "export.skp.paper_space_dropped";
/// A caller budget (`ExportLimits`) was exceeded; nothing was written.
pub const LIMITS_EXCEEDED: &str = "export.skp.limits_exceeded";
/// The database holds nothing to write at all.
pub const NO_GEOMETRY: &str = "export.skp.no_geometry";
/// `skpwright` refused the model (`Document::from_model_with`).
pub const MODEL_REJECTED: &str = "export.skp.model_rejected";
/// `skpwright` failed while serializing the container.
pub const WRITE_FAILED: &str = "export.skp.write_failed";

/// Fixed save time recorded in every written file (2023-11-14T22:13:20Z).
///
/// A system-clock value would make two exports of the same document differ, so
/// determinism wins over "when was this written"; the model's own `saved_at`
/// semantics are not document content here.
pub const WRITE_UNIX_TIME: u32 = 1_700_000_000;

/// Fixed GUID/header entropy seed; see [`export_env`].
pub const WRITE_SEED: u64 = 0x5941_4352_534b_5000;

/// Stable refusal in the `CadError::Unsupported("<stable code>: <detail>")` shape
/// every exporter uses.
pub(crate) fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{code}: {detail}"))
}

/// The declared source-unit scale: metres per one `cad-db` coordinate unit.
///
/// `skpwright` model vertices are **metres** and it writes inches internally, so
/// the scale is mandatory. `Unit::DrawingUnits`, a missing context or a
/// non-finite factor is refused: assuming millimetres or metres would silently
/// rescale the model.
pub fn metres_per_source_unit(units: Option<&UnitContext>) -> CadResult<f64> {
    let units = units.ok_or_else(|| {
        refusal(
            UNITS_UNKNOWN,
            "the job has no unit context; SketchUp vertices are metres and the source scale \
             cannot be assumed",
        )
    })?;
    let scale: f64 = match units.source {
        Unit::Millimeter => 0.001,
        Unit::Centimeter => 0.01,
        Unit::Meter => 1.0,
        Unit::Inch => 0.0254,
        Unit::Foot => 0.3048,
        Unit::DrawingUnits => {
            return Err(refusal(
                UNITS_UNKNOWN,
                "source unit is DrawingUnits; the document does not declare a length",
            ))
        }
    };
    if !scale.is_finite() || scale <= 0.0 {
        return Err(refusal(
            UNITS_UNKNOWN,
            format!("source unit scale {scale} is not a finite positive ratio"),
        ));
    }
    Ok(scale)
}

/// The authoring environment for `Document::from_model_with`.
///
/// The save time is the fixed [`WRITE_UNIX_TIME`]; the seed is the fixed
/// [`WRITE_SEED`] mixed with the document id, so the same document always writes
/// the same bytes while two documents do not share a model GUID.
pub fn export_env(document: cad_domain::DocumentId) -> skpwright::WriteEnv {
    let id = document.0 as u64 ^ (document.0 >> 64) as u64;
    skpwright::WriteEnv {
        unix_time: WRITE_UNIX_TIME,
        seed: WRITE_SEED ^ id,
    }
}

/// Author the 2026 container bytes for `model`.
pub(crate) fn write_model(model: &skpwright::Model, job: &ExportJob<'_>) -> CadResult<Vec<u8>> {
    let document = skpwright::Document::from_model_with(model, export_env(job.document_id))
        .map_err(|error| refusal(MODEL_REJECTED, error))?;
    document
        .write()
        .map_err(|error| refusal(WRITE_FAILED, error))
}

/// Unit Newell normal of a triangle: `u × v` normalized, `None` when the triangle
/// has zero (or non-finite) area.
///
/// Newell's method on a three-point ring reduces to the cross product of two of
/// its edges, which is what `skpwright`'s own faces carry.
pub(crate) fn triangle_normal(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Option<[f64; 3]> {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let normal = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let length_squared = normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2];
    if !length_squared.is_finite() || length_squared <= 0.0 {
        return None;
    }
    let length = length_squared.sqrt();
    let unit = [normal[0] / length, normal[1] / length, normal[2] / length];
    unit.iter().all(|value| value.is_finite()).then_some(unit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::Unit;

    fn units(source: Unit) -> UnitContext {
        UnitContext {
            source: source.clone(),
            display: source,
            display_per_source: Some(1.0),
            decimal_places: 3,
        }
    }

    #[test]
    fn every_declared_unit_maps_to_metres() {
        assert_eq!(
            metres_per_source_unit(Some(&units(Unit::Millimeter))).unwrap(),
            0.001
        );
        assert_eq!(
            metres_per_source_unit(Some(&units(Unit::Centimeter))).unwrap(),
            0.01
        );
        assert_eq!(
            metres_per_source_unit(Some(&units(Unit::Meter))).unwrap(),
            1.0
        );
        assert_eq!(
            metres_per_source_unit(Some(&units(Unit::Inch))).unwrap(),
            0.0254
        );
        assert_eq!(
            metres_per_source_unit(Some(&units(Unit::Foot))).unwrap(),
            0.3048
        );
    }

    #[test]
    fn drawing_units_and_missing_context_are_refused() {
        for context in [Some(units(Unit::DrawingUnits)), None] {
            let error = metres_per_source_unit(context.as_ref()).unwrap_err();
            let CadError::Unsupported(message) = error else {
                panic!("expected Unsupported");
            };
            assert!(message.starts_with(UNITS_UNKNOWN), "{message}");
        }
    }

    #[test]
    fn a_triangle_normal_is_unit_length_and_right_handed() {
        let normal = triangle_normal([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        assert_eq!(normal, [0.0, 0.0, 1.0]);
        let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        assert!((length - 1.0).abs() < 1e-12);
    }

    #[test]
    fn collinear_and_repeated_vertices_have_no_normal() {
        assert!(triangle_normal([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]).is_none());
        assert!(triangle_normal([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0]).is_none());
        assert!(
            triangle_normal([0.0, 0.0, 0.0], [f64::INFINITY, 0.0, 0.0], [0.0, 1.0, 0.0]).is_none()
        );
    }

    #[test]
    fn the_authoring_environment_is_fixed_per_document() {
        let first = export_env(cad_domain::DocumentId(7));
        let again = export_env(cad_domain::DocumentId(7));
        let other = export_env(cad_domain::DocumentId(8));
        assert_eq!(first, again);
        assert_ne!(first, other);
        assert_eq!(first.unix_time, WRITE_UNIX_TIME);
    }
}
