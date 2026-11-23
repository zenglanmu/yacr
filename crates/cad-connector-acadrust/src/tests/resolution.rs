//! Style resolution (transparency/color/lineweight/linetype) and proxy fragments.

use super::*;
// ---- F14/B21: transparency resolution + proxy fragment preservation ----

#[test]
fn transparency_resolution_keeps_the_symbolic_by_layer_form() {
    // acadrust packs transparency as a byte: 0 opaque, 255 transparent.
    assert_eq!(
        resolve_entity_transparency(acadrust::Transparency::new(0)),
        EntityTransparency::Explicit(1.0)
    );
    // ByObject (Explicit) is the one form resolved here, byte-identically.
    assert_eq!(
        resolve_entity_transparency(acadrust::Transparency::new(128)),
        EntityTransparency::Explicit(1.0 - 128.0 / 255.0)
    );
    // ByLayer stays symbolic: the representation resolves it from the owning
    // layer's stored `LayerAttributes::opacity` at draw time, so a layer opacity
    // edit changes the entity without an entity write. (Before this change the
    // layer opacity was flattened into `Explicit` here, which froze it at import
    // time and made a later layer edit change nothing.)
    assert_eq!(
        resolve_entity_transparency(acadrust::Transparency::BY_LAYER),
        EntityTransparency::ByLayer
    );
    // ByBlock stays symbolic so INSERT expansion can supply the value.
    assert_eq!(
        resolve_entity_transparency(acadrust::Transparency::BY_BLOCK),
        EntityTransparency::ByBlock
    );
}

#[test]
fn layer_opacity_maps_dwg_bytes_to_opacity() {
    assert_eq!(layer_opacity(acadrust::Transparency::OPAQUE), 1.0);
    assert_eq!(layer_opacity(acadrust::Transparency::TRANSPARENT), 0.0);
    // A degenerate ByLayer/ByBlock on a layer is opaque, not fabricated.
    assert_eq!(layer_opacity(acadrust::Transparency::BY_LAYER), 1.0);
}

// ---- §3.2/§7.1: colour and lineweight resolution ----

#[test]
fn color_resolution_preserves_source_provenance() {
    // ByObject true colour wins over the layer and is stored byte-identically.
    assert_eq!(
        resolve_entity_color(acadrust::Color::from_rgb(10, 20, 30)),
        EntityColor::Explicit([10, 20, 30])
    );
    // An ACI index resolves through acadrust's canonical table, not a guess.
    assert_eq!(
        resolve_entity_color(acadrust::Color::Index(1)),
        EntityColor::Explicit([255, 0, 0])
    );
    // ByLayer stays symbolic: the representation resolves it against the owning
    // layer's `LayerAttributes::color.rgb` at draw time, so editing a layer
    // colour recolours the entity without an entity write. (Before this change
    // the importer flattened the layer RGB into `Explicit` here.)
    assert_eq!(
        resolve_entity_color(acadrust::Color::ByLayer),
        EntityColor::ByLayer
    );
    // A materialised `None` is DXF's "no colour", i.e. by layer; keeping it
    // symbolic is what makes it follow the layer instead of drawing black.
    assert_eq!(
        resolve_entity_color(acadrust::Color::None),
        EntityColor::ByLayer
    );
    // ByBlock stays symbolic so INSERT expansion can supply the value.
    assert_eq!(
        resolve_entity_color(acadrust::Color::ByBlock),
        EntityColor::ByBlock
    );
}

#[test]
fn layer_rgb_resolves_index_and_rgb_and_falls_back_to_white() {
    assert_eq!(layer_rgb(acadrust::Color::from_rgb(9, 8, 7)), [9, 8, 7]);
    assert_eq!(layer_rgb(acadrust::Color::Index(5)), [0, 0, 255]);
    // A degenerate symbolic layer colour falls back to white.
    assert_eq!(layer_rgb(acadrust::Color::ByLayer), [255, 255, 255]);
}

