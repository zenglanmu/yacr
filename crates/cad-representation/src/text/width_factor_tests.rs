//! Entity-level horizontal width factor through the shaping entry point.
//!
//! `SemanticGeometry::Text::width_factor` is composed into every parsed run by
//! [`FontEngine::shape`], so the glyph advances and glyph x coordinates are
//! multiplied while the height and anchor stay put.

use super::*;
use cad_domain::{Point3, TextAlignH, TextAlignV};
use std::sync::Arc;

#[test]
fn entity_width_factor_multiplies_glyph_advances() {
    let mut engine = FontEngine::new();
    engine
        .register(
            "osifont.ttf",
            Arc::from(include_bytes!("../../../../fonts/osifont.ttf").as_slice()),
        )
        .unwrap();
    engine.set_fallback(vec!["osifont.ttf".into()]);

    let span = |width_factor: f64| {
        let shaped = engine
            .shape(
                "osifont.ttf",
                "AB",
                Point3::default(),
                10.0,
                0.0,
                TextAlignH::Left,
                TextAlignV::Baseline,
                None,
                width_factor,
            )
            .expect("the bundled face shapes");
        let max_x = shaped
            .outlines
            .iter()
            .flatten()
            .map(|p| p.x)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(max_x > 0.0, "the run advances to the right");
        max_x
    };

    let neutral = span(1.0);
    let stretched = span(2.0);
    assert!(
        (stretched - neutral * 2.0).abs() < 1e-6,
        "a width factor of 2 doubles every glyph advance: {neutral} -> {stretched}"
    );
    // The shaped runs carry the composed factor too, so hosts that apply it
    // downstream do not divide it away.
    let shaped = engine
        .shape(
            "osifont.ttf",
            "AB",
            Point3::default(),
            10.0,
            0.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            None,
            1.25,
        )
        .expect("shapes");
    assert!(shaped
        .runs
        .iter()
        .all(|run| (run.width_factor - 1.25).abs() < 1e-12));
}
