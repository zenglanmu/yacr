//! End-to-end document acceptance for `.ymodel` (ADR-0037 D2/D5/D11/D12).
//!
//! Every case starts from a real [`DrawingDatabase`], writes a real archive
//! through [`ZipDocumentStore`], reads it back and hydrates a second database.
//! No test inspects the store's private state, and no test asserts a value that
//! was not handed back out of an archive.
//!
//! The one exception is the last test: it hand-builds an archive holding an entry
//! this build does not model, which is precisely the situation the store must
//! preserve and which no API can produce. It reuses the fixture shape of
//! `tests/archive.rs` rather than inventing a second mechanism.
//!
//! The sample drawing is one shared fixture, populated once with every persisted
//! surface (the ten tables it writes, the header variables, the hidden set, render
//! attributes) so the fixed-point test cannot pass by writing a near-empty
//! document.

mod common;

use std::collections::BTreeMap;

use serde_json::Value;

use cad_db::{
    BimDefinition, BimDefinitions, BimElement, BimEvidence, BimProperty, BimPropertySet, BimSource,
    BimValue, BlockDefinition, ChangeScope, DbEntity, DbObject, DimStyle, DocumentSnapshot,
    DocumentStore, DrawingDatabase, DrawingDatabaseBuilder, EntityColor, EntityLineWeight,
    EntityRenderAttributes, ExternalReference, ExternalReferenceStatus, Layer, LayerAttributes,
    LayerColor, LayerLineWeight, Layout, LineType, LinetypePattern, MLeaderStyle, MemorySink,
    MemorySource, PaperViewport, PlotMargins, PlotPaperUnits, PlotProvenance, PlotRotation,
    PlotSettingsRecord, PlotType, SaveReport, Scale, Style,
};
use cad_domain::{
    BlockId, Completeness, DatabaseId, DimStyleId, DocumentIdentity, EntityId, LayerId, LayoutId,
    LinetypeId, MLeaderStyleId, ObjectId, Point3, Revision, ScaleId, SemanticGeometry, SpaceId,
    StyleId, TransactionId, Transform3, UnitContext,
};
use cad_store_zip::ZipDocumentStore;

use common::{assemble, explode};

const FOREIGN_PATH: &str = "raw/source-17.bin";
const FOREIGN_BODY: &[u8] = b"\x00\x01opaque source bytes\xff";

// ---------------------------------------------------------------- fixtures ---

fn point(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn units() -> UnitContext {
    UnitContext::drawing_units()
}

fn identity() -> DocumentIdentity {
    DocumentIdentity::Temporary(3)
}

fn resource_keys() -> Vec<String> {
    vec!["arial.ttf".to_string(), "logo.png".to_string()]
}

fn line(id: u128, layer: LayerId, space: SpaceId) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".to_string(),
            revision: Revision(0),
            source_handle: Some(format!("{id:X}")),
        },
        id: EntityId(id),
        layer,
        space,
        geometry: SemanticGeometry::Line {
            start: point(0.0, 0.0, 0.0),
            end: point(10.0, 0.0, 0.0),
        },
        draw_order: 0,
    }
}

fn viewport(layer_overrides: BTreeMap<LayerId, bool>) -> PaperViewport {
    PaperViewport {
        clip: vec![
            point(0.0, 0.0, 0.0),
            point(100.0, 0.0, 0.0),
            point(100.0, 50.0, 0.0),
            point(0.0, 50.0, 0.0),
        ],
        model_to_paper: Transform3::translation(point(5.0, 7.0, 0.0)),
        completeness: Completeness::Partial(vec!["viewport.clip_partial".to_string()]),
        layer_overrides,
    }
}

