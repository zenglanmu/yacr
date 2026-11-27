//! The Materials tray's document-material list: `cad_materials` /
//! `kind: "materials"`.
//!
//! The built-in palette is a frontend constant; this read is the **document's**
//! own `tables/materials.json` rows, so the tray can show the materials a user
//! actually painted with and repaint from them. A row carries name, sRGB colour
//! and transparency; painting is not this read (the panel dispatches
//! `PAINT <name>` through the ordinary command path).
//!
//! A carrier with no lane answers [`MATERIALS_UNSUPPORTED`] instead of an empty
//! list that would read as "this document has no materials".

use serde::{Deserialize, Serialize};

/// The wire spelling of the materials request kind.
pub const MATERIALS_KIND: &str = "materials";

/// Stable refusal for a carrier with no document-material lane.
pub const MATERIALS_UNSUPPORTED: &str = "unsupported-runtime: materials";

/// The maximum rows one answer carries (the document table is capped at 256).
pub const MATERIALS_DEFAULT_LIMIT: usize = 256;

/// One document material row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialRow {
    /// Stored name (the key `PAINT` takes).
    pub name: String,
    /// sRGB colour, one byte per channel.
    pub color: [u8; 3],
    /// `0.0` opaque … `1.0` fully transparent.
    pub transparency: f64,
}

/// The document-material list and its page facts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialsResponse {
    /// Rows in table order (material sequence).
    pub materials: Vec<MaterialRow>,
    /// Total materials in the document table.
    pub total: usize,
    /// Whether `materials` was cut at the wire budget.
    pub truncated: bool,
}

impl MaterialsResponse {
    /// Project the host's read onto the wire row.
    pub fn from_host(list: cad_host::MaterialList) -> Self {
        MaterialsResponse {
            materials: list
                .rows
                .into_iter()
                .map(|row| MaterialRow {
                    name: row.name,
                    color: row.color,
                    transparency: row.transparency,
                })
                .collect(),
            total: list.total,
            truncated: list.truncated,
        }
    }
}
