//! Document-bundled external resource contracts (ADR-0040 A3/A5).
//!
//! What must hold for the feature to be honest:
//!
//! 1. a bundled key is the canonical `<32 lower hex>.<allowed ext>` form or
//!    nothing — the same refusal at insert time and at load time;
//! 2. `(kind, name)` is unique, so one logical reference never resolves to two
//!    payloads (D4);
//! 3. the bytes, the name mapping and the collection-time source are document
//!    state: inserted through the builder, carried by [`DocumentSnapshot`], and
//!    readable through `resource*`/`resource_name`/`resource_key_for_name`;
//! 4. the policy is document state too: it round-trips through `document.json`
//!    and a `None` document is byte-compatible with a minor ≤ 4 archive.

use std::sync::Arc;

use cad_domain::*;

use crate::{DocumentSnapshot, DrawingDatabase, DrawingDatabaseBuilder};

const FONT_KEY: &str = "9f3a1c0d5e7b2468ace013579bdf2468.ttf";
const OTHER_FONT_KEY: &str = "1b074c2f2e0a5f3d9b7c1e8a6d4b2c0f.ttf";
const XREF_KEY: &str = "2c184d3f0e1b9a7c5d2e0f8b6a4c3e12.dwg";
const IMAGE_KEY: &str = "3d295e4f1a2c0b8d6e3f1a9c7b5d4e23.png";

fn font_bytes() -> Arc<[u8]> {
    Arc::from(vec![0x00u8, 0x01, 0x00, 0x00])
}

fn xref_bytes() -> Arc<[u8]> {
    Arc::from(b"AC1032".to_vec())
}

fn image_bytes() -> Arc<[u8]> {
    Arc::from(vec![0x89u8, 0x50, 0x4e, 0x47])
}

fn database_with_resources() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_resource(BundledKind::Font, FONT_KEY, "arial.ttf", font_bytes())
        .expect("font");
    builder
        .set_resource_source(BundledKind::Font, FONT_KEY, Some("C:/proj/fonts/arial.ttf"))
        .expect("font source");
    builder
        .insert_resource(BundledKind::Xref, XREF_KEY, "STRUCT", xref_bytes())
        .expect("xref");
    builder
        .insert_resource(BundledKind::Image, IMAGE_KEY, "logo.png", image_bytes())
        .expect("image");
    builder.finish().expect("database")
}

fn snapshot_of(db: &DrawingDatabase) -> DocumentSnapshot {
    db.snapshot(
        UnitContext::drawing_units(),
        DocumentIdentity::Temporary(1),
        Vec::new(),
        Revision(1),
    )
    .expect("snapshot")
}

// ---------------------------------------------------------------------------
// Insert-time refusals.
// ---------------------------------------------------------------------------

#[test]
fn insert_resource_refuses_a_key_the_archive_could_not_name() {
    for (kind, key) in [
        // Paths, separators and traversal never name an entry.
        (BundledKind::Font, "/tmp/arial.ttf".to_string()),
        (BundledKind::Font, "resources/arial.ttf".to_string()),
        (BundledKind::Font, "..".to_string()),
        (BundledKind::Font, "C:\\fonts\\arial.ttf".to_string()),
        // Case is never normalised.
        (BundledKind::Font, FONT_KEY.to_uppercase()),
        (
            BundledKind::Font,
            "9f3a1c0d5e7b2468ace013579bdf2468.TTF".to_string(),
        ),
        // Not in this kind's allowlist.
        (
            BundledKind::Font,
            "9f3a1c0d5e7b2468ace013579bdf2468.png".to_string(),
        ),
        (BundledKind::Image, FONT_KEY.to_string()),
        (BundledKind::Xref, FONT_KEY.to_string()),
        // Not the canonical hex prefix.
        (BundledKind::Font, "arial.ttf".to_string()),
        (BundledKind::Font, "9f3a.ttf".to_string()),
        (
            BundledKind::Font,
            "9f3a1c0d5e7b2468ace013579bdf24689.ttf".to_string(),
        ),
        (BundledKind::Font, String::new()),
    ] {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
        let error = builder
            .insert_resource(kind, &key, "arial.ttf", font_bytes())
            .expect_err("an invalid key must be refused");
        assert!(
            matches!(&error, CadError::InvalidInput(_)),
            "{kind} {key:?}: {error:?}"
        );
        assert!(
            error.to_string().contains("resource.unsupported_kind"),
            "{kind} {key:?}: {error}"
        );
    }
    // The canonical form is accepted for every kind.
    for (kind, key) in [
        (BundledKind::Font, FONT_KEY),
        (BundledKind::Xref, XREF_KEY),
        (BundledKind::Image, IMAGE_KEY),
    ] {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
        builder
            .insert_resource(kind, key, "name", font_bytes())
            .expect("canonical key");
    }
}

