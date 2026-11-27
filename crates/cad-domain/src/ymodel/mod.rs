//! ymodel: the persistent document model (ADR-0037).
//!
//! This module owns the pieces of the on-disk format that are *not* a view of
//! `cad-db` state: stable identity kinds, the document header, the class
//! descriptor registry, and the JSON codecs that both the store and the
//! connectors share.
//!
//! It contains no I/O. Reading and writing `.ymodel` archives lives in
//! `cad-store-zip`, which is deliberately not reachable from any `cad-*`
//! shared layer (`scripts/check-architecture.py`).
//!
//! Two rules from the ADR are enforced here rather than left to convention:
//!
//! 1. Identity kinds and their numeric values are part of the file format.
//!    They are never renumbered and never reused.
//! 2. A persisted identifier is always ≤ 2^53 so JSON numbers stay exact in
//!    f64-based tooling (`jq`, browser `JSON.parse`). Larger identifiers must
//!    be written as strings by the codec that owns them.
//!
//! Modules:
//!
//! * [`id`] — the identity kinds and the `(kind << 45) | seq` persistent id.
//! * [`json`] — the numeric codec: the `NaN`/`±inf` escape and the canonical
//!   (sorted) key order of every entry this format writes.
//! * [`header`] — `document.json`: units, identity, resource keys and the
//!   persisted id high-water marks.
//! * [`descriptor`] — the class registry: one class per `SemanticGeometry`
//!   variant plus the database-owned `cad:Element` envelope.
//! * [`payload`] — `SemanticGeometry` ⇄ payload JSON, strict in both directions
//!   and a fixed point.

pub mod descriptor;
pub mod header;
mod header_domain;
pub mod id;
pub mod json;
pub mod payload;

pub use descriptor::{
    all as all_classes, by_class_id, by_type_key, ClassDescriptor, ClassId, FieldDescriptor,
    FieldKind, SchemaId, CAD_ELEMENT, CLASSES, ELEMENT_SCHEMA, GEOMETRY_SCHEMA,
};
pub use header::{DocumentHeader, HeaderError, IdCounters, COUNTER_CEILING};
pub use id::{
    shard_of, IdKind, ModelKey, PersistentId, PersistentIdOf, Shard, MAX_ID, MAX_SEQ, SEQ_BITS,
    SHARD_ELEMENTS,
};
pub use json::{decode_f64, decode_point3, encode_f64, encode_point3, JsonError};
pub use payload::{decode as decode_payload, encode as encode_payload, PayloadError};

/// Container name written into `manifest.json`.
pub const FORMAT_NAME: &str = "ymodel";

/// The `Read.Write` version this build understands. Read and write are equal:
/// the gate is an **equality** gate tied to the entry codec (`codec.rs`/`read.rs`
/// in `cad-store-zip`), not a minimum.
///
/// `2` changed the payload of every modelled zip entry from deflated JSON to one
/// RFC 8878 zstd frame in a `Stored` entry, and added the manifest's `codec` /
/// `len_c` fields plus a `sha256_c` this build computes and verifies before
/// decompression. A version-1 archive has a different entry codec this build does
/// not carry, so it is refused with [`markers::UNSUPPORTED_VERSION`] and must be
/// re-imported from its source document. `write` is the version it emits.
pub const FORMAT_READ: u32 = 2;
pub const FORMAT_WRITE: u32 = 2;

