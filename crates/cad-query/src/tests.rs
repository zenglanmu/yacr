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
