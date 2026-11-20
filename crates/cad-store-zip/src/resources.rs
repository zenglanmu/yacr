//! `resources/<kind>/<key>` binary entries and the
//! `tables/resources.json` mapping (ADR-0040 D1/D4/D5/D6/D8).
//!
//! Three kinds of asset can be packed into an archive beside the document:
//! `font`, `xref` and `image`. The bytes live in one content-addressed binary
//! entry each (`<sha256 prefix as 32 lowercase hex>.<ext>`), and the logical name
//! that references them lives in `tables/resources.json` — a **plain row array**
//! like every other table, never an object envelope (D4: an object shell would
//! make every archive carrying resources unreadable).
//!
//! # One entry predicate, not a prefix match
//!
//! [`resource_entry`] is the only definition of "this path is a modelled resource
//! entry": `resources/<kind>/<key>` with a kind this build knows and a key form
//! [`cad_domain::is_valid_bundled_key`] accepts. It is used by the read side (is
//! this entry modelled, and what does it hold), by the planner (what path does
//! this snapshot resource become) and by the mapping check. A bare `resources/`
//! prefix must **not** be treated as modelled: `is_modelled` is an allow-list of
//! what the next save re-encodes, so accepting a path nothing decodes would carry
//! it into the snapshot-less void and drop its bytes on the next save (D6).
//!
//! # Key/content invariant
//!
//! A key's hex prefix **is** the payload's SHA-256 prefix. That cannot be checked
//! in `cad-db` (it has no hash dependency), so it is asserted here, on both
//! sides, at the one place that already has the digest:
//!
//! * writing: [`check_write_key_matches_digest`] runs before the zip entry is
//!   started, and reports `resource.invalid_key` (a collector bug);
//! * reading: the manifest's `sha256` has already been verified against the bytes
//!   by the reader, and a key that does not match them is
//!   `ymodel.invalid_document` ([`check_read_key_matches_digest`]).
//!
//! Without the write-side check a collector that computed the wrong key would
//! store bytes under a name that does not describe them, and every later save
//! would raw-copy those stale bytes under the wrong key (D1/D8).
//!
//! # What is *not* here
//!
//! `textures/<key>` (ADR-0038) keeps its own predicate and is never raw-copied
//! (ADR-0040 D14). `raw/` entries stay foreign: never parsed, never re-encoded.

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use cad_db::{DocumentSnapshot, StoreError, TableKind};
use cad_domain::ymodel::markers;
use cad_domain::{is_valid_bundled_key, BundledKind};

use crate::crc32::Crc32;
use crate::json::{malformed, sha256_hex, Reader, Sha256Sum};

/// The prefix every bundled-resource entry carries.
const RESOURCES_PREFIX: &str = "resources/";

/// The `tables/resources.json` context used in every refusal from this module.
const TABLE: &str = "tables/resources.json";

/// The logical name and collection-time source of each bundled resource, keyed by
/// `(kind, key)` exactly like [`cad_db::DocumentSnapshot::resources`].
///
/// `sources` holds an entry **only when a path was recorded**: "no path" has one
/// spelling (the absent key), matching `cad-db`'s builder and the snapshot field.
/// A `null` or absent `source` row therefore does not invent a `None` entry, which
/// is what keeps a load → snapshot cycle a fixed point.
pub(crate) type ResourceTables = (
    BTreeMap<(BundledKind, String), String>,
    BTreeMap<(BundledKind, String), Option<String>>,
);

/// The kind and key of a `resources/<kind>/<32 lowercase hex>.<ext>` entry.
///
/// `None` for anything else — a different prefix, an unknown kind, a key with a
/// path component, an uppercase extension, an extension the kind does not allow.
/// There is no repair and no case folding (ADR-0040 D1).
pub(crate) fn resource_entry(path: &str) -> Option<(BundledKind, &str)> {
    let rest = path.strip_prefix(RESOURCES_PREFIX)?;
    let (token, key) = rest.split_once('/')?;
    let kind = BundledKind::parse(token)?;
    is_valid_bundled_key(kind, key).then_some((kind, key))
}