#[test]
fn insert_resource_refuses_an_empty_payload_or_name() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    let error = builder
        .insert_resource(
            BundledKind::Font,
            FONT_KEY,
            "arial.ttf",
            Arc::from(Vec::new()),
        )
        .expect_err("a zero-byte font can never resolve");
    assert!(
        error.to_string().contains("resource.unavailable"),
        "{error}"
    );

    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for name in ["", "   "] {
        let error = builder
            .insert_resource(BundledKind::Font, FONT_KEY, name, font_bytes())
            .expect_err("a nameless resource is unreachable");
        assert!(matches!(&error, CadError::InvalidInput(_)), "{error:?}");
        assert!(error.to_string().contains("empty logical name"), "{error}");
    }
}

#[test]
fn insert_resource_is_idempotent_for_one_content_addressed_key() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_resource(BundledKind::Font, FONT_KEY, "arial.ttf", font_bytes())
        .expect("first");
    builder
        .insert_resource(BundledKind::Font, FONT_KEY, "arial.ttf", font_bytes())
        .expect("the same key and bytes are a no-op");
    // Different bytes under one content-addressed key is a caller bug, not a
    // silent replace: whatever references the key must keep its payload.
    assert!(builder
        .insert_resource(BundledKind::Font, FONT_KEY, "arial.ttf", xref_bytes())
        .is_err());
    // Same bytes, different logical name: one payload cannot answer to two names.
    let error = builder
        .insert_resource(BundledKind::Font, FONT_KEY, "arial-bold.ttf", font_bytes())
        .expect_err("a second name for one key");
    assert!(
        error.to_string().contains("resource.name_conflict"),
        "{error}"
    );

    let db = builder.finish().expect("database");
    assert_eq!(db.resources().count(), 1);
    assert_eq!(
        db.resource_name(BundledKind::Font, FONT_KEY),
        Some("arial.ttf")
    );
}

#[test]
fn a_second_key_cannot_claim_a_used_name() {
    // Distinct bytes, same logical name, same kind: the name would resolve to
    // two payloads, so it is refused (ADR-0040 D4).
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_resource(BundledKind::Font, FONT_KEY, "arial.ttf", font_bytes())
        .expect("first");
    let error = builder
        .insert_resource(
            BundledKind::Font,
            OTHER_FONT_KEY,
            "arial.ttf",
            Arc::from(vec![7u8, 7, 7]),
        )
        .expect_err("duplicate name");
    assert!(matches!(&error, CadError::InvalidInput(_)), "{error:?}");
    assert!(
        error.to_string().contains("resource.name_conflict"),
        "{error}"
    );
    assert!(error.to_string().contains(FONT_KEY), "{error}");

    // `(kind, name)` is the uniqueness key, so the same name is fine for another
    // kind (an image and a font may both be called `logo`).
    builder
        .insert_resource(BundledKind::Image, IMAGE_KEY, "arial.ttf", image_bytes())
        .expect("a name is unique per kind, not globally");
}

// ---------------------------------------------------------------------------
// Snapshot round trip.
// ---------------------------------------------------------------------------

