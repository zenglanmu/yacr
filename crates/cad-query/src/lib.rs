//! Paged, revision-bound projections. No duplicate mutable document truth.
use cad_db::{AnnotationDatabase, ChangeSet, DrawingDatabase};
use cad_domain::*;
pub enum PropertyValue<T> {
    Value(T),
    Mixed,
    Unset,
}
pub struct QueryRequest {
    pub document: DocumentId,
    pub revision: Revision,
    pub request: RequestId,
    pub offset: usize,
    pub limit: usize,
}
pub struct QueryPage<T> {
    pub request: QueryRequest,
    pub rows: Vec<T>,
    pub total: usize,
}
pub struct LayerRow {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
}
pub struct AnnotationRow {
    pub id: AnnotationId,
    pub label: String,
    pub hidden: bool,
}
pub struct SelectionProperty {
    pub key: String,
    pub value: PropertyValue<String>,
}
pub struct ToolStateModel {
    pub prompt_key: String,
    pub can_confirm: bool,
    pub can_cancel: bool,
}
pub struct QueryService;
impl QueryService {
    pub fn layers(
        &self,
        _database: &DrawingDatabase,
        _request: QueryRequest,
    ) -> CadResult<QueryPage<LayerRow>> {
        pending("query.layers")
    }
    pub fn annotations(
        &self,
        _database: &AnnotationDatabase,
        _request: QueryRequest,
    ) -> CadResult<QueryPage<AnnotationRow>> {
        pending("query.annotations")
    }
    pub fn properties(
        &self,
        _selection: &[SelectionRef],
        _request: QueryRequest,
    ) -> CadResult<QueryPage<SelectionProperty>> {
        pending("query.properties")
    }
    pub fn update(&self, _changes: &ChangeSet) -> CadResult<()> {
        pending("query.incremental_update")
    }
}
