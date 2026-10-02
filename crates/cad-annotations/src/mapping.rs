//! Validated affine mapping applied when importing a mismatched file (audit B10).

use super::*;

/// A validated affine mapping applied when importing a mismatched file.
pub(crate) struct CoordinateMapping {
    transform: Transform3,
}

impl CoordinateMapping {
    /// Reject non-finite or singular (non-invertible) mappings (audit B10).
    pub(crate) fn validate(transform: &Transform3) -> CadResult<Self> {
        for row in &transform.matrix {
            for v in row {
                if !v.is_finite() {
                    return Err(CadError::InvalidInput(
                        "coordinate mapping contains non-finite values".into(),
                    ));
                }
            }
        }
        if transform.determinant().abs() < 1e-12 {
            return Err(CadError::InvalidInput(
                "coordinate mapping is singular and cannot be applied".into(),
            ));
        }
        Ok(CoordinateMapping {
            transform: *transform,
        })
    }

    fn map_point(&self, p: Point3) -> CadResult<Point3> {
        let out = self.transform.apply_point(p);
        if !out.x.is_finite() || !out.y.is_finite() || !out.z.is_finite() {
            return Err(CadError::InvalidInput(
                "coordinate mapping produced a non-finite point".into(),
            ));
        }
        Ok(out)
    }

    /// Move all geometry and anchor fallbacks, then mark anchors Unresolved:
    /// the match was not verified, so they must not claim `Valid`.
    pub(crate) fn apply_annotation(&self, annotation: &mut Annotation) -> CadResult<()> {
        annotation.geometry = self.map_geometry(&annotation.geometry)?;
        if let Some(anchor) = annotation.anchor.as_mut() {
            anchor.fallback = self.map_point(anchor.fallback)?;
            anchor.status = AnchorStatus::Unresolved;
        }
        Ok(())
    }

    fn map_geometry(&self, geometry: &AnnotationGeometry) -> CadResult<AnnotationGeometry> {
        let map = |p: Point3| self.map_point(p);
        Ok(match geometry {
            AnnotationGeometry::Text(p) => AnnotationGeometry::Text(map(*p)?),
            AnnotationGeometry::Leader(v) => {
                AnnotationGeometry::Leader(v.iter().map(|p| map(*p)).collect::<CadResult<_>>()?)
            }
            AnnotationGeometry::Rectangle(pair) => {
                AnnotationGeometry::Rectangle([map(pair[0])?, map(pair[1])?])
            }
            AnnotationGeometry::Ellipse {
                center,
                axis_u,
                axis_v,
            } => AnnotationGeometry::Ellipse {
                center: map(*center)?,
                // Axes are directions: use the linear part without translation.
                axis_u: self.map_vector(*axis_u)?,
                axis_v: self.map_vector(*axis_v)?,
            },
            AnnotationGeometry::Freehand(v) => {
                AnnotationGeometry::Freehand(v.iter().map(|p| map(*p)).collect::<CadResult<_>>()?)
            }
            AnnotationGeometry::Cloud(v) => {
                AnnotationGeometry::Cloud(v.iter().map(|p| map(*p)).collect::<CadResult<_>>()?)
            }
            AnnotationGeometry::Measurement(m) => {
                let mut m = m.clone();
                m.inputs = m.inputs.iter().map(|p| map(*p)).collect::<CadResult<_>>()?;
                if let Some(plane) = m.plane.as_mut() {
                    plane.origin = map(plane.origin)?;
                    plane.u = self.map_vector(plane.u)?;
                    plane.v = self.map_vector(plane.v)?;
                }
                AnnotationGeometry::Measurement(m)
            }
        })
    }

    fn map_vector(&self, v: Point3) -> CadResult<Point3> {
        let origin = self.map_point(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        })?;
        let tip = self.map_point(v)?;
        let out = Point3 {
            x: tip.x - origin.x,
            y: tip.y - origin.y,
            z: tip.z - origin.z,
        };
        if !out.x.is_finite() || !out.y.is_finite() || !out.z.is_finite() {
            return Err(CadError::InvalidInput(
                "coordinate mapping produced a non-finite vector".into(),
            ));
        }
        Ok(out)
    }

    /// Transform a camera/placement transform by composing `mapping` on the
    /// left: the mapped world of the file feeds the original transform.
    pub(crate) fn map_transform(&self, t: &Transform3) -> CadResult<Transform3> {
        let composed = self.transform.matrix_mul(t);
        for row in &composed.matrix {
            for v in row {
                if !v.is_finite() {
                    return Err(CadError::InvalidInput(
                        "coordinate mapping produced a non-finite transform".into(),
                    ));
                }
            }
        }
        Ok(composed)
    }
}
