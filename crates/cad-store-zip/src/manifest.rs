//! `manifest.json`: the container's table of contents and the format's version
//! gate (ADR-0037 D11, D13).
//!
//! ```text
//! {"document_uuid": "…", "entries": [{"codec": "zstd", "crc": 1,
//!                                     "foreign": false, "len": 2, "len_c": 1,
//!                                     "path": "document.json",
//!                                     "sha256": "…", "sha256_c": "…"}, …],
//!  "format": {"minor": 2, "read": 2, "write": 2}, "generator": "yacr/0.1.16",
//!  "open_revision": 0, "saved_at": null, "saved_revision": 7,
//!  "shard_map": {"10": ["elements/10-0.json"]}}
//! ```
//!
//! Every encoded zip entry is `Stored` (a copied foreign entry keeps its original
//! compression method); `codec` says how to read its payload (`zstd`, `raw` or
//! `opaque`), `len` is the logical (decompressed) length and `len_c` the stored
//! payload's length. `sha256` describes the logical bytes, `sha256_c` the stored
//! payload (absent exactly for `opaque` entries), and `crc` is the central
//! directory's CRC — for a `Stored` entry, the stored payload's.
//!
//! Rules enforced here rather than left to convention:
//!
//! * `manifest.json` never lists itself — a self-referential hash has no fixed
//!   point, and its integrity is the zip CRC (D11);
//! * `entries` is sorted by `path`, so two writes of one snapshot produce one
//!   sequence of entry records;
//! * `shard_map` keys are the raw decimal model ids, values the shard entries of
//!   that model in sorted order;
//! * every hash is 64 lowercase hex digits, the single spelling this format
//!   emits (an uppercase hash is a different string, so accepting both would make
//!   the archive's bytes reader-dependent);
//! * unknown manifest fields are refused: a newer writer's field would be
//!   dropped by the next save (D11).

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use cad_db::{FormatVersion, ManifestSummary, StoreError};
use cad_domain::Revision;

use crate::json::{malformed, Reader};
use crate::MANIFEST_ENTRY;

/// Context string for every error raised while decoding this entry.
pub(crate) const CONTEXT: &str = "manifest.json";

/// How one entry's stored payload is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Codec {
    /// One RFC 8878 zstd frame holding the entry's JSON bytes.
    Zstd,
    /// The payload is the entry's bytes verbatim.
    Raw,
    /// A foreign/unmodelled entry: never decoded, only copied.
    Opaque,
}

impl Codec {
    /// The manifest spelling of this codec.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Codec::Zstd => "zstd",
            Codec::Raw => "raw",
            Codec::Opaque => "opaque",
        }
    }

    /// Parse the manifest spelling; anything else is malformed.
    fn parse(token: &str) -> Option<Codec> {
        match token {
            "zstd" => Some(Codec::Zstd),
            "raw" => Some(Codec::Raw),
            "opaque" => Some(Codec::Opaque),
            _ => None,
        }
    }
}

/// One row of `manifest.entries`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntryRecord {
    pub path: String,
    /// How the stored payload is interpreted.
    pub codec: Codec,
    /// Lowercase hex SHA-256 of the **logical** bytes (after decoding).
    pub sha256: String,
    /// Lowercase hex SHA-256 of the **stored** payload, when this build modelled
    /// it; `None` exactly for [`Codec::Opaque`].
    pub sha256_c: Option<String>,
    /// Logical (decompressed) length.
    pub len: u64,
    /// Stored payload length, i.e. the zip entry's `compressed_size`.
    pub len_c: u64,
    /// The zip CRC-32 the central directory records for this entry. For a
    /// `Stored` entry (everything this build encodes) that is the stored payload's
    /// CRC; a copied foreign entry keeps its writer's value.
    pub crc: u32,
    /// An entry this build does not model and only copies through.
    pub foreign: bool,
}

/// The decoded `manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Manifest {
    pub format: FormatVersion,
    pub generator: String,
    pub document_uuid: String,
    pub open_revision: u64,
    pub saved_revision: u64,
    pub saved_at: Option<String>,
    /// Raw model id (decimal) → this model's shard entries, sorted.
    pub shard_map: BTreeMap<String, Vec<String>>,
    /// Every entry of the archive except `manifest.json`, sorted by path.
    pub entries: Vec<EntryRecord>,
}

