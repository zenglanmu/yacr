//! Annotation payloads and their validation.

use cad_domain::*;

use crate::math::{add, dot, length, sub};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorStatus {
    Valid,
    Stale,
    Unresolved,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntityAnchor {
    pub source_handle: String,
    pub instance: InstancePath,
    pub sub_element: Option<SubElementId>,
    pub fallback: Point3,
    pub status: AnchorStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationGeometry {
    Text(Point3),
    Leader(Vec<Point3>),
    Rectangle([Point3; 2]),
    Ellipse {
        center: Point3,
        axis_u: Point3,
        axis_v: Point3,
    },
    Freehand(Vec<Point3>),
    Cloud(Vec<Point3>),
    Measurement(MeasurementRecord),
}

impl AnnotationGeometry {
    /// All world points, used for bounds and cache invalidation.
    pub fn points(&self) -> Vec<Point3> {
        match self {
            AnnotationGeometry::Text(p) => vec![*p],
            AnnotationGeometry::Leader(v)
            | AnnotationGeometry::Freehand(v)
            | AnnotationGeometry::Cloud(v) => v.clone(),
            AnnotationGeometry::Rectangle(pair) => pair.to_vec(),
            AnnotationGeometry::Ellipse {
                center,
                axis_u,
                axis_v,
            } => vec![
                add(*center, *axis_u),
                add(*center, *axis_v),
                sub(*center, *axis_u),
                sub(*center, *axis_v),
            ],
            AnnotationGeometry::Measurement(m) => m.inputs.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MeasurementAlgorithm {
    Distance2d,
    Distance3d,
    PolylineLength,
    Angle3Points,
    PlanarPolygonArea,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeasurementRecord {
    pub algorithm: MeasurementAlgorithm,
    pub inputs: Vec<Point3>,
    pub plane: Option<WorkPlane>,
    pub value: f64,
    pub units: UnitContext,
    pub source: GeometrySource,
    pub precision: Precision,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationStyle {
    pub rgba: [u8; 4],
    pub logical_width: f64,
    pub text_height: f64,
}

impl Default for AnnotationStyle {
    fn default() -> Self {
        AnnotationStyle {
            rgba: [0xE5, 0x39, 0x35, 0xFF],
            logical_width: 2.0,
            text_height: 2.5,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    /// UUID supplied by the host; never a GPU index.
    pub id: AnnotationId,
    pub space: SpaceId,
    pub geometry: AnnotationGeometry,
    pub text: String,
    pub style: AnnotationStyle,
    pub created_unix_ms: i64,
    pub modified_unix_ms: i64,
    pub anchor: Option<EntityAnchor>,
    pub precision: Precision,
}

/// Validate an annotation before it is written (audit B12).
///
/// Checks id/geometry finiteness, style bounds and anchor references. This is
/// deliberately conservative: a value that cannot be drawn or measured safely
/// is refused rather than stored.
pub fn validate_annotation(annotation: &Annotation) -> CadResult<()> {
    fn finite(p: Point3) -> bool {
        p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
    }
    let geometry_ok = match &annotation.geometry {
        AnnotationGeometry::Text(p) => finite(*p),
        AnnotationGeometry::Leader(v)
        | AnnotationGeometry::Freehand(v)
        | AnnotationGeometry::Cloud(v) => !v.is_empty() && v.iter().all(|p| finite(*p)),
        AnnotationGeometry::Rectangle(pair) => finite(pair[0]) && finite(pair[1]),
        AnnotationGeometry::Ellipse {
            center,
            axis_u,
            axis_v,
        } => {
            finite(*center)
                && finite(*axis_u)
                && finite(*axis_v)
                // A zero-length axis collapses the ellipse; refuse it rather
                // than storing an undrawable annotation.
                && length(*axis_u) > 1e-12
                && length(*axis_v) > 1e-12
        }
        AnnotationGeometry::Measurement(record) => finite_measurement(record),
    };
    if !geometry_ok {
        return Err(CadError::Invariant(
            "annotation geometry is degenerate or non-finite".to_string(),
        ));
    }
    if !finite_style(&annotation.style) {
        return Err(CadError::Invariant(
            "annotation style has non-finite or negative values".to_string(),
        ));
    }
    // Timestamps: modified must not precede created.
    if annotation.modified_unix_ms < annotation.created_unix_ms {
        return Err(CadError::Invariant(
            "annotation modified time precedes created time".to_string(),
        ));
    }
    if let Some(anchor) = &annotation.anchor {
        if anchor.source_handle.trim().is_empty() {
            return Err(CadError::Invariant(
                "annotation anchor has an empty source handle".to_string(),
            ));
        }
        if !finite(anchor.fallback) {
            return Err(CadError::Invariant(
                "annotation anchor fallback is non-finite".to_string(),
            ));
        }
    }
    Ok(())
}

fn finite_style(style: &AnnotationStyle) -> bool {
    style.logical_width.is_finite()
        && style.logical_width >= 0.0
        && style.text_height.is_finite()
        && style.text_height >= 0.0
}

fn finite_measurement(record: &MeasurementRecord) -> bool {
    if !record.value.is_finite()
        || !record
            .inputs
            .iter()
            .all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite())
    {
        return false;
    }
    if let Some(plane) = &record.plane {
        let finite = |p: Point3| p.x.is_finite() && p.y.is_finite() && p.z.is_finite();
        if !finite(plane.origin) || !finite(plane.u) || !finite(plane.v) {
            return false;
        }
        // A degenerate or skewed basis cannot support a projected area.
        let lu = length(plane.u);
        let lv = length(plane.v);
        if lu < 1e-12 || lv < 1e-12 {
            return false;
        }
        let ortho = dot(plane.u, plane.v).abs() / (lu * lv);
        if ortho > 1e-6 {
            return false;
        }
    }
    true
}
