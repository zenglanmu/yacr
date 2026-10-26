//! AutoCAD's "layer `0` inside a block" resolution, as one shared answer.
//!
//! AutoCAD's documented rule: geometry that lives on layer `0` inside a block
//! definition is drawn as if it were on the layer the block reference (`INSERT`)
//! is placed on. A member on any other layer keeps its own layer. So the
//! effective layer of an entity is
//!
//! ```text
//! own layer                       (own != layer "0")
//! containing INSERT's layer       (own == layer "0")
//! ```
//!
//! and the rule **chains**: an `INSERT` that is itself a layer-`0` member of
//! another block presents its *effective* layer to its own children, so a
//! layer-`0` member of a layer-`0` `INSERT` placed on `RED` resolves to `RED`.
//!
//! This is a **display-resolution** rule only. `DbEntity::layer` keeps the fact
//! the source stored — the properties panel, the exporter and a round-trip all
//! read it — and nothing here writes it. The rule answers two questions from one
//! resolution: which layer row owns an entity's `ByLayer` styles
//! (`cad-representation`'s expansion) and which layer's visibility decides
//! whether the entity is drawn and picked (`cad-app`'s visibility walks).
//!
//! Layer `0` is identified **by name** ([`DEFAULT_LAYER_NAME`]), never as
//! `LayerId(0)`: layer ids are table indices, so the row the source calls `0`
//! can carry any id, and `LayerId(0)` can be a row with another name. A drawing
//! with no row named `0` has **no** inheritance
//! ([`DrawingDatabase::layer_zero_id`] is `None`): no member can be on a layer
//! the drawing does not have, so the honest answer is "keep the own layer"
//! rather than a guess or a panic.

use cad_domain::LayerId;

use super::{DrawingDatabase, DEFAULT_LAYER_NAME};

impl DrawingDatabase {
    /// The id of the layer row named `name`, matched **exactly**.
    ///
    /// The comparison is the one the importer uses when it falls back to layer
    /// `0` for an entity whose layer name is missing from the source's layer
    /// table, so a name that resolved to a row at import resolves to the same
    /// row here. Case-insensitive matching is deliberately **not** used: this
    /// answers a display question about the source's own spelling, and a fuzzy
    /// match could pick a different row than the importer did.
    ///
    /// `None` when the table has no such row. If a database (invalid in AutoCAD,
    /// where layer names are unique) holds two rows with the same name, the
    /// lowest [`LayerId`] wins, because [`DrawingDatabase::layers`] iterates a
    /// `BTreeMap` by id.
    pub fn layer_id_by_name(&self, name: &str) -> Option<LayerId> {
        self.layers()
            .find(|layer| layer.name == name)
            .map(|layer| layer.id)
    }

    /// The id of this drawing's layer `0`, or `None` when it has no such row.
    ///
    /// Resolve this **once** per walk (one scene expansion, one selection pass)
    /// and carry it as a [`LayerScope`]; it is a layer-table scan and is never
    /// meant to run per entity.
    pub fn layer_zero_id(&self) -> Option<LayerId> {
        self.layer_id_by_name(DEFAULT_LAYER_NAME)
    }

    /// The layer-inheritance scope of a model-space entity: this drawing's layer
    /// `0` and no containing `INSERT`.
    ///
    /// One layer-table scan — hoist it out of a per-entity loop.
    pub fn root_layer_scope(&self) -> LayerScope {
        LayerScope {
            layer_zero: self.layer_zero_id(),
            parent_layer: None,
        }
    }
}

/// Where one entity sits in the layer-`0` chain.
///
/// Built once from [`DrawingDatabase::root_layer_scope`] and then carried down a
/// block walk with [`Self::child_scope`], so a recursive expansion and an
/// iterative instance-path walk produce the **same** answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerScope {
    /// This drawing's layer-`0` id; `None` means the drawing has no layer `0`,
    /// which turns the inheritance rule off entirely.
    pub layer_zero: Option<LayerId>,
    /// The **effective** layer of the containing `INSERT`; `None` at the model
    /// root. Storing the effective layer (not the stored one) is what makes the
    /// rule chain through nested blocks.
    pub parent_layer: Option<LayerId>,
}

impl LayerScope {
    /// The layer an entity in this scope resolves its `ByLayer` styles and its
    /// visibility against.
    pub fn effective_layer(&self, own: LayerId) -> LayerId {
        match (self.layer_zero, self.parent_layer) {
            // Layer `0` adopts the containing INSERT's layer. `parent_layer` is
            // itself already resolved, so a nested chain needs no extra loop.
            (Some(zero), Some(parent)) if own == zero => parent,
            // Any other layer keeps its own identity, and layer `0` at the model
            // root (no containing INSERT) stays layer `0`.
            _ => own,
        }
    }

    /// The scope the children of an `INSERT` expand under.
    ///
    /// `insert_layer` must be that `INSERT`'s **effective** layer — what
    /// [`Self::effective_layer`] returned for the `INSERT` itself — so a
    /// layer-`0` `INSERT` passes its own containing block's layer one level
    /// further down.
    pub fn child_scope(&self, insert_layer: LayerId) -> Self {
        Self {
            layer_zero: self.layer_zero,
            parent_layer: Some(insert_layer),
        }
    }
}
