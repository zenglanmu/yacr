//! Contract tests for relation export (ADR-0042 D10 leftovers, D3 tail).
//!
//! **Evidence boundary.** The roundtrip is *self-consistency* only: the bytes
//! this writer produces are read back by the pinned IFC reader in this crate
//! (`crate::IfcImporter`). It is not a golden file, not an external IFC checker,
//! not a vendor application opening the file, and not a certification.

use std::sync::Arc;

use cad_db::{
    BimDefinition, BimDefinitions, BimElement, BimEvidence, BimKey, BimRelation, BimRelationKind,
    BimRelations, BimSource, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer,
    Layout,
};
use cad_domain::{
    Completeness, DatabaseId, DocumentId, EntityId, LayerId, LayoutId, Mesh, ObjectId, Point3,
    Revision, SemanticGeometry, SpaceId, SupportStatus, UnitContext,
};
use cad_connector_core::{
    ExportJob, ExportLimits, ExportSpace, Exporter, ImportLimits, ImportRequest, Importer,
};

use super::*;
use crate::IfcExporter;

/// The document id (the source's `IfcProject.GlobalId`).
const FIXTURE_DOCUMENT_ID: &str = "2O2Fr$t4X7Zf8NOew3FLSE";
/// The two storeys' source GlobalIds (valid IFC ids, so they are reused).
const LEVEL0_GID: &str = "2O2Fr$t4X7Zf8NOew3FLSH";
const LEVEL1_GID: &str = "2O2Fr$t4X7Zf8NOew3FLSI";
/// The wall's and slab's source GlobalIds.
const WALL_GID: &str = "2O2Fr$t4X7Zf8NOew3FLSJ";
const SLAB_GID: &str = "2O2Fr$t4X7Zf8NOew3FLSK";
/// A second wall and the shared `IfcWallType`'s source GlobalIds.
const WALL2_GID: &str = "2O2Fr$t4X7Zf8NOew3FLSM";
const TYPE_GID: &str = "2O2Fr$t4X7Zf8NOew3FLSL";

fn exporter() -> IfcExporter {
    IfcExporter::new(&crate::DESCRIPTOR_IFC)
}

fn triangle_mesh() -> Mesh {
    Mesh {
        vertices: vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ],
        triangles: vec![[0, 1, 2]],
        ..Mesh::default()
    }
}

fn entity(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "TestMesh".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Mesh(triangle_mesh()),
        draw_order: 0,
    }
}

fn bim_source(id: Option<&str>) -> BimSource {
    BimSource {
        connector: "ifc".into(),
        document_id: Some(FIXTURE_DOCUMENT_ID.into()),
        id: id.map(Into::into),
        native_id: None,
    }
}

fn wall() -> BimElement {
    BimElement {
        category: Some("IfcWall".into()),
        name: Some("Wall A".into()),
        source: bim_source(Some(WALL_GID)),
        level: Some("level:ifc:doc:0".into()),
        element_type: None,
        material: None,
        psets: Vec::new(),
        evidence: BimEvidence::Declared,
    }
}

/// The slab carries no level, so its default container is the building.
fn slab() -> BimElement {
    BimElement {
        category: Some("IfcSlab".into()),
        name: Some("Slab".into()),
        source: bim_source(Some(SLAB_GID)),
        level: None,
        element_type: None,
        material: None,
        psets: Vec::new(),
        evidence: BimEvidence::Declared,
    }
}

fn definitions() -> BimDefinitions {
    let mut definitions = BimDefinitions::new();
    for (id, name, elevation, source_id) in [
        ("level:ifc:doc:0", "Level 0", 0.0, LEVEL0_GID),
        ("level:ifc:doc:1", "Level 1", 3.0, LEVEL1_GID),
    ] {
        definitions
            .insert(BimDefinition::Level {
                id: id.into(),
                name: name.into(),
                elevation: Some(elevation),
                source: bim_source(Some(source_id)),
                evidence: BimEvidence::Declared,
            })
            .unwrap();
    }
    definitions
}

fn key(id: &str) -> BimKey {
    BimKey::Source {
        connector: "ifc".into(),
        document_id: FIXTURE_DOCUMENT_ID.into(),
        id: id.into(),
    }
}