/// The archive path of one snapshot resource, validated first.
///
/// It is defined in terms of [`resource_entry`] rather than re-checking the key
/// form: the planner and the reader must agree on what a resource entry is, or
/// this build would write an archive it then refuses to open. The wrapper exists
/// only to turn "not a valid key" into a marker-bearing error.
pub(crate) fn resource_path(kind: BundledKind, key: &str) -> Result<String, StoreError> {
    let path = format!("{RESOURCES_PREFIX}{kind}/{key}");
    match resource_entry(&path) {
        Some((found_kind, found_key)) if found_kind == kind && found_key == key => Ok(path),
        _ => Err(StoreError::new(
            markers::RESOURCE_UNSUPPORTED_KIND,
            format!("{key:?} is not a valid {kind} resource key"),
        )),
    }
}

/// Refuse a key whose hex prefix is not the payload's SHA-256 prefix, on **read**.
///
/// `sha256` is the digest the manifest records, already verified against the
/// decompressed bytes by [`crate::read`]: a key that does not name them describes
/// an archive that lies about its own content, i.e. `ymodel.invalid_document`.
pub(crate) fn check_read_key_matches_digest(
    kind: BundledKind,
    key: &str,
    sha256: &str,
    path: &str,
) -> Result<(), StoreError> {
    key_matches_digest(key, sha256)
        .then_some(())
        .ok_or_else(|| malformed(path, key_mismatch(kind, key, sha256)))
}

/// Refuse a key whose hex prefix is not the payload's SHA-256 prefix, on **write**.
///
/// The bytes are the caller's, and the manifest will be generated from them, so a
/// mismatch is not a corrupt file: it is a collector that computed the wrong key
/// (`resource.invalid_key`, ADR-0040 D1/D12). Reporting it as
/// `ymodel.invalid_document` would blame the document for a caller bug.
pub(crate) fn check_write_key_matches_digest(
    kind: BundledKind,
    key: &str,
    sha256: &str,
    path: &str,
) -> Result<(), StoreError> {
    key_matches_digest(key, sha256)
        .then_some(())
        .ok_or_else(|| {
            StoreError::new(
                markers::RESOURCE_INVALID_KEY,
                format!("{path}: {}", key_mismatch(kind, key, sha256)),
            )
        })
}

/// Whether `sha256` starts with the key's 32-hex prefix.
fn key_matches_digest(key: &str, sha256: &str) -> bool {
    let hex = key.split_once('.').map_or(key, |(hex, _)| hex);
    sha256.len() == 64 && sha256.starts_with(hex)
}

fn key_mismatch(kind: BundledKind, key: &str, sha256: &str) -> String {
    let hex = key.split_once('.').map_or(key, |(hex, _)| hex);
    format!(
        "the {kind} resource key {key:?} does not name its own payload: \
         expected the sha256 to start with {hex:?}, got {sha256:?}"
    )
}

/// The per-entry read budget for one bundled resource (ADR-0040 D10).
///
/// The same 32 MiB the collector enforces: the archive is untrusted input, so a
/// manifest-declared length above it is refused before the entry is decompressed
/// instead of being allocated.
///
/// `cad-host`'s collector states the same number for the way in
/// (`save_resources::RESOURCE_BUDGET_ENTRY_BYTES`); the store cannot share that
/// constant without depending on a crate above it, so this is the one place the
/// read side states the limit.
pub(crate) const MAX_RESOURCE_BYTES: u64 = 32 * 1024 * 1024;

/// Refuse one resource entry whose declared length is over the read budget.
pub(crate) fn check_budget(path: &str, len: u64) -> Result<(), StoreError> {
    if len > MAX_RESOURCE_BYTES {
        return Err(StoreError::new(
            markers::RESOURCE_BUDGET_ENTRY,
            format!(
                "{path}: {len} bytes exceeds the {MAX_RESOURCE_BYTES}-byte \
                 per-entry resource budget"
            ),
        ));
    }
    Ok(())
}

/// A previous archive holds **different** bytes at a modelled resource path.
pub(crate) fn conflict(path: &str) -> StoreError {
    StoreError::new(
        markers::RESOURCE_CONFLICT,
        format!("{path:?} already holds different bytes in the previous archive"),
    )
}

/// The chunk size binary payloads are measured in.
pub(crate) const BINARY_CHUNK: usize = 64 * 1024;