#[test]
fn lineweight_resolution_keeps_the_symbolic_by_layer_form() {
    // A concrete weight is 1/100 mm; 35 -> 0.35 mm, byte-identical.
    assert_eq!(
        resolve_entity_lineweight(acadrust::LineWeight::Value(35)),
        EntityLineWeight::Explicit(0.35)
    );
    // acadrust's Default keeps its explicit meaning.
    assert_eq!(
        resolve_entity_lineweight(acadrust::LineWeight::Default),
        EntityLineWeight::Default
    );
    // ByBlock stays symbolic so INSERT expansion can supply the value.
    assert_eq!(
        resolve_entity_lineweight(acadrust::LineWeight::ByBlock),
        EntityLineWeight::ByBlock
    );
    // ByLayer stays symbolic too: the representation resolves it from the
    // owning layer's stored `LayerLineWeight` at draw time. (Before this change
    // the layer's millimetres were flattened into `Explicit` here, so a layer
    // lineweight edit changed nothing.)
    assert_eq!(
        resolve_entity_lineweight(acadrust::LineWeight::ByLayer),
        EntityLineWeight::ByLayer
    );
}

#[test]
fn lineweight_mm_only_reports_concrete_values() {
    assert_eq!(lineweight_mm(acadrust::LineWeight::Value(100)), Some(1.0));
    assert_eq!(lineweight_mm(acadrust::LineWeight::W0_25), Some(0.25));
    assert_eq!(lineweight_mm(acadrust::LineWeight::ByLayer), None);
    assert_eq!(lineweight_mm(acadrust::LineWeight::ByBlock), None);
    assert_eq!(lineweight_mm(acadrust::LineWeight::Default), None);
}

// ---- §3.2/§7.1: linetype resolution ----

fn dashed_pattern() -> LinetypePattern {
    LinetypePattern::from_elements([0.5, -0.25])
}

#[test]
fn linetype_resolution_keeps_the_symbolic_by_layer_form() {
    let dashed = dashed_pattern();
    // An explicit named linetype wins over the layer and is stored
    // byte-identically (pattern and entity scale).
    assert_eq!(
        resolve_entity_linetype("Dashed", 2.0, Some(dashed.clone())),
        EntityLineType::Explicit {
            name: "Dashed".into(),
            pattern: dashed.clone(),
            scale: 2.0,
        }
    );
    // ByLayer stays symbolic: the representation looks the owning layer's
    // linetype name up in the drawing's linetype table at draw time, so a layer
    // linetype edit changes the dashes without an entity write. (Before this
    // change the layer's pattern was substituted here as an `Explicit` copy,
    // which froze it at import time.) The variant itself carries no scale by
    // design: the entity's own group `48` factor (1.5 here) is stored beside it
    // on `EntityRenderAttributes::linetype_scale`, so it is preserved for a
    // symbolic linetype too.
    assert_eq!(
        resolve_entity_linetype("ByLayer", 1.5, None),
        EntityLineType::ByLayer
    );
    // An empty name is also ByLayer.
    assert_eq!(
        resolve_entity_linetype("", 1.0, None),
        EntityLineType::ByLayer
    );
    // ByBlock stays symbolic so INSERT expansion can substitute the value.
    assert_eq!(
        resolve_entity_linetype("ByBlock", 1.0, Some(dashed_pattern())),
        EntityLineType::ByBlock
    );
}

#[test]
fn unknown_named_linetype_is_an_explicit_continuous_fallback() {
    // The caller passes `None` for an unknown name; the entity gets an explicit
    // continuous pattern carrying the source name, never invented dashes.
    let resolved = resolve_entity_linetype("NoSuchLine", 1.0, None);
    match resolved {
        EntityLineType::Explicit {
            name,
            pattern,
            scale,
        } => {
            assert_eq!(name, "NoSuchLine");
            assert!(pattern.is_continuous());
            assert_eq!(scale, 1.0);
        }
        other => panic!("expected explicit continuous fallback, got {other:?}"),
    }
}

