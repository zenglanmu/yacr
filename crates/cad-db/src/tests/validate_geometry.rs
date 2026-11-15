//! `validate_geometry` contract tests.
//!
//! Moved out of `validate.rs` when the source file reached the 1000-line gate.

use crate::validate::{
    validate_geometry, REASON_POLYLINE_WIDTH, REASON_TEXT_LINE_SPACING, REASON_TEXT_WIDTH_FACTOR,
};
use cad_domain::*;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

#[test]
fn rejects_non_finite_and_degenerate_geometry() {
    assert!(validate_geometry(&SemanticGeometry::Line {
        start: p(0.0, 0.0),
        end: p(1.0, 0.0)
    })
    .is_ok());
    assert!(validate_geometry(&SemanticGeometry::Line {
        start: p(0.0, 0.0),
        end: p(0.0, 0.0)
    })
    .is_err());
    assert!(validate_geometry(&SemanticGeometry::Line {
        start: p(f64::NAN, 0.0),
        end: p(1.0, 0.0)
    })
    .is_err());
    assert!(validate_geometry(&SemanticGeometry::Circle {
        center: p(0.0, 0.0),
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0
        },
        radius: 0.0
    })
    .is_err());
    assert!(validate_geometry(&SemanticGeometry::Polyline {
        points: vec![p(0.0, 0.0)],
        bulges: Vec::new(),
        closed: false,
        width: 0.0,
    })
    .is_err());
    // A stored wrap boundary must be usable; `None` means "no wrap".
    let text = |wrap_width| SemanticGeometry::Text {
        text: "A".into(),
        position: p(0.0, 0.0),
        style: cad_domain::StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: cad_domain::TextAlignH::Left,
        v_align: cad_domain::TextAlignV::Baseline,
        wrap_width,
        width_factor: 1.0,
        line_spacing: cad_domain::TextLineSpacing::default(),
    };
    assert!(validate_geometry(&text(None)).is_ok());
    assert!(validate_geometry(&text(Some(4.0))).is_ok());
    assert!(validate_geometry(&text(Some(0.0))).is_err());
    assert!(validate_geometry(&text(Some(-1.0))).is_err());
    assert!(validate_geometry(&text(Some(f64::NAN))).is_err());
}

#[test]
fn text_width_factor_must_be_positive_and_finite() {
    let text = |width_factor| SemanticGeometry::Text {
        text: "A".into(),
        position: p(0.0, 0.0),
        style: cad_domain::StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: cad_domain::TextAlignH::Left,
        v_align: cad_domain::TextAlignV::Baseline,
        wrap_width: None,
        width_factor,
        line_spacing: cad_domain::TextLineSpacing::default(),
    };
    // The neutral factor and a genuine stretch are both representable.
    assert!(validate_geometry(&text(1.0)).is_ok());
    assert!(validate_geometry(&text(2.5)).is_ok());
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let error = validate_geometry(&text(bad)).expect_err("rejected");
        let message = error.to_string();
        assert!(
            message.contains(REASON_TEXT_WIDTH_FACTOR),
            "the refusal must carry the stable reason: {message}"
        );
    }
}

#[test]
fn text_line_spacing_factor_must_be_positive_and_finite() {
    let text = |line_spacing| SemanticGeometry::Text {
        text: "A".into(),
        position: p(0.0, 0.0),
        style: cad_domain::StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: cad_domain::TextAlignH::Left,
        v_align: cad_domain::TextAlignV::Baseline,
        wrap_width: None,
        width_factor: 1.0,
        line_spacing,
    };
    // The neutral spacing, a compressed one and a stretched one with the
    // "exactly" style are all representable.
    assert!(validate_geometry(&text(cad_domain::TextLineSpacing::default())).is_ok());
    assert!(validate_geometry(&text(cad_domain::TextLineSpacing {
        factor: 0.5,
        style: cad_domain::TextLineSpacingStyle::Exactly,
    }))
    .is_ok());
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let error = validate_geometry(&text(cad_domain::TextLineSpacing {
            factor: bad,
            style: cad_domain::TextLineSpacingStyle::AtLeast,
        }))
        .expect_err("rejected");
        let message = error.to_string();
        assert!(
            message.contains(REASON_TEXT_LINE_SPACING),
            "the refusal must carry the stable reason: {message}"
        );
    }
}