/// One database that touches every persisted surface: the ten tables it writes,
/// a block registered out of id order, two viewports, header variables (including
/// a non-finite `$PDSIZE`) and the display side tables.
fn database() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(7));

    // Layers: the default row plus one with non-default attributes, plus an
    // unused row the never-reuse test can delete.
    for (id, name, visible) in [
        (LayerId(0), "0", true),
        (LayerId(1), "Walls", false),
        (LayerId(2), "Scratch", true),
    ] {
        builder
            .insert_layer(Layer {
                id,
                name: name.to_string(),
                visible,
            })
            .expect("insert layer");
    }
    builder
        .set_layer_attributes(
            LayerId(1),
            LayerAttributes {
                off: true,
                frozen: true,
                color: LayerColor {
                    aci: Some(1),
                    rgb: [255, 0, 0],
                },
                linetype: "DASHED".to_string(),
                lineweight: LayerLineWeight::Hundredths(35),
                opacity: 0.5,
                plot_style: Some("Normal".to_string()),
                ..LayerAttributes::default()
            },
        )
        .expect("layer attributes");

    // Blocks: id 1 owns a member entity and a source handle; both are named out
    // of id order, so `block_name_order` is not the id order.
    //
    // Block id 1 shares its sequence with the paper layout below (layout id 1):
    // each model family has its own identifier kind, so the two models must both
    // round-trip rather than encode to one shard entry.
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(3)],
            dynamic_visibility: None,
        })
        .expect("insert block");
    builder
        .insert_block(BlockDefinition {
            id: BlockId(2),
            entities: vec![],
            dynamic_visibility: None,
        })
        .expect("insert named block");
    builder
        .set_block_metadata(BlockId(2), "Window", point(0.0, 0.0, 0.0))
        .expect("block metadata");
    builder
        .set_block_metadata(BlockId(1), "Door", point(1.5, 2.5, 0.0))
        .expect("block metadata");
    builder.set_block_source_handle(BlockId(1), 42);

    // Layouts: the synthetic model row plus a paper sheet with two viewports.
    builder
        .insert_layout(Layout {
            id: LayoutId(0),
            name: "Model".to_string(),
            viewports: vec![],
        })
        .expect("model layout");
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet".to_string(),
            viewports: vec![
                viewport(BTreeMap::new()),
                viewport(BTreeMap::from([(LayerId(1), true)])),
            ],
        })
        .expect("paper layout");
    builder
        .set_plot_settings(PlotSettingsRecord {
            layout: LayoutId(1),
            paper_size_name: "ISO_A3".to_string(),
            paper_width: 297.0,
            paper_height: 420.0,
            margins: PlotMargins::uniform(12.0),
            rotation: PlotRotation::Degrees90,
            scale_numerator: 1.0,
            scale_denominator: 100.0,
            plot_type: PlotType::Layout,
            paper_units: PlotPaperUnits::Millimeters,
            provenance: PlotProvenance::Imported,
        })
        .expect("plot settings");

    builder
        .insert_style(Style {
            id: StyleId(1),
            name: "Notes".to_string(),
            resource_keys: vec!["arial.ttf".to_string()],
        })
        .expect("text style");
    let mut dim_style = DimStyle::standard_metric(DimStyleId(3));
    dim_style.name = "Fancy".to_string();
    dim_style.text_height = 3.5;
    builder.insert_dim_style(dim_style).expect("dim style");
    let mut mleader_style = MLeaderStyle::standard_metric(MLeaderStyleId(4));
    mleader_style.name = "Callout".to_string();
    mleader_style.left_attachment = 3;
    builder
        .insert_mleader_style(mleader_style)
        .expect("mleader style");
    builder
        .insert_linetype(LineType {
            id: LinetypeId(1),
            name: "DASHED".to_string(),
            pattern: LinetypePattern::from_elements([0.5, -0.25]),
            complex: false,
        })
        .expect("linetype");
    builder
        .insert_scale(Scale {
            id: ScaleId(1),
            name: "1:100".to_string(),
            paper_units: 1.0,
            drawing_units: 100.0,
        })
        .expect("scale");
    builder
        .insert_external_reference(ExternalReference {
            name: "Site".to_string(),
            raw_path: "site.dwg".to_string(),
            resolved_path: Some("site.dwg".to_string()),
            block: Some(BlockId(2)),
            overlay: false,
            depth: 0,
            status: ExternalReferenceStatus::Missing,
            reason: Some("xref.missing".to_string()),
        })
        .expect("xref");

    // One entity in each space: model, the paper layout, and the block.
    builder
        .insert_entity(line(1, LayerId(0), SpaceId::Model))
        .expect("model entity");
    builder
        .insert_entity(line(2, LayerId(0), SpaceId::Paper(LayoutId(1))))
        .expect("paper entity");
    builder
        .insert_entity(line(3, LayerId(1), SpaceId::Block(BlockId(1))))
        .expect("block member");
    builder
        .set_entity_render_attributes(
            EntityId(1),
            EntityRenderAttributes {
                color: EntityColor::Explicit([10, 20, 30]),
                lineweight: EntityLineWeight::Explicit(0.25),
                ..EntityRenderAttributes::default()
            },
        )
        .expect("render attributes");
    // The paper entity is the one the source file baked hidden.
    builder
        .set_hidden_entities([EntityId(2)])
        .expect("hidden set");

    // Header variables: `$LTSCALE`, `$PDSIZE` (non-finite, stored raw),
    // `$DWGCODEPAGE` and `CANNOSCALE`.
    builder.set_linetype_scale(2.5).expect("ltscale");
    builder.set_point_display_mode(3);
    builder.set_point_display_size(f64::NAN);
    builder.set_code_page(Some("ANSI_936"));
    builder
        .set_active_annotation_scale("1:100", 0.01)
        .expect("active annotation scale");

    builder.finish().expect("build the sample database")
}