impl Manifest {
    /// The record of one entry, if the manifest describes it.
    pub fn entry(&self, path: &str) -> Option<&EntryRecord> {
        self.entries.iter().find(|entry| entry.path == path)
    }

    /// The subset a caller outside this crate is allowed to see.
    pub fn summary(&self, foreign_entries: Vec<String>) -> ManifestSummary {
        ManifestSummary {
            format: self.format,
            document_uuid: self.document_uuid.clone(),
            open_revision: Revision(self.open_revision),
            saved_revision: Revision(self.saved_revision),
            foreign_entries,
        }
    }

    /// The `manifest.json` object, with `entries` sorted by path.
    pub fn to_json_value(&self) -> Value {
        let mut sorted: Vec<&EntryRecord> = self.entries.iter().collect();
        sorted.sort_by(|left, right| left.path.cmp(&right.path));
        let entries: Vec<Value> = sorted.iter().map(|entry| entry.to_json_value()).collect();

        let mut shard_map = Map::new();
        for (model, paths) in &self.shard_map {
            shard_map.insert(
                model.clone(),
                Value::Array(
                    paths
                        .iter()
                        .map(|path| Value::String(path.clone()))
                        .collect(),
                ),
            );
        }

        let mut format = Map::new();
        format.insert("read".to_string(), Value::from(self.format.read));
        format.insert("write".to_string(), Value::from(self.format.write));
        format.insert("minor".to_string(), Value::from(self.format.minor));

        let mut map = Map::new();
        map.insert("format".to_string(), Value::Object(format));
        map.insert(
            "generator".to_string(),
            Value::String(self.generator.clone()),
        );
        map.insert(
            "document_uuid".to_string(),
            Value::String(self.document_uuid.clone()),
        );
        map.insert("open_revision".to_string(), Value::from(self.open_revision));
        map.insert(
            "saved_revision".to_string(),
            Value::from(self.saved_revision),
        );
        map.insert(
            "saved_at".to_string(),
            match &self.saved_at {
                Some(text) => Value::String(text.clone()),
                None => Value::Null,
            },
        );
        map.insert("shard_map".to_string(), Value::Object(shard_map));
        map.insert("entries".to_string(), Value::Array(entries));
        Value::Object(map)
    }

    /// Read a `manifest.json` object.
    ///
    /// Every rule this module documents is checked here; nothing is repaired and
    /// no default is invented.
    pub fn from_json_value(value: &Value) -> Result<Manifest, StoreError> {
        let mut reader = Reader::new(value, CONTEXT)?;

        let mut format_reader = Reader::new(&reader.object("format")?, "manifest.format")?;
        let read = u32_field(&mut format_reader, "read")?;
        let write = u32_field(&mut format_reader, "write")?;
        let minor = u32_field(&mut format_reader, "minor")?;
        format_reader.finish()?;

        let generator = reader.string("generator")?;
        let document_uuid = reader.string("document_uuid")?;
        let open_revision = reader.u64("open_revision")?;
        let saved_revision = reader.u64("saved_revision")?;
        let saved_at = reader.opt_string("saved_at")?;

        let shard_map_value = reader.object("shard_map")?;
        let mut shard_map = BTreeMap::new();
        let shard_map_object = shard_map_value
            .as_object()
            .expect("Reader::object returns an object");
        for (model, paths) in shard_map_object {
            if model.parse::<i64>().is_err() {
                return Err(malformed(
                    CONTEXT,
                    format!("shard_map key {model:?} is not a decimal model id"),
                ));
            }
            let Value::Array(items) = paths else {
                return Err(malformed(
                    CONTEXT,
                    format!("shard_map[{model:?}] must be an array of entry paths"),
                ));
            };
            let mut list = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::String(path) => list.push(path.clone()),
                    _ => {
                        return Err(malformed(
                            CONTEXT,
                            format!("shard_map[{model:?}] must hold strings"),
                        ))
                    }
                }
            }
            list.sort();
            shard_map.insert(model.clone(), list);
        }

        let mut entries = Vec::new();
        for item in reader.array("entries")? {
            entries.push(EntryRecord::from_json_value(&item)?);
        }
        reader.finish()?;

        entries.sort_by(|left, right| left.path.cmp(&right.path));
        for pair in entries.windows(2) {
            if pair[0].path == pair[1].path {
                return Err(malformed(
                    CONTEXT,
                    format!("entry {:?} is listed twice", pair[0].path),
                ));
            }
        }
        if let Some(entry) = entries.iter().find(|entry| entry.path == MANIFEST_ENTRY) {
            return Err(malformed(
                CONTEXT,
                format!("{} must not list itself", entry.path),
            ));
        }
        if entries.iter().any(|entry| entry.path.is_empty()) {
            return Err(malformed(CONTEXT, "an entry path must not be empty"));
        }

        Ok(Manifest {
            format: FormatVersion { read, write, minor },
            generator,
            document_uuid,
            open_revision,
            saved_revision,
            saved_at,
            shard_map,
            entries,
        })
    }
}

