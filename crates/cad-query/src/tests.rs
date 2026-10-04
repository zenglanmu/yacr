//! Unit tests.

use super::*;
use cad_db::{AnnotationDatabase, DrawingDatabaseBuilder, Layer};

fn request(offset: usize, limit: usize) -> QueryRequest {
    QueryRequest {
        document: DocumentId(1),
        revision: Revision(0),
        request: RequestId(1),
        offset,
        limit,
    }
}

#[test]
fn layers_are_paged() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for i in 0..5u128 {
        builder
            .insert_layer(Layer {
                id: LayerId(i),
                name: format!("L{i}"),
                visible: true,
            })
            .unwrap();
    }
    let db = builder.finish().unwrap();
    let service = QueryService::new();
    let page = service.layers(&db, request(1, 2)).unwrap();
    assert_eq!(page.total, 5);
    assert_eq!(page.rows.len(), 2);
    assert_eq!(page.rows[0].name, "L1");
}

#[test]
fn empty_selection_reports_unset() {
    let service = QueryService::new();
    let page = service.properties(&[], request(0, 10)).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.rows.len(), 2);
    let entities: Vec<_> = page.rows.iter().filter(|r| r.key == "entity").collect();
    assert_eq!(entities.len(), 1);
    assert!(matches!(entities[0].value, PropertyValue::Unset));
    let count = page.rows.iter().find(|r| r.key == "count").unwrap();
    assert!(matches!(&count.value, PropertyValue::Value(value) if value == "0"));

    let first = service.properties(&[], request(0, 1)).unwrap();
    assert_eq!(first.total, 2);
    assert_eq!(first.rows.len(), 1);
    assert_eq!(first.rows[0].key, "entity");
    assert!(matches!(first.rows[0].value, PropertyValue::Unset));
}

#[test]
fn properties_reject_foreign_document_selections_without_publishing() {
    for documents in [vec![DocumentId(2)], vec![DocumentId(1), DocumentId(2)]] {
        let service = QueryService::new();
        let selection: Vec<_> = documents
            .into_iter()
            .map(|document| SelectionRef {
                document,
                entity: EntityId(1),
                instance: InstancePath::default(),
                sub_element: None,
            })
            .collect();
        assert!(matches!(
            service.properties(&selection, request(0, 10)),
            Err(CadError::InvalidInput(_))
        ));
        assert_eq!(service.last_revision(), None);
        service.properties(&[], request(0, 10)).unwrap();
    }
}

#[test]
fn properties_reject_document_switch_without_changing_query_state() {
    let service = QueryService::new();
    let db = DrawingDatabaseBuilder::new(DatabaseId(1)).finish().unwrap();
    service.layers(&db, request(0, 10)).unwrap();
    let mut other = request(0, 10);
    other.document = DocumentId(2);
    other.revision = Revision(7);
    assert_eq!(
        service.properties(&[], other).unwrap_err(),
        CadError::StaleResult
    );
    assert_eq!(service.last_revision(), Some(db.revision()));
    service.layers(&db, request(0, 10)).unwrap();
}

#[test]
fn zero_limit_returns_no_rows_but_preserves_total() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(1),
            name: "L1".into(),
            visible: true,
        })
        .unwrap();
    let db = builder.finish().unwrap();
    let service = QueryService::new();
    let layers = service.layers(&db, request(0, 0)).unwrap();
    assert_eq!(layers.total, 1);
    assert!(layers.rows.is_empty());
    assert_eq!(layers.request.limit, 0);
    let properties = service.properties(&[], request(0, 0)).unwrap();
    assert_eq!(properties.total, 2);
    assert!(properties.rows.is_empty());
    assert_eq!(properties.request.limit, 0);
}

#[test]
fn multi_select_differing_values_are_mixed() {
    let service = QueryService::new();
    let selection = vec![
        SelectionRef {
            document: DocumentId(1),
            entity: EntityId(1),
            instance: InstancePath::default(),
            sub_element: None,
        },
        SelectionRef {
            document: DocumentId(1),
            entity: EntityId(2),
            instance: InstancePath::default(),
            sub_element: None,
        },
    ];
    let page = service.properties(&selection, request(0, 10)).unwrap();
    let entity = page.rows.iter().find(|r| r.key == "entity").unwrap();
    assert!(matches!(entity.value, PropertyValue::Mixed));
}

#[test]
fn document_switch_is_stale() {
    let service = QueryService::new();
    let db = DrawingDatabaseBuilder::new(DatabaseId(1)).finish().unwrap();
    service.layers(&db, request(0, 10)).unwrap();
    let mut other = request(0, 10);
    other.document = DocumentId(2);
    assert_eq!(
        service.layers(&db, other).unwrap_err(),
        CadError::StaleResult
    );
}

#[test]
fn annotation_query_works() {
    let db = AnnotationDatabase::new(DatabaseId(2));
    let service = QueryService::new();
    let page = service.annotations(&db, request(0, 10)).unwrap();
    assert_eq!(page.total, 0);
}

fn change(database: DatabaseId, before: u64, after: u64) -> ChangeSet {
    ChangeSet {
        database,
        before: Revision(before),
        after: Revision(after),
        transaction: TransactionId(1),
        reason: "query update regression".into(),
        changes: Vec::new(),
    }
}