/// Feature level inside one `Read.Write` version pair.
///
/// A document whose `manifest.format.minor` is greater than [`FORMAT_MINOR`]
/// is refused ([`markers::UNSUPPORTED_MINOR`]) instead of being opened and
/// re-saved with unknown fields dropped: the writer re-serializes whole
/// shards, so it cannot preserve fields it does not model.
///
/// `2` adds the host-owned metadata tables `tables/sources.json` and
/// `tables/compat.json` (ADR-0037 D11/D14). A build that only knows `1` must
/// not open such a file: saving it back would drop the import provenance and
/// the object-level report.
///
/// `3` adds mesh UV0 + base-colour textures (ADR-0038 D4): `cad:Mesh` gains
/// `uvs`/`texture`, and the `textures/<key>` binary entries carry the image
/// bytes. Those two payload fields are the format's documented
/// "absent means empty/None" exceptions, scoped to `decode_mesh` only, so a
/// minor-2 archive still opens; every other missing field remains an error.
///
/// `4` adds `cad:Mesh.pbr` (ADR-0039 D2): the metallic-roughness scalars and the
/// four PBR texture keys, as `null` or one nested object. It joins the same
/// absent-means-`None` exception, so a minor-3 archive still opens; the four
/// texture slots reuse the existing `textures/<key>` entries.
///
/// `5` adds bundled external resources (ADR-0040): `resources/<kind>/<key>`
/// binary entries, the `tables/resources.json` name ⇄ key mapping and the
/// `embed_resources` header flag. A writer stamps `5` **per document** — only
/// when it packs a resource or sets that flag — so an ordinary document stays a
/// minor-4 archive that an older build can still open (D12). A reader that only
/// knows `4` must refuse a `5` archive rather than drop the entries it cannot
/// model.
///
/// `6` adds the optional `bim` block on an element row and the
/// `tables/bim-definitions.json` table (ADR-0042 D2/D3/D6). The block is
/// written only when an element carries BIM data — `None` omits the key
/// entirely, never `"bim": null`, because a minor-5 reader is strict about
/// unknown element fields — and the table only when it holds rows, so an
/// ordinary document keeps the minor its resources (or its absence) imply.
///
/// `7` adds `tables/bim-relations.json` (ADR-0042 D3 tail/D12): the bounded
/// many-to-many `aggregates`/`nests`/`contains`/`group`/`connects`/other edges
/// between BIM components. The table is written only when it holds rows, but
/// the **tier** is shared with the minor-6 features: a document with BIM
/// definitions/element blocks but no relation table is also written at `7`
/// (both landed in one release, and a minor-6 reader refuses the element block
/// anyway, so splitting them would not let an older build open anything more).
/// No `IdKind` 12/13 and no header counter backs these rows.
///
/// `8` adds document parameters (ADR-0043 D11): the `tables/parameters.json`
/// row array and the optional `params` block on an element row. Both are
/// written only when the document actually holds them, so an ordinary document
/// keeps the minor its other features imply. The block is the third
/// absent-means-`None` element field (`bim`, then this), and a minor-7 reader
/// refuses it rather than dropping it.
///
/// `9` adds the P1 expression/naming features (ADR-0043 P1): `neg`/`pow`/`fn`/`if`
/// tags in a parameter expression, plus the P1 configuration table. The tier is
/// per document (`has_p1_params`), because a minor-8 reader would refuse an
/// unknown expression tag as `ymodel.invalid_document` rather than
/// `unsupported_minor`.
///
/// `10` adds `tables/features.json` (ADR-0043 P2 schema) and `11` adds
/// `tables/relations.json` (ADR-0043 P3 in-context edges). Both are written only
/// when non-empty; their evaluators are not wired, so the tables are the
/// explicit schema boundary rather than a claimed feature pipeline.
///
/// `12` adds the host-owned `tables/vendor.json` (ADR-0030 D7): the
/// version-independent vendor semantics for imported vendor objects, one row per
/// entity keyed by entity id. It is written only when the document has vendor
/// rows, so a document without them keeps the minor its other features imply.
///
/// `13` adds the `document.json` `application_domain` scalar (ADR-0053 D2): the
/// interface domain the document is edited in. It is written only when the
/// document names one, so an older archive and a document that never chose a
/// profile stay at their previous minor.
///
/// `14` adds block attributes (ADR-0055): the `cad:AttributeDefinition` class and
/// the optional `cad:Insert.attributes` array. Both are per-document tiers — a
/// definition is a block member entity and a value list rides in the insert's
/// element row, so no archive table is added. The value array is omitted when
/// empty, so an ordinary drawing and a minor-13 archive keep their minor; a
/// minor-13 reader refuses a document that really carries attributes rather than
/// dropping them.
///
/// `15` adds the `cad:Text.mirrored` boolean (the `MIRRTEXT=1` glyph-frame flag).
/// It is omitted when false, so ordinary text and every older document keep their
/// minor; a minor-14 reader refuses a document that really carries a mirrored run
/// rather than drawing it unmirrored.
///
/// `16` is reserved for the bounded 2D constraint table (ADR-0054).
///
/// `17` adds persisted entity groups (`tables/groups.json`): a named, ordered
/// member set with a table-local sequence identity, a description and the
/// DXF selectable flag.
///
/// `18` adds persisted named views (`tables/views.json`): the camera, 2D/3D
/// mode, work plane and active space of every saved view. A document without a
/// row keeps the lower tier; a reader below `18` refuses the table path instead
/// of dropping the views.
///
/// `19` adds document materials (`tables/materials.json`, SketchUp Paint
/// Bucket): named flat materials plus one assignment per painted entity, inline
/// in the row. A document nobody painted keeps the lower tier; a pre-19 reader
/// refuses the table path instead of dropping the paint.
///
/// `20` adds scenes (`tables/scenes.json`, SketchUp pages): a named camera/space
/// snapshot plus the visual-style token. A document without scenes keeps the
/// lower tier; a pre-20 reader refuses the table path instead of dropping them.
///
/// `21` adds the per-entity `render_attributes.soften` angle (SketchUp's Soften
/// Edges). The key is omitted unless set, so an unsoftened document keeps the
/// lower tier; a pre-21 reader refuses the key instead of drawing hard edges.
pub const FORMAT_MINOR: u32 = 21;

