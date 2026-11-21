//! The authoritative, read-only-after-import drawing database.
//!
//! The implementation is split across the submodules of this directory; every
//! moved item is re-exported here under its original name, so the module path
//! `crate::drawing::…` (and therefore the crate's public surface) is unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use cad_domain::*;

use crate::bim::{BimDefinitions, BimElement, BimRelations};
use crate::dim_style::DimStyle;
use crate::entity::{DbEntity, EntityRenderAttributes};
use crate::mleader_style::MLeaderStyle;
use crate::tables::{
    ActiveAnnotationScale, BlockDefinition, BlockMetadata, Layer, LayerAttributes, Layout,
    LineType, PlotSettingsRecord, Scale, Style,
};
use crate::xref::ExternalReference;

mod access;
mod bim;
mod block_edit;
mod bounds;
mod edit;
mod header_geometry;
mod header_meta;
mod layer_edit;
mod layer_inherit;
mod layer_patch;
mod layout_edit;
mod linetype_edit;
mod linetype_scale;
mod lock;
mod named_delete;
pub(crate) mod param;
pub(crate) mod param_build;
mod point_style;
mod scale_edit;
mod style_edit;
mod transaction;

#[cfg(test)]
mod test_support;

pub use self::bim::BimPatch;
pub use access::{EntityDisplayState, VisibilityMembership, VisibilityStateMembership};
pub use block_edit::{BlockPatch, BlockRow};
pub use layer_edit::{DEFAULT_LAYER_NAME, ILLEGAL_LAYER_NAME_CHARS};
pub use layer_inherit::LayerScope;
pub use layer_patch::{LayerPatch, LayerRow};
pub use layout_edit::{LayoutPatch, LayoutRow, ILLEGAL_LAYOUT_NAME_CHARS, MAX_LAYOUT_NAME_CHARS};
pub use linetype_edit::LinetypePatch;
pub use param::{
    ParamBindingPatch, ParamDiagnostic, ParamPatch, ParametricOutcome, PARAM_MAX_CONFIGS,
    PARAM_MAX_CONFIG_BYTES, PARAM_MAX_TABLE_BYTES,
};
pub use scale_edit::{ScalePatch, ScaleReferenceCounts};
pub use style_edit::{
    DimStylePatch, MLeaderStylePatch, StylePatch, StyleTableChanges, DEFAULT_DIM_STYLE_NAME,
};
pub use transaction::DrawingTransaction;

pub(crate) use bounds::BoundsCache;

/// Which geometry rule an entity batch of a drawing write is checked under.
///
/// The two write kinds are not the same question:
///
/// * [`EntityGeometryCheck::NewWrite`] — a command, a transaction commit or any
///   direct `apply_drawing_changes*` call. The stored geometry must be drawable
///   and measurable, so `validate_geometry` runs and a degenerate row
///   (zero-length line, empty polyline, zero radius, …) is refused.
/// * [`EntityGeometryCheck::RestoreStored`] — history replay (undo/redo of a
///   recorded patch). The batch reinstalls rows **this database already held**,
///   so their geometry is written back byte-for-byte. The importer is
///   deliberately permissive (it records whatever the file holds and reports
///   completeness), so a real drawing can contain geometry the write validator
///   rejects — an imported zero-length line inside a block definition or a paper
///   layout is the real-machine case. Re-validating it would make the database
///   refuse to restore its own previous state, breaking the whole undo step for
///   that drawing. That is why this mode exists.
///
/// Referential integrity is **not** relaxed by the replay mode: layer, block and
/// layout existence (widened by the ids the same batch stages), the `SpaceId`
/// shape, an `INSERT`'s block target, the entity identity relationship, the
/// display-state rules, the block member-list invariant, the layer/layout/style/
/// linetype batch rules and the layer-lock rule all still run.
///
/// The type is crate-private and is selected by exactly one entry point
/// (`DrawingDatabase::replay_drawing_changes_with_blocks_display_layers_layouts_styles_and_linetypes`),
/// so no command can reach the relaxed rule and `validate_geometry` itself stays
/// unchanged. See `drawing::block_edit` for the phase-level contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntityGeometryCheck {
    /// A new or edited row: geometry must survive `crate::validate_geometry`.
    NewWrite,
    /// A row read back out of this database: geometry is restored verbatim.
    RestoreStored,
}

