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
fn a_symbolic_linetype_carries_the_entity_linetype_scale() {
    // Inverted by the group-48 change: `cad_db` used to have no slot for a
    // per-entity linetype scale beside `ByLayer`/`ByBlock` (only `Explicit`
    // carried one), so a symbolic source with group 48 set lost that factor and
    // drew at the identity. The factor now lives on
    // `EntityRenderAttributes::linetype_scale` for every provenance, and export
    // writes it back, so the stored value is both symbolic **and** scaled.
    let mut doc = acadrust::CadDocument::new();
    let mut dashed = acadrust::tables::LineType::new("DASHED");
    dashed.add_element(acadrust::tables::LineTypeElement::dash(0.5));
    dashed.add_element(acadrust::tables::LineTypeElement::space(0.25));
    doc.line_types.add_or_replace(dashed);
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
    let imported = drawing.database.entity_render_attributes(entity.id);
    assert_eq!(
        imported.linetype,
        EntityLineType::ByLayer,
        "the provenance is still symbolic, so a layer linetype edit still applies"
    );
    assert_eq!(
        imported.linetype_scale, 2.5,
        "group 48 is kept beside the symbolic variant, not only inside `Explicit`"
    );

    // Group 48 must also come back out of a written DXF for a symbolic linetype.
    let (exported, _report) = crate::export::export(&crate::export::ExportRequest {
        database: &drawing.database,
        document_id: DocumentId(1),
        space: crate::export::ExportSpace::Model,
        format: crate::export::ExportFormat::DxfText,
        limits: crate::export::ExportLimits::default(),
        units: None,
    })
    .expect("the imported database exports");
    let reimported = AcadrustImporter::new()
        .import(&request(exported), &|| false)
        .expect("the exported DXF re-imports");
    let entity = reimported
        .database
        .entities()
        .next()
        .expect("one line re-imports");
    let round_tripped = reimported.database.entity_render_attributes(entity.id);
    assert_eq!(
        round_tripped.linetype,
        EntityLineType::ByLayer,
        "the symbolic linetype survives the written DXF"
    );
    assert_eq!(
        round_tripped.linetype_scale, 2.5,
        "group 48 is written for a symbolic entity linetype"
    );
}
