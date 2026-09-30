use cad_db::{Annotation, AnnotationDatabase, ChangeSet};
use cad_domain::*;
#[derive(Debug, Clone)]
pub struct AnnotationPatch { pub id: AnnotationId, pub before: Option<Annotation>, pub after: Option<Annotation> }
#[derive(Debug, Clone)]
pub struct UndoRecord { pub transaction: TransactionId, pub label: String, pub patches: Vec<AnnotationPatch>, pub merge_key: Option<String> }
pub struct History { pub memory_budget_bytes: usize }
impl History {
    pub fn record(&mut self, _record: UndoRecord) -> CadResult<()> { pending("history.record") }
    pub fn undo(&mut self, _database: &mut AnnotationDatabase) -> CadResult<ChangeSet> { pending("history.undo") }
    pub fn redo(&mut self, _database: &mut AnnotationDatabase) -> CadResult<ChangeSet> { pending("history.redo") }
}
pub trait RecoveryJournal {
    fn append(&mut self, record: &UndoRecord) -> CadResult<()>;
    fn recover(&self, database: &mut AnnotationDatabase) -> CadResult<Vec<ChangeSet>>;
}
