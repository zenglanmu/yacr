//! `$INSUNITS` → [`UnitContext`] mapping (audit B31: centimetres).
//!
//! The bytes come from acadrust's own DXF writer, so the header goes through the
//! same reader a real file does; nothing here fabricates the parsed header.

use super::*;

/// Import a one-line DXF whose header declares `$INSUNITS = code`.
fn import_with_insertion_units(code: i16) -> ImportedDrawing {
    let mut doc = acadrust::CadDocument::new();
    doc.header.insertion_units = code;
    doc.add_entity(EntityType::Line(acadrust::entities::Line::from_coords(
        0.0, 0.0, 0.0, 10.0, 0.0, 0.0,
    )))
    .expect("add line");
    let bytes = acadrust::io::dxf::DxfWriter::new(&doc)
        .write_to_vec()
        .expect("write DXF");
    AcadrustImporter::new()
        .import(&request(bytes), &|| false)
        .expect("fixture imports")
}

#[test]
fn centimetre_insunits_is_a_centimetre_context_not_a_metre_one() {
    let drawing = import_with_insertion_units(5);
    assert_eq!(drawing.units.source, Unit::Centimeter);
    assert_eq!(drawing.units.display, Unit::Centimeter);
    assert_eq!(
        drawing.units.display_per_source,
        Some(1.0),
        "one centimetre is one source unit; the 100× ratio belongs to the unit, not to a \
         display scale"
    );
}

#[test]
fn inch_foot_millimetre_and_metre_codes_keep_their_contexts() {
    for (code, unit) in [
        (1, Unit::Inch),
        (2, Unit::Foot),
        (4, Unit::Millimeter),
        (6, Unit::Meter),
    ] {
        let drawing = import_with_insertion_units(code);
        assert_eq!(drawing.units.source, unit, "$INSUNITS {code}");
        assert_eq!(drawing.units.display, unit, "$INSUNITS {code}");
        assert_eq!(
            drawing.units.display_per_source,
            Some(1.0),
            "$INSUNITS {code}"
        );
    }
}

#[test]
fn unitless_and_unmodelled_codes_stay_drawing_units() {
    for code in [0, 3, 7, 8, 9, 10] {
        let drawing = import_with_insertion_units(code);
        assert_eq!(
            drawing.units.source,
            Unit::DrawingUnits,
            "$INSUNITS {code} is not modelled and must not be guessed"
        );
        assert_eq!(drawing.units.display_per_source, None, "$INSUNITS {code}");
    }
}
