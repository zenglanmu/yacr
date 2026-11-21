//! The `manifest.format.minor` one document needs (ADR-0037 D13, ADR-0040 D12,
//! ADR-0042 D6/D3-tail).
//!
//! The version gate fires **before** any entry is decoded, so a minor is a
//! promise about the whole archive: writing a higher one than necessary makes an
//! older build refuse a file it could have opened. Features therefore pick the
//! level **per document**, never from [`cad_domain::ymodel::FORMAT_MINOR`]
//! directly:
//!
//! | the document has | minor | what a minor-4 reader misses |
//! |---|---|---|
//! | BIM data (elements, definitions or **relations**) | `7` | the element `bim` block, `tables/bim-definitions.json`, `tables/bim-relations.json` |
//! | only bundled resources | `5` | `resources/<kind>/<key>`, `tables/resources.json`, the flag |
//! | neither | `4` | — |
//!
//! `6` is the **read-only legacy** BIM tier: this build opens a minor-6 archive
//! (the writer that produced it had no relation table), but a resave stamps `7`.
//! Relations and the element/definition blocks are one release, so a BIM document
//! **without** relations is also written at `7` — a `≤6` reader would refuse it
//! for the element block anyway, so there is no lower tier to gain by splitting
//! them.
//!
//! The tiers are spelled out rather than nested as `if/else` on a single
//! `FORMAT_MINOR`, because the failure this module exists to prevent is exactly
//! "resources-only documents were stamped with the newest minor" (ADR-0042 D6
//! and risk 2). A unit test pins the top tier to `FORMAT_MINOR` so raising the
//! format constant without this table fails.

use cad_db::{DocumentSnapshot, TableKind};

/// A document with no minor-5, minor-6 or minor-7 feature.
pub(crate) const FORMAT_MINOR_PLAIN: u32 = 4;

/// A document that packs or maps external resources (ADR-0040 D12).
pub(crate) const FORMAT_MINOR_RESOURCES: u32 = 5;

/// The former BIM tier (minor 6): elements/definitions only, no relation table.
///
/// Still **read**; never written by this build (a resave stamps
/// [`FORMAT_MINOR_BIM`]). Kept as a named tier so the version chain is explicit.
pub(crate) const FORMAT_MINOR_BIM_ELEMENTS: u32 = 6;

/// A document that carries BIM data, with or without relation edges (ADR-0042
/// D6 and D3 tail: one release, so one minor).
pub(crate) const FORMAT_MINOR_BIM: u32 = 7;

// The chain is a compile-time fact, not a convention: a future tier inserted in
// the middle cannot silently leave the constants out of order.
const _: () = assert!(FORMAT_MINOR_BIM_ELEMENTS + 1 == FORMAT_MINOR_BIM);
const _: () = assert!(FORMAT_MINOR_RESOURCES + 1 == FORMAT_MINOR_BIM_ELEMENTS);
const _: () = assert!(FORMAT_MINOR_PLAIN + 1 == FORMAT_MINOR_RESOURCES);

/// The `manifest.format.minor` this document needs.
///
/// `has_bim` is [`DocumentSnapshot::has_bim`] — `cad-db` owns that predicate
/// (definitions table, relation table or any element `bim` block), so this crate
/// never names the element `bim` field or the relation table path. The resource
/// tier here is about archive entries and the header flag, both of which the
/// store already models.
pub(crate) fn document_minor(snapshot: &DocumentSnapshot) -> u32 {
    if snapshot.has_bim() {
        return FORMAT_MINOR_BIM;
    }
    // The resource tier: a payload, the mapping table (**even an empty one**), or
    // the self-containment flag in `document.json`. The table, not the payload
    // count, decides — the writer emits `tables/resources.json` whenever the
    // snapshot holds it, and a minor-4 reader refuses a table path it does not
    // model.
    let has_resource_table = snapshot.tables.get(TableKind::Resources).is_some();
    if snapshot.header.embed_resources || !snapshot.resources.is_empty() || has_resource_table {
        return FORMAT_MINOR_RESOURCES;
    }
    FORMAT_MINOR_PLAIN
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The top tier is the format constant, not an independent number.
    #[test]
    fn the_bim_tier_is_the_format_minor() {
        assert_eq!(FORMAT_MINOR_BIM, cad_domain::ymodel::FORMAT_MINOR);
        assert_eq!(FORMAT_MINOR_BIM, FORMAT_MINOR_BIM_ELEMENTS + 1);
        assert_eq!(FORMAT_MINOR_BIM_ELEMENTS, FORMAT_MINOR_RESOURCES + 1);
        assert_eq!(FORMAT_MINOR_RESOURCES, FORMAT_MINOR_PLAIN + 1);
    }
}
