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
//! | block attributes (ADR-0055) | `14` | `cad:AttributeDefinition`, `cad:Insert.attributes` |
//! | host-owned vendor semantics (ADR-0030 D7) | `12` | `tables/vendor.json` |
//! | in-context relations (ADR-0043 P3) | `11` | `tables/relations.json` |
//! | feature definitions (ADR-0043 P2) | `10` | `tables/features.json` |
//! | a P1 expression node or configuration (ADR-0043 P1) | `9` | `neg`/`pow`/`fn`/`if` tags, `tables/configurations.json` |
//! | parameter rows or element bindings (ADR-0043) | `8` | `tables/parameters.json`, the element `params` block |
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

/// A document that carries parameter rows or element bindings (ADR-0043 D11).
pub(crate) const FORMAT_MINOR_PARAMS: u32 = 8;

/// A document that uses a P1 expression node or a P1 configuration
/// (ADR-0043 P1).
pub(crate) const FORMAT_MINOR_EXPRESSIONS: u32 = 9;

/// A document that carries feature definitions (ADR-0043 P2).
pub(crate) const FORMAT_MINOR_FEATURES: u32 = 10;

/// A document that carries self-authored in-context relations (ADR-0043 P3).
pub(crate) const FORMAT_MINOR_RELATIONS: u32 = 11;

/// A document that carries host-owned vendor semantics rows (ADR-0030 D7).
pub(crate) const FORMAT_MINOR_VENDOR: u32 = 12;

/// A document that names an interface domain in `document.json` (ADR-0053 D2).
pub(crate) const FORMAT_MINOR_APPLICATION_DOMAIN: u32 = 13;

/// A document that carries block attributes (ADR-0055): an `ATTDEF` member or an
/// `INSERT` with values.
///
/// The definitions and values are both element-payload data, so the tier is
/// decided by [`DocumentSnapshot::has_block_attributes`] before the header-based
/// application-domain tier: a domain document with attributes must be written at
/// `14`, not `13`.
pub(crate) const FORMAT_MINOR_BLOCK_ATTRIBUTES: u32 = 14;

// The chain is a compile-time fact, not a convention: a future tier inserted in
// the middle cannot silently leave the constants out of order.
const _: () = assert!(FORMAT_MINOR_APPLICATION_DOMAIN + 1 == FORMAT_MINOR_BLOCK_ATTRIBUTES);
const _: () = assert!(FORMAT_MINOR_VENDOR + 1 == FORMAT_MINOR_APPLICATION_DOMAIN);
const _: () = assert!(FORMAT_MINOR_RELATIONS + 1 == FORMAT_MINOR_VENDOR);
const _: () = assert!(FORMAT_MINOR_FEATURES + 1 == FORMAT_MINOR_RELATIONS);
const _: () = assert!(FORMAT_MINOR_EXPRESSIONS + 1 == FORMAT_MINOR_FEATURES);
const _: () = assert!(FORMAT_MINOR_PARAMS + 1 == FORMAT_MINOR_EXPRESSIONS);
const _: () = assert!(FORMAT_MINOR_BIM + 1 == FORMAT_MINOR_PARAMS);
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
    // Block attributes are the newest tier and can coexist with any other
    // feature (including a named interface domain), so they decide first: a
    // minor-13 reader would refuse the `cad:AttributeDefinition` class or the
    // insert `attributes` array, and a domain-bearing document would otherwise
    // be stamped 13 and lose them on the reader side.
    if snapshot.has_block_attributes() {
        return FORMAT_MINOR_BLOCK_ATTRIBUTES;
    }
    // A named interface domain is a `document.json` scalar: a minor-12 reader
    // would refuse the unknown header key, so it decides before every
    // table-based tier.
    if snapshot.header.application_domain.is_some() {
        return FORMAT_MINOR_APPLICATION_DOMAIN;
    }
    // The vendor tier is about a host-owned table only: no geometry, no element
    // block. An empty (or absent) table is not a vendor document, so an ordinary
    // `.ymodel` never pays the minor-12 gate.
    if snapshot
        .tables
        .get(TableKind::Vendor)
        .is_some_and(|rows| !rows.is_empty())
    {
        return FORMAT_MINOR_VENDOR;
    }
    if snapshot.has_relations() {
        return FORMAT_MINOR_RELATIONS;
    }
    if snapshot.has_features() {
        return FORMAT_MINOR_FEATURES;
    }
    // P1 expression tags need their own tier: a minor-8 reader would refuse the
    // unknown tag as a malformed document instead of `unsupported_minor`. The
    // configuration table shares the tier (both landed in P1).
    if snapshot.has_p1_params() || snapshot.has_configurations() {
        return FORMAT_MINOR_EXPRESSIONS;
    }
    // Parameters are the newest P0 tier and imply nothing about BIM: a document
    // may carry both, and minor 8 is the only level that models the table and
    // the element `params` block.
    if snapshot.has_params() {
        return FORMAT_MINOR_PARAMS;
    }
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
    fn the_top_tier_is_the_format_minor() {
        assert_eq!(
            FORMAT_MINOR_BLOCK_ATTRIBUTES,
            cad_domain::ymodel::FORMAT_MINOR
        );
        assert_eq!(
            FORMAT_MINOR_BLOCK_ATTRIBUTES,
            FORMAT_MINOR_APPLICATION_DOMAIN + 1
        );
        assert_eq!(FORMAT_MINOR_APPLICATION_DOMAIN, FORMAT_MINOR_VENDOR + 1);
        assert_eq!(FORMAT_MINOR_VENDOR, FORMAT_MINOR_RELATIONS + 1);
        assert_eq!(FORMAT_MINOR_RELATIONS, FORMAT_MINOR_FEATURES + 1);
        assert_eq!(FORMAT_MINOR_FEATURES, FORMAT_MINOR_EXPRESSIONS + 1);
        assert_eq!(FORMAT_MINOR_EXPRESSIONS, FORMAT_MINOR_PARAMS + 1);
        assert_eq!(FORMAT_MINOR_PARAMS, FORMAT_MINOR_BIM + 1);
        assert_eq!(FORMAT_MINOR_BIM, FORMAT_MINOR_BIM_ELEMENTS + 1);
        assert_eq!(FORMAT_MINOR_BIM_ELEMENTS, FORMAT_MINOR_RESOURCES + 1);
        assert_eq!(FORMAT_MINOR_RESOURCES, FORMAT_MINOR_PLAIN + 1);
    }
}
