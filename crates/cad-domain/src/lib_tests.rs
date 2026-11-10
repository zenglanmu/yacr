#[cfg(test)]
mod tests {
    use crate::*;

    fn point(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    fn sample_image() -> SemanticGeometry {
        SemanticGeometry::Image {
            origin: point(1.0, 2.0, 0.0),
            u: point(0.5, 0.0, 0.0),
            v: point(0.0, 0.25, 0.0),
            pixels: [640.0, 480.0],
            file: Some("logo.png".to_string()),
            clip: Some(ImageClip {
                vertices: vec![[0.0, 0.0], [640.0, 0.0], [640.0, 480.0]],
                inside: true,
            }),
            visible: true,
            adjust: ImageAdjust::default(),
        }
    }

    fn sample_mask() -> SemanticGeometry {
        SemanticGeometry::Mask {
            boundary: vec![
                point(0.0, 0.0, 0.0),
                point(10.0, 0.0, 0.0),
                point(10.0, 10.0, 0.0),
                point(0.0, 10.0, 0.0),
            ],
            inverted: false,
        }
    }

    #[test]
    fn image_variant_constructs_and_compares() {
        let image = sample_image();
        assert_eq!(image.clone(), image);
        match &image {
            SemanticGeometry::Image {
                origin,
                pixels,
                file,
                clip,
                visible,
                ..
            } => {
                assert_eq!(*origin, point(1.0, 2.0, 0.0));
                assert_eq!(*pixels, [640.0, 480.0]);
                assert_eq!(file.as_deref(), Some("logo.png"));
                assert!(clip.as_ref().expect("clip").inside);
                assert!(*visible);
            }
            other => panic!("expected Image, got {other:?}"),
        }
    }

    #[test]
    fn image_clip_constructs_and_compares() {
        let clip = ImageClip {
            vertices: vec![[1.0, 2.0], [3.0, 4.0]],
            inside: false,
        };
        assert_eq!(clip.clone(), clip);
        assert_eq!(
            clip,
            ImageClip {
                vertices: vec![[1.0, 2.0], [3.0, 4.0]],
                inside: false,
            }
        );
        assert_ne!(
            clip,
            ImageClip {
                vertices: vec![[1.0, 2.0], [3.0, 4.0]],
                inside: true,
            }
        );
    }

    #[test]
    fn mask_variant_constructs_and_compares() {
        let mask = sample_mask();
        assert_eq!(mask.clone(), mask);
        match &mask {
            SemanticGeometry::Mask { boundary, inverted } => {
                assert_eq!(boundary.len(), 4);
                assert!(!*inverted);
            }
            other => panic!("expected Mask, got {other:?}"),
        }
    }

    #[test]
    fn apply_vector_is_the_linear_part_of_apply_point() {
        // A uniform scale composed with a translation: the linear part is the
        // scale, so the point/vector difference is exactly the translation.
        let t = Transform3::translation(point(3.0, -4.0, 5.0)).matrix_mul(&Transform3::scale(2.0));
        let v = point(1.5, -2.0, 0.25);
        let p = t.apply_point(v);
        let linear = t.apply_vector(v);
        assert_eq!(linear, point(3.0, -4.0, 0.5));
        assert_eq!(p.x - linear.x, 3.0);
        assert_eq!(p.y - linear.y, -4.0);
        assert_eq!(p.z - linear.z, 5.0);
        // A pure translation leaves every vector unchanged.
        let tr = Transform3::translation(point(9.0, 8.0, 7.0));
        assert_eq!(tr.apply_vector(v), v);
        assert_eq!(tr.apply_point(v), point(10.5, 6.0, 7.25));
    }

    #[test]
    fn apply_vector_composes_through_nested_transforms() {
        // Non-uniform scale (2, 0.5) as the inner (block) transform, a rotation
        // about Z plus a translation as the outer (insert) transform.
        let inner = Transform3::from_basis(
            point(2.0, 0.0, 0.0),
            point(0.0, 0.5, 0.0),
            point(1.0, 1.0, 0.0),
        );
        let (s, c) = (
            std::f64::consts::FRAC_1_SQRT_2,
            std::f64::consts::FRAC_1_SQRT_2,
        );
        let rotation =
            Transform3::from_basis(point(c, s, 0.0), point(-s, c, 0.0), point(10.0, -3.0, 2.0));
        let composed = rotation.matrix_mul(&inner);
        let v = point(1.0, -2.0, 0.75);
        let nested = rotation.apply_vector(inner.apply_vector(v));
        let direct = composed.apply_vector(v);
        assert!(
            (direct.x - nested.x).abs() < 1e-9,
            "{direct:?} vs {nested:?}"
        );
        assert!(
            (direct.y - nested.y).abs() < 1e-9,
            "{direct:?} vs {nested:?}"
        );
        assert!(
            (direct.z - nested.z).abs() < 1e-9,
            "{direct:?} vs {nested:?}"
        );
        // The composed translation is still excluded from a vector.
        let via_point = composed.apply_point(v);
        assert!((via_point.x - direct.x - composed.matrix[0][3]).abs() < 1e-9);
        assert!((via_point.y - direct.y - composed.matrix[1][3]).abs() < 1e-9);
        assert!((via_point.z - direct.z - composed.matrix[2][3]).abs() < 1e-9);
    }
    #[test]
    fn image_adjust_default_is_the_neutral_transfer() {
        let adjust = ImageAdjust::default();
        assert_eq!(
            adjust,
            ImageAdjust {
                brightness: 50,
                contrast: 50,
                fade: 0,
                monochrome: false,
            }
        );
        assert_eq!(adjust, ImageAdjust::default(), "Copy/Eq plain data");
        // The neutral transfer is what an untouched imported image carries.
        match sample_image() {
            SemanticGeometry::Image { adjust, .. } => {
                assert_eq!(adjust, ImageAdjust::default());
            }
            other => panic!("expected Image, got {other:?}"),
        }
    }
}
