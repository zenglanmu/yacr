//! The Outliner tray's wire row: `cad_outliner` / `kind: "outliner"`.
//!
//! The panel needs the document hierarchy as a flat pre-order list: each row's
//! kind, label, depth, whether it has children, and **exactly** the entity ids a
//! click should select (a group sends all its members in one request). The
//! session projects the open document's groups/blocks/entities; a carrier with
//! no seam answers [`OUTLINER_UNSUPPORTED`] instead of an empty tree that would
//! read as "this document has no objects".
//!
//! Selection is not part of this read: the panel dispatches the existing
//! `cad_select_entities` wire command with `selectIds`, so there is no second
//! selection lane.

use serde::{Deserialize, Serialize};

/// The wire spelling of the Outliner request kind.
pub const OUTLINER_KIND: &str = "outliner";

/// Stable refusal for a carrier with no Outliner lane.
pub const OUTLINER_UNSUPPORTED: &str = "unsupported-runtime: outliner";

/// One Outliner row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlinerRowWire {
    /// `"group"`, `"insert"`, `"entity"` or `"member"`.
    pub kind: String,
    /// Entity id, or the group sequence for a group row.
    pub id: u64,
    /// Display label (`LINE #12`, the block name, or the group name).
    pub label: String,
    /// Nesting depth (top-level rows are `0`).
    pub depth: u32,
    /// Whether the row is temporarily hidden in this session.
    pub hidden: bool,
    /// Whether the row has children in this answer or beyond the page.
    pub has_children: bool,
    /// Whether a click can select this row.
    pub selectable: bool,
    /// The entity ids a click sends (empty when not selectable).
    pub select_ids: Vec<u64>,
}

/// The document hierarchy plus its page facts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlinerResponse {
    /// The document revision the rows were read from (`0` with no document).
    pub revision: u64,
    /// The first page of rows, in pre-order.
    pub rows: Vec<OutlinerRowWire>,
    /// The whole tree's row count (not the page length).
    pub total: usize,
    /// Whether `rows` was cut at the wire budget.
    pub truncated: bool,
}

impl OutlinerResponse {
    /// Project the host's read onto the wire row.
    pub fn from_host(tree: cad_host::OutlinerTree) -> Self {
        OutlinerResponse {
            revision: tree.revision,
            rows: tree
                .rows
                .into_iter()
                .map(|row| OutlinerRowWire {
                    kind: row.kind.to_string(),
                    id: row.id,
                    label: row.label,
                    depth: row.depth,
                    hidden: row.hidden,
                    has_children: row.has_children,
                    selectable: row.selectable,
                    select_ids: row.select_ids,
                })
                .collect(),
            total: tree.total,
            truncated: tree.truncated,
        }
    }
}
