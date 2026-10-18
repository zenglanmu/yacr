//! The authoritative, read-only-after-import drawing database.
//!
//! The implementation is split across the submodules of this directory; every
//! moved item is re-exported here under its original name, so the module path
//! `crate::drawing::…` (and therefore the crate's public surface) is unchanged.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use cad_domain::*;

use crate::entity::{DbEntity, EntityRenderAttributes};
use crate::tables::{
    ActiveAnnotationScale, BlockDefinition, Layer, Layout, LineType, PlotSettingsRecord, Scale,
    Style,
};

mod access;
mod bounds;
mod edit;
mod transaction;

#[cfg(test)]
mod test_support;

pub use access::{EntityDisplayState, VisibilityMembership, VisibilityStateMembership};
pub use transaction::DrawingTransaction;

pub(crate) use bounds::BoundsCache;

/// The authoritative, read-only-after-import drawing database.
#[derive(Debug, Clone)]
pub struct DrawingDatabase {
    pub(crate) id: DatabaseId,
    pub(crate) revision: Revision,
    pub(crate) entities: BTreeMap<EntityId, DbEntity>,
    pub(crate) layers: BTreeMap<LayerId, Layer>,
    pub(crate) blocks: BTreeMap<BlockId, BlockDefinition>,
    pub(crate) layouts: BTreeMap<LayoutId, Layout>,
    pub(crate) styles: BTreeMap<StyleId, Style>,
    /// Named linetype table entries (spec §3.2). Empty for hand-built
    /// databases; the importer fills it from the source drawing.
    pub(crate) linetypes: BTreeMap<LinetypeId, LineType>,
    /// Drawing-global linetype scale (`$LTSCALE`). `1.0` is the DWG default and
    /// the value used when a database was not built by the importer.
    pub(crate) linetype_scale: f64,
    /// Plot configuration per paper-space layout. Absent entries are resolved
    /// to an explicit default page by [`DrawingDatabase::plot_settings_for`].
    pub(crate) plot_settings: BTreeMap<LayoutId, PlotSettingsRecord>,
    /// Per-entity display attributes that the importer resolved from the source
    /// (transparency, geometry source). Absent entries are fully opaque
    /// analytic geometry, so older/hand-built databases stay valid.
    pub(crate) render_attributes: BTreeMap<EntityId, EntityRenderAttributes>,
    /// Named annotation scales from the drawing's `ACAD_SCALELIST`. Empty for
    /// hand-built databases; the importer fills it from the source drawing.
    pub(crate) scales: BTreeMap<ScaleId, Scale>,
    /// Drawing-global active annotation scale (`CANNOSCALE`), when set.
    pub(crate) active_annotation_scale: Option<ActiveAnnotationScale>,
    /// Next entity id to hand out. Monotonic and never reset when entities are
    /// deleted, so a deleted id is never reused (its `ObjectId` would otherwise
    /// alias a new object). Initialised by the builder from the largest existing
    /// entity/block-member id.
    pub(crate) next_entity_id: u128,
    /// Cache of [`Self::bounds`] tagged with the [`Revision`] it was computed
    /// at. Interior mutability is required because `bounds` takes `&self` (the
    /// scene-preparation worker shares one `Arc<DrawingDatabase>` across
    /// threads), and the cache is shared by clones. Correctness is guaranteed by
    /// the revision tag, never by freshness of the shared cell: a cache entry is
    /// only served when its tag equals the current revision.
    pub(crate) bounds_cache: Arc<RwLock<BoundsCache>>,
}