/// Stable failure markers.
///
/// These strings, not process exit codes, are the contract: wasm and browser
/// carriers have no exit status, and the desktop hosts only map these onto
/// exit code 8. Callers must match on the marker.
pub mod markers {
    /// `manifest.format.read` is not [`super::FORMAT_READ`] (an equality gate:
    /// the archive uses an entry codec this build does not read).
    pub const UNSUPPORTED_VERSION: &str = "ymodel.unsupported_version";
    /// `manifest.format.minor` is newer than [`super::FORMAT_MINOR`].
    pub const UNSUPPORTED_MINOR: &str = "ymodel.unsupported_minor";
    /// A foreign (never-modelled) entry could not be preserved.
    pub const FOREIGN_ENTRIES_UNREADABLE: &str = "ymodel.foreign_entries_unreadable";
    /// The archive is malformed: missing entry, bad hash, bad size, bad JSON.
    pub const INVALID_DOCUMENT: &str = "ymodel.invalid_document";
    /// A model key has no id this format can read back, so a save would write an
    /// entry its own reader rejects.
    ///
    /// Sequence 0 belongs to model space: [`super::ModelKey::from_persistent`]
    /// refuses a `PaperModel`/`BlockModel` id with sequence 0, which is exactly
    /// what `Paper(LayoutId(0))`/`Block(BlockId(0))` encode. The writer refuses
    /// such a snapshot up front (defense in depth: the allocators never issue 0)
    /// instead of producing an `elements/…` or `tables/models.json` row no build
    /// can decode. The detail names the entry path and the offending key.
    pub const UNREPRESENTABLE_MODEL: &str = "ymodel.unrepresentable_model";
    /// The file on disk changed since it was opened (`sha256`/`mtime` drift).
    pub const CONFLICT: &str = "ymodel.conflict";
    /// Writing would exceed the carrier's serialization budget.
    pub const PERSISTENCE_TOO_LARGE: &str = "web.persistence_too_large";
    /// A resource key names a kind/extension this build cannot model
    /// (ADR-0040 D1/D12).
    pub const RESOURCE_UNSUPPORTED_KIND: &str = "resource.unsupported_kind";
    /// A caller computed a resource key that does not name the bytes it hands in
    /// (ADR-0040 D1/D12): a collector bug, not a malformed document.
    pub const RESOURCE_INVALID_KEY: &str = "resource.invalid_key";
    /// Two distinct keys claim one logical resource name (ADR-0040 D4/D12).
    pub const RESOURCE_NAME_CONFLICT: &str = "resource.name_conflict";
    /// A resource could not be collected or read (ADR-0040 D9/D12).
    pub const RESOURCE_UNAVAILABLE: &str = "resource.unavailable";
    /// One collected resource exceeds the per-entry budget (ADR-0040 D10).
    pub const RESOURCE_BUDGET_ENTRY: &str = "resource.budget.entry";
    /// The collected resources exceed the total budget (ADR-0040 D10).
    pub const RESOURCE_BUDGET_TOTAL: &str = "resource.budget.total";
    /// A previous archive holds different bytes at a modelled resource path
    /// (ADR-0040 D8): the collision is refused instead of silently overwritten.
    pub const RESOURCE_CONFLICT: &str = "resource.conflict";
    /// A recovery archive dropped embedded resources (ADR-0040 D11).
    pub const RECOVERY_RESOURCES_OMITTED: &str = "recovery.resources_omitted";
}

/// Exit code the CLI and `native-below` hosts map [`markers`] onto.
///
/// Carriers without a process exit status (wasm, browser) report the marker
/// instead; this constant exists so the native mapping is written once.
pub const EXIT_CODE_UNSUPPORTED: i32 = 8;
