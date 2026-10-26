//! Layer projection and temporary visibility overrides (spec F03).
//!
//! The drawing database is the only authority for the layer table: `Layer` rows
//! (id, name, `visible`) come straight from [`DrawingDatabase::layers`] and are
//! never rewritten here. What this module adds is a **session-scoped override
//! set**: a temporary hide/show the user applies while viewing. It is stored in
//! [`LayerOverrideSet`], consulted when deciding whether an entity is drawn, and
//! cleared by `RestoreLayers`. It never mutates the DWG layer table, never
//! raises the database revision and therefore never forces a re-parse
//! (spec §4.8, audit F03).
//!
//! Override semantics are deliberately explicit:
//!
//! - no override for a layer → the database `visible` flag decides;
//! - override `Some(true)`  → shown even if the database layer says hidden;
//! - override `Some(false)` → hidden even if the database layer says visible.
//!
//! Each row also carries the layer's stored [`cad_db::LayerAttributes`], so the
//! layer panel and the attribute write path (`SETLAYERPROPERTY`) read the same
//! state. Carrying them changes no behaviour: only `off`/`frozen` are reflected
//! in the `visible` flag, and that flag is the database's, not this module's.

use std::collections::BTreeMap;

use cad_db::{DbEntity, DrawingDatabase, Layer, LayerAttributes};
use cad_domain::{CadResult, LayerId, Revision};
use cad_query::{QueryPage, QueryRequest, QueryService};

/// A read-only projection of one layer row for the layer panel.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerRow {
    pub id: LayerId,
    pub name: String,
    /// Visibility stored in the DWG layer table. Never modified by the UI.
    pub database_visible: bool,
    /// The temporary override, when the session set one. `None` means the
    /// database flag is authoritative for this layer.
    pub override_visible: Option<bool>,
    /// The visibility the scene should honour right now.
    pub effective_visible: bool,
    /// The row's stored attributes (colour, linetype, lineweight, transparency,
    /// off/frozen/locked/plottable), as [`DrawingDatabase::layer_attributes`]
    /// answers them.
    ///
    /// Carried on the row so the panel's projection and a property write address
    /// the **same** stored state: the wire layer (`CadLayer`) formats these
    /// values with [`crate::layer_property`]'s canonical projections, so a value
    /// a row shows can be sent straight back to `SETLAYERPROPERTY`. This is a
    /// read-only snapshot — `locked`, `plottable`, the colour and the line
    /// properties have **no behaviour** yet (see the module docs of
    /// `cad_db::LayerAttributes`).
    pub attributes: LayerAttributes,
}

impl LayerRow {
    pub fn is_overridden(&self) -> bool {
        self.override_visible.is_some()
    }
}

/// Session-scoped, temporary layer visibility. Never writes the drawing DB.
///
/// This is a plain value type so it can be cloned into the renderer/scene
/// request path and compared to detect changes without touching the database.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayerOverrideSet {
    overrides: BTreeMap<LayerId, bool>,
}

