//! The annotation service: transactional mutation, sidecar decode/encode.

use super::*;

static TX_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_transaction() -> TransactionId {
    TransactionId(TX_COUNTER.fetch_add(1, Ordering::Relaxed) as u128)
}

impl AnnotationService {
    /// Apply a command as a single transaction and return its change set.
    pub fn apply(
        &self,
        database: &mut AnnotationDatabase,
        command: AnnotationCommand,
    ) -> CadResult<ChangeSet> {
        let reason = match &command {
            AnnotationCommand::Create(_) => "create annotation",
            AnnotationCommand::Update(_) => "update annotation",
            AnnotationCommand::Delete(_) => "delete annotation",
        };
        let tx = database.begin(reason, next_transaction())?;
        let mut tx = tx;
        match command {
            AnnotationCommand::Create(a) => {
                tx.insert_annotation(a)?;
            }
            AnnotationCommand::Update(a) => {
                tx.update_annotation(a)?;
            }
            AnnotationCommand::Delete(id) => {
                tx.delete_annotation(id)?;
            }
        }
        tx.commit()
    }

    /// Decode and migrate an annotation file.
    ///
    /// Thin wrapper over [`AnnotationService::decode_with_report`] that discards
    /// the fingerprint report. Use the report form when the caller must explain
    /// (or log) how a mismatch was reconciled.
    pub fn decode(
        &self,
        bytes: &[u8],
        identity: &DocumentIdentity,
        policy: FingerprintPolicy,
    ) -> CadResult<AnnotationFile> {
        Ok(self.decode_with_report(bytes, identity, policy)?.file)
    }

