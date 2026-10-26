//! Read-only queries over [`DrawingDatabase`] and the display-state snapshots.

use cad_domain::*;

use super::DrawingDatabase;
use crate::dim_style::DimStyle;
use crate::entity::{DbEntity, EntityRenderAttributes};
use crate::mleader_style::{MLeaderStyle, MLeaderStyleResolution};
use crate::tables::{
    ActiveAnnotationScale, BlockDefinition, DynamicBlockVisibility, Layer, LayerAttributes, Layout,
    LineType, PlotMargins, PlotPaperUnits, PlotProvenance, PlotRotation, PlotSettingsRecord,
    PlotType, Scale, Style,
};

/// Dynamic-visibility membership of one entity, snapshotted before a delete.
///
/// The delete path prunes these lists (see [`DrawingDatabase::remove_entity`]),
/// so an undo must capture and restore them or a deleted block member would lose
/// its visibility-state membership.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VisibilityMembership {
    /// Blocks whose plain `entities` member list contained the entity.
    pub blocks: Vec<BlockId>,
    /// Dynamic-visibility placements, one per block that governed the entity.
    pub visibility: Vec<VisibilityStateMembership>,
}

/// One block's dynamic-visibility placement of an entity before a delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibilityStateMembership {
    pub block: BlockId,
    /// Whether the entity was in the block's governed `member_entities`.
    pub member: bool,
    /// Names of the visibility states that contained the entity.
    pub states: Vec<String>,
}

/// The full display state of an entity, snapshotted before a delete so an undo
/// can restore it exactly (render attributes + dynamic-visibility membership).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EntityDisplayState {
    pub attributes: EntityRenderAttributes,
    pub visibility: VisibilityMembership,
}

