//! External-reference provenance notes for `FIND`'s report.
//!
//! Split out of `find_text.rs` for the file-size ceiling. A reference whose
//! content was never attached has no text in the imported model at all, so
//! `FIND` names it with its recorded status instead of silently missing it. The
//! attached half (a loaded reference's materialized block) is searched by
//! `find_text`'s block channel and gated by `FINDINCLUDE`.

use cad_db::DrawingDatabase;

/// One external reference whose text is **not** reachable by `FIND`.
pub(super) struct XrefSourceNote {
    pub(super) name: String,
    pub(super) status: &'static str,
}

/// Every external reference whose content is not attached, in name order.
///
/// `is_loaded()` alone is not enough: a reference marked loaded but carrying no
/// provenance block also has nothing the search could traverse, so both states
/// are named.
pub(super) fn collect_xref_sources(drawing: &DrawingDatabase) -> Vec<XrefSourceNote> {
    let mut references: Vec<_> = drawing.external_references().collect();
    references.sort_by(|left, right| left.name.cmp(&right.name));
    references
        .into_iter()
        .filter(|reference| !reference.status.is_loaded() || reference.block.is_none())
        .map(|reference| XrefSourceNote {
            name: reference.name.clone(),
            status: reference.status.as_str(),
        })
        .collect()
}
