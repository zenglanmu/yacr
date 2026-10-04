//! Validated affine mapping applied when importing a mismatched file (audit B10).

use super::*;

/// A validated affine mapping applied when importing a mismatched file.
pub(crate) struct CoordinateMapping {
    transform: Transform3,
}

impl CoordinateMapping {
    /// Reject non-affine, non-finite or singular mappings (audit B10).
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
        // Point application assumes this canonical affine homogeneous row;
        // accepting a projective matrix would silently discard its perspective.
        if transform.matrix[3] != [0.0, 0.0, 0.0, 1.0] {
            return Err(CadError::InvalidInput(
                "coordinate mapping must have an affine homogeneous row".into(),
            ));
        }
        let determinant = transform.determinant();
        if !determinant.is_finite() {
            return Err(CadError::InvalidInput(
                "coordinate mapping has a non-finite linear determinant".into(),
            ));
        }
        if determinant.abs() < 1e-12 {
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
        // Subtracting two translated points loses small directions when the
        // translation is large. Directions only use the linear matrix part.
        let m = &self.transform.matrix;
        let out = Point3 {
            x: m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
            y: m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
            z: m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_vectors_use_rotation_shear_and_scale_without_translation() {
        let transform = Transform3 {
            matrix: [
                [0.0, -2.0, 0.5, 10.0],
                [3.0, 0.0, 0.0, -4.0],
                [0.0, 0.0, -1.0, 7.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        };
        let mapping = CoordinateMapping::validate(&transform).unwrap();
        let vector = Point3 {
            x: 1.0,
            y: 2.0,
            z: 4.0,
        };
        assert_eq!(
            mapping.map_vector(vector).unwrap(),
            Point3 {
                x: -2.0,
                y: 3.0,
                z: -4.0,
            }
        );
        assert_eq!(
            mapping.map_point(vector).unwrap(),
            Point3 {
                x: 8.0,
                y: -1.0,
                z: 3.0,
            }
        );
    }

    #[test]
    fn mapping_rejects_non_finite_inputs_and_vector_outputs() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut transform = Transform3::identity();
            transform.matrix[0][3] = value;
            assert!(matches!(
                CoordinateMapping::validate(&transform),
                Err(CadError::InvalidInput(_))
            ));
        }
        let mapping = CoordinateMapping::validate(&Transform3::scale(2.0)).unwrap();
        for value in [f64::MAX, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                mapping.map_vector(Point3 {
                    x: value,
                    y: 0.0,
                    z: 0.0,
                }),
                Err(CadError::InvalidInput(_))
            ));
        }
    }

    #[test]
    fn mapping_rejects_non_affine_homogeneous_rows() {
        for row in [
            [0.25, 0.0, 0.0, 1.0],
            [0.0, 0.25, 0.0, 1.0],
            [0.0, 0.0, 0.25, 1.0],
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 2.0],
        ] {
            let mut transform = Transform3::identity();
            transform.matrix[3] = row;
            assert!(matches!(
                CoordinateMapping::validate(&transform),
                Err(CadError::InvalidInput(_))
            ));
        }
    }

    #[test]
    fn mapping_rejects_non_finite_determinants_of_finite_matrices() {
        let infinite = Transform3::scale(1e200);
        let mut nan = infinite;
        nan.matrix[0][1] = 1e200;
        nan.matrix[1][0] = 1e200;
        nan.matrix[1][2] = 1e200;
        nan.matrix[2][1] = 1e200;
        for transform in [infinite, nan] {
            assert!(!transform.determinant().is_finite());
            assert!(matches!(
                CoordinateMapping::validate(&transform),
                Err(CadError::InvalidInput(_))
            ));
        }
    }
}