fn snapshot_of(db: &DrawingDatabase, revision: Revision) -> DocumentSnapshot {
    db.snapshot(units(), identity(), resource_keys(), revision)
        .expect("encode the snapshot")
}

// --------------------------------------------------------- archive helpers ---

fn write_archive(
    store: &mut ZipDocumentStore,
    snapshot: &DocumentSnapshot,
) -> (Vec<u8>, SaveReport) {
    let mut sink = MemorySink::new();
    let report = store
        .write(snapshot, &ChangeScope::default(), &mut sink)
        .expect("write the archive");
    let bytes = sink.committed_bytes().expect("the save committed").to_vec();
    (bytes, report)
}

fn read_archive(store: &mut ZipDocumentStore, bytes: &[u8]) -> cad_db::LoadedDocument {
    let mut source = MemorySource::new(bytes.to_vec());
    store.read(&mut source).expect("read the archive")
}

/// One write + read + hydrate through fresh store instances.
fn round_trip(db: &DrawingDatabase) -> DrawingDatabase {
    let snapshot = snapshot_of(db, Revision(7));
    let (bytes, _) = write_archive(&mut ZipDocumentStore::new(), &snapshot);
    let loaded = read_archive(&mut ZipDocumentStore::new(), &bytes);
    DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate the reopened document")
}

// ------------------------------------------------------------ archive surgery ---

/// Add an entry this build does not model to `base`, leaving it unlisted.
///
/// This is the `listed: false` shape of `tests/archive.rs::with_foreign`: the
/// entry exists in the zip but nowhere in `manifest.entries`, so the next archive
/// has to measure it rather than trust a record.
fn with_unlisted_entry(base: &[u8], path: &str, body: &[u8]) -> Vec<u8> {
    let (mut entries, manifest) = explode(base);
    entries.push((path.to_string(), body.to_vec()));
    // `assemble` writes the unlisted entry `Stored` verbatim and frames the
    // modelled JSON entries; the unchanged manifest still describes those.
    assemble(&entries, &manifest)
}

// ------------------------------------------------------------------ tests ---

/// Property 1: `snapshot -> archive -> database -> snapshot` is a fixed point.
#[test]
fn a_document_is_a_fixed_point_through_the_archive() {
    let db = database();
    let first = snapshot_of(&db, Revision(7));

    let (bytes, report) = write_archive(&mut ZipDocumentStore::new(), &first);
    assert_eq!(report.saved_revision, Revision(7));
    assert_eq!(report.bytes_written, bytes.len() as u64);

    let loaded = read_archive(&mut ZipDocumentStore::new(), &bytes);
    let reopened = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
    let second = snapshot_of(&reopened, Revision(7));

    assert_eq!(
        second, first,
        "database -> archive -> database -> snapshot must not lose a field"
    );
    // The snapshot equality above is shape-only for binary payloads (ADR-0040
    // D16); the bytes need their own assertion.
    assert!(
        second.bytes_equal(&first),
        "database -> archive -> database must not corrupt textures/resources"
    );
    assert_eq!(second.revision, Revision(7));
    assert_eq!(
        second.tables.rows.len(),
        10,
        "the fixture must cover every table it writes, or the fixed point is \
         trivially true; `tables/resources.json` is written only when the \
         document carries resources (ADR-0040 D12) and this fixture packs none"
    );
}