impl DrawingDatabase {
    pub fn id(&self) -> DatabaseId {
        self.id
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// The source drawing's declared text code page, if it declared one.
    ///
    /// `None` means the drawing did not declare a code page; callers must treat
    /// the text locale as unknown (the platform maps it to its Latin default),
    /// never as an implied Chinese/Japanese/Korean page.
    pub fn code_page(&self) -> Option<&str> {
        self.code_page.as_deref()
    }

    /// Content identity used to decide whether a render/scene cache is stale.
    ///
    /// A bare `DatabaseId` is not enough: hosts may reuse the same id for a
    /// different drawing, which would leave old GPU batches on screen (audit
    /// B04). The identity therefore mixes the id, revision, structural counts
    /// and model-space bounds.
    pub fn scene_identity(&self) -> SceneIdentity {
        let (min, max) = self
            .bounds()
            .unwrap_or((Point3::default(), Point3::default()));
        SceneIdentity {
            database: self.id,
            revision: self.revision,
            entities: self.entities.len() as u64,
            layers: self.layers.len() as u64,
            bounds: [min, max],
        }
    }

    pub fn entity(&self, id: EntityId) -> Option<&DbEntity> {
        self.entities.get(&id)
    }

    /// Display attributes the importer resolved for an entity.
    ///
    /// Missing entries mean "fully opaque analytic geometry", so a database
    /// built without an importer (tests, hand-built fixtures) keeps working.
    pub fn entity_render_attributes(&self, id: EntityId) -> EntityRenderAttributes {
        self.render_attributes.get(&id).cloned().unwrap_or_default()
    }

    /// Dynamic-visibility membership of `id`, captured before a delete so a
    /// later undo can restore it.
    pub fn entity_visibility_membership(&self, id: EntityId) -> VisibilityMembership {
        let mut membership = VisibilityMembership::default();
        for definition in self.blocks.values() {
            if definition.entities.contains(&id) {
                membership.blocks.push(definition.id);
            }
            if let Some(visibility) = definition.dynamic_visibility.as_ref() {
                let member = visibility.member_entities.contains(&id);
                let states: Vec<String> = visibility
                    .states
                    .iter()
                    .filter(|state| state.entities.contains(&id))
                    .map(|state| state.name.clone())
                    .collect();
                if member || !states.is_empty() {
                    membership.visibility.push(VisibilityStateMembership {
                        block: definition.id,
                        member,
                        states,
                    });
                }
            }
        }
        membership
    }

    /// Capture the full display state (render attributes + dynamic-visibility
    /// membership) of `id`.
    ///
    /// An undo of a delete re-inserts the [`DbEntity`] but the delete pruned
    /// these side structures, so the caller must snapshot and restore them.
    pub fn entity_display_state(&self, id: EntityId) -> EntityDisplayState {
        EntityDisplayState {
            attributes: self.entity_render_attributes(id),
            visibility: self.entity_visibility_membership(id),
        }
    }

    /// Restore a display state captured by [`Self::entity_display_state`].
    ///
    /// Infallible: it writes the same map/vector entries the delete removed, so
    /// a re-inserted entity again carries its resolved colour/linetype/lineweight
    /// and its dynamic-visibility membership.
    pub fn restore_entity_display_state(&mut self, id: EntityId, state: &EntityDisplayState) {
        // This path changes block membership (and therefore bounds) without
        // raising the revision, so it must clear the revision-keyed cache.
        self.invalidate_bounds_cache();
        self.render_attributes.insert(id, state.attributes.clone());
        for block in &state.visibility.blocks {
            if let Some(definition) = self.blocks.get_mut(block) {
                if !definition.entities.contains(&id) {
                    definition.entities.push(id);
                }
            }
        }
        for placement in &state.visibility.visibility {
            if let Some(definition) = self.blocks.get_mut(&placement.block) {
                if let Some(visibility) = definition.dynamic_visibility.as_mut() {
                    if placement.member && !visibility.member_entities.contains(&id) {
                        visibility.member_entities.push(id);
                    }
                    for state_entry in &mut visibility.states {
                        if placement.states.contains(&state_entry.name)
                            && !state_entry.entities.contains(&id)
                        {
                            state_entry.entities.push(id);
                        }
                    }
                }
            }
        }
    }

    pub fn entities(&self) -> impl Iterator<Item = &DbEntity> {
        self.entities.values()
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.get(&id)
    }

    pub fn layers(&self) -> impl Iterator<Item = &Layer> {
        self.layers.values()
    }

    /// Resolved attributes of a layer table row.
    ///
    /// Missing entries resolve to [`LayerAttributes::default`] — on, thawed,
    /// unlocked, plottable, ACI 7 white, `Continuous`, ByLayer weight, opaque —
    /// so a hand-built database (tests, fixtures, imported drawings that carried
    /// no layer attributes) keeps working without a migration. Mirroring
    /// [`Self::entity_render_attributes`], the value is cloned rather than
    /// borrowed so callers do not hold a borrow of the database.
    pub fn layer_attributes(&self, id: LayerId) -> LayerAttributes {
        self.layer_attributes.get(&id).cloned().unwrap_or_default()
    }

    /// Borrowing variant of [`Self::layer_attributes`], for callers that only
    /// need to read one field (the exporter loops the whole table).
    ///
    /// `None` when this database has no recorded attributes for `id`; the caller
    /// then uses `LayerAttributes::default()` semantics explicitly rather than
    /// receiving a silently fabricated row.
    pub fn layer_attributes_ref(&self, id: LayerId) -> Option<&LayerAttributes> {
        self.layer_attributes.get(&id)
    }

    pub fn block(&self, id: BlockId) -> Option<&BlockDefinition> {
        self.blocks.get(&id)
    }

    /// The dynamic-block visibility descriptor of a block, if it has one.
    pub fn block_dynamic_visibility(&self, id: BlockId) -> Option<&DynamicBlockVisibility> {
        self.blocks.get(&id)?.dynamic_visibility.as_ref()
    }

    /// Member entities the block's active visibility state makes visible.
    ///
    /// `None` when the block carries no visibility descriptor. When a
    /// descriptor exists but its active state is unknown, this returns the full
    /// governed set so an unresolved state never silently hides geometry.
    pub fn block_visible_entities(&self, id: BlockId) -> Option<Vec<EntityId>> {
        let block = self.blocks.get(&id)?;
        let visibility = block.dynamic_visibility.as_ref()?;
        let visible: Vec<EntityId> = block
            .entities
            .iter()
            .copied()
            .filter(|e| visibility.is_visible(*e))
            .collect();
        Some(visible)
    }

    pub fn blocks(&self) -> impl Iterator<Item = &BlockDefinition> {
        self.blocks.values()
    }

    pub fn layout(&self, id: LayoutId) -> Option<&Layout> {
        self.layouts.get(&id)
    }

    pub fn layouts(&self) -> impl Iterator<Item = &Layout> {
        self.layouts.values()
    }

    /// Plot settings stored for a layout, exactly as read.
    ///
    /// `None` means the drawing carried no PLOTSETTINGS data for that layout;
    /// callers that need to render must use [`Self::plot_settings_for`], which
    /// makes the fallback explicit instead of inventing values.
    pub fn plot_settings(&self, id: LayoutId) -> Option<&PlotSettingsRecord> {
        self.plot_settings.get(&id)
    }

    /// Plot settings for a layout, or an explicit documented default page.
    ///
    /// The default is the ISO A4 sheet (210 × 297 mm) with zero margins and no
    /// rotation, marked [`PlotProvenance::DefaultPage`]. A4 is an ISO standard,
    /// not a vendor value; it is chosen as a neutral canvas so a layout that
    /// carries no plot data is still plottable, and the provenance makes clear
    /// that nothing was read from the file.
    pub fn plot_settings_for(&self, id: LayoutId) -> PlotSettingsRecord {
        if let Some(record) = self.plot_settings.get(&id) {
            return record.clone();
        }
        PlotSettingsRecord {
            layout: id,
            paper_size_name: "ISO_A4_(210.00_x_297.00_MM)".to_string(),
            paper_width: 210.0,
            paper_height: 297.0,
            margins: PlotMargins::default(),
            rotation: PlotRotation::None,
            scale_numerator: 1.0,
            scale_denominator: 1.0,
            plot_type: PlotType::Layout,
            paper_units: PlotPaperUnits::Millimeters,
            provenance: PlotProvenance::DefaultPage {
                reason: "layout has no PLOTSETTINGS object or embedded plot data; \
                         using the ISO A4 (210x297 mm) default page"
                    .to_string(),
            },
        }
    }

    pub fn style(&self, id: StyleId) -> Option<&Style> {
        self.styles.get(&id)
    }

    /// Named linetype table entries.
    pub fn linetypes(&self) -> impl Iterator<Item = &LineType> {
        self.linetypes.values()
    }

    /// A linetype table entry by identity.
    pub fn linetype(&self, id: LinetypeId) -> Option<&LineType> {
        self.linetypes.get(&id)
    }

    /// Drawing-global linetype scale (`$LTSCALE`). Defaults to `1.0`.
    pub fn linetype_scale(&self) -> f64 {
        self.linetype_scale
    }

    /// Drawing-global POINT display mode (`$PDMODE`), raw as stored. `0` (dot)
    /// is the DWG default.
    ///
    /// This is a **display** value only: it changes how the `cad-app` overlay
    /// marker is drawn, not the point's geometry, and there is no `SETVAR`
    /// equivalent, so it can only arrive from the source drawing.
    pub fn point_display_mode(&self) -> i16 {
        self.point_display_mode
    }

    /// Drawing-global POINT display size (`$PDSIZE`), raw as stored. `0.0` is
    /// the DWG default ("viewport-relative"); the sign is interpreted by the
    /// display layer, which also sanitises a non-finite/absurd value.
    pub fn point_display_size(&self) -> f64 {
        self.point_display_size
    }

    /// Named annotation scales from the drawing's `ACAD_SCALELIST`.
    pub fn scales(&self) -> impl Iterator<Item = &Scale> {
        self.scales.values()
    }

    /// A named annotation scale by identity.
    pub fn scale(&self, id: ScaleId) -> Option<&Scale> {
        self.scales.get(&id)
    }

    /// A named annotation scale by name, matched exactly then case-insensitively.
    ///
    /// Drawing scale names are case-sensitive in principle but vendor exports
    /// vary; the case-insensitive pass is the documented fallback and never
    /// invents a factor.
    pub fn scale_by_name(&self, name: &str) -> Option<&Scale> {
        if let Some(scale) = self.scales.values().find(|s| s.name == name) {
            return Some(scale);
        }
        self.scales
            .values()
            .find(|s| s.name.eq_ignore_ascii_case(name.trim()))
    }

    /// The raw active annotation scale as set by the importer, when present.
    pub fn active_annotation_scale(&self) -> Option<&ActiveAnnotationScale> {
        self.active_annotation_scale.as_ref()
    }

    /// DIMSTYLE table entries (ADR 0017 B.1).
    pub fn dim_styles(&self) -> impl Iterator<Item = &DimStyle> {
        self.dim_styles.values()
    }

    /// A DIMSTYLE table entry by identity.
    pub fn dim_style(&self, id: DimStyleId) -> Option<&DimStyle> {
        self.dim_styles.get(&id)
    }

    /// A DIMSTYLE table entry by name, matched exactly then case-insensitively.
    ///
    /// The name index resolves the common case; the case-insensitive fallback
    /// mirrors [`Self::scale_by_name`] and never invents a style.
    pub fn dim_style_by_name(&self, name: &str) -> Option<&DimStyle> {
        if let Some(id) = self.dim_style_ids_by_name.get(name) {
            return self.dim_styles.get(id);
        }
        self.dim_styles
            .values()
            .find(|style| style.name.eq_ignore_ascii_case(name.trim()))
    }

    /// Resolve a named DIMSTYLE into the entity-level snapshot the creation path
    /// (P5c) uses, applying `DIMSCALE` and resolving the text-style identity
    /// from the STYLE table.
    ///
    /// The text style is resolved by name (exact then case-insensitive); its
    /// font key is the first resource key. When the text style is absent the
    /// snapshot falls back to `StyleId(0)` with no font rather than failing —
    /// the measurement text is still drawn (its font is a host-gated `Partial`,
    /// ADR 0017 D.2).
    pub fn resolve_dim_style(&self, name: &str) -> Option<DimensionStyleValues> {
        let style = self.dim_style_by_name(name)?;
        let text_style_name = style.text_style_name.clone();
        let resolved = self.style_by_name(&text_style_name);
        let text_style = resolved.map(|s| s.id).unwrap_or(StyleId(0));
        let text_font = resolved.and_then(|s| s.resource_keys.first().cloned());
        Some(style.resolved(text_style_name, text_style, text_font))
    }

    /// MLEADERSTYLE table entries (ADR 0019 decision D, P7c).
    pub fn mleader_styles(&self) -> impl Iterator<Item = &MLeaderStyle> {
        self.mleader_styles.values()
    }

    /// A MLEADERSTYLE table entry by identity.
    pub fn mleader_style(&self, id: MLeaderStyleId) -> Option<&MLeaderStyle> {
        self.mleader_styles.get(&id)
    }

    /// A MLEADERSTYLE table entry by name, matched exactly then
    /// case-insensitively (the [`Self::dim_style_by_name`] rule).
    pub fn mleader_style_by_name(&self, name: &str) -> Option<&MLeaderStyle> {
        if let Some(id) = self.mleader_style_ids_by_name.get(name) {
            return self.mleader_styles.get(id);
        }
        self.mleader_styles
            .values()
            .find(|style| style.name.trim() == name.trim())
            .or_else(|| {
                self.mleader_styles
                    .values()
                    .find(|style| style.name.eq_ignore_ascii_case(name.trim()))
            })
    }

    /// Resolve a named MLEADERSTYLE into the creation snapshot the `MLEADER`
    /// command stores (ADR 0019 decision D/F.2, P7c).
    ///
    /// The text style is resolved by name (exact then case-insensitive); its
    /// font key is the first resource key. When the text style is absent the
    /// snapshot falls back to `StyleId(0)` with no font, mirroring
    /// [`Self::resolve_dim_style`] — the leader text is still drawn.
    pub fn resolve_mleader_style(&self, name: &str) -> Option<MLeaderStyleResolution> {
        let style = self.mleader_style_by_name(name)?;
        let text_style_name = style.text_style_name.clone();
        let resolved = self.style_by_name(&text_style_name);
        let text_style = resolved.map(|s| s.id).unwrap_or(StyleId(0));
        let text_font = resolved.and_then(|s| s.resource_keys.first().cloned());
        Some(style.resolved(text_style_name, text_style, text_font))
    }

    /// A text style table entry by name, matched exactly then case-insensitively.
    pub fn style_by_name(&self, name: &str) -> Option<&Style> {
        if let Some(style) = self.styles.values().find(|s| s.name == name) {
            return Some(style);
        }
        self.styles
            .values()
            .find(|s| s.name.eq_ignore_ascii_case(name.trim()))
    }

    /// The active annotation scale name, when one is set.
    pub fn annotation_scale_name(&self) -> Option<&str> {
        self.active_annotation_scale
            .as_ref()
            .map(|s| s.name.as_str())
            .filter(|name| !name.is_empty())
    }

    /// The drawing's active annotation scale, resolved for rendering.
    ///
    /// Resolution order:
    /// 1. A named [`Scale`] in the table matching the active name — the table is
    ///    authoritative and its [`Scale::factor`] is used.
    /// 2. The header `CANNOSCALEVALUE`, when finite and positive.
    /// 3. `1.0` (no scaling), the safe fallback for a database with no scale
    ///    information at all.
    ///
    /// Returns `(name, factor, resolved)`. `resolved` is `false` only when the
    /// name was set but neither the table nor a valid header value could supply
    /// a factor; callers that render annotative entities must report that as an
    /// explicit `Partial` rather than silently scaling by `1.0`.
    pub fn annotation_scale(&self) -> (Option<String>, f64, bool) {
        let Some(active) = &self.active_annotation_scale else {
            return (None, 1.0, true);
        };
        let name = (!active.name.is_empty()).then(|| active.name.clone());
        if let Some(scale) = self.scale_by_name(&active.name) {
            return (name, scale.factor(), true);
        }
        if active.value.is_finite() && active.value > 0.0 {
            return (name, active.value, false);
        }
        (name, 1.0, false)
    }

    /// Entities in model space, in draw order.
    pub fn model_space(&self) -> Vec<&DbEntity> {
        let mut v: Vec<&DbEntity> = self
            .entities
            .values()
            .filter(|e| matches!(e.space, SpaceId::Model))
            .collect();
        v.sort_by_key(|e| e.draw_order);
        v
    }

    /// Entities of a block definition, in draw order.
    ///
    /// For a dynamic block with a resolved active visibility state, only the
    /// entities that state makes visible are returned (spec §3.2); ungoverned
    /// entities always remain. Without a descriptor (or with an unresolved
    /// active state) every member is returned, so a guess can never hide
    /// geometry.
    pub fn block_entities(&self, id: BlockId) -> Vec<&DbEntity> {
        let Some(block) = self.blocks.get(&id) else {
            return Vec::new();
        };
        let mut v: Vec<&DbEntity> = block
            .entities
            .iter()
            .filter(|e| {
                block
                    .dynamic_visibility
                    .as_ref()
                    .map(|vis| vis.is_visible(**e))
                    .unwrap_or(true)
            })
            .filter_map(|e| self.entities.get(e))
            .collect();
        v.sort_by_key(|e| e.draw_order);
        v
    }

    /// Entities belonging to a layout, in draw order.
    pub fn layout_entities(&self, id: LayoutId) -> Vec<&DbEntity> {
        let mut v: Vec<&DbEntity> = self
            .entities
            .values()
            .filter(|e| matches!(&e.space, SpaceId::Paper(l) if *l == id))
            .collect();
        v.sort_by_key(|e| e.draw_order);
        v
    }
}
