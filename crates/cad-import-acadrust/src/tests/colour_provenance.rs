//! Colour provenance survives import.
//!
//! The importer used to flatten `ByLayer` into an `Explicit` copy of the layer's
//! RGB, which made a later layer colour edit recolour nothing. `ByLayer` and
//! `ByBlock` are now kept symbolic and only a really explicit source colour
//! becomes `Explicit`, byte-identically.

use super::*;

/// sRGB of an ACI index through acadrust's own canonical table, so the test
/// never hard-codes a triple the reader might map differently.
fn aci_rgb(index: u8) -> [u8; 3] {
    let (r, g, b) = acadrust::Color::Index(index)
        .rgb()
        .expect("a non-zero ACI index resolves");
    [r, g, b]
}

/// One model-space line at `x`, on `layer`, with `color` as its source colour.
fn line_at(layer: &str, color: acadrust::Color, x: f64) -> EntityType {
    let mut line = acadrust::entities::Line::new();
    line.start = acadrust::types::Vector3::new(x, 0.0, 0.0);
    line.end = acadrust::types::Vector3::new(x, 1.0, 0.0);
    line.common.layer = layer.to_string();
    line.common.color = color;
    EntityType::Line(line)
}

/// A writer-produced DXF carrying one entity per colour source form.
///
/// Uses acadrust's own writer, like the other synthetic fixtures, so the bytes
/// are a real DXF document rather than a hand-rolled approximation.
fn colour_dxf() -> Vec<u8> {
    let mut doc = acadrust::CadDocument::new();
    let mut walls = acadrust::tables::Layer::new("WALLS");
    walls.color = acadrust::Color::Index(1);
    doc.layers.add_or_replace(walls);

    // One entity per source form, each at its own x so the assertions can key on
    // geometry instead of entity-id order.
    let entities = [
        line_at("WALLS", acadrust::Color::ByLayer, 0.0),
        line_at("WALLS", acadrust::Color::from_rgb(10, 20, 30), 1.0),
        line_at("WALLS", acadrust::Color::ByBlock, 2.0),
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
fn imported_entities_keep_their_source_colour_provenance() {
    let drawing = AcadrustImporter::new()
        .import(&request(colour_dxf()), &|| false)
        .expect("synthetic colour fixture imports");
    let db = &drawing.database;

    let mut colours: Vec<(f64, EntityColor)> = db
        .entities()
        .filter_map(|entity| {
            start_x(entity).map(|x| (x, db.entity_render_attributes(entity.id).color))
        })
        .collect();
    colours.sort_by(|a, b| a.0.total_cmp(&b.0));

    // The layer table keeps the source colour the symbolic forms will resolve
    // against at draw time.
    let walls = db
        .layers()
        .find(|layer| layer.name == "WALLS")
        .expect("the WALLS layer imports");
    let walls_attributes = db.layer_attributes(walls.id);
    assert_eq!(walls_attributes.color.aci, Some(1));
    assert_eq!(walls_attributes.color.rgb, aci_rgb(1));

    assert_eq!(
        colours,
        vec![
            // DXF colour 256 / absent: stays symbolic, so a layer colour edit
            // recolours it. (Before this change it was flattened to
            // `Explicit(aci_rgb(1))` here.)
            (0.0, EntityColor::ByLayer),
            // A true-colour source stays byte-identical.
            (1.0, EntityColor::Explicit([10, 20, 30])),
            // DXF colour 0: symbolic inheritance from the containing INSERT.
            (2.0, EntityColor::ByBlock),
        ]
    );
}
