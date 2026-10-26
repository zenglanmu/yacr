//! The authoritative, read-only-after-import drawing database.
//!
//! The implementation is split across the submodules of this directory; every
//! moved item is re-exported here under its original name, so the module path
//! `crate::drawing::…` (and therefore the crate's public surface) is unchanged.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use cad_domain::*;

use crate::dim_style::DimStyle;
use crate::entity::{DbEntity, EntityRenderAttributes};
use crate::mleader_style::MLeaderStyle;
use crate::tables::{
    ActiveAnnotationScale, BlockDefinition, Layer, LayerAttributes, Layout, LineType,
    PlotSettingsRecord, Scale, Style,
};

mod access;
mod bounds;
mod edit;
mod layer_edit;
mod layer_inherit;
mod lock;
mod transaction;

#[cfg(test)]
mod test_support;

pub use access::{EntityDisplayState, VisibilityMembership, VisibilityStateMembership};
pub use layer_edit::{DEFAULT_LAYER_NAME, ILLEGAL_LAYER_NAME_CHARS};
pub use layer_inherit::LayerScope;
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
    /// Drawing-global POINT display mode (`$PDMODE`), exactly as the source
    /// declared it. `0` (a dot) is the DWG default and the value a database that
    /// was not built by the importer gets.
    ///
    /// Stored **raw**: the display layer interprets the value — including the
    /// marker shapes this project cannot express — and reports them, so the db
    /// never invents a marker shape and never silently rewrites the header.
    pub(crate) point_display_mode: i16,
    /// Drawing-global POINT display size (`$PDSIZE`), exactly as the source
    /// declared it. `0.0` — the DWG default — means "relative to the viewport"
    /// (see `cad_app::render_scene::point_marker_overlay`); `> 0` is an absolute
    /// world size and `< 0` a percentage of the viewport height.
    ///
    /// Stored **raw** (a non-finite or absurd value is kept as read): the
    /// display layer owns the single sanitisation rule, so the db does not keep a
    /// second copy of it and a corrupt header stays observable.
    pub(crate) point_display_size: f64,
    /// Plot configuration per paper-space layout. Absent entries are resolved
    /// to an explicit default page by [`DrawingDatabase::plot_settings_for`].
    pub(crate) plot_settings: BTreeMap<LayoutId, PlotSettingsRecord>,
    /// Per-entity display attributes the importer resolved from the source
    /// (transparency, geometry source). Absent entries resolve to
    /// [`EntityRenderAttributes::default`] (ByLayer opacity/colour/lineweight,
    /// continuous linetype, analytic geometry), so older/hand-built databases
    /// stay valid.
    pub(crate) render_attributes: BTreeMap<EntityId, EntityRenderAttributes>,
    /// Per-layer attributes the importer resolved from the source (flags,
    /// colour, linetype, lineweight, plottable, opacity, plot style).
    ///
    /// Kept beside [`Self::layers`] rather than inside `Layer` so the row stays
    /// the three fields every existing consumer reads. Absent entries resolve to
    /// [`LayerAttributes::default`] (on, thawed, unlocked, plottable, ACI 7
    /// white, `Continuous`, ByLayer weight, opaque), so older and hand-built
    /// databases stay valid without a migration.
    pub(crate) layer_attributes: BTreeMap<LayerId, LayerAttributes>,
    /// Named annotation scales from the drawing's `ACAD_SCALELIST`. Empty for
    /// hand-built databases; the importer fills it from the source drawing.
    pub(crate) scales: BTreeMap<ScaleId, Scale>,
    /// DIMSTYLE table entries (ADR 0017 B.1). Always contains a `Standard`
    /// (the pinned metric default for a hand-built database, or the source
    /// `Standard` when one was imported). Read-only after import: P5b provides
    /// no style-edit command (ADR 0017 B.3).
    pub(crate) dim_styles: BTreeMap<DimStyleId, DimStyle>,
    /// Name index over [`Self::dim_styles`] so a style resolves by name without
    /// scanning the table.
    pub(crate) dim_style_ids_by_name: BTreeMap<String, DimStyleId>,
    /// MLEADERSTYLE table entries (ADR 0019 decision D, P7c). Always contains a
    /// `Standard` (the pinned constant row for a hand-built database, or the
    /// source `Standard` when one was imported). Read-only after import: there is
    /// no style-edit command.
    pub(crate) mleader_styles: BTreeMap<MLeaderStyleId, MLeaderStyle>,
    /// Name index over [`Self::mleader_styles`] so a style resolves by name
    /// without scanning the table.
    pub(crate) mleader_style_ids_by_name: BTreeMap<String, MLeaderStyleId>,
    /// Drawing-global active annotation scale (`CANNOSCALE`), when set.
    pub(crate) active_annotation_scale: Option<ActiveAnnotationScale>,
    /// The source drawing's declared text code page (`$DWGCODEPAGE` /
    /// `DOCUMENT.header.code_page`), exactly as the reader reported it.
    ///
    /// `None` means the source declared nothing (or an empty string): the text
    /// locale is then unknown and callers must not guess one. The db deliberately
    /// stores the raw code page, not a locale, so the platform layer owns the
    /// single mapping table.
    pub(crate) code_page: Option<String>,
    /// Next entity id to hand out. Monotonic and never reset when entities are
    /// deleted, so a deleted id is never reused (its `ObjectId` would otherwise
    /// alias a new object). Initialised by the builder from the largest existing
    /// entity/block-member id.
    pub(crate) next_entity_id: u128,
    /// Next layer id to hand out. Monotonic like [`Self::next_entity_id`]: it is
    /// initialised by the builder from `max(existing layer id) + 1` and never
    /// rewound by a delete, so a deleted layer id is never reused (a stale layer
    /// reference could otherwise alias a newly created layer).
    pub(crate) next_layer_id: u128,
    /// Cache of [`Self::bounds`] tagged with the [`Revision`] it was computed
    /// at. Interior mutability is required because `bounds` takes `&self` (the
    /// scene-preparation worker shares one `Arc<DrawingDatabase>` across
    /// threads), and the cache is shared by clones. Correctness is guaranteed by
    /// the revision tag, never by freshness of the shared cell: a cache entry is
    /// only served when its tag equals the current revision.
    pub(crate) bounds_cache: Arc<RwLock<BoundsCache>>,
}
