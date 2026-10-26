//! Import contract tests.
//!
//! Split into cohesive submodules so every file stays within the 1000-line
//! budget. Shared helpers live here and are reached by children through
//! `use super::*;`.

use super::*;

mod acis;
mod basics;
mod colour_provenance;
mod dimension;
mod dxf;
mod hatch;
mod hatch_pattern_table;
mod insert;
mod layer_attributes;
mod linetype_weight_provenance;
mod mesh;
mod mtext;
mod ocs;
mod plot;
mod point_display_style;
mod resolution;
mod transparency_provenance;
mod viewport;
mod visibility;

fn request(bytes: Vec<u8>) -> ImportRequest {
    ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
    }
}

/// A synthetic, writer-produced DWG with `count` model-space lines.
///
/// Uses acadrust's own writer, so the bytes are a real AC10xx DWG rather than a
/// vendored sample (no authorized fixture is required for these contract tests).
fn synthetic_dwg(count: usize) -> Vec<u8> {
    let mut doc = acadrust::CadDocument::new();
    for i in 0..count {
        let mut line = acadrust::entities::Line::new();
        line.start = acadrust::types::Vector3::new(i as f64, 0.0, 0.0);
        line.end = acadrust::types::Vector3::new(i as f64, 1.0, 0.0);
        doc.add_entity(EntityType::Line(line)).expect("add line");
    }
    acadrust::DwgWriter::write_to_vec(&doc).expect("write synthetic DWG")
}