#[test]
fn standard_continuous_linetype_is_explicit_and_solid() {
    let resolved = resolve_entity_linetype("Continuous", 1.0, None);
    match resolved {
        EntityLineType::Explicit { pattern, .. } => assert!(pattern.is_continuous()),
        other => panic!("expected explicit continuous, got {other:?}"),
    }
}

#[test]
fn non_finite_linetype_scale_falls_back_to_one() {
    let resolved = resolve_entity_linetype("Dashed", f64::NAN, Some(dashed_pattern()));
    match resolved {
        EntityLineType::Explicit { scale, .. } => assert_eq!(scale, 1.0),
        other => panic!("expected explicit, got {other:?}"),
    }
    let resolved = resolve_entity_linetype("Dashed", -2.0, Some(dashed_pattern()));
    match resolved {
        EntityLineType::Explicit { scale, .. } => assert_eq!(scale, 1.0),
        other => panic!("expected explicit, got {other:?}"),
    }
}

#[test]
fn linetype_pattern_keeps_complex_flag_and_skips_non_finite() {
    let mut lt = acadrust::LineType::new("Fenceline");
    lt.add_element(acadrust::tables::LineTypeElement::dash(1.0));
    lt.add_element(acadrust::tables::LineTypeElement::space(0.5));
    let (pattern, complex) = linetype_pattern(&lt);
    assert!(!complex);
    assert_eq!(pattern.elements, vec![1.0, -0.5]);
    assert!((pattern.cycle - 1.5).abs() < 1e-12);

    let mut complex_lt = acadrust::LineType::new("Gasline");
    let mut elem = acadrust::tables::LineTypeElement::dash(1.0);
    elem.complex = Some(acadrust::tables::LineTypeComplexData::default());
    complex_lt.add_element(elem);
    complex_lt.add_element(acadrust::tables::LineTypeElement::space(0.5));
    let (pattern, complex) = linetype_pattern(&complex_lt);
    assert!(complex, "a shape/text element must be flagged");
    assert_eq!(pattern.elements, vec![1.0, -0.5]);
}

#[test]
fn pattern_builder_drops_non_finite_and_keeps_valid_elements() {
    let pattern = LinetypePattern::from_elements([1.0, f64::NAN, -0.5, f64::INFINITY]);
    assert_eq!(pattern.elements, vec![1.0, -0.5]);
    assert!((pattern.cycle - 1.5).abs() < 1e-12);
    // A pattern with only non-finite elements degrades to continuous.
    assert!(LinetypePattern::from_elements([f64::NAN]).is_continuous());
    assert!(LinetypePattern::from_elements([0.0, 0.0]).is_continuous());
}

#[test]
fn all_proxy_fragments_survive_as_a_compound() {
    let fragment = |id: u128| SemanticGeometry::Line {
        start: Point3::default(),
        end: Point3 {
            x: id as f64,
            y: 0.0,
            z: 0.0,
        },
    };
    match proxy_geometry_compound(vec![fragment(1), fragment(2), fragment(3)]) {
        SemanticGeometry::Compound(children) => {
            assert_eq!(children.len(), 3, "every proxy fragment must survive");
        }
        other => panic!("expected a compound, got {other:?}"),
    }
    // A single fragment stays itself: no needless wrapper.
    assert!(matches!(
        proxy_geometry_compound(vec![fragment(1)]),
        SemanticGeometry::Line { .. }
    ));
}

#[test]
fn proxy_cache_is_only_used_for_entities_without_semantic_geometry() {
    let line = EntityType::Line(acadrust::entities::Line::new());
    assert!(
        !proxy_geometry_allowed(&line),
        "a LINE is drawn from semantics; its cache must not double-draw"
    );
    let unknown = EntityType::Unknown(acadrust::entities::UnknownEntity::new("ACAD_PROXY_ENTITY"));
    assert!(proxy_geometry_allowed(&unknown));
    let vendor = EntityType::Unknown(acadrust::entities::UnknownEntity::new("TCH_WALL"));
    assert!(proxy_geometry_allowed(&vendor));
}
