//! Paged, revision-bound projections. No duplicate mutable document truth.
//!
//! Spec v2.0 §4.9: UI models are derived from the database; a paged query is
//! bound to a revision so stale results can be discarded after a document
//! switch. Programmatic model refreshes must not re-enter the command path.

use cad_db::{AnnotationDatabase, ChangeSet, DrawingDatabase};
use cad_domain::*;

#[derive(Debug)]
pub enum PropertyValue<T> {
    Value(T),
    Mixed,
    Unset,
}

#[derive(Debug)]
pub struct QueryRequest {
    pub document: DocumentId,
    pub revision: Revision,
    pub request: RequestId,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug)]
pub struct QueryPage<T> {
    pub request: QueryRequest,
    pub rows: Vec<T>,
    pub total: usize,
}

#[derive(Debug)]
pub struct LayerRow {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
}

#[derive(Debug)]
pub struct AnnotationRow {
    pub id: AnnotationId,
    pub label: String,
    pub hidden: bool,
}

#[derive(Debug)]
pub struct SelectionProperty {
    pub key: String,
    pub value: PropertyValue<String>,
}

pub struct ToolStateModel {
    pub prompt_key: String,
    pub can_confirm: bool,
    pub can_cancel: bool,
}

/// Tracks the last revision a query published, so gaps can trigger a rebuild.
#[derive(Default)]
pub struct QueryService {
    last_revision: Option<Revision>,
    last_document: Option<DocumentId>,
}

impl QueryService {
    pub fn new() -> Self {
        Self::default()
    }

    fn check_fresh(&self, request: &QueryRequest) -> CadResult<()> {
        if let Some(doc) = &self.last_document {
            if doc != &request.document {
                // Document switched: the caller should rebuild, not reuse.
                return Err(CadError::StaleResult);
            }
        }
        Ok(())
    }

    fn remember(&mut self, request: &QueryRequest) {
        self.last_revision = Some(request.revision);
        self.last_document = Some(request.document.clone());
    }

    pub fn layers(&self, database: &DrawingDatabase, mut request: QueryRequest) -> CadResult<QueryPage<LayerRow>> {
        self.check_fresh(&request)?;
        let all: Vec<LayerRow> = database
            .layers()
            .map(|l| LayerRow { id: l.id, name: l.name.clone(), visible: l.visible })
            .collect();
        request.revision = database.revision();
        emit(request, all)
    }

    pub fn annotations(&self, database: &AnnotationDatabase, mut request: QueryRequest) -> CadResult<QueryPage<AnnotationRow>> {
        self.check_fresh(&request)?;
        let all: Vec<AnnotationRow> = database
            .annotations()
            .map(|a| AnnotationRow {
                id: a.id,
                label: if a.text.is_empty() { format!("{:?}", a.geometry) } else { a.text.clone() },
                hidden: false,
            })
            .collect();
        request.revision = database.revision();
        emit(request, all)
    }

    /// Selection properties; multi-select differing values become `Mixed`.
    pub fn properties(&self, selection: &[SelectionRef], mut request: QueryRequest) -> CadResult<QueryPage<SelectionProperty>> {
        let mut rows = Vec::new();
        let entities: Vec<String> = selection.iter().map(|s| format!("{}", s.entity.0)).collect();
        rows.push(SelectionProperty {
            key: "entity".into(),
            value: match unify(&entities) {
                Some(v) => PropertyValue::Value(v),
                None => PropertyValue::Mixed,
            },
        });
        rows.push(SelectionProperty {
            key: "count".into(),
            value: PropertyValue::Value(selection.len().to_string()),
        });
        if selection.is_empty() {
            rows.push(SelectionProperty { key: "entity".into(), value: PropertyValue::Unset });
            rows.dedup_by(|a, b| a.key == b.key && matches!(a.value, PropertyValue::Unset));
        }
        request.revision = Revision(request.revision.0);
        emit(request, rows)
    }

    /// Note a change set; a non-continuous revision forces a snapshot rebuild.
    pub fn update(&mut self, changes: &ChangeSet) -> CadResult<()> {
        if let Some(last) = self.last_revision {
            if changes.after.0 != last.0 && changes.after.0 != last.0 + 1 {
                // Subscribers must rebuild from the database (spec §4.6).
                self.last_revision = Some(changes.after);
                return Err(CadError::StaleResult);
            }
        }
        self.last_revision = Some(changes.after);
        Ok(())
    }

    pub fn last_revision(&self) -> Option<Revision> {
        self.last_revision
    }
}

fn unify(values: &[String]) -> Option<String> {
    let first = values.first()?;
    if values.iter().all(|v| v == first) {
        Some(first.clone())
    } else {
        None
    }
}

fn emit<T>(request: QueryRequest, all: Vec<T>) -> CadResult<QueryPage<T>> {
    let total = all.len();
    let rows = all
        .into_iter()
        .skip(request.offset)
        .take(request.limit.max(1))
        .collect();
    Ok(QueryPage { request, rows, total })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{AnnotationDatabase, DrawingDatabaseBuilder, Layer};

    fn request(offset: usize, limit: usize) -> QueryRequest {
        QueryRequest { document: DocumentId(1), revision: Revision(0), request: RequestId(1), offset, limit }
    }

    #[test]
    fn layers_are_paged() {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
        for i in 0..5u128 {
            builder
                .insert_layer(Layer { id: LayerId(i), name: format!("L{i}"), visible: true })
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
        assert!(page.rows.iter().any(|r| matches!(r.value, PropertyValue::Unset)));
    }

    #[test]
    fn multi_select_differing_values_are_mixed() {
        let service = QueryService::new();
        let selection = vec![
            SelectionRef { document: DocumentId(1), entity: EntityId(1), instance: InstancePath::default(), sub_element: None },
            SelectionRef { document: DocumentId(1), entity: EntityId(2), instance: InstancePath::default(), sub_element: None },
        ];
        let page = service.properties(&selection, request(0, 10)).unwrap();
        let entity = page.rows.iter().find(|r| r.key == "entity").unwrap();
        assert!(matches!(entity.value, PropertyValue::Mixed));
    }

    #[test]
    fn document_switch_is_stale() {
        let mut service = QueryService::new();
        let db = DrawingDatabaseBuilder::new(DatabaseId(1)).finish().unwrap();
        service.layers(&db, request(0, 10)).unwrap();
        let mut other = request(0, 10);
        other.document = DocumentId(2);
        assert_eq!(service.layers(&db, other).unwrap_err(), CadError::StaleResult);
    }

    #[test]
    fn annotation_query_works() {
        let db = AnnotationDatabase::new(DatabaseId(2));
        let service = QueryService::new();
        let page = service.annotations(&db, request(0, 10)).unwrap();
        assert_eq!(page.total, 0);
    }
}