#[test]
fn polyline_width_defaults_to_hairline_and_rejects_bad_values() {
    let polyline = |width| SemanticGeometry::Polyline {
        points: vec![p(0.0, 0.0), p(1.0, 0.0)],
        bulges: vec![0.0, 0.0],
        closed: false,
        width,
    };
    // `0.0` is the hairline default; a positive uniform width is a real
    // stroke and both are representable.
    assert!(validate_geometry(&polyline(0.0)).is_ok());
    assert!(validate_geometry(&polyline(0.75)).is_ok());
    for bad in [-1.0, f64::NAN, f64::INFINITY] {
        let error = validate_geometry(&polyline(bad)).expect_err("rejected");
        let message = error.to_string();
        assert!(
            message.contains(REASON_POLYLINE_WIDTH),
            "the refusal must carry the stable reason: {message}"
        );
    }
}

#[test]
fn accepts_well_formed_image_and_mask() {
    let image = SemanticGeometry::Image {
        origin: p(1.0, 2.0),
        u: p(0.5, 0.0),
        v: p(0.0, 0.25),
        pixels: [640.0, 480.0],
        file: None,
        clip: None,
        visible: true,
        adjust: ImageAdjust::default(),
    };
    assert!(validate_geometry(&image).is_ok());

    let mask = SemanticGeometry::Mask {
        boundary: vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0)],
        inverted: false,
    };
    assert!(validate_geometry(&mask).is_ok());
}

#[test]
fn rejects_malformed_image_and_mask() {
    // An image with non-positive pixel dimensions cannot be placed.
    assert!(validate_geometry(&SemanticGeometry::Image {
        origin: p(0.0, 0.0),
        u: p(1.0, 0.0),
        v: p(0.0, 1.0),
        pixels: [0.0, 10.0],
        file: None,
        clip: None,
        visible: true,
        adjust: ImageAdjust::default(),
    })
    .is_err());
    // A non-finite edge vector is rejected.
    assert!(validate_geometry(&SemanticGeometry::Image {
        origin: p(0.0, 0.0),
        u: p(f64::NAN, 0.0),
        v: p(0.0, 1.0),
        pixels: [10.0, 10.0],
        file: None,
        clip: None,
        visible: true,
        adjust: ImageAdjust::default(),
    })
    .is_err());
    // A mask needs at least three points to bound an area.
    assert!(validate_geometry(&SemanticGeometry::Mask {
        boundary: vec![p(0.0, 0.0), p(1.0, 1.0)],
        inverted: false,
    })
    .is_err());
}

#[test]
fn rejects_out_of_range_image_adjustment_and_accepts_the_edges() {
    let image_with = |adjust: ImageAdjust| SemanticGeometry::Image {
        origin: p(0.0, 0.0),
        u: p(1.0, 0.0),
        v: p(0.0, 1.0),
        pixels: [10.0, 10.0],
        file: None,
        clip: None,
        visible: true,
        adjust,
    };
    // 0 and 100 are the real DXF percentage bounds.
    for adjust in [
        ImageAdjust {
            brightness: 0,
            contrast: 100,
            fade: 0,
            monochrome: true,
        },
        ImageAdjust {
            brightness: 100,
            contrast: 0,
            fade: 100,
            monochrome: false,
        },
    ] {
        assert!(validate_geometry(&image_with(adjust)).is_ok(), "{adjust:?}");
    }
    // `u8` can hold >100; validation rejects it instead of clamping.
    for adjust in [
        ImageAdjust {
            brightness: 101,
            ..ImageAdjust::default()
        },
        ImageAdjust {
            contrast: 200,
            ..ImageAdjust::default()
        },
        ImageAdjust {
            fade: 255,
            ..ImageAdjust::default()
        },
    ] {
        assert!(
            validate_geometry(&image_with(adjust)).is_err(),
            "{adjust:?} must be rejected"
        );
    }
}