/// Property 2 (D2): saving must never advance the import baseline.
#[test]
fn the_import_baseline_never_advances_across_saves() {
    let db = database();
    let first = snapshot_of(&db, Revision(7));
    let (bytes, _) = write_archive(
        &mut ZipDocumentStore::new().with_open_revision(Some(3)),
        &first,
    );

    // Open through the store, so the baseline it reads is the one it writes.
    let mut store = ZipDocumentStore::new();
    let loaded = read_archive(&mut store, &bytes);
    assert_eq!(loaded.manifest.open_revision, Revision(3));
    assert_eq!(loaded.manifest.saved_revision, Revision(7));
    assert_eq!(loaded.snapshot.revision, Revision(7));
    assert_ne!(
        loaded.manifest.open_revision, loaded.manifest.saved_revision,
        "an edited document must not claim to still be its source bytes"
    );

    let reopened = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
    assert_eq!(reopened.revision(), Revision(7));

    let second = snapshot_of(&reopened, Revision(8));
    let (bytes, report) = write_archive(&mut store, &second);
    assert_eq!(report.saved_revision, Revision(8));

    let loaded = read_archive(&mut ZipDocumentStore::new(), &bytes);
    assert_eq!(
        loaded.manifest.open_revision,
        Revision(3),
        "the baseline is the import revision, not the last save"
    );
    assert_eq!(loaded.manifest.saved_revision, Revision(8));
    assert_eq!(loaded.snapshot.revision, Revision(8));
    assert_ne!(
        loaded.manifest.open_revision,
        loaded.manifest.saved_revision
    );
}

/// Property 3 (D5): counters survive, and a deleted high-water id is not reused.
#[test]
fn identity_counters_survive_and_never_reuse_a_deleted_id() {
    let db = database();
    let mut reopened = round_trip(&db);
    assert_eq!(
        reopened.id_counters(),
        db.id_counters(),
        "the nine counters are document state"
    );

    // `Scratch` is the highest layer id and nothing references it.
    let deleted = LayerId(2);
    assert_eq!(
        reopened.layer(deleted).map(|layer| layer.name.as_str()),
        Some("Scratch")
    );
    reopened
        .delete_layer(deleted, TransactionId(3), "test")
        .expect("delete the highest layer");

    let (bytes, _) = write_archive(
        &mut ZipDocumentStore::new(),
        &snapshot_of(&reopened, Revision(8)),
    );
    let loaded = read_archive(&mut ZipDocumentStore::new(), &bytes);
    let mut after_reopen = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
    assert_eq!(after_reopen.layer(deleted), None);

    let (created, _) = after_reopen
        .create_layer("After", TransactionId(4), "test")
        .expect("create a layer after reopen");
    assert!(
        created.0 > deleted.0,
        "a deleted high-water id must never be handed out again: deleted {}, created {}",
        deleted.0,
        created.0
    );
}

/// Property 4: a load -> save cycle reproduces the archive byte for byte.
#[test]
fn two_saves_of_one_document_are_byte_identical() {
    let db = database();
    let first_snapshot = snapshot_of(&db, Revision(7));
    let first = write_archive(&mut ZipDocumentStore::new(), &first_snapshot).0;

    let mut store = ZipDocumentStore::new();
    let loaded = read_archive(&mut store, &first);
    let reopened = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
    let second_snapshot = snapshot_of(&reopened, Revision(7));
    assert_eq!(
        second_snapshot, first_snapshot,
        "the second save must encode the same document, or determinism is untested"
    );

    let second = write_archive(&mut store, &second_snapshot).0;
    assert_eq!(
        first, second,
        "the archive is a pure function of the snapshot"
    );
}

/// Property 5: content spot-checks a table-only or element-only check would miss.
#[test]
fn content_that_a_table_only_check_would_miss_survives() {
    let db = database();
    let reopened = round_trip(&db);

    // Entities: same count, same ids.
    let source_ids: Vec<EntityId> = db.entities().map(|entity| entity.id).collect();
    let reopened_ids: Vec<EntityId> = reopened.entities().map(|entity| entity.id).collect();
    assert_eq!(reopened.entity_count(), 3);
    assert_eq!(reopened_ids, source_ids, "the id set, not just the count");

    // The second layer's lineweight (its attributes are a separate table).
    assert_eq!(
        reopened.layer_attributes(LayerId(1)).lineweight,
        LayerLineWeight::Hundredths(35)
    );

    // The block's member list (stored on the block row and in the element shard).
    assert_eq!(
        reopened
            .block(BlockId(1))
            .map(|block| block.entities.clone()),
        Some(vec![EntityId(3)])
    );

    // Both viewports of the paper layout, including the per-viewport override.
    let sheet = reopened.layout(LayoutId(1)).expect("paper layout");
    assert_eq!(sheet.viewports.len(), 2);
    assert_eq!(sheet.viewports[0].layer_overrides.len(), 0);
    assert_eq!(sheet.viewports[1].clip.len(), 4);
    assert_eq!(
        sheet.viewports[1].layer_overrides.get(&LayerId(1)),
        Some(&true)
    );

    // Drawing-global scalars.
    assert_eq!(reopened.linetype_scale(), 2.5);
    assert!(reopened.point_display_size().is_nan());
    assert_eq!(reopened.code_page(), Some("ANSI_936"));

    // Hidden-entity membership: present for the paper entity, absent elsewhere.
    assert!(reopened.entity_hidden(EntityId(2)));
    assert!(!reopened.entity_hidden(EntityId(1)));

    // Exactly one entity in the paper space, and it is the hidden one.
    let paper = reopened.layout_entities(LayoutId(1));
    assert_eq!(paper.len(), 1);
    assert_eq!(paper[0].id, EntityId(2));
}

