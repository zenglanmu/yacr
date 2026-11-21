//! Read-only BIM accessors over [`DrawingDatabase`] (ADR-0042 D2/D3).
//!
//! The data itself lives in two side tables on the database
//! (`bim_elements`/`bim_definitions`), exactly like render attributes: the
//! entity row keeps its six fields and a document without BIM data stores
//! nothing here. These accessors are the whole read surface commit B (the
//! property panel) and the exporters need; nothing here mutates.

use cad_domain::EntityId;

use super::DrawingDatabase;
use crate::bim::{BimDefinition, BimDefinitions, BimElement, BimKey, BimRelations};

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
