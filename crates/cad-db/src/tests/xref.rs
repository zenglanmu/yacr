//! External-reference metadata and provenance.

use super::*;

fn reference(name: &str, block: Option<BlockId>) -> ExternalReference {
    ExternalReference {
        name: name.to_string(),
        raw_path: r".\xref\base.dwg".to_string(),
        resolved_path: Some("/work/drawings/xref/base.dwg".to_string()),
        block,
        overlay: false,
        depth: 0,
        status: ExternalReferenceStatus::Loaded,
        reason: None,
    }
}

fn builder_with_block() -> DrawingDatabaseBuilder {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(7));
    builder
        .insert_block(BlockDefinition {
            id: BlockId(3),
            entities: Vec::new(),
            dynamic_visibility: None,
        })
        .unwrap();
    builder
}

#[test]
fn external_reference_round_trips_through_the_builder() {
    let mut builder = builder_with_block();
    let inserted = reference("XREF-A", Some(BlockId(3)));
    builder.insert_external_reference(inserted.clone()).unwrap();
    let database = builder.finish().unwrap();

    assert_eq!(database.external_references().count(), 1);
    let stored = database
        .external_reference("XREF-A")
        .expect("reference is stored");
    assert_eq!(stored, &inserted);
    assert_eq!(stored.raw_path, r".\xref\base.dwg");
    assert_eq!(
        stored.resolved_path.as_deref(),
        Some("/work/drawings/xref/base.dwg")
    );
    assert_eq!(
        database.external_reference_for_block(BlockId(3)),
        Some(&inserted)
    );
    assert!(database.is_xref_block(BlockId(3)));
    assert!(!database.is_xref_block(BlockId(4)));
    assert!(!database.is_xref_block(BlockId(0)));
    assert!(database.external_reference("missing").is_none());
    // Import metadata does not invent a revision or a transaction.
    assert_eq!(database.revision(), Revision(0));
}

#[test]
fn duplicate_external_reference_names_are_refused() {
    let mut builder = builder_with_block();
    builder
        .insert_external_reference(reference("XREF-A", Some(BlockId(3))))
        .unwrap();
    let duplicate = builder.insert_external_reference(reference("XREF-A", None));
    assert!(matches!(duplicate, Err(CadError::InvalidInput(_))));
    // The refused insert left the original entry intact.
    assert!(builder
        .insert_external_reference(reference("XREF-B", None))
        .is_ok());
    let database = builder.finish().unwrap();
    assert_eq!(database.external_references().count(), 2);
    assert_eq!(
        database.external_reference("XREF-A").unwrap().block,
        Some(BlockId(3))
    );
}

#[test]
fn dangling_provenance_block_is_refused_at_finish() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(8));
    builder
        .insert_external_reference(reference("XREF-MISSING", Some(BlockId(99))))
        .unwrap();
    let error = builder.finish().unwrap_err();
    match error {
        CadError::Invariant(message) => {
            assert!(
                message.contains("external reference 'XREF-MISSING'"),
                "unexpected message: {message}"
            );
            assert!(
                message.contains("missing block"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected an invariant failure, got {other:?}"),
    }
}

#[test]
fn databases_without_xrefs_have_an_empty_reference_map() {
    let database = DrawingDatabaseBuilder::new(DatabaseId(9)).finish().unwrap();
    assert_eq!(database.external_references().count(), 0);
    assert!(database.external_reference("anything").is_none());
    assert!(database.external_reference_for_block(BlockId(0)).is_none());
    assert!(!database.is_xref_block(BlockId(0)));
}

#[test]
fn external_reference_status_strings_are_stable() {
    let cases = [
        (ExternalReferenceStatus::Loaded, "loaded", true),
        (ExternalReferenceStatus::Missing, "missing", false),
        (
            ExternalReferenceStatus::BrowserUnavailable,
            "browser-unavailable",
            false,
        ),
        (ExternalReferenceStatus::RefusedPath, "refused-path", false),
        (ExternalReferenceStatus::Unloaded, "unloaded", false),
        (ExternalReferenceStatus::Cycle, "cycle", false),
        (
            ExternalReferenceStatus::DepthExceeded,
            "depth-exceeded",
            false,
        ),
        (
            ExternalReferenceStatus::SkippedOverlay,
            "skipped-overlay",
            false,
        ),
        (ExternalReferenceStatus::Unsupported, "unsupported", false),
    ];
    for (status, spelling, loaded) in cases {
        assert_eq!(status.as_str(), spelling);
        assert_eq!(status.is_loaded(), loaded);
    }
}
