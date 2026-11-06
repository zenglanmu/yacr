//! Narrow, command-authorized incremental display transition.
use std::sync::Arc;

use cad_db::{ChangeSet, DrawingDatabase, ObjectChange};
use cad_domain::{EntityId, SemanticGeometry, SpaceId};

use crate::{CommandId, RawCommandTrace};

/// A successful entity-only CreateLine transition, not an arbitrary insert list.
/// Block/table writes are not fully described by ChangeSet, so callers must
/// supply the actual command trace as well as the authoritative snapshots.
#[derive(Clone)]
pub struct DrawingEditReceipt {
    pub(super) before: Arc<DrawingDatabase>,
    pub(super) after: Arc<DrawingDatabase>,
    pub(super) inserted: EntityId,
}

impl DrawingEditReceipt {
    pub(crate) fn from_create_line_trace(
        before: Arc<DrawingDatabase>,
        after: Arc<DrawingDatabase>,
        traces: &[RawCommandTrace],
    ) -> Option<Self> {
        let [trace] = traces else { return None };
        if trace.command != CommandId::CreateLine || Arc::ptr_eq(&before, &after) {
            return None;
        }
        let outcome = trace.result.as_ref().ok()?;
        let changes: &ChangeSet = outcome.changes.as_deref()?;
        if !changes.follows(before.id(), before.revision())
            || after.id() != before.id()
            || after.revision() != changes.after
            || before.entity_count().checked_add(1)? != after.entity_count()
        {
            return None;
        }
        let [ObjectChange::Insert(object)] = changes.changes.as_slice() else {
            return None;
        };
        let inserted = EntityId(object.0);
        if before.entity(inserted).is_some() {
            return None;
        }
        let entity = after.entity(inserted)?;
        if entity.space != SpaceId::Model
            || !matches!(entity.geometry, SemanticGeometry::Line { .. })
            || entity.object.source_handle.is_some()
        {
            return None;
        }
        Some(Self {
            before,
            after,
            inserted,
        })
    }

    pub fn after(&self) -> &Arc<DrawingDatabase> {
        &self.after
    }
}

/// GPU append proof: the exact prepared CPU base that must already be resident.
#[derive(Clone)]
pub struct BaseAppend {
    pub predecessor: Arc<super::SceneDelta>,
    pub keep_batches: usize,
}
