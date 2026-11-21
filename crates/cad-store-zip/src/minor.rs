//! The `manifest.format.minor` one document needs (ADR-0037 D13, ADR-0040 D12,
//! ADR-0042 D6).
//!
//! The version gate fires **before** any entry is decoded, so a minor is a
//! promise about the whole archive: writing a higher one than necessary makes an
//! older build refuse a file it could have opened. Two features therefore pick
//! the level **per document**, never from [`cad_domain::ymodel::FORMAT_MINOR`]
//! directly:
//!
//! | the document has | minor | what a minor-4 reader misses |
//! |---|---|---|
//! | BIM data | `6` | the element `bim` block, `tables/bim-definitions.json` |
//! | only bundled resources | `5` | `resources/<kind>/<key>`, `tables/resources.json`, the flag |
//! | neither | `4` | — |
//!
//! The three tiers are spelled out rather than nested as `if/else` on a single
//! `FORMAT_MINOR`, because the failure this module exists to prevent is exactly
//! "resources-only documents were stamped with the newest minor" (ADR-0042 D6
//! and risk 2). A unit test pins the top tier to `FORMAT_MINOR` so raising the
//! format constant without this table fails.

use cad_db::{DocumentSnapshot, TableKind};

/// A document with no minor-5 or minor-6 feature.
pub(crate) const FORMAT_MINOR_PLAIN: u32 = 4;

/// A document that packs or maps external resources (ADR-0040 D12).
pub(crate) const FORMAT_MINOR_RESOURCES: u32 = 5;

/// A document that carries BIM data (ADR-0042 D6).
pub(crate) const FORMAT_MINOR_BIM: u32 = 6;

/// The `manifest.format.minor` this document needs.
///
/// `has_bim` is [`DocumentSnapshot::has_bim`] — `cad-db` owns that predicate, so
/// this crate never names the element `bim` field. The resource tier here is
/// about archive entries and the header flag, both of which the store already
/// models.
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
        assert_eq!(FORMAT_MINOR_BIM, FORMAT_MINOR_RESOURCES + 1);
        assert_eq!(FORMAT_MINOR_RESOURCES, FORMAT_MINOR_PLAIN + 1);
    }
}
