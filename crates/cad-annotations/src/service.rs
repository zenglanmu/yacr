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
    /// The format is versioned JSON. This build reads schema versions from
    /// [`MIN_SCHEMA_VERSION`] through [`SCHEMA_VERSION`] (currently 1). A file
    /// that claims a *newer* version is refused with [`CadError::Unsupported`]
    /// rather than silently downgraded. A file that claims an *older* version is
    /// migrated through [`AnnotationService::migrate_file`]; because no earlier
    /// version has ever shipped, there is no deterministic migration yet and
    /// such a file is reported as [`CadError::CorruptData`] instead of being
    /// guessed at. Unknown top-level fields are preserved verbatim so a newer
    /// writer's data survives an older reader; unknown fields inside annotation,
    /// geometry, style and precision objects are preserved in the
    /// `extensions_json` side-band. Malformed values are reported as
    /// [`CadError::CorruptData`] and never silently coerced (audit B11).
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
        if schema_version < MIN_SCHEMA_VERSION {
            // Deterministic migration only; never guess at an unknown past
            // format. No version below 1 has ever shipped, so this is a hard
            // `CorruptData` until a real migration exists.
            return Err(CadError::CorruptData(format!(
                "annotation schema version {schema_version} predates the oldest migratable version ({MIN_SCHEMA_VERSION}); no deterministic migration exists"
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
        let mut captured: Vec<(AnnotationId, AnnotationExtensions)> =
            Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            // A malformed annotation is reported, not silently dropped
            // (audit B11): a caller must never believe a lossy import is OK.
            let (annotation, extensions) = decode_annotation(item)
                .map_err(|e| corrupt(format!("annotation at index {index}: {}", e)))?;
            if !extensions.is_empty() {
                captured.push((annotation.id, extensions));
            }
            annotations.push(annotation);
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

        // Preserve unknown top-level fields verbatim. The private nested
        // side-band is decoded separately: an older reader still preserves it as
        // an ordinary unknown field, and a current reader expands it.
        let mut extensions_json = BTreeMap::new();
        for (k, v) in object {
            if !KNOWN_TOP_LEVEL.contains(&k.as_str()) {
                extensions_json.insert(k.clone(), v.to_string());
            }
        }
        let mut nested_extensions = NestedExtensions::default();
        for (id, extensions) in captured {
            nested_extensions.annotations.insert(id, extensions);
        }
        if let Some(raw) = object.get(NESTED_EXTENSIONS_KEY) {
            let payload = parse_nested_extensions(&raw.to_string())?;
            merge_nested_sideband(&mut nested_extensions, &payload, &annotations)?;
        }
        // The side-band is rebuilt from `nested_extensions` on encode, so it
        // stays normalized rather than carrying a stale reader copy verbatim.
        extensions_json.remove(NESTED_EXTENSIONS_KEY);

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

        // Timestamps are validated only for annotations that will actually be
        // kept. A file that claims a newer schema is refused above, so this
        // build must not choke on a future timestamp encoding it does not know.
        for annotation in &annotations {
            if annotation.modified_unix_ms < annotation.created_unix_ms {
                return Err(corrupt(format!(
                    "annotation {:?} modified time precedes created time",
                    annotation.id
                )));
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
            nested_extensions,
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
    /// a newer version is refused rather than silently downgraded. A file
    /// claiming an older version is migrated through
    /// [`AnnotationService::migrate_file`] first, or refused as
    /// [`CadError::CorruptData`] when no deterministic migration exists.
    /// Extension fields that collide with a known schema field are refused
    /// rather than silently dropped, so an export can never report success while
    /// losing data (audit B11).
    pub fn encode(&self, file: &AnnotationFile) -> CadResult<Vec<u8>> {
        if file.schema_version > SCHEMA_VERSION {
            return Err(CadError::Unsupported(format!(
                "cannot encode annotation schema version {} (this build writes {SCHEMA_VERSION})",
                file.schema_version
            )));
        }
        if file.schema_version < MIN_SCHEMA_VERSION {
            return Err(CadError::CorruptData(format!(
                "annotation schema version {} predates the oldest migratable version ({MIN_SCHEMA_VERSION}); no deterministic migration exists",
                file.schema_version
            )));
        }
        let mut seen_ids = std::collections::BTreeSet::new();
        for annotation in &file.annotations {
            if !seen_ids.insert(annotation.id) {
                return Err(CadError::InvalidInput(format!(
                    "annotation id {:?} appears more than once",
                    annotation.id
                )));
            }
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
        let annotations: Vec<Value> = file
            .annotations
            .iter()
            .map(|annotation| {
                let ext = file
                    .nested_extensions
                    .annotations
                    .get(&annotation.id)
                    .cloned()
                    .unwrap_or_default();
                encode_annotation(annotation, &ext)
            })
            .collect::<CadResult<Vec<_>>>()?;
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
            if k == NESTED_EXTENSIONS_KEY {
                return Err(CadError::InvalidInput(format!(
                    "extension field '{k}' is reserved; use nested_extensions instead"
                )));
            }
            try_insert_extension(&mut root, k, v)?;
        }
        // Rebuild the nested side-band from `nested_extensions`. Anything named
        // by the map that is not in the file is a caller bug: refuse instead of
        // writing a side-band no reader can attach.
        if let Some(payload) = build_nested_sideband(&file.nested_extensions, &file.annotations)? {
            root.insert(NESTED_EXTENSIONS_KEY.into(), payload);
        }
        serde_json::to_vec_pretty(&Value::Object(root))
            .map_err(|e| CadError::Invariant(format!("annotation encode failed: {e}")))
    }

    /// Migrate a decoded file up to [`SCHEMA_VERSION`].
    ///
    /// Only deterministic, explicitly-implemented migrations are allowed: a file
    /// already at [`SCHEMA_VERSION`] passes through unchanged; a newer file is
    /// [`CadError::Unsupported`]; any version below [`MIN_SCHEMA_VERSION`] is
    /// [`CadError::CorruptData`] because no historical schema has ever shipped
    /// and inventing a migration for it would be fabrication. Because
    /// [`MIN_SCHEMA_VERSION`] currently equals [`SCHEMA_VERSION`], the older
    /// branch is unreachable by construction and exists to pin the policy.
    pub fn migrate_file(&self, file: AnnotationFile) -> CadResult<AnnotationFile> {
        if file.schema_version > SCHEMA_VERSION {
            return Err(CadError::Unsupported(format!(
                "cannot migrate annotation schema version {} (this build writes {SCHEMA_VERSION})",
                file.schema_version
            )));
        }
        if file.schema_version == SCHEMA_VERSION {
            return Ok(file);
        }
        // A version below the oldest migratable one has no defined migration.
        // This build has never shipped any schema below v1, so there is no
        // legitimate migration to perform here; refuse rather than guess.
        Err(CadError::CorruptData(format!(
            "annotation schema version {} has no deterministic migration to {SCHEMA_VERSION}",
            file.schema_version
        )))
    }

    /// Atomically export `file` for the current database revision.
    ///
    /// The bytes are produced first; only if encoding fully succeeds is the
    /// captured revision marked saved. The file must contain exactly the current
    /// database annotations (in any order), not a stale or partial snapshot.
    /// A failed encode or snapshot mismatch therefore returns an
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
        if file.annotations.len() != database.len()
            || file
                .annotations
                .iter()
                .any(|annotation| database.get(annotation.id) != Some(annotation))
        {
            return Err(CadError::InvalidInput(
                "annotation export does not match the current database snapshot".into(),
            ));
        }
        database.mark_exported(revision)?;
        Ok(bytes)
    }
}

/// Insert a preserved top-level extension value, refusing invalid JSON.
fn try_insert_extension(root: &mut Map<String, Value>, key: &str, raw: &str) -> CadResult<()> {
    let parsed: Value = serde_json::from_str(raw)
        .map_err(|_| CadError::CorruptData(format!("extension field '{key}' is not valid JSON")))?;
    root.insert(key.to_string(), parsed);
    Ok(())
}

/// Build the [`NESTED_EXTENSIONS_KEY`] side-band for the annotations that carry
/// unknown nested fields. Returns `None` when there is nothing to preserve.
///
/// An entry naming an annotation that is not in `annotations` is a caller bug
/// (a stale map that would produce a side-band no reader can attach), so it is
/// refused rather than silently written.
fn build_nested_sideband(
    extensions: &NestedExtensions,
    annotations: &[Annotation],
) -> CadResult<Option<Value>> {
    if extensions.is_empty() {
        return Ok(None);
    }
    let known: std::collections::BTreeSet<AnnotationId> =
        annotations.iter().map(|a| a.id).collect();
    let mut entries = Vec::with_capacity(extensions.annotations.len());
    for (id, ext) in &extensions.annotations {
        if !known.contains(id) {
            return Err(CadError::InvalidInput(format!(
                "nested extensions name annotation {id:?} that is not in the file"
            )));
        }
        if ext.is_empty() {
            continue;
        }
        entries.push(nested_sideband_entry(*id, ext));
    }
    if entries.is_empty() {
        return Ok(None);
    }
    Ok(Some(json!({
        "version": NESTED_EXTENSIONS_VERSION,
        "annotations": entries,
    })))
}

fn nested_sideband_entry(id: AnnotationId, ext: &AnnotationExtensions) -> Value {
    let mut extensions = Map::new();
    let groups = [
        ("annotation", &ext.annotation),
        ("geometry", &ext.geometry),
        ("style", &ext.style),
        ("precision", &ext.precision),
        ("measurement_precision", &ext.measurement_precision),
    ];
    for (key, map) in groups {
        if map.is_empty() {
            continue;
        }
        let mut object = Map::new();
        for (k, v) in map {
            object.insert(k.clone(), Value::String(v.clone()));
        }
        extensions.insert(key.into(), Value::Object(object));
    }
    json!({
        "id": uuid_string(id.0),
        "extensions": Value::Object(extensions),
    })
}