/// The authoritative, read-only-after-import drawing database.
#[derive(Debug, Clone)]
pub struct DrawingDatabase {
    pub(crate) id: DatabaseId,
    pub(crate) revision: Revision,
    /// Process-global construction nonce, assigned once in
    /// [`crate::DrawingDatabaseBuilder::new`] from a monotonic `AtomicU64`.
    ///
    /// It is the "is this the same drawing instance?" half of
    /// [`Self::scene_identity`]: a host may reuse a [`DatabaseId`] for a
    /// different drawing, and this nonce — unlike model-space bounds — changes
    /// on construction, never on a geometry edit, so the identity stays O(1).
    ///
    /// Deliberately not named `generation`: that word already means the
    /// xref/import generation (`cad-import-acadrust`), the session tool
    /// generation and the render device generation.
    ///
    /// Preserved by `Clone`: a clone is the same drawing.
    pub(crate) construction_nonce: u64,
    pub(crate) entities: BTreeMap<EntityId, DbEntity>,
    pub(crate) layers: BTreeMap<LayerId, Layer>,
    pub(crate) blocks: BTreeMap<BlockId, BlockDefinition>,
    /// Import-time name/base-point pair per named block, kept beside
    /// [`Self::blocks`] so [`BlockDefinition`] itself stays the two-field row.
    /// Absent entries mean "unnamed / metadata unknown", never a zero base.
    pub(crate) block_metadata: BTreeMap<BlockId, BlockMetadata>,
    /// Name registrations in registration order, used by
    /// [`Self::block_by_name`] to resolve exact-first then
    /// case-insensitively while still returning the **last** registered
    /// duplicate. Empty names are never registered.
    pub(crate) block_name_order: Vec<(String, BlockId)>,
    /// DWG object handle of each imported block definition, when the source
    /// exposed one. Vendor derived representations reference block definitions
    /// by their **source handle** (the plugin has no table name), so this map
    /// is the only bridge from a vendor primitive back to a `BlockId`.
    /// Hand-built databases leave it empty; no zero handle is ever invented.
    pub(crate) block_source_handles: BTreeMap<u64, BlockId>,
    /// Entities the **source file itself** baked as hidden
    /// (`EntityCommon::invisible`), across top-level model/paper space entities
    /// **and** block members.
    ///
    /// This is the authoritative imported display state, not a guess. It is the
    /// only honest answer for a top-level entity flagged invisible, and for an
    /// evaluated/anonymous dynamic block whose visibility parameter could not be
    /// resolved (the unused states' geometry is really present and really flagged
    /// invisible, so drawing it stacks hidden geometry on the visible members).
    ///
    /// [`DrawingDatabase::block_member_visible`] applies the one priority rule:
    /// a block member governed by a **resolved** dynamic-visibility active state
    /// follows that state set (so state switching keeps working); every other
    /// case is hidden here. [`DrawingDatabase::entity_hidden`] exposes the
    /// top-level answer to display/plot callers.
    ///
    /// Kept beside [`Self::entities`] rather than inside `DbEntity` so the row
    /// stays the imported entity and switching never rewrites geometry. Empty for
    /// hand-built databases.
    pub(crate) hidden_entities: BTreeSet<EntityId>,
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
    /// hand-built databases; the importer fills it from the source drawing. After
    /// import the `SCALELISTEDIT` writes are the only writer (see
    /// [`scale_edit`]).
    pub(crate) scales: BTreeMap<ScaleId, Scale>,
    /// Model-space insertion base point (`$INSBASE`), or `None` when this
    /// database carries no value.
    ///
    /// `None` is the honest state: the importer does not read the variable, so a
    /// fabricated `(0,0,0)` would be indistinguishable from a real origin base.
    /// Written only by [`DrawingDatabase::set_insertion_base`] and exported from
    /// here.
    pub(crate) insertion_base: Option<Point3>,
    /// Model-space limits minimum corner (`$LIMMIN`), or `None` when unset.
    ///
    /// Independent from [`Self::limits_max`] because DXF models the two corners
    /// as separate header variables; [`DrawingDatabase::model_limits`] only
    /// returns a rectangle when both are stored.
    pub(crate) limits_min: Option<[f64; 2]>,
    /// Model-space limits maximum corner (`$LIMMAX`), or `None` when unset.
    pub(crate) limits_max: Option<[f64; 2]>,
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
    /// Next block id to hand out. Monotonic like [`Self::next_entity_id`]: the
    /// builder initialises it from `max(existing block id) + 1`, and neither a
    /// delete nor a replay of a recorded id rewinds it, so a deleted block id is
    /// never reused.
    pub(crate) next_block_id: u128,
    /// Next layer id to hand out. Monotonic like [`Self::next_entity_id`]: it is
    /// initialised by the builder from `max(existing layer id) + 1` and never
    /// rewound by a delete, so a deleted layer id is never reused (a stale layer
    /// reference could otherwise alias a newly created layer).
    pub(crate) next_layer_id: u128,
    /// Next layout id to hand out. Monotonic like [`Self::next_entity_id`]: the
    /// builder initialises it from `max(existing layout id) + 1` (at least 1) and
    /// it is never rewound by a delete, so a deleted layout id is never reused
    /// and `LayoutId(0)` (the reserved synthetic model row) is never allocated.
    pub(crate) next_layout_id: u128,
    /// Next text-style id to hand out. Monotonic like the layer allocator:
    /// initialised by the builder from `max(existing) + 1`, bumped by every
    /// installed row (including an undo restore), never rewound by a delete.
    pub(crate) next_style_id: u128,
    /// Next dimension-style id to hand out (same monotonic rule).
    pub(crate) next_dim_style_id: u128,
    /// Next multileader-style id to hand out (same monotonic rule).
    pub(crate) next_mleader_style_id: u128,
    /// Next linetype id to hand out. Monotonic like the layer allocator: the
    /// builder initialises it from `max(stored) + 1` on the import path (which
    /// has no persisted counters) and from the document's own high-water mark on
    /// a load, it is bumped by every installed row (including an undo restore),
    /// and a delete never rewinds it. That is what makes create → delete →
    /// create hand out a fresh id instead of the deleted one (ADR-0037 D5).
    pub(crate) next_linetype_id: u128,
    /// Next annotation-scale id to hand out (same monotonic rule: initialised by
    /// the builder from `max(stored) + 1`, bumped by every create, never rewound).
    pub(crate) next_scale_id: u128,
    /// External-reference metadata/provenance, keyed by the xref block name.
    ///
    /// Import-only, like every other table: it is populated by
    /// [`crate::DrawingDatabaseBuilder::insert_external_reference`] and never
    /// mutated by a transaction after [`crate::DrawingDatabaseBuilder::finish`].
    /// Empty for hand-built databases and drawings without xrefs.
    pub(crate) external_references: BTreeMap<String, ExternalReference>,
    /// Document-owned base-colour texture bytes, keyed by the logical key a
    /// [`cad_domain::Mesh::texture`] names (ADR-0038 D2/D3).
    ///
    /// The bytes are **document state**, not a file reference: they are written
    /// into the `.ymodel` as `textures/<key>` entries, so saving, reopening or
    /// switching carriers never depends on a source image still existing. The key
    /// is a plain `String` because `cad-db` must not depend on `cad-resources`
    /// (the layering forbids that edge); its form is
    /// [`cad_domain::is_valid_texture_key`]'s contract, enforced by
    /// [`crate::DrawingDatabaseBuilder::insert_texture`].
    ///
    /// Populated by the builder only (import and document load); there is no
    /// post-import mutation path.
    pub(crate) textures: BTreeMap<String, Arc<[u8]>>,
    /// Document-bundled external resource bytes (fonts, xref sources and
    /// `IMAGE` rasters), keyed by `(kind, key)` (ADR-0040 D1/D5).
    ///
    /// Same contract as [`Self::textures`], one level up: the bytes are document
    /// state, written into the `.ymodel` as `resources/<kind>/<key>` entries, and
    /// the key form is [`cad_domain::is_valid_bundled_key`]'s, enforced by
    /// [`crate::DrawingDatabaseBuilder::insert_resource`]. The value type is
    /// `Arc<[u8]>` so one save shares buffers instead of copying megabytes.
    pub(crate) resources: BTreeMap<(BundledKind, String), Arc<[u8]>>,
    /// The logical name each bundled resource is referenced by (ADR-0040 D4):
    /// the spelling that appears in `Style.resource_keys`/`Text.font`, in
    /// `cad:Image.file`, or as an `ExternalReference.name`.
    ///
    /// Kept beside [`Self::resources`] rather than in the key so a rename of the
    /// logical reference cannot silently re-point the bytes: `(kind, name)` is
    /// unique, and the name is what `tables/resources.json` maps back to a key.
    pub(crate) resource_names: BTreeMap<(BundledKind, String), String>,
    /// The collection-time source path of each bundled resource, for diagnostics
    /// only (ADR-0040 D4: never used for runtime resolution).
    ///
    /// `Some(path)` records where the bytes came from, `None` means the writer
    /// knew no path. Absent keys are equivalent to `None`.
    pub(crate) resource_sources: BTreeMap<(BundledKind, String), Option<String>>,
    /// Whether this document embeds external resources in its own archive
    /// (ADR-0040 D3, one toggle).
    ///
    /// Document state, not a carrier preference: it is written into
    /// `document.json` and seeded back on load, so a re-save keeps the user's
    /// choice. Defaults to `false` (opt-in) and is not written while `false`.
    pub(crate) embed_resources: bool,
    /// Per-element BIM data (ADR-0042 D2), kept **beside** [`Self::entities`]
    /// exactly like [`Self::render_attributes`], so [`crate::DbEntity`] keeps
    /// its six fields and a non-BIM document pays nothing.
    ///
    /// An absent entry means "this element carries no BIM data", which is what
    /// the stored row says (the `bim` key is omitted). Written as the optional
    /// `bim` block of the element's shard row.
    pub(crate) bim_elements: BTreeMap<EntityId, BimElement>,
    /// Shared BIM definition rows (`tables/bim-definitions.json`, ADR-0042 D3).
    ///
    /// Empty for every document without BIM data; the writer then emits no
    /// table at all, which keeps such a document at its resources-derived minor.
    pub(crate) bim_definitions: BimDefinitions,
    /// BIM relation edges (`tables/bim-relations.json`, ADR-0042 D3 tail/D12).
    ///
    /// Empty for every document without edges; the writer then emits no table,
    /// so a BIM document without relations stays on the same minor tier as one
    /// with them only because `has_bim` already names that release (minor 7),
    /// not because this table is written unconditionally.
    pub(crate) bim_relations: BimRelations,
    /// Document parameter rows (`tables/parameters.json`, ADR-0043 D4).
    ///
    /// Empty for every document without parameters; the writer then emits no
    /// table, so a document without parametric data keeps the minor tier its
    /// resources/BIM content earns. Identity is the table-local
    /// [`cad_domain::param::ParamSeq`], never a `PersistentId` (D4/D12), so this
    /// table never moves a header counter.
    pub(crate) parameters: cad_domain::param::ParamTable,
    /// Named parameter configurations (`tables/configurations.json`,
    /// ADR-0043 P1). Empty for every document without configurations; the
    /// writer then emits no table and the document keeps its lower minor tier.
    ///
    /// The base [`Self::parameters`] table stays the authority: a configuration
    /// is a sparse overlay resolved at read time, never written back.
    pub(crate) param_configs: cad_domain::param::ParamConfigTable,
    /// The configuration currently applied to derivations, or `None`.
    ///
    /// **Runtime only**: like [`Self::param_diagnostics`], this is not a
    /// `DocumentSnapshot` field, so it is never saved. Selecting a configuration
    /// re-derives the bound geometry but does not change persisted data.
    pub(crate) active_configuration: Option<String>,
    /// Per-element slot bindings (ADR-0043 D3/D7), kept **beside**
    /// [`Self::entities`] exactly like [`Self::bim_elements`]: the entity row
    /// keeps its six fields and a document without bindings pays nothing.
    ///
    /// An absent entry means "this element has no `params` block". The bound
    /// geometry itself is the element's last successful evaluation (D7); no
    /// derived value is stored here.
    pub(crate) param_bindings: BTreeMap<EntityId, cad_domain::param::ParamBindings>,
    /// Load-time reconciliation diagnostics (ADR-0043 D7/D6).
    ///
    /// **Never persisted**: the snapshot builder names its fields explicitly, so
    /// this list cannot reach `DocumentSnapshot`. It describes what the last
    /// `from_snapshot` derivation found — `param.recomputed` when the stored
    /// geometry disagreed with the table and was replaced, `param.blocked` when a
    /// slot failed and the stored geometry was kept (D6 second tier).
    pub(crate) param_diagnostics: Vec<ParamDiagnostic>,
    /// Parametric feature definitions (`tables/features.json`, ADR-0043 P2).
    ///
    /// Empty for every document without features; the writer then emits no
    /// table. Each row is attached to an existing element (see
    /// [`crate::feature`] for the documented identity deviation).
    pub(crate) features: crate::feature::FeatureDefinitions,
    /// Self-authored in-context relations (`tables/relations.json`, ADR-0043
    /// P3). Empty for every document without them.
    pub(crate) relations: crate::relation::InContextRelations,
    /// Cache of [`Self::bounds`] tagged with the [`Revision`] it was computed
    /// at. Interior mutability is required because `bounds` takes `&self` (the
    /// scene-preparation worker shares one `Arc<DrawingDatabase>` across
    /// threads), and the cache is shared by clones. Correctness is guaranteed by
    /// the revision tag, never by freshness of the shared cell: a cache entry is
    /// only served when its tag equals the current revision.
    pub(crate) bounds_cache: Arc<RwLock<BoundsCache>>,
}