/// Read only `format.{read,write,minor}` from a decoded `manifest.json`.
///
/// The read path runs the version gate **before** the full manifest is parsed:
/// an archive written by a build with a different entry codec must be refused
/// with `unsupported_version`/`unsupported_minor`, not with a confusing
/// `invalid_document` about a field it does not carry. `format` itself is strict
/// (unknown keys inside it are refused, like the full parse); every other
/// top-level field is ignored here and validated later by
/// [`Manifest::from_json_value`].
pub(crate) fn probe_format(value: &Value) -> Result<FormatVersion, StoreError> {
    let mut reader = Reader::new(value, CONTEXT)?;
    let mut format = Reader::new(&reader.object("format")?, "manifest.format")?;
    let read = u32_field(&mut format, "read")?;
    let write = u32_field(&mut format, "write")?;
    let minor = u32_field(&mut format, "minor")?;
    format.finish()?;
    Ok(FormatVersion { read, write, minor })
}

impl EntryRecord {
    fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        map.insert(
            "codec".to_string(),
            Value::String(self.codec.as_str().to_string()),
        );
        map.insert("path".to_string(), Value::String(self.path.clone()));
        map.insert("sha256".to_string(), Value::String(self.sha256.clone()));
        map.insert(
            "sha256_c".to_string(),
            match &self.sha256_c {
                Some(hash) => Value::String(hash.clone()),
                None => Value::Null,
            },
        );
        map.insert("len".to_string(), Value::from(self.len));
        map.insert("len_c".to_string(), Value::from(self.len_c));
        map.insert("crc".to_string(), Value::from(self.crc));
        map.insert("foreign".to_string(), Value::Bool(self.foreign));
        Value::Object(map)
    }

    fn from_json_value(value: &Value) -> Result<EntryRecord, StoreError> {
        let mut reader = Reader::new(value, CONTEXT)?;
        let token = reader.string("codec")?;
        let codec = Codec::parse(&token).ok_or_else(|| {
            malformed(
                CONTEXT,
                format!("codec: {token:?} is not zstd, raw or opaque"),
            )
        })?;
        let path = reader.string("path")?;
        let sha256 = reader.string("sha256")?;
        check_hash(&path, "sha256", &sha256)?;
        let sha256_c = match reader.opt_string("sha256_c")? {
            Some(hash) => {
                check_hash(&path, "sha256_c", &hash)?;
                Some(hash)
            }
            None => None,
        };
        let len = reader.u64("len")?;
        let len_c = reader.u64("len_c")?;
        let crc = reader.u64("crc")?;
        let crc = u32::try_from(crc)
            .map_err(|_| malformed(CONTEXT, format!("{path:?}: crc does not fit in 32 bits")))?;
        let foreign = reader.bool("foreign")?;
        reader.finish()?;

        // The codec decides which fields can be true. `opaque` is the one codec
        // this build does not hash, so its stored-payload digest must be absent;
        // every modelled entry records one. `raw` stores its logical bytes, so
        // the two digests and the two lengths must agree.
        match codec {
            Codec::Opaque => {
                if sha256_c.is_some() {
                    return Err(malformed(
                        CONTEXT,
                        format!("{path:?}: an opaque entry must not record sha256_c"),
                    ));
                }
            }
            Codec::Zstd | Codec::Raw => {
                if sha256_c.is_none() {
                    return Err(malformed(
                        CONTEXT,
                        format!("{path:?}: a {token} entry must record sha256_c"),
                    ));
                }
            }
        }
        if codec == Codec::Raw {
            if len != len_c {
                return Err(malformed(
                    CONTEXT,
                    format!("{path:?}: a raw entry must have len == len_c"),
                ));
            }
            if sha256_c.as_deref() != Some(sha256.as_str()) {
                return Err(malformed(
                    CONTEXT,
                    format!("{path:?}: a raw entry's sha256 and sha256_c must agree"),
                ));
            }
        }

        Ok(EntryRecord {
            path,
            codec,
            sha256,
            sha256_c,
            len,
            len_c,
            crc,
            foreign,
        })
    }
}

