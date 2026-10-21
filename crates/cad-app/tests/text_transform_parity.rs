//! Cross-face `Text` transform parity (ADR 0019 decision B).
//!
//! The repository has **two** transform surfaces: `cad_db::transform_geometry`
//! (the write path) and `cad_geometry`'s `GeometryEngine::transform` (the snap
//! path). Before ADR 0019 the snap face already transformed `Text` while the
//! write face refused it, so one input produced two different answers — exactly
//! the inconsistency ADR 0017 A.5 warned about.
//!
//! This test builds a `Text` once and applies the **same** transform through
//! both faces, asserting the results are identical (and that both refuse the
//! same non-similarity inputs with `Unsupported`). It lives in `cad-app` because
//! that is the lowest crate that depends on both surfaces.

use cad_domain::{Point3, SemanticGeometry, StyleId, TextAlignH, TextAlignV, Transform3};
use cad_geometry::{DefaultGeometryEngine, GeometryEngine};

fn text() -> SemanticGeometry {
    SemanticGeometry::Text {
        text: "Parity 42".into(),
        position: Point3 {
            x: 3.0,
            y: -4.0,
            z: 1.5,
        },
        style: StyleId(2),
        height: 2.5,
        // A non-zero source rotation makes the accumulation observable.
        rotation: 0.3,
        font: Some("arial.ttf".into()),
        h_align: TextAlignH::Center,
        v_align: TextAlignV::Middle,
        wrap_width: Some(4.0),
    }
}

fn transform_through_both(transform: &Transform3) -> (SemanticGeometry, SemanticGeometry) {
    let write = cad_db::transform_geometry(&text(), transform).expect("the write face accepts it");
    let snap = DefaultGeometryEngine
        .transform(&text(), transform)
        .expect("the snap face accepts it");
    (write, snap)
}

#[test]
fn translation_rotation_and_uniform_scale_agree_across_both_faces() {
    let quarter_turn = Transform3 {
        matrix: [
            [0.0, -1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    // A composed translation + quarter turn + uniform scale about the origin.
    let composed = Transform3 {
        matrix: [
            [0.0, -2.0, 0.0, 10.0],
            [2.0, 0.0, 0.0, -5.0],
            [0.0, 0.0, 2.0, 1.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    for (label, transform) in [
        (
            "translation",
            Transform3::translation(Point3 {
                x: 10.0,
                y: -5.0,
                z: 2.0,
            }),
        ),
        ("rotation", quarter_turn),
        ("uniform scale", Transform3::scale(3.0)),
        ("composed", composed),
    ] {
        let (write, snap) = transform_through_both(&transform);
        let SemanticGeometry::Text {
            text: write_text,
            position: write_position,
            style: write_style,
            height: write_height,
            rotation: write_rotation,
            font: write_font,
            h_align: write_h,
            v_align: write_v,
            wrap_width: write_wrap,
        } = &write
        else {
            panic!("{label}: the write face must return Text, got {write:?}");
        };
        let SemanticGeometry::Text {
            text: snap_text,
            position: snap_position,
            style: snap_style,
            height: snap_height,
            rotation: snap_rotation,
            font: snap_font,
            h_align: snap_h,
            v_align: snap_v,
            wrap_width: snap_wrap,
        } = &snap
        else {
            panic!("{label}: the snap face must return Text, got {snap:?}");
        };
        assert_eq!(write_text, snap_text, "{label}: value");
        assert_eq!(write_position, snap_position, "{label}: position");
        assert_eq!(write_style, snap_style, "{label}: style");
        assert_eq!(write_height, snap_height, "{label}: height");
        assert_eq!(write_rotation, snap_rotation, "{label}: rotation");
        assert_eq!(write_font, snap_font, "{label}: font");
        assert_eq!(write_h, snap_h, "{label}: horizontal alignment");
        assert_eq!(write_v, snap_v, "{label}: vertical alignment");
        assert_eq!(write_wrap, snap_wrap, "{label}: wrap boundary");
        // The parity is not vacuous: the source really moved/scaled.
        assert!(
            (*write_height - 2.5 * scale_of(&transform)).abs() < 1e-12,
            "{label}: height {} vs expected {}",
            write_height,
            2.5 * scale_of(&transform)
        );
    }
}

/// The uniform scale of an accepted similarity (mean column length), mirroring
/// the two faces' shared convention.
fn scale_of(transform: &Transform3) -> f64 {
    let m = &transform.matrix;
    let sx = (m[0][0] * m[0][0] + m[1][0] * m[1][0] + m[2][0] * m[2][0]).sqrt();
    let sy = (m[0][1] * m[0][1] + m[1][1] * m[1][1] + m[2][1] * m[2][1]).sqrt();
    let sz = (m[0][2] * m[0][2] + m[1][2] * m[1][2] + m[2][2] * m[2][2]).sqrt();
    (sx + sy + sz) / 3.0
}

#[test]
fn both_faces_refuse_non_uniform_scale_and_mirror_with_the_same_outcome() {
    let mut non_uniform = Transform3::identity().matrix;
    non_uniform[0][0] = 2.0;
    non_uniform[1][1] = 0.5;
    for (label, transform) in [
        (
            "non-uniform scale",
            Transform3 {
                matrix: non_uniform,
            },
        ),
        ("mirror", Transform3::scale(-1.0)),
    ] {
        let write = cad_db::transform_geometry(&text(), &transform);
        let snap = DefaultGeometryEngine.transform(&text(), &transform);
        assert!(
            matches!(write, Err(cad_domain::CadError::Unsupported(_))),
            "{label}: the write face must refuse, got {write:?}"
        );
        assert!(
            matches!(snap, Err(cad_domain::CadError::Unsupported(_))),
            "{label}: the snap face must refuse, got {snap:?}"
        );
    }
}