#[test]
fn resources_name_mapping_and_sources_round_trip_through_a_snapshot() {
    let db = database_with_resources();
    let snapshot = snapshot_of(&db);
    assert_eq!(snapshot.resources.len(), 3);
    assert_eq!(
        snapshot.resource_names[&(BundledKind::Font, FONT_KEY.to_string())],
        "arial.ttf"
    );
    assert_eq!(
        snapshot.resource_sources[&(BundledKind::Font, FONT_KEY.to_string())],
        Some("C:/proj/fonts/arial.ttf".to_string())
    );
    // An unrecorded source is simply absent from the map: "no path" and
    // `Some(None)` mean the same thing, and the table writes both as `null`.
    assert_eq!(
        snapshot
            .resource_sources
            .get(&(BundledKind::Xref, XREF_KEY.to_string())),
        None
    );

    let hydrated = DrawingDatabase::from_snapshot(&snapshot).expect("hydrate");
    // Bytes are document state: identical, not merely the same length.
    assert_eq!(
        hydrated
            .resource(BundledKind::Font, FONT_KEY)
            .map(|bytes| &bytes[..]),
        Some(&font_bytes()[..])
    );
    assert_eq!(
        hydrated
            .resource(BundledKind::Xref, XREF_KEY)
            .map(|bytes| &bytes[..]),
        Some(&xref_bytes()[..])
    );
    assert_eq!(
        hydrated
            .resource(BundledKind::Image, IMAGE_KEY)
            .map(|bytes| &bytes[..]),
        Some(&image_bytes()[..])
    );
    // The key ⇄ name mapping is identical in both directions.
    for (kind, key, name) in [
        (BundledKind::Font, FONT_KEY, "arial.ttf"),
        (BundledKind::Xref, XREF_KEY, "STRUCT"),
        (BundledKind::Image, IMAGE_KEY, "logo.png"),
    ] {
        assert_eq!(hydrated.resource_name(kind, key), Some(name));
        assert_eq!(hydrated.resource_key_for_name(kind, name), Some(key));
        assert_eq!(db.resource_name(kind, key), Some(name));
        assert_eq!(db.resource_key_for_name(kind, name), Some(key));
    }
    assert_eq!(
        hydrated.resource_source(BundledKind::Font, FONT_KEY),
        Some("C:/proj/fonts/arial.ttf")
    );
    assert_eq!(hydrated.resource_source(BundledKind::Xref, XREF_KEY), None);

    // A name is scoped to its kind: the xref named `STRUCT` does not answer as a
    // font, and an unknown name is `None` (never a guessed path).
    assert_eq!(
        hydrated.resource_key_for_name(BundledKind::Font, "STRUCT"),
        None
    );
    assert_eq!(
        hydrated.resource_key_for_name(BundledKind::Font, "missing.ttf"),
        None
    );
    assert_eq!(
        hydrated.resource(BundledKind::Font, XREF_KEY),
        None,
        "the same key text under another kind is a different resource"
    );

    // Snapshot -> database -> snapshot is a fixed point, bytes included.
    let again = snapshot_of(&hydrated);
    assert_eq!(again, snapshot);
    assert!(
        again.bytes_equal(&snapshot),
        "the payload bytes survive the round trip"
    );
}

#[test]
fn embed_resources_is_document_state_and_survives_a_snapshot() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_resource(BundledKind::Font, FONT_KEY, "arial.ttf", font_bytes())
        .expect("font");
    builder.set_embed_resources(true);
    let db = builder.finish().expect("database");
    assert!(db.embed_resources());

    let snapshot = snapshot_of(&db);
    assert!(snapshot.header.embed_resources);
    let hydrated = DrawingDatabase::from_snapshot(&snapshot).expect("hydrate");
    assert!(
        hydrated.embed_resources(),
        "the toggle is document state, not a carrier preference"
    );

    // `false` is the default and survives a round trip as `false`.
    let plain = DrawingDatabaseBuilder::new(DatabaseId(2))
        .finish()
        .expect("database");
    assert!(!plain.embed_resources());
    let hydrated = DrawingDatabase::from_snapshot(&snapshot_of(&plain)).expect("hydrate");
    assert!(!hydrated.embed_resources());
}

#[test]
fn drop_resource_removes_the_payload_name_and_source() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_resource(BundledKind::Font, FONT_KEY, "arial.ttf", font_bytes())
        .expect("font");
    builder
        .set_resource_source(BundledKind::Font, FONT_KEY, Some("arial.ttf"))
        .expect("source");
    assert!(builder.drop_resource(BundledKind::Font, FONT_KEY));
    // Dropping an absent resource is a no-op, not an error: a collector running
    // twice over a changed policy must not fail.
    assert!(!builder.drop_resource(BundledKind::Font, FONT_KEY));

    let db = builder.finish().expect("database");
    assert!(db.resource(BundledKind::Font, FONT_KEY).is_none());
    assert!(db.resource_name(BundledKind::Font, FONT_KEY).is_none());
    assert!(db.resource_source(BundledKind::Font, FONT_KEY).is_none());
    assert_eq!(db.resources().count(), 0);
}

#[test]
fn set_resource_source_refuses_an_absent_resource() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    let error = builder
        .set_resource_source(BundledKind::Font, FONT_KEY, Some("arial.ttf"))
        .expect_err("provenance for an absent resource is unreachable");
    assert!(matches!(&error, CadError::Invariant(_)), "{error:?}");

    // An empty path records `None`, never a `Some("")` a resolver could open.
    builder
        .insert_resource(BundledKind::Font, FONT_KEY, "arial.ttf", font_bytes())
        .expect("font");
    builder
        .set_resource_source(BundledKind::Font, FONT_KEY, Some("  "))
        .expect("blank source");
    assert_eq!(
        builder
            .finish()
            .expect("db")
            .resource_source(BundledKind::Font, FONT_KEY),
        None
    );
}