fn u32_field(reader: &mut Reader, field: &'static str) -> Result<u32, StoreError> {
    let raw = reader.u64(field)?;
    u32::try_from(raw).map_err(|_| {
        malformed(
            CONTEXT,
            format!("format.{field} {raw} does not fit in 32 bits"),
        )
    })
}

/// Every hash is exactly 64 lowercase hex digits.
fn check_hash(path: &str, field: &str, hash: &str) -> Result<(), StoreError> {
    let valid = hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if valid {
        Ok(())
    } else {
        Err(malformed(
            CONTEXT,
            format!("{path:?}: {field} is not 64 lowercase hex digits"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(path: &str) -> EntryRecord {
        EntryRecord {
            path: path.to_string(),
            codec: Codec::Zstd,
            sha256: "a".repeat(64),
            sha256_c: Some("b".repeat(64)),
            len: 12,
            len_c: 8,
            crc: 0xdead_beef,
            foreign: false,
        }
    }

    fn sample() -> Manifest {
        let mut shard_map = BTreeMap::new();
        shard_map.insert("10".to_string(), vec!["elements/10-0.json".to_string()]);
        Manifest {
            format: FormatVersion {
                read: 2,
                write: 2,
                minor: 2,
            },
            generator: "yacr/test".to_string(),
            document_uuid: "0f8fad5b-d9cb-469f-a165-70867728950e".to_string(),
            open_revision: 3,
            saved_revision: 9,
            saved_at: Some("2026-11-18T00:00:00Z".to_string()),
            shard_map,
            entries: vec![record("document.json")],
        }
    }

    fn round_trip(manifest: &Manifest) -> Manifest {
        let text = serde_json::to_string(&manifest.to_json_value()).expect("serialize");
        let parsed: Value = serde_json::from_str(&text).expect("parse");
        Manifest::from_json_value(&parsed).expect("decode")
    }

    #[test]
    fn manifest_round_trips_and_keeps_every_field() {
        let manifest = sample();
        assert_eq!(round_trip(&manifest), manifest);
    }

    #[test]
    fn entries_are_written_sorted_by_path() {
        let mut manifest = sample();
        manifest.entries = vec![record("tables/models.json"), record("document.json")];
        let value = manifest.to_json_value();
        let paths: Vec<&str> = value["entries"]
            .as_array()
            .expect("array")
            .iter()
            .map(|entry| entry["path"].as_str().expect("string"))
            .collect();
        assert_eq!(paths, ["document.json", "tables/models.json"]);
        // Reading sorts too, so the decoded order is the canonical one.
        let mut expected = manifest.entries.clone();
        expected.sort_by(|left, right| left.path.cmp(&right.path));
        assert_eq!(round_trip(&manifest).entries, expected);
    }

    #[test]
    fn a_manifest_listing_itself_is_refused() {
        let mut manifest = sample();
        manifest.entries = vec![record(MANIFEST_ENTRY)];
        let value = manifest.to_json_value();
        assert!(Manifest::from_json_value(&value).is_err());
    }

    #[test]
    fn duplicate_entries_and_bad_hashes_are_refused() {
        let mut manifest = sample();
        manifest.entries = vec![record("a.json"), record("a.json")];
        assert!(Manifest::from_json_value(&manifest.to_json_value()).is_err());

        let mut manifest = sample();
        manifest.entries[0].sha256 = "A".repeat(64);
        assert!(Manifest::from_json_value(&manifest.to_json_value()).is_err());

        let mut manifest = sample();
        manifest.entries[0].sha256 = "a".repeat(63);
        assert!(Manifest::from_json_value(&manifest.to_json_value()).is_err());

        let mut manifest = sample();
        manifest.entries[0].sha256_c = Some("z".repeat(64));
        assert!(Manifest::from_json_value(&manifest.to_json_value()).is_err());
    }

    #[test]
    fn the_codec_decides_which_hash_fields_may_be_present() {
        // An opaque entry is not hashed at the payload level; a modelled one is.
        let mut manifest = sample();
        manifest.entries[0].codec = Codec::Opaque;
        manifest.entries[0].sha256_c = None;
        assert!(Manifest::from_json_value(&manifest.to_json_value()).is_ok());

        let mut manifest = sample();
        manifest.entries[0].codec = Codec::Opaque;
        assert!(Manifest::from_json_value(&manifest.to_json_value()).is_err());

        let mut manifest = sample();
        manifest.entries[0].sha256_c = None;
        assert!(Manifest::from_json_value(&manifest.to_json_value()).is_err());

        // A raw entry stores its logical bytes, so both pairs must agree.
        let mut manifest = sample();
        manifest.entries[0].codec = Codec::Raw;
        manifest.entries[0].sha256_c = Some(manifest.entries[0].sha256.clone());
        manifest.entries[0].len_c = manifest.entries[0].len;
        assert!(Manifest::from_json_value(&manifest.to_json_value()).is_ok());

        let mut manifest = sample();
        manifest.entries[0].codec = Codec::Raw;
        manifest.entries[0].sha256_c = Some(manifest.entries[0].sha256.clone());
        assert!(
            Manifest::from_json_value(&manifest.to_json_value()).is_err(),
            "raw: len != len_c"
        );

        let mut manifest = sample();
        manifest.entries[0].codec = Codec::Raw;
        manifest.entries[0].len_c = manifest.entries[0].len;
        assert!(
            Manifest::from_json_value(&manifest.to_json_value()).is_err(),
            "raw: sha256 != sha256_c"
        );

        // An unknown codec token is not silently treated as opaque.
        let mut value = sample().to_json_value();
        value["entries"][0]["codec"] = Value::String("gzip".to_string());
        assert!(Manifest::from_json_value(&value).is_err());
    }

    #[test]
    fn probe_format_reads_only_the_version() {
        let value = sample().to_json_value();
        assert_eq!(
            probe_format(&value).expect("probe"),
            FormatVersion {
                read: 2,
                write: 2,
                minor: 2,
            }
        );

        // The gate runs before the rest is parsed: other fields may be damaged
        // or absent, and are not this function's business.
        let value = serde_json::json!({
            "format": {"read": 2, "write": 2, "minor": 3},
            "entries": "not an array",
        });
        assert_eq!(probe_format(&value).expect("probe").minor, 3);

        // Inside `format` it is still strict, and a missing `format` is refused.
        let value = serde_json::json!({
            "format": {"read": 2, "write": 2, "minor": 3, "future": true},
        });
        assert!(probe_format(&value).is_err());
        let value = serde_json::json!({"entries": []});
        assert!(probe_format(&value).is_err());
    }

    #[test]
    fn unknown_fields_and_bad_types_are_refused() {
        let mut value = sample().to_json_value();
        value["future"] = Value::Bool(true);
        assert!(Manifest::from_json_value(&value).is_err());

        let mut value = sample().to_json_value();
        value["format"]["future"] = Value::Bool(true);
        assert!(Manifest::from_json_value(&value).is_err());

        let mut value = sample().to_json_value();
        value["shard_map"] = serde_json::json!({"model": []});
        assert!(Manifest::from_json_value(&value).is_err());

        let mut value = sample().to_json_value();
        value["format"]["read"] = Value::from(u64::from(u32::MAX) + 1);
        assert!(Manifest::from_json_value(&value).is_err());

        let mut value = sample().to_json_value();
        value["format"]["minor"] = Value::from(-1);
        assert!(Manifest::from_json_value(&value).is_err());
    }

    #[test]
    fn the_summary_hides_the_hash_table() {
        let manifest = sample();
        let summary = manifest.summary(vec!["raw/x.bin".to_string()]);
        assert_eq!(summary.format, manifest.format);
        assert_eq!(summary.open_revision, Revision(3));
        assert_eq!(summary.saved_revision, Revision(9));
        assert_eq!(summary.foreign_entries, ["raw/x.bin"]);
    }
}