fn relation(kind: BimRelationKind, parent: BimKey, child: BimKey) -> BimRelation {
    BimRelation {
        kind,
        parent,
        child,
        source: None,
        label: None,
    }
}

fn layers(builder: &mut DrawingDatabaseBuilder) {
    for (id, name) in [(LayerId(0), "0"), (LayerId(1), "A-WALL")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
}

fn layout(builder: &mut DrawingDatabaseBuilder) {
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Layout1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
}

/// A database with two mesh rows carrying `first`/`second` as their BIM blocks.
fn bim_database(
    definitions: BimDefinitions,
    first: BimElement,
    second: BimElement,
    relations: Option<BimRelations>,
) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    layers(&mut builder);
    builder.insert_entity(entity(1)).unwrap();
    builder.insert_entity(entity(2)).unwrap();
    builder.set_entity_bim(EntityId(1), first).unwrap();
    builder.set_entity_bim(EntityId(2), second).unwrap();
    builder.set_bim_definitions(definitions).unwrap();
    layout(&mut builder);
    if let Some(relations) = relations {
        builder.set_bim_relations(relations).unwrap();
    }
    builder.finish().unwrap()
}

/// A database with a wall (level 0) and a slab (building), plus `relations`.
fn database(relations: Option<BimRelations>) -> DrawingDatabase {
    bim_database(definitions(), wall(), slab(), relations)
}

/// A database with two meshes and no BIM data at all.
fn mesh_database(relations: Option<BimRelations>) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    layers(&mut builder);
    builder.insert_entity(entity(1)).unwrap();
    builder.insert_entity(entity(2)).unwrap();
    layout(&mut builder);
    if let Some(relations) = relations {
        builder.set_bim_relations(relations).unwrap();
    }
    builder.finish().unwrap()
}

fn plain_database() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    layers(&mut builder);
    builder.insert_entity(entity(1)).unwrap();
    layout(&mut builder);
    builder.finish().unwrap()
}

fn export(database: &DrawingDatabase, exporter: &IfcExporter) -> cad_connector_core::ExportOutput {
    let units = UnitContext::from_meter();
    exporter
        .export(
            "ifc",
            &ExportJob {
                database,
                document_id: DocumentId(1),
                units: Some(&units),
                space: ExportSpace::Model,
                limits: ExportLimits::default(),
            },
        )
        .expect("the model exports")
}

fn text(output: &cad_connector_core::ExportOutput) -> String {
    std::str::from_utf8(&output.bytes)
        .expect("IFC is ASCII")
        .to_string()
}

/// FNV-1a 128 over the bytes: the regression pin for the BIM-free output.
fn fnv(bytes: &[u8]) -> String {
    let mut hash: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for byte in bytes {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
    }
    format!("{hash:032x}")
}

/// The step id of the record carrying `global_id`.
fn step_id(text: &str, global_id: &str) -> u32 {
    let needle = format!("'{global_id}'");
    for line in text.lines() {
        if line.contains(&needle) {
            let rest = line.strip_prefix('#').expect("every record starts with #");
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            return digits.parse().expect("a numeric step id");
        }
    }
    panic!("no record carries GlobalId {global_id}");
}

/// Every `#N` reference in a record body, in text order.
fn refs(record: &str) -> Vec<u32> {
    record
        .split('#')
        .skip(1)
        .filter_map(|part| {
            let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().ok()
        })
        .collect()
}

/// `(relating, related)` of every containment record, in file order.
///
/// `IfcRelContainedInSpatialStructure` is `GlobalId, OwnerHistory, Name,
/// Description, RelatedElements, RelatingStructure`: the last reference is the
/// structure, and the related list is everything between the owner history and
/// it. The writer always synthesizes an owner history for added objects.
fn containment_records(text: &str) -> Vec<(u32, Vec<u32>)> {
    text.lines()
        .filter(|line| line.contains("IFCRELCONTAINEDINSPATIALSTRUCTURE"))
        .filter_map(|line| {
            let body = line.split_once('=').map_or(line, |(_, rest)| rest);
            let refs = refs(body);
            if refs.len() < 2 {
                return None;
            }
            Some((refs[refs.len() - 1], refs[1..refs.len() - 1].to_vec()))
        })
        .collect()
}