/// `(sha256, len, crc32)` of `bytes` in **one** pass over them (ADR-0040 D15).
///
/// `len` is `bytes.len()` rather than an accumulator, so the pass really is one:
/// hash and CRC share the same chunk sequence and nothing is copied.
pub(crate) fn measure(bytes: &[u8]) -> (String, u64, u32) {
    let mut hasher = Sha256Sum::new();
    let mut crc = Crc32::new();
    for chunk in bytes.chunks(BINARY_CHUNK) {
        hasher.update(chunk);
        crc.update(chunk);
    }
    (hasher.hex(), bytes.len() as u64, crc.finish())
}

/// One resource the save must write, derived from the snapshot.
pub(crate) struct PlannedResource<'a> {
    pub(crate) path: String,
    pub(crate) kind: BundledKind,
    pub(crate) key: &'a str,
    pub(crate) bytes: &'a [u8],
}

/// The `resources/<kind>/<key>` entries a snapshot implies, in `(kind, key)` order.
///
/// The snapshot is the authority (D5): a key it does not hold is never planned, so
/// a dropped resource leaves no orphan entry.
///
/// # The mapping exists in two forms, and they must agree
///
/// The rows actually written are `snapshot.tables[Resources]`, while
/// `resource_names`/`resource_sources` are the snapshot's own view. Decoding the
/// rows and requiring them to equal those maps means a name or source edited in
/// one representation only refuses the save instead of being silently reverted on
/// load (ADR-0040 D4). "No source path" has one spelling: an absent key on both
/// sides.
pub(crate) fn plan(snapshot: &DocumentSnapshot) -> Result<Vec<PlannedResource<'_>>, StoreError> {
    // 1. Every payload must have a name and a canonical path. This runs first so a
    //    caller that computed a wrong key is refused as `resource.unsupported_kind`,
    //    not as a mapping disagreement caused by the same bug.
    let mut planned = Vec::with_capacity(snapshot.resources.len());
    for (id, bytes) in &snapshot.resources {
        let (kind, key) = (id.0, id.1.as_str());
        if !snapshot.resource_names.contains_key(id) {
            return Err(malformed(
                RESOURCES_PREFIX,
                format!("{RESOURCES_PREFIX}{kind}/{key}: a bundled resource has no logical name"),
            ));
        }
        planned.push(PlannedResource {
            path: resource_path(kind, key)?,
            kind,
            key,
            bytes: &bytes[..],
        });
    }

    // 2. The rows written are `tables[Resources]`; they must say the same thing as
    //    the snapshot's own maps (names **and** sources).
    let decoded = decode_rows(snapshot.tables.get(TableKind::Resources).unwrap_or(&[]))?;
    let declared: ResourceTables = (
        snapshot.resource_names.clone(),
        snapshot
            .resource_sources
            .iter()
            // `None` and an absent key are one state (`cad-db`'s builder removes
            // the entry), so a stray `None` is not a disagreement.
            .filter(|(_, source)| source.is_some())
            .map(|(id, source)| (id.clone(), source.clone()))
            .collect(),
    );
    if decoded != declared {
        return Err(malformed(
            TABLE,
            "the resource rows and the snapshot's name/source maps disagree; \
             one representation was edited without the other",
        ));
    }

    // 3. A name without a payload would be written and then refused on read.
    for (id, name) in &snapshot.resource_names {
        if !snapshot.resources.contains_key(id) {
            let (kind, key) = (id.0, id.1.as_str());
            return Err(malformed(
                TABLE,
                format!(
                    "{kind} {name:?} has no {RESOURCES_PREFIX}{kind}/{key} payload in the snapshot"
                ),
            ));
        }
    }
    Ok(planned)
}

/// The minor a document that packs nothing is written at (ADR-0040 D12).
pub(crate) const FORMAT_MINOR_PLAIN: u32 = 4;

/// The `manifest.format.minor` this document needs (ADR-0040 D12).
///
/// `5` whenever the archive carries something a minor-4 reader cannot model: a
/// resource payload, the `tables/resources.json` table (**even an empty one**), or
/// the self-containment flag. Everything else stays `4`, because the version gate
/// fires before any entry is decoded and an unconditional `5` would make a
/// resource-free document unopenable by an older build.
///
/// The table, not the payload count, is what decides: the writer emits
/// `tables/resources.json` whenever the snapshot holds the table, and a minor-4
/// reader refuses a table path it does not model.
pub(crate) fn document_minor(snapshot: &DocumentSnapshot) -> u32 {
    let has_table = snapshot.tables.get(TableKind::Resources).is_some();
    if snapshot.header.embed_resources || !snapshot.resources.is_empty() || has_table {
        cad_domain::ymodel::FORMAT_MINOR
    } else {
        FORMAT_MINOR_PLAIN
    }
}