/// Property 6: an unmodelled entry survives two real saves and is reported.
#[test]
fn a_foreign_entry_survives_a_real_save_cycle() {
    let snapshot = snapshot_of(&database(), Revision(7));
    let base = write_archive(&mut ZipDocumentStore::new(), &snapshot).0;
    let previous = with_unlisted_entry(&base, FOREIGN_PATH, FOREIGN_BODY);

    // First save: the store is handed the archive that holds the unknown entry.
    let mut store = ZipDocumentStore::new().with_previous(Some(previous));
    let (bytes, report) = write_archive(&mut store, &snapshot);
    // The foreign entry is one of the raw copies; untouched shards are the others.
    assert!(report.entries_copied >= 1, "{report:?}");

    // Second save: a fresh store reads the archive (keeping it for preservation)
    // and writes it again.
    let mut store = ZipDocumentStore::new();
    let loaded = read_archive(&mut store, &bytes);
    assert_eq!(
        loaded.manifest.foreign_entries,
        vec![FOREIGN_PATH.to_string()],
        "the reopened manifest must report the carried entry"
    );
    assert_eq!(loaded.snapshot, snapshot, "the document still round-trips");
    let (bytes, report) = write_archive(&mut store, &loaded.snapshot);
    assert!(report.entries_copied >= 1, "{report:?}");

    // Reopen the final archive: the entry is still there, byte-identical, and
    // recorded as foreign rather than modelled.
    let loaded = read_archive(&mut ZipDocumentStore::new(), &bytes);
    assert_eq!(
        loaded.manifest.foreign_entries,
        vec![FOREIGN_PATH.to_string()]
    );
    let (entries, manifest) = explode(&bytes);
    let carried = entries
        .iter()
        .find(|(name, _)| name == FOREIGN_PATH)
        .expect("the foreign entry is preserved");
    assert_eq!(carried.1, FOREIGN_BODY);
    let record = manifest["entries"]
        .as_array()
        .expect("manifest entries")
        .iter()
        .find(|record| record["path"] == FOREIGN_PATH)
        .expect("the carried entry is recorded");
    assert_eq!(record["foreign"], Value::Bool(true));
}

/// A paper layout and a block with the same sequence are two distinct models.
///
/// Both allocators start at 1, so this is the ordinary case, not a contrived
/// one: a document whose paper space and block both carry sequence 1 must store
/// one shard per model and read both back.
#[test]
fn paper_and_block_models_with_equal_sequences_do_not_collide() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(11));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("insert layer");
    builder
        .insert_layout(Layout {
            id: LayoutId(0),
            name: "Model".to_string(),
            viewports: vec![],
        })
        .expect("model layout");
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet".to_string(),
            viewports: vec![],
        })
        .expect("paper layout");
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .expect("insert block");
    // One element per model, so a collision cannot hide behind an empty model.
    builder
        .insert_entity(line(1, LayerId(0), SpaceId::Paper(LayoutId(1))))
        .expect("paper entity");
    builder
        .insert_entity(line(2, LayerId(0), SpaceId::Block(BlockId(1))))
        .expect("block entity");
    let db = builder.finish().expect("build the colliding database");

    let reopened = round_trip(&db);
    assert_eq!(
        reopened
            .layout_entities(LayoutId(1))
            .iter()
            .map(|entity| entity.id)
            .collect::<Vec<_>>(),
        vec![EntityId(1)],
        "the paper model keeps its own element"
    );
    assert_eq!(
        reopened
            .block(BlockId(1))
            .map(|block| block.entities.clone()),
        Some(vec![EntityId(2)]),
        "the block model keeps its own element"
    );
    assert_eq!(
        reopened
            .entity(EntityId(1))
            .map(|entity| entity.space.clone()),
        Some(SpaceId::Paper(LayoutId(1)))
    );
    assert_eq!(
        reopened
            .entity(EntityId(2))
            .map(|entity| entity.space.clone()),
        Some(SpaceId::Block(BlockId(1)))
    );
}

