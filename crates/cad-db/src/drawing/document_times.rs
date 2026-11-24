//! Persisted document create/update timestamps (`document.json` → header).
//!
//! These are the two clock facts `TIME` and `EDITTIME` report: when the document
//! was first written to its own archive and when it was last written. They are
//! **persistence metadata**, not drawing content: recording a save does not
//! raise a revision, invalidate bounds or produce a `ChangeSet`. The save path in
//! `cad-host` stamps them once per durable write (the single write-path clock),
//! and `DrawingDatabase::from_snapshot` reads them back on open.
//!
//! A document with no timestamps writes **no** `document_times` key at all, so a
//! pre-timestamp archive stays byte-identical; an absent key decodes to
//! [`DocumentTimes::default`] (`None`/`None`, "not stored"), never to a fabricated
//! epoch. The first save of such a document stamps both fields.
//!
//! Times are Unix seconds. A clock before the epoch is not representable; the
//! host passes `None` and nothing is stamped rather than writing a negative or
//! invented value.

use serde_json::{Map, Value};

use super::DrawingDatabase;
use crate::snapshot::SnapshotError;

/// The document's persisted create/update instants, in Unix seconds.
///
/// `None` is a real answer ("this document stores no such timestamp"), not a
/// zero. The save path fills both fields on the first save and only
/// [`Self::updated_at`] afterwards; `created_at` is never rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DocumentTimes {
    /// The first durable save of this document, or `None` for a document that
    /// has never been saved and for a pre-timestamp archive.
    pub created_at: Option<u64>,
    /// The most recent durable save, or `None` under the same rules as
    /// [`Self::created_at`].
    pub updated_at: Option<u64>,
}

impl DocumentTimes {
    /// This record with one save stamped at `saved_at`.
    ///
    /// `None` (a clock before the Unix epoch) is an authoritative no-change:
    /// the stored values stay exactly what they were, because there is no real
    /// instant to record. A first save stamps `created_at` and `updated_at` with
    /// the same instant; later saves only move `updated_at`.
    pub fn with_saved_at(self, saved_at: Option<u64>) -> DocumentTimes {
        match saved_at {
            None => self,
            Some(seconds) => DocumentTimes {
                created_at: self.created_at.or(Some(seconds)),
                updated_at: Some(seconds),
            },
        }
    }

    /// Whether neither instant is stored.
    pub fn is_empty(self) -> bool {
        self.created_at.is_none() && self.updated_at.is_none()
    }

    /// The persisted object, or `None` when there is nothing to write.
    pub fn to_json_value(self) -> Option<Value> {
        if self.is_empty() {
            return None;
        }
        let mut map = Map::new();
        map.insert(
            "created_at".to_string(),
            self.created_at.map_or(Value::Null, Value::from),
        );
        map.insert(
            "updated_at".to_string(),
            self.updated_at.map_or(Value::Null, Value::from),
        );
        Some(Value::Object(map))
    }

    /// Read the persisted object. Missing, `null` and an empty object are all
    /// "not stored"; a present field must be a non-negative integer or `null`.
    ///
    /// Unknown keys are refused, like every other strict header variable. An
    /// `updated_at` earlier than `created_at` is kept verbatim rather than
    /// rejected: a system-clock adjustment is a real event, not corrupt data.
    pub fn from_json_value(value: &Value) -> Result<DocumentTimes, SnapshotError> {
        match value {
            Value::Null => Ok(DocumentTimes::default()),
            Value::Object(map) => {
                let mut times = DocumentTimes::default();
                for (key, value) in map {
                    let slot = match key.as_str() {
                        "created_at" => &mut times.created_at,
                        "updated_at" => &mut times.updated_at,
                        other => {
                            return Err(SnapshotError::invalid(format!(
                                "document.json variables.document_times: unknown field {other:?}"
                            )))
                        }
                    };
                    *slot = read_seconds(key, value)?;
                }
                Ok(times)
            }
            _ => Err(SnapshotError::invalid(
                "document.json variables.document_times: expected an object or null",
            )),
        }
    }
}

fn read_seconds(field: &str, value: &Value) -> Result<Option<u64>, SnapshotError> {
    match value {
        Value::Null => Ok(None),
        Value::Number(number) => number.as_u64().map(Some).ok_or_else(|| {
            SnapshotError::invalid(format!(
                "document.json variables.document_times.{field}: expected a non-negative \
                 integer or null"
            ))
        }),
        _ => Err(SnapshotError::invalid(format!(
            "document.json variables.document_times.{field}: expected a non-negative integer \
             or null"
        ))),
    }
}

impl DrawingDatabase {
    /// The document's persisted create/update instants.
    pub fn document_times(&self) -> DocumentTimes {
        self.document_times
    }

    /// Record one durable save atomically, without raising a revision.
    ///
    /// This is the in-memory half of the save stamp: the bytes just written
    /// carry the same instants (the snapshot was encoded with them), and the
    /// live database now reports them to `TIME`/`EDITTIME`. Nothing in the
    /// drawing changed, so there is no `ChangeSet`, no undo step and no dirty
    /// scope.
    pub fn record_document_saved_at(&mut self, saved_at: u64) {
        self.document_times = self.document_times.with_saved_at(Some(saved_at));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_key_is_not_stored_and_writes_nothing() {
        let times = DocumentTimes::default();
        assert!(times.is_empty());
        assert_eq!(times.to_json_value(), None);
        assert_eq!(
            DocumentTimes::from_json_value(&Value::Null).unwrap(),
            DocumentTimes::default()
        );
    }

    #[test]
    fn the_first_save_stamps_creation_and_later_saves_only_update() {
        let first = DocumentTimes::default().with_saved_at(Some(100));
        assert_eq!(first.created_at, Some(100));
        assert_eq!(first.updated_at, Some(100));
        let later = first.with_saved_at(Some(250));
        assert_eq!(later.created_at, Some(100), "creation is never rewritten");
        assert_eq!(later.updated_at, Some(250));
        assert_eq!(
            first.with_saved_at(None),
            first,
            "an unavailable clock changes nothing"
        );
    }

    #[test]
    fn the_persisted_object_round_trips_and_refuses_unknown_fields() {
        let times = DocumentTimes {
            created_at: Some(10),
            updated_at: Some(20),
        };
        let value = times.to_json_value().expect("stored");
        assert_eq!(DocumentTimes::from_json_value(&value).unwrap(), times);
        let mut map = value.as_object().unwrap().clone();
        map.insert("saved_at".into(), Value::from(30));
        let error = DocumentTimes::from_json_value(&Value::Object(map)).unwrap_err();
        assert_eq!(error.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
        assert!(error.detail.contains("unknown field"), "{}", error.detail);
    }

    #[test]
    fn a_null_field_reads_as_absent() {
        let value = serde_json::json!({"created_at": null, "updated_at": 42});
        let times = DocumentTimes::from_json_value(&value).expect("valid");
        assert_eq!(times.created_at, None);
        assert_eq!(times.updated_at, Some(42));
    }

    #[test]
    fn a_wrong_type_or_negative_value_is_refused() {
        for value in [
            serde_json::json!({"created_at": -1}),
            serde_json::json!({"created_at": 1.5}),
            serde_json::json!({"created_at": "1"}),
            serde_json::json!([1, 2]),
        ] {
            assert!(
                DocumentTimes::from_json_value(&value).is_err(),
                "{value} must be refused"
            );
        }
    }
}