#[test]
fn update_preserves_document_and_accepts_consecutive_changes() {
    let db = AnnotationDatabase::new(DatabaseId(42));
    let mut service = QueryService::new();
    service.annotations(&db, request(0, 10)).unwrap();
    service.update(&change(db.id(), 0, 1)).unwrap();
    service.update(&change(db.id(), 1, 2)).unwrap();
    assert_eq!(service.last_revision(), Some(Revision(2)));

    let mut other = request(0, 10);
    other.document = DocumentId(0);
    assert_eq!(
        service.annotations(&db, other).unwrap_err(),
        CadError::StaleResult
    );
    assert_eq!(service.last_revision(), Some(Revision(2)));
    service.annotations(&db, request(0, 10)).unwrap();
}

#[test]
fn update_without_snapshot_is_stale_without_binding_document() {
    let mut service = QueryService::new();
    assert_eq!(
        service.update(&change(DatabaseId(42), 0, 1)),
        Err(CadError::StaleResult)
    );
    assert_eq!(service.last_revision(), None);
    let db = AnnotationDatabase::new(DatabaseId(42));
    service.annotations(&db, request(0, 10)).unwrap();
    service.update(&change(db.id(), 0, 1)).unwrap();
}

#[test]
fn update_rejects_unknown_database_after_selection_query() {
    let mut service = QueryService::new();
    service.properties(&[], request(0, 10)).unwrap();
    assert_eq!(
        service.update(&change(DatabaseId(1), 0, 1)),
        Err(CadError::StaleResult)
    );
    assert_eq!(service.last_revision(), Some(Revision(0)));
    service.properties(&[], request(0, 10)).unwrap();
}

#[test]
fn selection_query_does_not_carry_forward_database_binding() {
    let db = AnnotationDatabase::new(DatabaseId(42));
    let mut service = QueryService::new();
    service.annotations(&db, request(0, 10)).unwrap();
    let mut selection_request = request(0, 10);
    selection_request.revision = Revision(7);
    service.properties(&[], selection_request).unwrap();
    assert_eq!(
        service.update(&change(db.id(), 7, 8)),
        Err(CadError::StaleResult)
    );
    assert_eq!(service.last_revision(), Some(Revision(7)));
    service.annotations(&db, request(0, 10)).unwrap();
    service.update(&change(db.id(), 0, 1)).unwrap();
}

#[test]
fn update_rejects_foreign_database_without_mutating_binding() {
    let db = AnnotationDatabase::new(DatabaseId(42));
    let mut service = QueryService::new();
    service.annotations(&db, request(0, 10)).unwrap();
    assert_eq!(
        service.update(&change(DatabaseId(1), 0, 1)),
        Err(CadError::StaleResult)
    );
    assert_eq!(service.last_revision(), Some(Revision(0)));
    service.update(&change(db.id(), 0, 1)).unwrap();
}

#[test]
fn update_rejects_discontinuous_and_non_advancing_revisions() {
    let db = AnnotationDatabase::new(DatabaseId(42));
    for (before, after) in [(7, 1), (0, 2), (0, 0), (7, 0), (2, 1)] {
        let mut service = QueryService::new();
        service.annotations(&db, request(0, 10)).unwrap();
        assert_eq!(
            service.update(&change(db.id(), before, after)),
            Err(CadError::StaleResult)
        );
        assert_eq!(service.last_revision(), Some(Revision(0)));
        service.update(&change(db.id(), 0, 1)).unwrap();
        service.annotations(&db, request(0, 10)).unwrap();
    }
}

#[test]
fn update_rejects_replay_without_silent_success() {
    let db = AnnotationDatabase::new(DatabaseId(42));
    let mut service = QueryService::new();
    service.annotations(&db, request(0, 10)).unwrap();
    let changes = change(db.id(), 0, 1);
    service.update(&changes).unwrap();
    assert_eq!(service.update(&changes), Err(CadError::StaleResult));
    assert_eq!(service.last_revision(), Some(Revision(1)));
    service.update(&change(db.id(), 1, 2)).unwrap();
}

#[test]
fn update_at_max_revision_is_stale_without_overflow() {
    let mut service = QueryService::new();
    service.remember(&DocumentId(1), Revision(u64::MAX), Some(DatabaseId(42)));
    for after in [0, u64::MAX] {
        assert_eq!(
            service.update(&change(DatabaseId(42), u64::MAX, after)),
            Err(CadError::StaleResult)
        );
        assert_eq!(service.last_revision(), Some(Revision(u64::MAX)));
    }
}

#[test]
fn database_backed_query_replaces_observed_database_binding() {
    let drawing = DrawingDatabaseBuilder::new(DatabaseId(41))
        .finish()
        .unwrap();
    let annotations = AnnotationDatabase::new(DatabaseId(42));
    let mut service = QueryService::new();
    service.layers(&drawing, request(0, 10)).unwrap();
    service.update(&change(drawing.id(), 0, 1)).unwrap();
    service.annotations(&annotations, request(0, 10)).unwrap();
    assert_eq!(
        service.update(&change(drawing.id(), 0, 1)),
        Err(CadError::StaleResult)
    );
    assert_eq!(service.last_revision(), Some(annotations.revision()));
    service.update(&change(annotations.id(), 0, 1)).unwrap();
}
