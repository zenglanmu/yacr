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
    last: std::cell::RefCell<Option<(DocumentId, Revision)>>,
}

impl QueryService {
    pub fn new() -> Self {
        Self::default()
    }

    fn check_fresh(&self, request: &QueryRequest) -> CadResult<()> {
        if let Some((doc, _)) = self.last.borrow().as_ref() {
            if doc != &request.document {
                // Document switched: the caller should rebuild, not reuse.
                return Err(CadError::StaleResult);
            }
        }
        Ok(())
    }

    fn remember(&self, document: &DocumentId, revision: Revision) {
        *self.last.borrow_mut() = Some((*document, revision));
    }

    pub fn layers(
        &self,
        database: &DrawingDatabase,
        mut request: QueryRequest,
    ) -> CadResult<QueryPage<LayerRow>> {
        self.check_fresh(&request)?;
        let all: Vec<LayerRow> = database
            .layers()
            .map(|l| LayerRow {
                id: l.id,
                name: l.name.clone(),
                visible: l.visible,
            })
            .collect();
        request.revision = database.revision();
        self.remember(&request.document, request.revision);
        emit(request, all)
    }

    pub fn annotations(
        &self,
        database: &AnnotationDatabase,
        mut request: QueryRequest,
    ) -> CadResult<QueryPage<AnnotationRow>> {
        self.check_fresh(&request)?;
        let all: Vec<AnnotationRow> = database
            .annotations()
            .map(|a| AnnotationRow {
                id: a.id,
                label: if a.text.is_empty() {
                    format!("{:?}", a.geometry)
                } else {
                    a.text.clone()
                },
                hidden: false,
            })
            .collect();
        request.revision = database.revision();
        self.remember(&request.document, request.revision);
        emit(request, all)
    }

    /// Selection properties; multi-select differing values become `Mixed`.
    pub fn properties(
        &self,
        selection: &[SelectionRef],
        mut request: QueryRequest,
    ) -> CadResult<QueryPage<SelectionProperty>> {
        let mut rows = Vec::new();
        let entities: Vec<String> = selection
            .iter()
            .map(|s| format!("{}", s.entity.0))
            .collect();
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
            rows.push(SelectionProperty {
                key: "entity".into(),
                value: PropertyValue::Unset,
            });
            rows.dedup_by(|a, b| a.key == b.key && matches!(a.value, PropertyValue::Unset));
        }
        request.revision = Revision(request.revision.0);
        self.remember(&request.document, request.revision);
        emit(request, rows)
    }

    /// Note a change set; a non-continuous revision forces a snapshot rebuild.
    pub fn update(&mut self, changes: &ChangeSet) -> CadResult<()> {
        let mut last = self.last.borrow_mut();
        if let Some((_, revision)) = last.as_ref() {
            if changes.after.0 != revision.0 && changes.after.0 != revision.0 + 1 {
                // Subscribers must rebuild from the database (spec §4.6).
                *last = Some((DocumentId(0), changes.after));
                return Err(CadError::StaleResult);
            }
        }
        *last = Some((DocumentId(0), changes.after));
        Ok(())
    }

    pub fn last_revision(&self) -> Option<Revision> {
        self.last.borrow().as_ref().map(|(_, r)| *r)
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
    Ok(QueryPage {
        request,
        rows,
        total,
    })
}

#[cfg(test)]
mod tests;
