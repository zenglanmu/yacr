//! Linetype/lineweight provenance survives import.
//!
//! The importer used to flatten `ByLayer` into an `Explicit` copy of the layer's
//! millimetres and dash pattern, which made a later layer linetype/lineweight
//! edit change nothing. `ByLayer` and `ByBlock` are now kept symbolic and only a
//! really explicit source value becomes `Explicit`, byte-identically; the
//! representation resolves the symbol against the owning layer's
//! `LayerAttributes` row at draw time.

use super::*;

/// One model-space line at `x` on `layer`, with the given source style.
fn line_at(
    layer: &str,
    linetype: &str,
    linetype_scale: f64,
    weight: acadrust::LineWeight,
    x: f64,
) -> EntityType {
    let mut line = acadrust::entities::Line::new();
    line.start = acadrust::types::Vector3::new(x, 0.0, 0.0);
    line.end = acadrust::types::Vector3::new(x, 1.0, 0.0);
    line.common.layer = layer.to_string();
    line.common.linetype = linetype.to_string();
    line.common.linetype_scale = linetype_scale;
    line.common.line_weight = weight;
    EntityType::Line(line)
}

/// A writer-produced DXF carrying one entity per linetype/lineweight source
/// form plus the layer row they resolve against.
///
/// Uses acadrust's own writer, like the other synthetic fixtures, so the bytes
/// are a real DXF document rather than a hand-rolled approximation.
fn style_dxf() -> Vec<u8> {
    let mut doc = acadrust::CadDocument::new();

    let mut dashed = acadrust::tables::LineType::new("DASHED");
    dashed.add_element(acadrust::tables::LineTypeElement::dash(0.5));
    dashed.add_element(acadrust::tables::LineTypeElement::space(0.25));
    doc.line_types.add_or_replace(dashed);

    let mut walls = acadrust::tables::Layer::new("WALLS");
    walls.line_type = "DASHED".to_string();
    walls.line_weight = acadrust::LineWeight::Value(35);
    doc.layers.add_or_replace(walls);

    let entities = [
        // ByLayer for both: must stay symbolic.
        line_at("WALLS", "ByLayer", 1.0, acadrust::LineWeight::ByLayer, 0.0),
        // Explicit for both: must stay byte-identical.
        line_at("WALLS", "DASHED", 2.5, acadrust::LineWeight::Value(50), 1.0),
        // Symbolic insertion inheritance.
        line_at("WALLS", "ByBlock", 1.0, acadrust::LineWeight::ByBlock, 2.0),
    ];
    for entity in entities {
        doc.add_entity(entity).expect("add line");
    }
    acadrust::DxfWriter::new(&doc)
        .write_to_vec()
        .expect("write synthetic DXF")
}

/// The x coordinate of a line's start point, used to identify the entity.
fn start_x(entity: &DbEntity) -> Option<f64> {
    match &entity.geometry {
        SemanticGeometry::Line { start, .. } => Some(start.x),
        _ => None,
    }
}

#[test]
fn imported_entities_keep_their_source_linetype_and_lineweight_provenance() {
    let drawing = AcadrustImporter::new()
        .import(&request(style_dxf()), &|| false)
        .expect("synthetic style fixture imports");
    let db = &drawing.database;

    let mut styles: Vec<(f64, EntityLineType, EntityLineWeight)> = db
        .entities()
        .filter_map(|entity| {
            let attributes = db.entity_render_attributes(entity.id);
            start_x(entity).map(|x| (x, attributes.linetype, attributes.lineweight))
        })
        .collect();
    styles.sort_by(|a, b| a.0.total_cmp(&b.0));

    // The layer row keeps the values the symbolic forms resolve against at draw
    // time, in the source's own vocabulary (1/100 mm and a linetype *name*).
    let walls = db
        .layers()
        .find(|layer| layer.name == "WALLS")
        .expect("the WALLS layer imports");
    let walls_attributes = db.layer_attributes(walls.id);
    assert_eq!(walls_attributes.linetype, "DASHED");
    assert_eq!(
        walls_attributes.lineweight,
        cad_db::LayerLineWeight::Hundredths(35)
    );

    assert_eq!(
        styles,
        vec![
            // DXF linetype "ByLayer" / weight `-1`: both stay symbolic, so a
            // layer linetype or lineweight edit changes this entity. (Before
            // this change they were flattened here into `Explicit(0.35)` and an
            // `Explicit` copy of the DASHED pattern.)
            (0.0, EntityLineType::ByLayer, EntityLineWeight::ByLayer),
            // A concrete weight is 1/100 mm (50 -> 0.50 mm) and a concrete name
            // keeps its pattern and the entity's own scale, byte-identically.
            (
                1.0,
                EntityLineType::Explicit {
                    name: "DASHED".into(),
                    pattern: LinetypePattern::from_elements([0.5, -0.25]),
                    scale: 2.5,
                },
                EntityLineWeight::Explicit(0.5),
            ),
            // DXF linetype "ByBlock" / weight `-2`: symbolic inheritance from
            // the containing INSERT.
            (2.0, EntityLineType::ByBlock, EntityLineWeight::ByBlock),
        ]
    );
}

#[test]
fn a_symbolic_linetype_does_not_carry_the_entity_linetype_scale() {
    // `cad_db::EntityLineType` has no slot for a per-entity linetype scale beside
    // `ByLayer`/`ByBlock` (only `Explicit` carries one), so a `ByLayer` source
    // with group 48 set loses that factor: the drawing's `$LTSCALE` is the whole
    // factor at draw time. This is stated rather than hidden — the stored value
    // is still symbolic and the layer's pattern still applies. Fixing it needs a
    // `cad_db` shape change, which is out of this change's scope.
    let mut doc = acadrust::CadDocument::new();
    let mut walls = acadrust::tables::Layer::new("WALLS");
    walls.line_type = "DASHED".to_string();
    doc.layers.add_or_replace(walls);
    doc.add_entity(line_at(
        "WALLS",
        "ByLayer",
        2.5,
        acadrust::LineWeight::ByLayer,
        0.0,
    ))
    .expect("add line");

    let bytes = acadrust::DxfWriter::new(&doc)
        .write_to_vec()
        .expect("write synthetic DXF");
    let drawing = AcadrustImporter::new()
        .import(&request(bytes), &|| false)
        .expect("synthetic fixture imports");
    let entity = drawing
        .database
        .entities()
        .next()
        .expect("one line imports");
    assert_eq!(
        drawing
            .database
            .entity_render_attributes(entity.id)
            .linetype,
        EntityLineType::ByLayer,
        "the provenance is still preserved; only the entity scale is not representable"
    );
}
