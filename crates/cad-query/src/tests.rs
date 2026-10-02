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
    assert!(page
        .rows
        .iter()
        .any(|r| matches!(r.value, PropertyValue::Unset)));
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