// ---------------------------------------------------------------------------
// Table presence: `tables/resources.json` only when bytes exist (ADR-0040 D12).
// ---------------------------------------------------------------------------

#[test]
fn a_document_without_resources_writes_no_resource_table_at_all() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(3));
    builder.set_embed_resources(false);
    let db = builder.finish().expect("database");
    let snapshot = snapshot_of(&db);
    assert!(snapshot.resources.is_empty());
    assert!(
        snapshot.tables.get(crate::TableKind::Resources).is_none(),
        "a plain document must not carry tables/resources.json: a pre-0040 reader \
         would refuse the entry it cannot model (D12)"
    );

    // Absence is "no resources", never an error: a document with no table loads
    // to an empty map and the round trip stays a fixed point.
    let hydrated = DrawingDatabase::from_snapshot(&snapshot).expect("hydrate without table");
    assert_eq!(hydrated.resources().count(), 0);
    assert!(hydrated.resource_names.is_empty());
    assert!(hydrated.resource_sources.is_empty());
    let again = snapshot_of(&hydrated);
    assert_eq!(again, snapshot);
    assert!(again.tables.get(crate::TableKind::Resources).is_none());
}

#[test]
fn a_document_with_resources_writes_the_resource_table() {
    let snapshot = snapshot_of(&database_with_resources());
    let rows = snapshot
        .tables
        .get(crate::TableKind::Resources)
        .expect("one resource means the mapping table exists");
    assert_eq!(rows.len(), 3, "one row per bundled resource");
}

// ---------------------------------------------------------------------------
// `tables/resources.json` row shape (ADR-0040 D4).
// ---------------------------------------------------------------------------

#[test]
fn the_resource_table_is_a_plain_row_array() {
    use crate::snapshot::resources;
    use crate::TableSnapshot;

    let db = database_with_resources();
    let rows = resources::encode(&db).expect("encode");
    // Every table is a `Vec<Value>` of rows: no wrapper object, no `version`.
    assert_eq!(rows.len(), 3, "one row per resource");
    for row in &rows {
        let map = row.as_object().expect("a row object");
        assert_eq!(map.len(), 4, "key/kind/name/source only: {map:?}");
        for field in ["key", "kind", "name", "source"] {
            assert!(map.contains_key(field), "{field} is required: {map:?}");
        }
        assert!(map.get("version").is_none());
    }
    let font = rows
        .iter()
        .find(|row| row["kind"] == "font")
        .expect("the font row");
    assert_eq!(font["source"], serde_json::json!("C:/proj/fonts/arial.ttf"));
    // A row with no recorded path writes `null`, not an omitted key.
    let image = rows
        .iter()
        .find(|row| row["kind"] == "image")
        .expect("the image row");
    assert_eq!(image["source"], serde_json::Value::Null);

    let mut tables = TableSnapshot::default();
    tables.set(crate::TableKind::Resources, rows.clone());
    let (names, sources) = resources::decode(&tables).expect("decode");
    assert_eq!(
        names[&(BundledKind::Font, FONT_KEY.to_string())],
        "arial.ttf"
    );
    assert_eq!(
        names[&(BundledKind::Image, IMAGE_KEY.to_string())],
        "logo.png"
    );
    assert_eq!(
        sources[&(BundledKind::Font, FONT_KEY.to_string())],
        Some("C:/proj/fonts/arial.ttf".to_string())
    );
    assert_eq!(sources[&(BundledKind::Image, IMAGE_KEY.to_string())], None);

    // A reader also accepts `source` being absent entirely: both spellings mean
    // "no recorded path" (ADR-0040 D4).
    let mut without = rows.clone();
    for row in &mut without {
        row.as_object_mut().expect("row").remove("source");
    }
    tables.set(crate::TableKind::Resources, without);
    let (names, sources) = resources::decode(&tables).expect("decode without source");
    assert_eq!(names.len(), 3);
    assert!(sources.values().all(Option::is_none));
}