/// `(relating, related)` of every `IfcRelNests`, in file order.
///
/// `IfcRelNests` is `GlobalId, OwnerHistory, Name, Description, RelatingObject,
/// RelatedObjects`: the relating object comes **before** the list, unlike
/// `IfcRelContainedInSpatialStructure`.
fn nest_records(text: &str) -> Vec<(u32, Vec<u32>)> {
    text.lines()
        .filter(|line| line.contains("IFCRELNESTS"))
        .filter_map(|line| {
            let body = line.split_once('=').map_or(line, |(_, rest)| rest);
            let refs = refs(body);
            if refs.len() < 3 {
                return None;
            }
            Some((refs[1], refs[2..].to_vec()))
        })
        .collect()
}

/// The relating object of the containment record naming `element`.
fn container_of(text: &str, element: u32) -> Option<u32> {
    containment_records(text)
        .into_iter()
        .find(|(_, related)| related.contains(&element))
        .map(|(relating, _)| relating)
}

/// The `IfcRelAggregates` records whose related list names `element`.
///
/// The skeleton itself aggregates the site/building/storeys, so "no
/// aggregates" is only meaningful about product-to-product edges.
fn aggregates_referencing(text: &str, element: u32) -> usize {
    text.lines()
        .filter(|line| line.contains("IFCRELAGGREGATES"))
        .filter(|line| {
            let body = line.split_once('=').map_or(*line, |(_, rest)| rest);
            refs(body).contains(&element)
        })
        .count()
}

