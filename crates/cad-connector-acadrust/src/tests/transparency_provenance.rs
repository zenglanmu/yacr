//! Entity transparency provenance survives import.
//!
//! The importer used to flatten `ByLayer` into an `Explicit` copy of the layer's
//! opacity, which made a later layer opacity edit change nothing. `ByLayer` and
//! `ByBlock` are now kept symbolic and only a really explicit source value
//! becomes `Explicit`, byte-identically (the inverse byte scale: `0` opaque,
//! `255` transparent).

use super::*;

/// One model-space line at `x`, on `layer`, with `transparency` as its source
/// transparency.
fn line_at(layer: &str, transparency: acadrust::Transparency, x: f64) -> EntityType {
    let mut line = acadrust::entities::Line::new();
    line.start = acadrust::types::Vector3::new(x, 0.0, 0.0);
    line.end = acadrust::types::Vector3::new(x, 1.0, 0.0);
    line.common.layer = layer.to_string();
    line.common.transparency = transparency;
    EntityType::Line(line)
}

/// A writer-produced DXF whose layer is 51/255 transparent and which carries one
/// entity per source transparency form.
///
/// Uses acadrust's own writer, like the other synthetic fixtures, so the bytes
/// are a real DXF document rather than a hand-rolled approximation: the layer
/// value travels as `AcCmTransparency` XDATA and the entity values as group 440.
fn transparency_dxf() -> Vec<u8> {
    let mut doc = acadrust::CadDocument::new();
    let mut walls = acadrust::tables::Layer::new("WALLS");
    walls.transparency = acadrust::Transparency::Explicit(51);
    doc.layers.add_or_replace(walls);

    let entities = [
        // The DXF default (no group 440 at all) is by layer.
        line_at("WALLS", acadrust::Transparency::BY_LAYER, 0.0),
        line_at("WALLS", acadrust::Transparency::new(102), 1.0),
        line_at("WALLS", acadrust::Transparency::BY_BLOCK, 2.0),
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
fn imported_entities_keep_their_source_transparency_provenance() {
    let drawing = AcadrustImporter::new()
        .import(&request(transparency_dxf()), &|| false)
        .expect("synthetic transparency fixture imports");
    let db = &drawing.database;

    let mut forms: Vec<(f64, EntityTransparency)> = db
        .entities()
        .filter_map(|entity| {
            start_x(entity).map(|x| (x, db.entity_render_attributes(entity.id).transparency))
        })
        .collect();
    forms.sort_by(|a, b| a.0.total_cmp(&b.0));

    // The layer table keeps the source opacity the symbolic forms resolve
    // against at draw time: 51/255 transparent is 1 - 51/255 opaque.
    let walls = db
        .layers()
        .find(|layer| layer.name == "WALLS")
        .expect("the WALLS layer imports");
    let walls_opacity = db.layer_attributes(walls.id).opacity;
    assert!(
        (walls_opacity - (1.0 - 51.0 / 255.0)).abs() < 1e-6,
        "unexpected layer opacity {walls_opacity}"
    );

    assert_eq!(
        forms,
        vec![
            // DXF has no group 440: stays symbolic, so a layer opacity edit
            // reaches it. (Before this change it was flattened to
            // `Explicit(1 - 51/255)` here.)
            (0.0, EntityTransparency::ByLayer),
            // A really explicit source value stays byte-identical.
            (1.0, EntityTransparency::Explicit(1.0 - 102.0 / 255.0)),
            // ByBlock stays symbolic for INSERT expansion.
            (2.0, EntityTransparency::ByBlock),
        ]
    );
}