impl LayerOverrideSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set an explicit temporary visibility for `layer`.
    pub fn set(&mut self, layer: LayerId, visible: bool) {
        self.overrides.insert(layer, visible);
    }

    /// Drop the override for one layer, restoring the database flag.
    pub fn remove(&mut self, layer: LayerId) -> Option<bool> {
        self.overrides.remove(&layer)
    }

    /// Drop every override (the `RestoreLayers` behaviour).
    pub fn clear(&mut self) {
        self.overrides.clear();
    }

    /// Drop every override, reporting whether anything changed.
    pub fn clear_changed(&mut self) -> bool {
        let changed = !self.overrides.is_empty();
        self.overrides.clear();
        changed
    }

    pub fn get(&self, layer: LayerId) -> Option<bool> {
        self.overrides.get(&layer).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }

    pub fn len(&self) -> usize {
        self.overrides.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (LayerId, bool)> + '_ {
        self.overrides.iter().map(|(id, v)| (*id, *v))
    }

    /// Effective visibility of a layer given the database's own flag.
    pub fn effective(&self, layer: LayerId, database_visible: bool) -> bool {
        self.get(layer).unwrap_or(database_visible)
    }

    /// Whether a layer should be drawn under this override set.
    ///
    /// The database flag of `layer` plus the temporary override. Callers that
    /// resolved `layer` through [`cad_db::LayerScope`] — a block member on layer
    /// `0` follows the containing INSERT's layer — pass the **effective** layer
    /// here, so the visibility answer and the `ByLayer` style resolution always
    /// come from one resolution.
    pub fn is_layer_visible(&self, database: &DrawingDatabase, layer: LayerId) -> bool {
        let base = database.layer(layer).map(|l| l.visible).unwrap_or(true);
        self.effective(layer, base)
    }

    /// Whether an entity should be drawn under this override set, judged by the
    /// layer it is **stored** on (`DbEntity::layer`).
    ///
    /// This is the rule for a model-space (top-level) entity, whose stored layer
    /// is its effective layer. An entity inside a block definition is judged on
    /// its effective layer instead — see
    /// [`crate::picking::ref_visibility`], the one walk that resolves the
    /// instance path's `LayerScope` chain and then calls
    /// [`Self::is_layer_visible`].
    pub fn is_entity_visible(&self, database: &DrawingDatabase, entity: &DbEntity) -> bool {
        self.is_layer_visible(database, entity.layer)
    }

    /// A stable, comparable fingerprint of the override state.
    ///
    /// A render bridge stores this so a pure visibility change triggers a scene
    /// rebuild while an unchanged snapshot does not (spec §4.8 incremental
    /// update; audit F03 "显隐需不重新生成全部几何").
    pub fn fingerprint(&self) -> u64 {
        // FNV-1a over the ordered (BTreeMap) layer ids and flags. Stable across
        // runs; collisions are not security-relevant, only used for change
        // detection.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for (id, visible) in self.iter() {
            for byte in id.0.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
            hash ^= u64::from(visible);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }
}

/// Build the full layer-panel projection for a drawing.
///
/// The order is the database's own `layers()` iteration order (a `BTreeMap`,
/// so it is deterministic by `LayerId`). No synthetic rows are invented; an
/// empty database yields an empty list, which the UI shows as an empty state.
pub fn layer_rows(database: &DrawingDatabase, overrides: &LayerOverrideSet) -> Vec<LayerRow> {
    database
        .layers()
        .map(|layer| LayerRow {
            id: layer.id,
            name: layer.name.clone(),
            database_visible: layer.visible,
            override_visible: overrides.get(layer.id),
            effective_visible: overrides.effective(layer.id, layer.visible),
            attributes: database.layer_attributes(layer.id),
        })
        .collect()
}

/// Case-insensitive substring filter over layer names.
///
/// An empty needle returns every row. This backs the panel's search entry
/// (audit F03/U03 "搜索/恢复入口").
pub fn filter_layer_rows(rows: &[LayerRow], needle: &str) -> Vec<LayerRow> {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return rows.to_vec();
    }
    rows.iter()
        .filter(|row| row.name.to_lowercase().contains(&needle))
        .cloned()
        .collect()
}

/// The model-space entities a scene build should include under `overrides`.
///
/// This is the exact, testable decision the render bridge uses; keeping it in
/// `cad-app` means the filtering rule is covered by `cargo test -p cad-app`
/// without the UI host's graphics/font toolchain (fontconfig/GTK). Order
/// follows [`DrawingDatabase::model_space`] (draw order).
pub fn visible_model_entities<'a>(
    database: &'a DrawingDatabase,
    overrides: &LayerOverrideSet,
) -> Vec<&'a DbEntity> {
    database
        .model_space()
        .into_iter()
        .filter(|entity| overrides.is_entity_visible(database, entity))
        .collect()
}

/// Paged layer query through the shared [`QueryService`], bound to a revision.
///
/// This is the revision-aware path a host uses when it wants to discard stale
/// projections after a document switch (spec §4.9). It returns the database's
/// own visibility flag; overrides are layered on top by [`layer_rows`].
pub fn query_layers(
    query: &QueryService,
    database: &DrawingDatabase,
    request: QueryRequest,
) -> CadResult<QueryPage<cad_query::LayerRow>> {
    query.layers(database, request)
}