// ------------------------------------------------------- BIM (ADR-0042 D6) ---

fn bim_source() -> BimSource {
    BimSource {
        connector: "ifc".to_string(),
        document_id: Some("doc-1".to_string()),
        id: Some("2O2Fr$t4X7Zf8Q".to_string()),
        native_id: Some("22".to_string()),
    }
}

/// A database carrying one BIM element and the three definition rows it
/// references. Deliberately separate from [`database()`]: the shared fixture must
/// stay a minor-4 archive so its "every table" count and byte assertions keep
/// meaning what they say.
fn bim_database() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(9));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("layer");
    builder
        .insert_entity(line(1, LayerId(0), SpaceId::Model))
        .expect("entity");

    let mut definitions = BimDefinitions::new();
    definitions
        .insert(BimDefinition::Level {
            id: "level:ifc:doc-1:L1".to_string(),
            name: "Level 1".to_string(),
            elevation: Some(0.0),
            source: bim_source(),
            evidence: BimEvidence::Declared,
        })
        .expect("level");
    definitions
        .insert(BimDefinition::Type {
            id: "type:ifc:doc-1:T1".to_string(),
            name: "Basic Wall:Generic 200mm".to_string(),
            class: Some("IfcWallType".to_string()),
            category: Some("IfcWall".to_string()),
            psets: Vec::new(),
            source: bim_source(),
            evidence: BimEvidence::Declared,
        })
        .expect("type");
    definitions
        .insert(BimDefinition::Material {
            id: "material:ifc:doc-1:M1".to_string(),
            name: "Concrete".to_string(),
            source: bim_source(),
            evidence: BimEvidence::Declared,
        })
        .expect("material");
    builder
        .set_bim_definitions(definitions)
        .expect("definitions");

    builder
        .set_entity_bim(
            EntityId(1),
            BimElement {
                category: Some("IfcWall".to_string()),
                name: Some("Basic Wall:Generic 200mm".to_string()),
                source: bim_source(),
                level: Some("level:ifc:doc-1:L1".to_string()),
                element_type: Some("type:ifc:doc-1:T1".to_string()),
                material: Some("material:ifc:doc-1:M1".to_string()),
                psets: vec![BimPropertySet {
                    name: "Pset_WallCommon".to_string(),
                    properties: vec![
                        BimProperty {
                            name: "IsExternal".to_string(),
                            value: BimValue::Bool(true),
                        },
                        BimProperty {
                            name: "Reference".to_string(),
                            value: BimValue::Real {
                                value: 0.2,
                                ifc_type: Some("IfcPositiveLengthMeasure".to_string()),
                                unit: Some("m".to_string()),
                            },
                        },
                        BimProperty {
                            name: "Comment".to_string(),
                            value: BimValue::Null,
                        },
                    ],
                }],
                evidence: BimEvidence::Declared,
            },
        )
        .expect("bim");
    builder.finish().expect("database")
}

#[test]
fn bim_data_is_a_fixed_point_through_a_minor_seven_archive() {
    let db = bim_database();
    let first = snapshot_of(&db, Revision(3));
    let (bytes, _) = write_archive(&mut ZipDocumentStore::new(), &first);
    let (_, manifest) = explode(&bytes);
    assert_eq!(
        manifest["format"]["minor"],
        Value::from(7),
        "BIM data earns minor 7 (ADR-0042 D3 tail/D6)"
    );

    let loaded = read_archive(&mut ZipDocumentStore::new(), &bytes);
    let reopened = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
    assert_eq!(
        reopened.bim_element(EntityId(1)),
        db.bim_element(EntityId(1)),
        "the element's BIM block survives the archive"
    );
    assert_eq!(
        reopened.bim_definitions(),
        db.bim_definitions(),
        "the shared definition table survives the archive"
    );
    assert_eq!(
        reopened.bim_key(EntityId(1)),
        db.bim_key(EntityId(1)),
        "the logical-component key is derived from the persisted source"
    );
    assert_eq!(
        snapshot_of(&reopened, Revision(3)),
        first,
        "database -> archive -> database -> snapshot must lose no BIM field"
    );
}