    /// Decode an annotation file, returning the fingerprint reconciliation
    /// report alongside it (audit B10).
    ///
    /// The format is versioned JSON. This build reads schema version
    /// [`SCHEMA_VERSION`] (currently 1); newer versions are refused with
    /// [`CadError::Unsupported`]. Unknown top-level fields are preserved verbatim
    /// so a newer writer's data survives an older reader. Malformed values are
    /// reported as [`CadError::CorruptData`] and never silently coerced
    /// (audit B11).
    pub fn decode_with_report(
        &self,
        bytes: &[u8],
        identity: &DocumentIdentity,
        policy: FingerprintPolicy,
    ) -> CadResult<DecodeOutcome> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|e| CadError::CorruptData(format!("annotation JSON is malformed: {e}")))?;
        let object = value
            .as_object()
            .ok_or_else(|| CadError::CorruptData("annotation file is not a JSON object".into()))?;

        let schema_version = match object.get("schema_version") {
            Some(v) => {
                let raw = v.as_u64().ok_or_else(|| {
                    CadError::CorruptData("schema_version must be an unsigned integer".into())
                })?;
                u32::try_from(raw).map_err(|_| {
                    CadError::CorruptData("schema_version does not fit in 32 bits".into())
                })?
            }
            None => {
                return Err(CadError::CorruptData(
                    "annotation file has no schema_version".into(),
                ))
            }
        };
        if schema_version == 0 {
            return Err(CadError::CorruptData(
                "annotation schema_version 0 is not valid".into(),
            ));
        }
        if schema_version > SCHEMA_VERSION {
            return Err(CadError::Unsupported(format!(
                "annotation schema version {schema_version} is newer than this build supports ({SCHEMA_VERSION})"
            )));
        }

        // A missing fingerprint is corruption, not an implicit match with
        // `Temporary(0)` (audit B11).
        let file_fingerprint = decode_identity(object.get("document_fingerprint"))?;
        let matches = file_fingerprint == *identity;
        if !matches {
            match policy {
                FingerprintPolicy::RejectMismatch => {
                    return Err(CadError::InvalidInput(
                        "annotation file does not match the open drawing; supply an explicit mapping policy".into(),
                    ));
                }
                FingerprintPolicy::ImportUnanchored
                | FingerprintPolicy::ExplicitCoordinateMapping(_) => {}
            }
        }

        let annotations_value = object.get("annotations").ok_or_else(|| {
            CadError::CorruptData("annotation file has no 'annotations' array".into())
        })?;
        let items = annotations_value
            .as_array()
            .ok_or_else(|| CadError::CorruptData("'annotations' must be an array".into()))?;
        let mut annotations = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            // A malformed annotation is reported, not silently dropped
            // (audit B11): a caller must never believe a lossy import is OK.
            let a = decode_annotation(item)
                .map_err(|e| corrupt(format!("annotation at index {index}: {}", e)))?;
            annotations.push(a);
        }
        // Duplicate ids would silently collapse on import; refuse them instead.
        let mut seen_ids = std::collections::BTreeSet::new();
        for annotation in &annotations {
            if !seen_ids.insert(annotation.id) {
                return Err(corrupt(format!(
                    "annotation id {:?} appears more than once",
                    annotation.id
                )));
            }
        }

        // Preserve unknown top-level fields verbatim.
        let mut extensions_json = BTreeMap::new();
        for (k, v) in object {
            if !KNOWN_TOP_LEVEL.contains(&k.as_str()) {
                extensions_json.insert(k.clone(), v.to_string());
            }
        }

        // Decode bookmarks before the fingerprint policy so an explicit mapping
        // can move their camera transforms too (audit B10).
        let mut view_bookmarks = match object.get("view_bookmarks") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    decode_bookmark(b)
                        .map_err(|e| corrupt(format!("view bookmark at index {i}: {}", e)))
                })
                .collect::<CadResult<Vec<_>>>()?,
            Some(_) => {
                return Err(CadError::CorruptData(
                    "'view_bookmarks' must be an array".into(),
                ))
            }
        };

        // Apply the fingerprint policy to the decoded content (audit B10). A
        // mismatch must not leave anchors pointing into a document they were not
        // bound to, and an explicit mapping must actually move the geometry.
        let mut anchors_detached = 0usize;
        let mut geometry_transformed = false;
        let mut bookmarks_transformed = 0usize;
        if !matches {
            match policy {
                FingerprintPolicy::RejectMismatch => unreachable!("rejected above"),
                FingerprintPolicy::ImportUnanchored => {
                    for annotation in &mut annotations {
                        if annotation.anchor.take().is_some() {
                            anchors_detached += 1;
                        }
                    }
                }
                FingerprintPolicy::ExplicitCoordinateMapping(transform) => {
                    let mapping = CoordinateMapping::validate(&transform)?;
                    for annotation in &mut annotations {
                        if annotation.anchor.is_some() {
                            anchors_detached += 1;
                        }
                        mapping.apply_annotation(annotation)?;
                    }
                    geometry_transformed = !annotations.is_empty();
                    for bookmark in &mut view_bookmarks {
                        bookmark.camera_transform =
                            mapping.map_transform(&bookmark.camera_transform)?;
                        bookmarks_transformed += 1;
                    }
                }
            }
        }

        let outcome_file = AnnotationFile {
            schema_version: schema_version.max(1),
            application_version: match object.get("application_version") {
                None | Some(Value::Null) => String::new(),
                Some(v) => v
                    .as_str()
                    .ok_or_else(|| corrupt("'application_version' must be a string"))?
                    .to_string(),
            },
            document_fingerprint: if matches {
                file_fingerprint.clone()
            } else {
                // The mismatch was accepted by policy, so the imported content
                // now belongs to the open drawing; rebind the identity instead
                // of re-emitting a fingerprint that can never match (audit B10).
                identity.clone()
            },
            document_name_hint: match object.get("document_name_hint") {
                None | Some(Value::Null) => String::new(),
                Some(v) => v
                    .as_str()
                    .ok_or_else(|| corrupt("'document_name_hint' must be a string"))?
                    .to_string(),
            },
            unit_context: decode_units(object.get("unit_context"))?,
            annotations,
            view_bookmarks: std::mem::take(&mut view_bookmarks),
            extensions_json,
        };

        Ok(DecodeOutcome {
            file: outcome_file,
            fingerprint: FingerprintReport {
                file_fingerprint,
                open_fingerprint: identity.clone(),
                matched: matches,
                rebound: !matches,
                policy,
                anchors_detached,
                geometry_transformed,
                bookmarks_transformed,
            },
        })
    }

    /// Encode an annotation file to JSON.
    ///
    /// The output always carries the current [`SCHEMA_VERSION`]; a file claiming
    /// a newer version is refused rather than silently downgraded. Extension
    /// fields that collide with a known schema field are refused rather than
    /// silently dropped, so an export can never report success while losing data
    /// (audit B11).
    pub fn encode(&self, file: &AnnotationFile) -> CadResult<Vec<u8>> {
        if file.schema_version > SCHEMA_VERSION {
            return Err(CadError::Unsupported(format!(
                "cannot encode annotation schema version {} (this build writes {SCHEMA_VERSION})",
                file.schema_version
            )));
        }
        let mut root = Map::new();
        root.insert("schema_version".into(), json!(SCHEMA_VERSION));
        root.insert(
            "application_version".into(),
            json!(file.application_version),
        );
        root.insert(
            "document_fingerprint".into(),
            encode_identity(&file.document_fingerprint),
        );
        root.insert("document_name_hint".into(), json!(file.document_name_hint));
        root.insert("unit_context".into(), encode_units(&file.unit_context));
        let annotations: Vec<Value> = file.annotations.iter().map(encode_annotation).collect();
        root.insert("annotations".into(), Value::Array(annotations));
        let bookmarks: Vec<Value> = file.view_bookmarks.iter().map(encode_bookmark).collect();
        root.insert("view_bookmarks".into(), Value::Array(bookmarks));
        for (k, v) in &file.extensions_json {
            // Never let a preserved/unknown field override a schema field
            // (audit B11): refuse rather than drop, so the caller learns the
            // export would be lossy instead of silently succeeding.
            if KNOWN_TOP_LEVEL.contains(&k.as_str()) {
                return Err(CadError::InvalidInput(format!(
                    "extension field '{k}' collides with a schema field"
                )));
            }
            if let Ok(parsed) = serde_json::from_str::<Value>(v) {
                root.insert(k.clone(), parsed);
            } else {
                return Err(CadError::CorruptData(format!(
                    "extension field '{k}' is not valid JSON"
                )));
            }
        }
        serde_json::to_vec_pretty(&Value::Object(root))
            .map_err(|e| CadError::Invariant(format!("annotation encode failed: {e}")))
    }

    /// Atomically export `file` for the current database revision.
    ///
    /// The bytes are produced first; only if encoding fully succeeds is the
    /// captured revision marked saved. A failed encode therefore returns an
    /// error, leaves the database dirty and does not advance its revision
    /// (audit B07/F09). Hosts that must wait for a durable write should instead
    /// call [`AnnotationService::encode`] and confirm with
    /// [`AnnotationDatabase::mark_exported`] once the write succeeds.
    pub fn export_sidecar(
        &self,
        database: &mut AnnotationDatabase,
        file: &AnnotationFile,
    ) -> CadResult<Vec<u8>> {
        let revision = database.revision();
        let bytes = self.encode(file)?;
        database.mark_exported(revision)?;
        Ok(bytes)
    }
}