fn import(bytes: &[u8]) -> DrawingDatabase {
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(2),
        bytes: Arc::from(bytes.to_vec().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    crate::IfcImporter::new()
        .import(&request, &|| false)
        .expect("the written IFC4 bytes are readable by the pinned reader")
        .database
}

/// A `contains` edge moves the element into the storey the source declared,
/// overriding the level-derived default.
#[test]
fn contained_relation_places_the_element_in_its_source_storey() {
    let mut relations = BimRelations::new();
    relations
        .insert(relation(
            BimRelationKind::Contains,
            key(LEVEL1_GID),
            key(WALL_GID),
        ))
        .unwrap();
    let output = export(&database(Some(relations)), &exporter());
    let text = text(&output);
    let wall = step_id(&text, WALL_GID);
    let level0 = step_id(&text, LEVEL0_GID);
    let level1 = step_id(&text, LEVEL1_GID);

    assert_eq!(
        container_of(&text, wall),
        Some(level1),
        "the contains edge, not the element's level, decides the storey:\n{text}"
    );
    assert_ne!(container_of(&text, wall), Some(level0));
    assert!(
        output.report.entries.is_empty(),
        "a written relation is not a loss: {:?}",
        output.report.entries
    );
    assert_eq!(output.report.counts.exact, 2);
    assert_eq!(output.report.completeness, Completeness::Complete);
}

/// A `nests` edge becomes `IfcRelNests`, and its host is written first even
/// when it comes later in source order.
#[test]
fn nested_relation_writes_ifcrelnests() {
    let mut relations = BimRelations::new();
    relations
        .insert(relation(
            BimRelationKind::Nests,
            key(SLAB_GID),
            key(WALL_GID),
        ))
        .unwrap();
    let output = export(&database(Some(relations)), &exporter());
    let text = text(&output);
    let wall = step_id(&text, WALL_GID);
    let slab = step_id(&text, SLAB_GID);

    let nests = nest_records(&text);
    assert_eq!(nests.len(), 1, "one IfcRelNests for the one host:\n{text}");
    let (relating, related) = &nests[0];
    assert_eq!((*relating, related.clone()), (slab, vec![wall]), "{text}");
    assert!(
        text.find(&format!("IFCSLAB('{SLAB_GID}'")).unwrap()
            < text.find(&format!("IFCWALL('{WALL_GID}'")).unwrap(),
        "the nesting host must be added before its child"
    );
    assert!(
        output.report.entries.is_empty(),
        "a written relation is not a loss: {:?}",
        output.report.entries
    );
}

/// A nesting order that differs from source order must not orphan a shared
/// type: whichever user is written first creates it.
#[test]
fn a_nesting_order_does_not_orphan_a_shared_type() {
    let mut definitions = definitions();
    definitions
        .insert(BimDefinition::Type {
            id: "type:ifc:doc:1".into(),
            name: "Basic Wall 200mm".into(),
            class: Some("IfcWallType".into()),
            category: Some("IfcWall".into()),
            psets: Vec::new(),
            source: bim_source(Some(TYPE_GID)),
            evidence: BimEvidence::Declared,
        })
        .unwrap();
    let mut first = wall();
    first.element_type = Some("type:ifc:doc:1".into());
    let mut second = wall();
    second.name = Some("Wall B".into());
    second.source = bim_source(Some(WALL2_GID));
    second.element_type = Some("type:ifc:doc:1".into());

    // The source order puts the type-owning mesh first; nesting reverses it.
    let mut relations = BimRelations::new();
    relations
        .insert(relation(
            BimRelationKind::Nests,
            key(WALL2_GID),
            key(WALL_GID),
        ))
        .unwrap();
    let output = export(
        &bim_database(definitions, first, second, Some(relations)),
        &exporter(),
    );
    let text = text(&output);
    assert_eq!(output.report.counts.exact, 2, "{:?}", output.report.entries);
    assert!(!output
        .report
        .entries
        .iter()
        .any(|entry| entry.code == crate::export::ADD_REFUSED));
    assert_eq!(
        text.matches("=IFCWALLTYPE(").count(),
        1,
        "the type is created once:\n{text}"
    );
    assert!(text.contains("IFCRELDEFINESBYTYPE"), "{text}");
    // The host is written (and creates the type) before its nested child.
    assert!(
        text.find(&format!("IFCWALL('{WALL2_GID}'")).unwrap()
            < text.find(&format!("IFCWALL('{WALL_GID}'")).unwrap()
    );
}

/// `aggregates` has no expression in the plan API: it is reported, never
/// faked as a container or a nest, and it changes no byte of the file.
#[test]
fn an_aggregate_relation_is_reported_not_faked() {
    let mut relations = BimRelations::new();
    relations
        .insert(relation(
            BimRelationKind::Aggregates,
            key(SLAB_GID),
            key(WALL_GID),
        ))
        .unwrap();
    let with_edge = export(&database(Some(relations)), &exporter());
    let without = export(&database(None), &exporter());
    assert_eq!(
        with_edge.bytes, without.bytes,
        "an unwritable edge must not change the written file"
    );
    let text = text(&with_edge);
    assert!(!text.contains("IFCRELNESTS"), "{text}");
    // The wall keeps its own level: the aggregate edge is not smuggled in as
    // a container, and no product-to-product aggregate appears (the skeleton's
    // own building → storey aggregates are not this edge).
    let wall = step_id(&text, WALL_GID);
    assert_eq!(aggregates_referencing(&text, wall), 0, "{text}");
    assert_eq!(container_of(&text, wall), Some(step_id(&text, LEVEL0_GID)));

    let entry = with_edge
        .report
        .entries
        .iter()
        .find(|entry| entry.code == RELATION_NOT_WRITTEN)
        .expect("the unwritable edge is reported");
    assert_eq!(entry.status, SupportStatus::Partial);
    assert!(entry.message.contains("aggregates"), "{}", entry.message);
    assert!(entry.message.contains('1'), "{}", entry.message);
    assert!(matches!(
        with_edge.report.completeness,
        Completeness::Partial(_)
    ));
}

/// An endpoint the export did not write is skipped and counted, not invented.
#[test]
fn a_relation_endpoint_missing_is_skipped_and_counted() {
    let mut relations = BimRelations::new();
    // A storey the document names but this export never writes (no definition
    // row, so no IfcBuildingStorey).
    relations
        .insert(relation(
            BimRelationKind::Contains,
            key("2O2Fr$t4X7Zf8NOew3FLSO"),
            key(WALL_GID),
        ))
        .unwrap();
    let output = export(&database(Some(relations)), &exporter());
    let text = text(&output);
    // The element keeps its default container and is written exactly.
    assert_eq!(output.report.counts.exact, 2);
    assert_eq!(output.report.counts.dropped, 0);
    let wall = step_id(&text, WALL_GID);
    assert_eq!(container_of(&text, wall), Some(step_id(&text, LEVEL0_GID)));

    let entry = output
        .report
        .entries
        .iter()
        .find(|entry| entry.code == RELATION_ENDPOINT_MISSING)
        .expect("the missing endpoint is reported");
    assert_eq!(entry.status, SupportStatus::Partial);
    assert!(entry.message.contains("contains"), "{}", entry.message);
    assert!(!output
        .report
        .entries
        .iter()
        .any(|entry| entry.code == RELATION_NOT_WRITTEN));
}

/// No relation rows: the pre-relation bytes and reports are unchanged.
#[test]
fn a_database_without_relations_exports_the_same_bytes_as_before() {
    // The original pin still holds for a BIM-free database.
    let plain = export(&plain_database(), &exporter());
    assert_eq!(plain.bytes.len(), 2196);
    assert_eq!(fnv(&plain.bytes), "ec72ad09a8420e2a991991a7fb3d804b");

    // A BIM database with an empty relation table writes the same bytes twice
    // and no relation record or diagnostic.
    let first = export(&database(None), &exporter());
    let second = export(&database(None), &exporter());
    assert_eq!(first.bytes, second.bytes, "the writer stays deterministic");
    let text = text(&first);
    let wall = step_id(&text, WALL_GID);
    assert!(
        !text.contains("IFCRELNESTS"),
        "no relation table may write IFCRELNESTS"
    );
    assert_eq!(
        aggregates_referencing(&text, wall),
        0,
        "no relation table may aggregate a product"
    );
    assert!(
        first.report.entries.is_empty(),
        "{:?}",
        first.report.entries
    );
    // With nothing nested, meshes are written in source order.
    assert!(text.find(WALL_GID).unwrap() < text.find(SLAB_GID).unwrap());
}

/// A relation table on a database with no BIM elements is read (its endpoints
/// can be entity keys) and an unwritable edge still writes the same mesh bytes.
#[test]
fn a_relation_table_alone_does_not_change_the_mesh_bytes() {
    let baseline = export(&mesh_database(None), &exporter());
    let mut relations = BimRelations::new();
    relations
        .insert(relation(
            BimRelationKind::Group,
            BimKey::Entity(EntityId(2)),
            BimKey::Entity(EntityId(1)),
        ))
        .unwrap();
    let with_edge = export(&mesh_database(Some(relations)), &exporter());
    assert_eq!(with_edge.bytes, baseline.bytes);
    assert_eq!(with_edge.report.counts.exact, 2);
    let entry = with_edge
        .report
        .entries
        .iter()
        .find(|entry| entry.code == RELATION_NOT_WRITTEN)
        .expect("the group edge is reported");
    assert!(entry.message.contains("group"), "{}", entry.message);
}

/// The writable relations survive a self-consistency roundtrip through the
/// pinned reader.
#[test]
fn relations_round_trip_through_the_importer() {
    let mut relations = BimRelations::new();
    relations
        .insert(relation(
            BimRelationKind::Contains,
            key(LEVEL1_GID),
            key(WALL_GID),
        ))
        .unwrap();
    relations
        .insert(relation(
            BimRelationKind::Nests,
            key(SLAB_GID),
            key(WALL_GID),
        ))
        .unwrap();
    let output = export(&database(Some(relations)), &exporter());
    assert!(
        text(&output).contains("IFCRELNESTS"),
        "the nest is written:\n{}",
        text(&output)
    );

    let database = import(&output.bytes);
    let read_back = database.bim_relations();
    let nests = read_back
        .iter()
        .filter(|row| row.kind == BimRelationKind::Nests)
        .count();
    assert_eq!(nests, 1, "the reader recovers the IfcRelNests edge");
    let contains = read_back
        .iter()
        .filter(|row| row.kind == BimRelationKind::Contains)
        .count();
    assert!(contains >= 1, "the reader recovers the containment edge");
    assert!(
        read_back.iter().any(|row| matches!(
            &row.child,
            BimKey::Source { id, .. } if id == WALL_GID
        )),
        "an endpoint is the wall's own GlobalId"
    );
}