/// Revision a layer projection is bound to.
pub fn layer_revision(database: &DrawingDatabase) -> Revision {
    database.revision()
}

/// A layer's database row, for callers that need the raw stored flag.
pub fn database_layer(database: &DrawingDatabase, id: LayerId) -> Option<&Layer> {
    database.layer(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
    use cad_domain::{
        BlockId, DatabaseId, DocumentId, EntityId, InstancePath, ObjectId, Point3, Revision,
        SelectionRef, SemanticGeometry, SpaceId, Transform3,
    };

    fn database_with_layers() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_layer(Layer {
            id: LayerId(1),
            name: "WALLS".into(),
            visible: false,
        })
        .unwrap();
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(1),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Line {
                start: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                end: Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
            draw_order: 0,
        })
        .unwrap();
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(2),
                type_key: "AcDbCircle".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(2),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Point(Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }),
            draw_order: 1,
        })
        .unwrap();
        b.finish().unwrap()
    }

    #[test]
    fn rows_come_from_the_database_and_honour_the_stored_flag() {
        let db = database_with_layers();
        let rows = layer_rows(&db, &LayerOverrideSet::new());
        assert_eq!(rows.len(), 2);
        // Layer 0 is stored visible, layer 1 is stored hidden: no invention.
        assert!(rows[0].database_visible);
        assert!(!rows[1].database_visible);
        assert!(rows[0].effective_visible);
        assert!(!rows[1].effective_visible);
        assert!(!rows[1].is_overridden());
    }

    #[test]
    fn override_can_show_a_hidden_layer_without_touching_the_database() {
        let db = database_with_layers();
        let before = db.revision();
        let mut overrides = LayerOverrideSet::new();
        overrides.set(LayerId(1), true);

        let rows = layer_rows(&db, &overrides);
        let walls = rows.iter().find(|r| r.id == LayerId(1)).unwrap();
        // Effective visibility flipped, but the stored flag is untouched.
        assert!(walls.effective_visible);
        assert!(!walls.database_visible);
        assert_eq!(walls.override_visible, Some(true));
        assert_eq!(db.revision(), before);
        assert!(!database_layer(&db, LayerId(1)).unwrap().visible);
    }

    #[test]
    fn override_can_hide_a_visible_layer() {
        let mut overrides = LayerOverrideSet::new();
        overrides.set(LayerId(0), false);
        assert!(!overrides.effective(LayerId(0), true));
        // Clearing restores the database flag.
        overrides.remove(LayerId(0));
        assert!(overrides.effective(LayerId(0), true));
    }

    #[test]
    fn entity_visibility_follows_the_override_set() {
        let db = database_with_layers();
        let hidden_layer_entity = db.entity(EntityId(1)).unwrap();
        let visible_layer_entity = db.entity(EntityId(2)).unwrap();

        // No override: the entity's layer visibility decides.
        let empty = LayerOverrideSet::new();
        assert!(!empty.is_entity_visible(&db, hidden_layer_entity));
        assert!(empty.is_entity_visible(&db, visible_layer_entity));

        // Override shows the hidden-layer entity without touching the DB.
        let mut shown = LayerOverrideSet::new();
        shown.set(LayerId(1), true);
        assert!(shown.is_entity_visible(&db, hidden_layer_entity));

        // Override hides the visible-layer entity; unrelated layers are safe.
        let mut hidden = LayerOverrideSet::new();
        hidden.set(LayerId(0), false);
        assert!(!hidden.is_entity_visible(&db, visible_layer_entity));
        assert!(!hidden.is_entity_visible(&db, hidden_layer_entity));
    }

    #[test]
    fn restore_layers_reports_whether_anything_changed() {
        let mut overrides = LayerOverrideSet::new();
        assert!(!overrides.clear_changed());
        overrides.set(LayerId(3), false);
        assert!(overrides.clear_changed());
        assert!(overrides.is_empty());
        assert!(!overrides.clear_changed());
    }

    #[test]
    fn fingerprint_tracks_override_state_only() {
        let a = LayerOverrideSet::new();
        let b = LayerOverrideSet::new();
        assert_eq!(a.fingerprint(), b.fingerprint());

        let mut hidden = LayerOverrideSet::new();
        hidden.set(LayerId(1), false);
        assert_ne!(a.fingerprint(), hidden.fingerprint());

        // Order of insertion must not affect the fingerprint (BTreeMap).
        let mut one = LayerOverrideSet::new();
        one.set(LayerId(1), false);
        one.set(LayerId(2), true);
        let mut two = LayerOverrideSet::new();
        two.set(LayerId(2), true);
        two.set(LayerId(1), false);
        assert_eq!(one.fingerprint(), two.fingerprint());
    }

    #[test]
    fn search_filters_by_name_case_insensitively() {
        let db = database_with_layers();
        let rows = layer_rows(&db, &LayerOverrideSet::new());
        assert_eq!(filter_layer_rows(&rows, "").len(), 2);
        assert_eq!(filter_layer_rows(&rows, "wall").len(), 1);
        assert_eq!(filter_layer_rows(&rows, "WALLS")[0].id, LayerId(1));
        // A miss is an empty list, never a fabricated row.
        assert!(filter_layer_rows(&rows, "roof").is_empty());
    }

    #[test]
    fn visible_model_entities_filters_hidden_layers() {
        let db = database_with_layers();
        // Entity 1 is on hidden layer 1, entity 2 on visible layer 0.
        let none = LayerOverrideSet::new();
        let visible = visible_model_entities(&db, &none);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, EntityId(2));

        // Override layer 1 visible: both entities are now in the scene.
        let mut shown = LayerOverrideSet::new();
        shown.set(LayerId(1), true);
        assert_eq!(visible_model_entities(&db, &shown).len(), 2);

        // Override layer 0 hidden: nothing remains.
        let mut hidden = LayerOverrideSet::new();
        hidden.set(LayerId(0), false);
        assert!(visible_model_entities(&db, &hidden).is_empty());
    }

    #[test]
    fn paged_query_is_revision_bound_and_matches_rows() {
        let db = database_with_layers();
        let query = QueryService::new();
        let request = QueryRequest {
            document: cad_domain::DocumentId(1),
            revision: Revision(0),
            request: cad_domain::RequestId(1),
            offset: 0,
            limit: 10,
        };
        let page = query_layers(&query, &db, request).unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.rows[0].name, "0");
        assert_eq!(page.request.revision, layer_revision(&db));
    }

    // ---- Layer `0` inside a block: the effective-layer rule ----

    /// A line `id` on `layer` inside `space`.
    fn line_on(id: u128, space: SpaceId, layer: LayerId) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer,
            space,
            geometry: SemanticGeometry::Line {
                start: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                end: Point3 {
                    x: 10.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
            draw_order: 0,
        }
    }

    /// Layers `0`(0)/BLUE(2, on)/RED(3, off) and block 9 with two members: 30 on
    /// layer `0` and 31 on RED. Two model INSERTs of that block: entity 2 on
    /// BLUE, entity 3 on RED.
    fn database_with_a_layer_zero_block_member() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        for (id, name, visible) in [
            (LayerId(0), "0", true),
            (LayerId(2), "BLUE", true),
            (LayerId(3), "RED", false),
        ] {
            b.insert_layer(Layer {
                id,
                name: name.into(),
                visible,
            })
            .unwrap();
        }
        b.insert_block(cad_db::BlockDefinition {
            id: BlockId(9),
            entities: vec![EntityId(30), EntityId(31)],
            dynamic_visibility: None,
        })
        .unwrap();
        b.insert_entity(line_on(30, SpaceId::Block(BlockId(9)), LayerId(0)))
            .unwrap();
        b.insert_entity(line_on(31, SpaceId::Block(BlockId(9)), LayerId(3)))
            .unwrap();
        for (id, layer) in [(2u128, LayerId(2)), (3, LayerId(3))] {
            let mut insert = line_on(id, SpaceId::Model, layer);
            insert.geometry = SemanticGeometry::Insert {
                block: BlockId(9),
                transform: Transform3::identity(),
            };
            b.insert_entity(insert).unwrap();
        }
        b.finish().unwrap()
    }

    fn block_member_ref(insert: u128, member: u128) -> SelectionRef {
        SelectionRef {
            document: DocumentId(1),
            entity: EntityId(member),
            instance: InstancePath(vec![EntityId(insert)]),
            sub_element: None,
        }
    }

    #[test]
    fn a_layer_zero_block_member_follows_the_insert_layer_for_visibility() {
        // The one visibility answer: a member stored on layer `0` inside an INSERT
        // is drawn/selected under the INSERT's layer, so hiding that layer hides
        // the member even though layer `0` itself is on.
        let db = database_with_a_layer_zero_block_member();
        let root = db.root_layer_scope();
        let member = block_member_ref(2, 30);
        assert_eq!(
            crate::picking::ref_visibility(&db, &LayerOverrideSet::new(), root, &member),
            Some(true),
            "BLUE (the INSERT's layer) and layer 0 are both on"
        );
        assert_eq!(
            crate::picking::ref_effective_layer(&db, root, &member),
            Some(LayerId(2)),
            "the member belongs to the INSERT's layer, not to layer 0"
        );

        let mut hide_insert_layer = LayerOverrideSet::new();
        hide_insert_layer.set(LayerId(2), false);
        assert_eq!(
            crate::picking::ref_visibility(&db, &hide_insert_layer, root, &member),
            Some(false),
            "hiding the INSERT's layer hides its layer-0 member"
        );
        assert!(
            hide_insert_layer.is_entity_visible(&db, db.entity(EntityId(30)).unwrap()),
            "the stored-layer predicate alone still says 'on': only the shared \
             effective-layer walk follows the INSERT"
        );

        // A member on a non-zero layer keeps its own layer: RED is off, so it is
        // hidden even though its INSERT (BLUE) is visible.
        assert_eq!(
            crate::picking::ref_visibility(
                &db,
                &LayerOverrideSet::new(),
                root,
                &block_member_ref(2, 31)
            ),
            Some(false)
        );
    }

    #[test]
    fn a_hidden_layer_zero_does_not_hide_a_member_of_a_visible_insert() {
        // The mirror case: layer `0` itself off. The member is drawn on BLUE, so it
        // stays visible — again only through the shared walk.
        let db = database_with_a_layer_zero_block_member();
        let root = db.root_layer_scope();
        let mut hide_zero = LayerOverrideSet::new();
        hide_zero.set(LayerId(0), false);
        let member = block_member_ref(2, 30);
        assert!(!hide_zero.is_entity_visible(&db, db.entity(EntityId(30)).unwrap()));
        assert_eq!(
            crate::picking::ref_visibility(&db, &hide_zero, root, &member),
            Some(true),
            "the layer-0 row being off must not hide a member drawn on BLUE"
        );
        // A model-space entity stored on layer `0` has no INSERT to inherit from,
        // so the same override does hide it.
        let mut solo = block_member_ref(2, 30);
        solo.instance = InstancePath::default();
        assert_eq!(
            crate::picking::ref_visibility(&db, &hide_zero, root, &solo),
            Some(false),
            "no containing INSERT means no inheritance"
        );
    }

    #[test]
    fn an_unresolvable_instance_step_reports_no_visibility_answer() {
        // Never fabricate `true`/`false` for a ref whose path does not resolve.
        let db = database_with_a_layer_zero_block_member();
        let reference = SelectionRef {
            document: DocumentId(1),
            entity: EntityId(30),
            instance: InstancePath(vec![EntityId(999)]),
            sub_element: None,
        };
        assert_eq!(
            crate::picking::ref_visibility(
                &db,
                &LayerOverrideSet::new(),
                db.root_layer_scope(),
                &reference
            ),
            None
        );
        assert!(!crate::picking::ref_is_visible(
            &db,
            &LayerOverrideSet::new(),
            &reference
        ));
    }
}
