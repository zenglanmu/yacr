//! The Materials tray's document-material read.
//!
//! Host content only: the projection reads `tables/materials.json` off the
//! current database and never mutates it. A document without materials answers
//! an empty list (the panel's empty state), never an error; a carrier with no
//! seam still answers the kind's own refusal at the wire layer.

use cad_domain::material::MaterialDocument;

use super::HostCore;

/// The maximum rows one read returns (the document table is capped at 256).
pub const MATERIAL_LIST_MAX_ROWS: usize = 256;

/// One document material the panel can list and paint.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialListRow {
    /// Stored name (the key `PAINT` takes).
    pub name: String,
    /// sRGB colour, one byte per channel.
    pub color: [u8; 3],
    /// `0.0` opaque … `1.0` fully transparent.
    pub transparency: f64,
}

/// The document-material list plus its table facts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaterialList {
    /// Rows in table order (material sequence).
    pub rows: Vec<MaterialListRow>,
    /// Total materials in the document table.
    pub total: usize,
    /// Whether `rows` was cut at [`MATERIAL_LIST_MAX_ROWS`].
    pub truncated: bool,
}

impl HostCore {
    /// The open document's materials, in table order.
    pub fn materials(&self) -> MaterialList {
        let controller = self.controller.borrow();
        let Some(database) = controller.drawing() else {
            return MaterialList::default();
        };
        let table: &MaterialDocument = database.materials();
        let total = table.len();
        let rows = table
            .materials()
            .iter()
            .take(MATERIAL_LIST_MAX_ROWS)
            .map(|material| MaterialListRow {
                name: material.name.clone(),
                color: material.color,
                transparency: material.transparency,
            })
            .collect::<Vec<_>>();
        MaterialList {
            truncated: total > rows.len(),
            total,
            rows,
        }
    }
}