#[test]
fn a_duplicate_resource_name_or_key_is_refused() {
    use crate::snapshot::resources;
    use crate::TableSnapshot;

    fn row(key: &str, name: &str) -> serde_json::Value {
        serde_json::json!({ "key": key, "kind": "font", "name": name, "source": null })
    }
    let mut tables = TableSnapshot::default();

    // Two keys, one logical name: the name would resolve to two payloads.
    tables.set(
        crate::TableKind::Resources,
        vec![row(FONT_KEY, "arial.ttf"), row(OTHER_FONT_KEY, "arial.ttf")],
    );
    let error = resources::decode(&tables).expect_err("duplicate name");
    assert_eq!(error.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
    assert!(
        error.detail.contains("resource.name_conflict"),
        "{}",
        error.detail
    );
    assert!(
        error.detail.contains("tables/resources.json"),
        "{}",
        error.detail
    );

    // One key twice: the mapping is a map, not a list of claims.
    tables.set(
        crate::TableKind::Resources,
        vec![row(FONT_KEY, "arial.ttf"), row(FONT_KEY, "arial-bold.ttf")],
    );
    let error = resources::decode(&tables).expect_err("duplicate key");
    assert!(
        error.detail.contains("duplicate resource key"),
        "{}",
        error.detail
    );

    // A kind this build cannot model, and a key the format cannot name.
    tables.set(
        crate::TableKind::Resources,
        vec![serde_json::json!({
            "key": FONT_KEY, "kind": "texture", "name": "arial.ttf", "source": null })],
    );
    let error = resources::decode(&tables).expect_err("unknown kind");
    assert!(
        error.detail.contains("unknown resource kind"),
        "{}",
        error.detail
    );
    tables.set(
        crate::TableKind::Resources,
        vec![row("../escape.ttf", "arial.ttf")],
    );
    let error = resources::decode(&tables).expect_err("bad key");
    assert!(
        error.detail.contains("not a valid bundled-resource key"),
        "{}",
        error.detail
    );

    // A stray field is refused (the writer re-serializes whole rows, so an
    // unmodelled field would be dropped by the next save).
    tables.set(
        crate::TableKind::Resources,
        vec![serde_json::json!({
            "key": FONT_KEY, "kind": "font", "name": "arial.ttf",
            "source": null, "sha256": "00" })],
    );
    let error = resources::decode(&tables).expect_err("unknown field");
    assert!(error.detail.contains("unknown field"), "{}", error.detail);
}

// ---------------------------------------------------------------------------
// Load-time refusals.
// ---------------------------------------------------------------------------

#[test]
fn a_payload_without_a_table_row_or_a_row_without_bytes_refuses_the_load() {
    // (a) bytes with no logical name: nothing could reference them, and the next
    // save would drop them silently.
    let mut snapshot = snapshot_of(&database_with_resources());
    let rows = snapshot
        .tables
        .rows
        .get_mut(&crate::TableKind::Resources)
        .expect("the resource table is written");
    rows.retain(|row| row["key"] != FONT_KEY);
    let error = DrawingDatabase::from_snapshot(&snapshot).expect_err("orphan payload");
    assert_eq!(error.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
    assert!(error.detail.contains("not listed"), "{error}");

    // (b) a listed name with no payload entry: a dangling reference.
    let mut snapshot = snapshot_of(&database_with_resources());
    snapshot
        .resources
        .remove(&(BundledKind::Xref, XREF_KEY.to_string()));
    let error = DrawingDatabase::from_snapshot(&snapshot).expect_err("dangling name");
    assert_eq!(error.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
    assert!(error.detail.contains("no resources/xref"), "{error}");
}

#[test]
fn an_unusable_resource_in_a_snapshot_refuses_the_load() {
    // A store-side bug (or a hand-edited archive) must not put a resource the
    // format cannot name into memory: the table codec re-runs the key checks on
    // both sides (ADR-0040 D1/D12).
    let mut snapshot = snapshot_of(&database_with_resources());
    let rows = snapshot
        .tables
        .rows
        .get_mut(&crate::TableKind::Resources)
        .expect("the resource table is written");
    let image_row = rows
        .iter_mut()
        .find(|row| row["kind"] == "image")
        .expect("the image row");
    image_row["key"] = serde_json::Value::from("../escape.png");
    snapshot.resources.insert(
        (BundledKind::Image, "../escape.png".to_string()),
        Arc::from(vec![1u8]),
    );
    snapshot
        .resources
        .remove(&(BundledKind::Image, IMAGE_KEY.to_string()));

    let error = DrawingDatabase::from_snapshot(&snapshot).expect_err("refused");
    assert_eq!(error.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
    assert!(error.detail.contains("tables/resources.json"), "{error}");
    assert!(
        error.detail.contains("not a valid bundled-resource key"),
        "{error}"
    );
}
