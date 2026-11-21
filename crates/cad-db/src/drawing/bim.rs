//! Read-only BIM accessors over [`DrawingDatabase`] (ADR-0042 D2/D3).
//!
//! The data itself lives in two side tables on the database
//! (`bim_elements`/`bim_definitions`), exactly like render attributes: the
//! entity row keeps its six fields and a document without BIM data stores
//! nothing here. These accessors are the whole read surface commit B (the
//! property panel) and the exporters need; nothing here mutates.
//!
//! The one write **record** type lives here too ([`BimPatch`]): `cad-history`
//! carries the exact before/after shape instead of inventing a second one, the
//! same reason [`super::LinetypePatch`] lives beside the linetype write path.

use cad_domain::EntityId;

use super::DrawingDatabase;
use crate::bim::{BimDefinition, BimDefinitions, BimElement, BimKey, BimRelations};

/// A before/after record of one element's BIM data.
///
/// Both sides are always present: the BIM side table is **replace-only** (an
/// element either has a `bim` block or it does not), so an undo restores
/// `before` by writing it back over `after`, and a redo writes `after`. There is
/// deliberately no `Option` here — this patch can never create or delete an
/// element's BIM data, only overwrite an existing block (use the importer
/// builder to attach one in the first place).
#[derive(Debug, Clone, PartialEq)]
pub struct BimPatch {
    pub id: EntityId,
    pub before: BimElement,
    pub after: BimElement,
}

impl DrawingDatabase {
    /// The BIM data of one element, if it carries any.
    pub fn bim_element(&self, id: EntityId) -> Option<&BimElement> {
        self.bim_elements.get(&id)
    }

    /// Every element with BIM data, keyed by element id.
    pub fn bim_elements(&self) -> &std::collections::BTreeMap<EntityId, BimElement> {
        &self.bim_elements
    }

    /// The shared definition table. Empty when the document has no BIM data.
    pub fn bim_definitions(&self) -> &BimDefinitions {
        &self.bim_definitions
    }

    /// The BIM relation table (ADR-0042 D3 tail/D12). Empty when the document
    /// has no relation edges.
    pub fn bim_relations(&self) -> &BimRelations {
        &self.bim_relations
    }

    /// One shared definition row by composite id.
    pub fn bim_definition(&self, id: &str) -> Option<&BimDefinition> {
        self.bim_definitions.get(id)
    }

    /// The logical component an element belongs to (ADR-0042 D5).
    ///
    /// `None` for an element without BIM data — it has no BIM identity, and
    /// returning its own id would pretend it were a component.
    pub fn bim_key(&self, id: EntityId) -> Option<BimKey> {
        self.bim_elements.get(&id).map(|element| element.key(id))
    }
}