/// Whether the previous archive's foreign bytes at `path` are `new_bytes`.
///
/// Only meaningful for a content-addressed `resources/<kind>/<key>` path (D8): the
/// same key is the same asset, so identical bytes let the modelled entry take over
/// the foreign record instead of refusing the save. `false` means the collision is
/// real and the caller must refuse with [`conflict`].
///
/// The comparison is derived from the **blob**, not from the source manifest: a
/// `foreign: true` record's `sha256` is never verified at load (foreign entries are
/// only CRC-checked), so trusting it would both refuse byte-identical blobs and
/// let a lying record win a takeover.
pub(crate) fn foreign_has_same_content(
    previous_bytes: &[u8],
    path: &str,
    new_bytes: &[u8],
) -> Result<bool, StoreError> {
    let measured = crate::previous::measure(previous_bytes, path)?.sha256;
    Ok(measured == sha256_hex(new_bytes))
}

/// Decode `tables/resources.json` into the two maps the snapshot carries.
///
/// Strict in the same way every table codec is: unknown fields, wrong types, an
/// unknown `kind`, a key that is not in canonical form, an empty `name`, one
/// `(kind, name)` bound to two keys, and one key listed twice are all refused —
/// the writer re-serializes whole rows, so it cannot preserve a field it does not
/// model, and a name that resolves to two payloads is not a mapping.
///
/// `source` may be a string, `null` or absent; all three mean "no recorded path"
/// (D4). An absent table is an empty mapping.
pub(crate) fn decode_rows(rows: &[Value]) -> Result<ResourceTables, StoreError> {
    let mut names: BTreeMap<(BundledKind, String), String> = BTreeMap::new();
    let mut sources: BTreeMap<(BundledKind, String), Option<String>> = BTreeMap::new();
    let mut by_name: BTreeMap<(BundledKind, String), String> = BTreeMap::new();

    for row in rows {
        let mut reader = Reader::new(row, TABLE)?;
        let key = reader.string("key")?;
        let token = reader.string("kind")?;
        let kind = BundledKind::parse(&token).ok_or_else(|| {
            malformed(
                TABLE,
                format!("kind: unknown resource kind {token:?} (expected font, xref or image)"),
            )
        })?;
        if !is_valid_bundled_key(kind, &key) {
            return Err(malformed(
                TABLE,
                format!("key: {kind} {key:?} is not a valid bundled-resource key"),
            ));
        }
        let name = reader.string("name")?;
        if name.trim().is_empty() {
            return Err(malformed(
                TABLE,
                format!("name: {kind} {key:?} has an empty logical name"),
            ));
        }
        let source = reader.string_if_present("source")?;
        reader.finish()?;

        // `(kind, name)` is the uniqueness key (D4): a second key for one name
        // would make the name resolve to two payloads.
        match by_name.entry((kind, name.clone())) {
            Entry::Occupied(existing) if *existing.get() == key => {
                return Err(malformed(
                    TABLE,
                    format!("{kind} {name:?} is listed twice for key {key:?}"),
                ));
            }
            Entry::Occupied(existing) => {
                return Err(StoreError::new(
                    markers::RESOURCE_NAME_CONFLICT,
                    format!(
                        "{TABLE}: {kind} {name:?} is bound to both {:?} and {key:?}",
                        existing.get()
                    ),
                ));
            }
            Entry::Vacant(slot) => {
                slot.insert(key.clone());
            }
        }
        if names.insert((kind, key.clone()), name).is_some() {
            return Err(malformed(TABLE, format!("duplicate resource key {key:?}")));
        }
        // One spelling for "no path": the map holds an entry only when a path was
        // recorded (`cad-db`'s builder removes it otherwise), so a `null` row does
        // not resurrect a `None` entry and a load → snapshot stays a fixed point.
        if let Some(source) = source {
            sources.insert((kind, key), Some(source));
        }
    }
    Ok((names, sources))
}

/// Refuse a payload no row names, or a row with no payload (D4/D5).
///
/// The two are one set: the row is the logical name of exactly one entry, and the
/// entry exists because the snapshot holds it. A mismatch would make the loaded
/// snapshot describe a resource the archive does not carry (or the reverse), which
/// `cad-db` refuses later with a weaker message.
pub(crate) fn check_mapping(
    payloads: &BTreeMap<(BundledKind, String), Arc<[u8]>>,
    tables: &ResourceTables,
) -> Result<(), StoreError> {
    let (names, _) = tables;
    for (kind, key) in payloads.keys() {
        if !names.contains_key(&(*kind, key.clone())) {
            return Err(malformed(
                &format!("{RESOURCES_PREFIX}{kind}/{key}"),
                format!("the payload is not listed in {TABLE}"),
            ));
        }
    }
    for ((kind, key), name) in names {
        if !payloads.contains_key(&(*kind, key.clone())) {
            return Err(malformed(
                TABLE,
                format!("{kind} {name:?} has no {RESOURCES_PREFIX}{kind}/{key} payload entry"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical 32-hex prefix of a key.
    const HEX: &str = "9f3a1c0d5e7b2468ace013579bdf2468";

    #[test]
    fn only_the_canonical_entry_form_is_modelled() {
        for (path, kind, key) in [
            (
                format!("resources/font/{HEX}.ttf"),
                BundledKind::Font,
                format!("{HEX}.ttf"),
            ),
            (
                format!("resources/xref/{HEX}.dwg"),
                BundledKind::Xref,
                format!("{HEX}.dwg"),
            ),
            (
                format!("resources/image/{HEX}.webp"),
                BundledKind::Image,
                format!("{HEX}.webp"),
            ),
        ] {
            assert_eq!(resource_entry(&path), Some((kind, key.as_str())));
            assert_eq!(
                resource_path(kind, &key).expect("valid"),
                path,
                "the path helper must agree with the predicate"
            );
        }
        for bad in [
            String::new(),
            "resources/".to_string(),
            "resources/font/".to_string(),
            format!("resources/font/{HEX}.TTF"),
            format!("resources/font/{HEX}.docx"),
            format!("resources/texture/{HEX}.png"),
            format!("resources/font/../{HEX}.ttf"),
            format!("resources/font//tmp/{HEX}.ttf"),
            format!("resources/font/{HEX}.ttf/"),
            format!("resources//{HEX}.ttf"),
            format!("textures/{HEX}.png"),
            format!("raw/{HEX}.ttf"),
            format!("{HEX}.ttf"),
        ] {
            assert_eq!(resource_entry(&bad), None, "accepted {bad:?}");
        }
        // A key that is not canonical cannot become a path either.
        assert_eq!(
            resource_path(BundledKind::Font, "../x.ttf")
                .unwrap_err()
                .marker,
            markers::RESOURCE_UNSUPPORTED_KIND
        );
    }

    #[test]
    fn a_key_must_name_its_payload_on_both_sides() {
        let bytes = b"a font payload";
        let digest = crate::json::sha256_hex(bytes);
        let key = format!("{}.ttf", &digest[..32]);
        check_read_key_matches_digest(BundledKind::Font, &key, &digest, "resources/font/x")
            .expect("the key names its content");
        check_write_key_matches_digest(BundledKind::Font, &key, &digest, "resources/font/x")
            .expect("the key names its content");

        let wrong = format!("{}.ttf", "0".repeat(32));
        // Load side: the manifest already described the bytes, so this is a
        // malformed archive.
        let error = check_read_key_matches_digest(BundledKind::Font, &wrong, &digest, "p")
            .expect_err("wrong prefix");
        assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
        // Write side: the caller computed the wrong key, which is not a corrupt
        // file (`resource.invalid_key`, not `ymodel.invalid_document`).
        let error = check_write_key_matches_digest(BundledKind::Font, &wrong, &digest, "p")
            .expect_err("wrong prefix");
        assert_eq!(error.marker, markers::RESOURCE_INVALID_KEY, "{error}");
    }

    #[test]
    fn the_read_budget_is_the_adr_per_entry_limit() {
        assert_eq!(MAX_RESOURCE_BYTES, 32 * 1024 * 1024);
        check_budget("resources/font/x", MAX_RESOURCE_BYTES).expect("at the limit");
        let error =
            check_budget("resources/font/x", MAX_RESOURCE_BYTES + 1).expect_err("over the limit");
        assert_eq!(error.marker, markers::RESOURCE_BUDGET_ENTRY, "{error}");
    }

    #[test]
    fn rows_round_trip_names_and_sources() {
        let hex = HEX;
        let rows = vec![
            serde_json::json!({
                "key": format!("{hex}.ttf"),
                "kind": "font",
                "name": "arial.ttf",
                "source": "C:/proj/fonts/arial.ttf",
            }),
            serde_json::json!({
                "key": format!("{hex}.png"),
                "kind": "image",
                "name": "logo.png",
                "source": null,
            }),
            // `source` may be absent, which decodes like `null` (D4).
            serde_json::json!({
                "key": format!("{hex}.dxf"),
                "kind": "xref",
                "name": "STRUCT",
            }),
        ];
        let (names, sources) = decode_rows(&rows).expect("decode");
        assert_eq!(names.len(), 3);
        assert_eq!(
            names[&(BundledKind::Font, format!("{hex}.ttf"))],
            "arial.ttf"
        );
        assert_eq!(
            sources[&(BundledKind::Font, format!("{hex}.ttf"))],
            Some("C:/proj/fonts/arial.ttf".to_string())
        );
        // "No path" has one spelling on both sides of a save: the absent key.
        assert!(!sources.contains_key(&(BundledKind::Image, format!("{hex}.png"))));
        assert!(!sources.contains_key(&(BundledKind::Xref, format!("{hex}.dxf"))));
        assert_eq!(sources.len(), 1);
        // An absent table is an empty mapping.
        assert_eq!(decode_rows(&[]).expect("empty").0.len(), 0);
    }

    #[test]
    fn malformed_rows_are_refused() {
        let key = format!("{HEX}.ttf");
        let good = serde_json::json!({
            "key": key,
            "kind": "font",
            "name": "arial.ttf",
            "source": null,
        });
        assert!(decode_rows(std::slice::from_ref(&good)).is_ok());

        let mut bad_rows: Vec<Value> = Vec::new();
        // Unknown field: the writer could not preserve it.
        let mut row = good.clone();
        row["future"] = Value::Bool(true);
        bad_rows.push(row);
        // Unknown kind.
        let mut row = good.clone();
        row["kind"] = Value::from("texture");
        bad_rows.push(row);
        // Non-canonical key.
        let mut row = good.clone();
        row["key"] = Value::from("../arial.ttf");
        bad_rows.push(row);
        // Empty name.
        let mut row = good.clone();
        row["name"] = Value::from("  ");
        bad_rows.push(row);
        // `source` of the wrong type.
        let mut row = good.clone();
        row["source"] = Value::from(7);
        bad_rows.push(row);
        for row in bad_rows {
            assert!(
                decode_rows(std::slice::from_ref(&row)).is_err(),
                "accepted {row}"
            );
        }

        // One name bound to two keys is `resource.name_conflict` (D4).
        let mut other = good.clone();
        other["key"] = Value::from(format!("{}.otf", "0".repeat(32)));
        let error = decode_rows(&[good.clone(), other]).expect_err("name conflict");
        assert_eq!(error.marker, markers::RESOURCE_NAME_CONFLICT, "{error}");

        // The same row twice is a duplicate, not a conflict.
        let error = decode_rows(&[good.clone(), good]).expect_err("duplicate");
        assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
    }

    #[test]
    fn the_mapping_and_the_payloads_must_describe_the_same_set() {
        let key = format!("{HEX}.ttf");
        let id = (BundledKind::Font, key.clone());
        let payloads = BTreeMap::from([(id.clone(), Arc::from(&b"font"[..]))]);
        let names = BTreeMap::from([(id.clone(), "arial.ttf".to_string())]);
        let sources = BTreeMap::from([(id.clone(), None)]);
        check_mapping(&payloads, &(names.clone(), sources.clone())).expect("consistent");

        let mut orphan_names = names.clone();
        orphan_names.insert(
            (BundledKind::Xref, format!("{HEX}.dwg")),
            "STRUCT".to_string(),
        );
        assert!(check_mapping(&payloads, &(orphan_names, sources.clone())).is_err());
        assert!(check_mapping(&BTreeMap::new(), &(names, sources)).is_err());
    }
}
