//! Contract tests for the BIM-aware IFC4 export path (ADR-0042 Phase 2, D10).
//!
//! **Evidence boundary.** The "roundtrip" here is *self-consistency* only: the
//! bytes this writer produces are read back by the pinned `cad-import-ifc`
//! reader. It is not a golden file, not an external IFC checker, not a vendor
//! application opening the file, and not a certification. The GlobalId
//! roundtrip uses a GlobalId copied from the synthetic IFC fixture, not a real
//! model.

use std::sync::Arc;

use cad_db::{
    BimDefinition, BimDefinitions, BimElement, BimEvidence, BimProperty, BimPropertySet, BimSource,
    BimValue, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer, Layout,
};
use cad_domain::{
    Completeness, DatabaseId, DocumentId, EntityId, LayerId, LayoutId, Mesh, ObjectId, Point3,
    Revision, SemanticGeometry, SpaceId, SupportStatus, UnitContext,
};
use cad_import_core::{
    ExportJob, ExportLimits, ExportSpace, Exporter, ImportLimits, ImportRequest, Importer,
};
use ifc_lite_export::plan::PropertyValue;

use super::*;
use crate::bim_value::{convert_psets, decode_global_id, real_value};
use crate::IfcExporter;

/// The wall GlobalId of `fixtures/ifc/synthetic-wall-semantics.ifc`.
const FIXTURE_WALL_GLOBAL_ID: &str = "2O2Fr$t4X7Zf8NOew3FLSJ";
/// The document id (the fixture's `IfcProject.GlobalId`).
const FIXTURE_DOCUMENT_ID: &str = "2O2Fr$t4X7Zf8NOew3FLSE";

fn exporter() -> IfcExporter {
    IfcExporter::new(&cad_import_ifc::DESCRIPTOR_IFC)
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

fn bim_source(document_id: Option<&str>, id: Option<&str>) -> BimSource {
    BimSource {
        connector: "ifc".into(),
        document_id: document_id.map(Into::into),
        id: id.map(Into::into),
        native_id: None,
    }
}

fn wall() -> BimElement {
    BimElement {
        category: Some("IfcWall".into()),
        name: Some("Wall A".into()),
        source: bim_source(Some(FIXTURE_DOCUMENT_ID), Some(FIXTURE_WALL_GLOBAL_ID)),
        level: Some("level:ifc:doc:0".into()),
        element_type: Some("type:ifc:doc:1".into()),
        material: Some("material:ifc:doc:1".into()),
        psets: vec![BimPropertySet {
            name: "Pset_WallCommon".into(),
            properties: vec![
                BimProperty {
                    name: "IsExternal".into(),
                    value: BimValue::Bool(true),
                },
                BimProperty {
                    name: "Reference".into(),
                    value: BimValue::Text("W-200-A".into()),
                },
                BimProperty {
                    name: "Height".into(),
                    value: BimValue::Real {
                        value: 2.5,
                        ifc_type: Some("IfcPositiveLengthMeasure".into()),
                        unit: Some("m".into()),
                    },
                },
            ],
        }],
        evidence: BimEvidence::Declared,
    }
}

fn slab() -> BimElement {
    BimElement {
        category: Some("IfcSlab".into()),
        name: Some("Slab".into()),
        source: bim_source(Some(FIXTURE_DOCUMENT_ID), None),
        level: Some("level:ifc:doc:1".into()),
        element_type: None,
        material: None,
        psets: Vec::new(),
        evidence: BimEvidence::Declared,
    }
}

fn definitions() -> BimDefinitions {
    let mut definitions = BimDefinitions::new();
    for (id, name, elevation, source_id) in [
        ("level:ifc:doc:0", "Level 0", 0.0, "2O2Fr$t4X7Zf8NOew3FLSH"),
        ("level:ifc:doc:1", "Level 1", 3.0, "2O2Fr$t4X7Zf8NOew3FLSI"),
    ] {
        definitions
            .insert(BimDefinition::Level {
                id: id.into(),
                name: name.into(),
                elevation: Some(elevation),
                source: bim_source(Some(FIXTURE_DOCUMENT_ID), Some(source_id)),
                evidence: BimEvidence::Declared,
            })
            .unwrap();
    }
    definitions
        .insert(BimDefinition::Type {
            id: "type:ifc:doc:1".into(),
            name: "Basic Wall 200mm".into(),
            class: Some("IfcWallType".into()),
            category: Some("IfcWall".into()),
            psets: Vec::new(),
            source: bim_source(Some(FIXTURE_DOCUMENT_ID), Some("2O2Fr$t4X7Zf8NOew3FLSL")),
            evidence: BimEvidence::Declared,
        })
        .unwrap();
    definitions
        .insert(BimDefinition::Material {
            id: "material:ifc:doc:1".into(),
            name: "Concrete".into(),
            source: bim_source(Some(FIXTURE_DOCUMENT_ID), None),
            evidence: BimEvidence::Declared,
        })
        .unwrap();
    definitions
}

/// A database holding two mesh rows with `wall` and `slab` as their BIM blocks.
fn bim_database(
    definitions: BimDefinitions,
    wall: BimElement,
    slab: BimElement,
) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(1), "A-WALL")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    builder.insert_entity(entity(1)).unwrap();
    builder.insert_entity(entity(2)).unwrap();
    builder.set_entity_bim(EntityId(1), wall).unwrap();
    builder.set_entity_bim(EntityId(2), slab).unwrap();
    builder.set_bim_definitions(definitions).unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Layout1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    builder.finish().unwrap()
}

fn default_bim_database() -> DrawingDatabase {
    bim_database(definitions(), wall(), slab())
}

fn plain_database() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(1), "A-WALL")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    builder.insert_entity(entity(1)).unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Layout1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    builder.finish().unwrap()
}

fn export(database: &DrawingDatabase, exporter: &IfcExporter) -> cad_import_core::ExportOutput {
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

fn text(output: &cad_import_core::ExportOutput) -> String {
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

/// The GlobalId decoder is the exact inverse of the pinned encoder.
#[test]
fn ifc_global_id_round_trips_through_the_encoder() {
    for value in [
        0u128,
        1,
        0x0123_4567_89ab_cdef_0123_4567_89ab_cdef,
        0xdead_beef_cafe_babe_0011_2233_4455_6677,
        u128::MAX,
    ] {
        let encoded = GlobalId(value).encode();
        assert_eq!(decode_global_id(&encoded), Some(value), "{encoded}");
        assert_eq!(
            GlobalId(decode_global_id(&encoded).unwrap()).encode(),
            encoded
        );
    }
    // Known vectors, independent of the encoder: 22 characters, MSB first,
    // the first one carrying only 2 bits.
    assert_eq!(decode_global_id("0000000000000000000000"), Some(0));
    assert_eq!(decode_global_id("0000000000000000000001"), Some(1));
    assert_eq!(decode_global_id("3$$$$$$$$$$$$$$$$$$$$$"), Some(u128::MAX));
    // The fixture's own GlobalId decodes and re-encodes to itself.
    assert_eq!(
        decode_global_id(FIXTURE_WALL_GLOBAL_ID)
            .map(GlobalId)
            .map(GlobalId::encode),
        Some(FIXTURE_WALL_GLOBAL_ID.to_string())
    );

    // Not a GlobalId: wrong length, a character outside the alphabet, and a
    // first character that would need a 129th bit.
    assert_eq!(decode_global_id(""), None);
    assert_eq!(decode_global_id("000000000000000000000"), None);
    assert_eq!(decode_global_id("00000000000000000000000"), None);
    assert_eq!(decode_global_id("4000000000000000000000"), None);
    assert_eq!(decode_global_id("000000000000000000000!"), None);
}

/// A database with no BIM data writes exactly what it wrote before this path
/// existed: the pinned bytes, one `IfcBuildingElementProxy`, no BIM entity.
#[test]
fn a_bim_free_database_exports_the_same_bytes_as_before() {
    let database = plain_database();
    let first = export(&database, &exporter());
    let second = export(&database, &exporter());
    assert_eq!(first.bytes, second.bytes, "the writer stays deterministic");

    // Pins captured from the writer *before* the BIM path existed.
    assert_eq!(first.bytes.len(), 2196);
    assert_eq!(fnv(&first.bytes), "ec72ad09a8420e2a991991a7fb3d804b");

    let text = text(&first);
    assert!(text.contains("IFCBUILDINGELEMENTPROXY"));
    for bim_entity in [
        "IFCBUILDINGSTOREY",
        "IFCWALL(",
        "IFCWALLTYPE",
        "IFCPROPERTYSET",
        "IFCMATERIAL",
        "IFCRELDEFINESBYTYPE",
        "IFCRELDEFINESBYPROPERTIES",
        "IFCRELASSOCIATESMATERIAL",
    ] {
        assert!(
            !text.contains(bim_entity),
            "a BIM-free database must not write {bim_entity}"
        );
    }
    assert_eq!(first.report.counts.exact, 1);
    assert_eq!(first.report.counts.converted, 0);
    assert_eq!(first.report.counts.dropped, 0);
    assert!(first.report.entries.is_empty());
    assert_eq!(first.report.completeness, Completeness::Complete);

    // The empty database keeps its own pinned bytes too.
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Layout1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    let empty = builder.finish().unwrap();
    let out = export(&empty, &exporter());
    assert_eq!(out.bytes.len(), 1374);
    assert_eq!(fnv(&out.bytes), "c56f8cdb68e66d6e30a597da59c6658c");
}

/// Storeys, the category class, the type, the PSets and the material all land
/// in the file, and the pinned reader reads the result back.
#[test]
fn a_bim_database_writes_storeys_types_psets_and_materials() {
    let output = export(&default_bim_database(), &exporter());
    let text = text(&output);

    assert!(text.contains("IFCBUILDINGSTOREY"), "{text}");
    assert!(text.contains("'Level 0'"));
    assert!(text.contains("'Level 1'"));
    assert!(text.contains("IFCWALL("), "the wall keeps its category");
    assert!(text.contains("IFCSLAB("), "the slab keeps its category");
    assert!(text.contains("IFCWALLTYPE"));
    assert!(text.contains("IFCRELDEFINESBYTYPE"));
    assert!(text.contains("IFCPROPERTYSET"));
    assert!(text.contains("IFCRELDEFINESBYPROPERTIES"));
    assert!(text.contains("IFCMATERIAL("));
    assert!(text.contains("IFCRELASSOCIATESMATERIAL"));
    assert!(
        text.contains("'Concrete'"),
        "the material name is written as declared"
    );
    assert!(
        text.contains("IFCPROPERTYSINGLEVALUE('IsExternal'"),
        "{text}"
    );
    assert!(
        !text.contains("IFCBUILDINGELEMENTPROXY"),
        "both categories are writable, so no element falls back to a proxy"
    );
    // One material record, not one per occurrence: the executor de-duplicates
    // by name (an IfcMaterial carries no GlobalId in IFC4).
    assert_eq!(text.matches("=IFCMATERIAL(").count(), 1);

    // Storeys are written in (elevation, name) order.
    let storey_zero = text.find("'Level 0'").unwrap();
    let storey_one = text.find("'Level 1'").unwrap();
    assert!(storey_zero < storey_one);

    let report = &output.report;
    assert_eq!(report.counts.exact, 2, "two meshes, storeys are not meshes");
    assert_eq!(report.counts.dropped, 0, "{:?}", report.entries);
    assert!(report.entries.is_empty(), "{:?}", report.entries);
    assert_eq!(report.completeness, Completeness::Complete);

    // Self-consistency: the pinned reader reads the written bytes back and
    // finds both meshes. Not vendor acceptance.
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(2),
        bytes: Arc::from(output.bytes.clone().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    let drawing = cad_import_ifc::IfcImporter::new()
        .import(&request, &|| false)
        .expect("the written IFC4 bytes are readable by the pinned reader");
    let meshes = drawing
        .database
        .entities()
        .filter(|entity| matches!(entity.geometry, SemanticGeometry::Mesh(_)))
        .count();
    assert_eq!(meshes, 2, "both meshes survive the roundtrip");
}

/// The element's GlobalId is the source's, not a derived one: IFC → ymodel →
/// IFC keeps the identity (ADR-0042 D10).
#[test]
fn ifc_global_id_round_trips_through_the_database() {
    let output = export(&default_bim_database(), &exporter());
    let text = text(&output);
    assert!(
        text.contains(&format!("IFCWALL('{FIXTURE_WALL_GLOBAL_ID}'")),
        "the wall is written with the source GlobalId:\n{text}"
    );
    assert!(!output
        .report
        .entries
        .iter()
        .any(|entry| entry.code == GLOBAL_ID_INVALID || entry.code == GLOBAL_ID_COLLISION));
    // The two storeys keep theirs too.
    assert!(text.contains("IFCBUILDINGSTOREY('2O2Fr$t4X7Zf8NOew3FLSH'"));
    assert!(text.contains("IFCBUILDINGSTOREY('2O2Fr$t4X7Zf8NOew3FLSI'"));

    // And the pinned reader puts the same GlobalId back into `bim.source.id`,
    // so a second export would write it again unchanged (self-consistency
    // only: no vendor checker ran).
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(2),
        bytes: Arc::from(output.bytes.clone().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    let drawing = cad_import_ifc::IfcImporter::new()
        .import(&request, &|| false)
        .expect("the written IFC4 bytes are readable by the pinned reader");
    assert!(
        drawing
            .database
            .bim_elements()
            .values()
            .any(|element| element.source.id.as_deref() == Some(FIXTURE_WALL_GLOBAL_ID)),
        "the reader recovers the wall's GlobalId as its BIM source id"
    );
}

/// An invalid source GlobalId is never written; the deterministic mesh id is,
/// and the fallback is reported.
#[test]
fn an_invalid_source_global_id_falls_back_and_reports() {
    let mut element = wall();
    element.source.id = Some("not-a-global-id".into());
    let database = bim_database(definitions(), element, slab());
    let out = export(&database, &exporter());
    let text = text(&out);
    assert!(!text.contains("not-a-global-id"));
    let derived = mesh_global_id(ObjectId(1));
    assert!(
        text.contains(&format!("IFCWALL('{}'", derived.encode())),
        "the derived GlobalId is written instead"
    );
    let entry = out
        .report
        .entries
        .iter()
        .find(|entry| entry.code == GLOBAL_ID_INVALID)
        .expect("the invalid source id is reported");
    assert_eq!(entry.status, SupportStatus::Partial);
    assert!(matches!(out.report.completeness, Completeness::Partial(_)));
}

/// Two rows claiming one source GlobalId cannot both write it: the second
/// falls back and the collision is reported.
#[test]
fn a_duplicate_source_global_id_falls_back_and_reports() {
    let mut element = wall();
    let mut other = slab();
    element.source.id = Some(FIXTURE_WALL_GLOBAL_ID.into());
    other.source.id = Some(FIXTURE_WALL_GLOBAL_ID.into());
    let database = bim_database(definitions(), element, other);

    let out = export(&database, &exporter());
    let text = text(&out);
    assert_eq!(
        text.matches(FIXTURE_WALL_GLOBAL_ID).count(),
        1,
        "one record keeps the source id"
    );
    let entry = out
        .report
        .entries
        .iter()
        .find(|entry| entry.code == GLOBAL_ID_COLLISION)
        .expect("the collision is reported");
    assert_eq!(entry.status, SupportStatus::Partial);
    assert_eq!(out.report.counts.exact, 2, "both meshes are still written");
}

/// Heuristic property sets are not written by default (an IFC file cannot say
/// "unverified"); the explicit policy includes them.
#[test]
fn a_heuristic_pset_is_skipped_by_default_and_included_on_request() {
    let mut element = wall();
    element.evidence = BimEvidence::Heuristic;
    let database = bim_database(definitions(), element, slab());

    let default = export(&database, &exporter());
    let default_text = text(&default);
    assert!(
        !default_text.contains("IFCPROPERTYSET"),
        "a heuristic set must not be written by default"
    );
    let entry = default
        .report
        .entries
        .iter()
        .find(|entry| entry.code == UNVERIFIED_SKIPPED)
        .expect("the skip is reported");
    assert_eq!(entry.status, SupportStatus::Partial);

    let included = export(
        &database,
        &exporter().with_bim_policy(BimExportPolicy {
            include_unverified: true,
        }),
    );
    let included_text = text(&included);
    assert!(included_text.contains("IFCPROPERTYSET"));
    assert!(!included
        .report
        .entries
        .iter()
        .any(|entry| entry.code == UNVERIFIED_SKIPPED));
}

/// A type whose category is outside the derived-type table is skipped, never
/// given an invented class: the element is still written exactly.
#[test]
fn a_derived_type_class_outside_the_whitelist_is_skipped_not_faked() {
    let mut types = BimDefinitions::new();
    for row in &definitions() {
        match row {
            BimDefinition::Type { .. } => {}
            other => types.insert(other.clone()).unwrap(),
        }
    }
    types
        .insert(BimDefinition::Type {
            id: "type:ifc:doc:1".into(),
            name: "Proxy type".into(),
            class: None,
            category: Some("IfcBuildingElementProxy".into()),
            psets: Vec::new(),
            source: bim_source(Some(FIXTURE_DOCUMENT_ID), None),
            evidence: BimEvidence::Declared,
        })
        .unwrap();

    let database = bim_database(types, wall(), slab());
    let output = export(&database, &exporter());
    let text = text(&output);
    assert!(text.contains("IFCWALL("), "the element keeps its category");
    assert!(
        !text.contains("IFCTYPE") && !text.contains("IFCRELDEFINESBYTYPE"),
        "no type may be invented for an out-of-table category"
    );
    let entry = output
        .report
        .entries
        .iter()
        .find(|entry| entry.code == TYPE_NOT_WRITTEN)
        .expect("the skipped type is reported");
    assert_eq!(entry.status, SupportStatus::Partial);
}

/// A declared class that does not match the category is a skip, not a
/// substitution.
#[test]
fn an_inconsistent_declared_type_class_is_skipped_not_replaced() {
    let mut types = BimDefinitions::new();
    for row in &definitions() {
        match row {
            BimDefinition::Type { .. } => {}
            other => types.insert(other.clone()).unwrap(),
        }
    }
    types
        .insert(BimDefinition::Type {
            id: "type:ifc:doc:1".into(),
            name: "Slab typed as a wall".into(),
            class: Some("IfcWallType".into()),
            category: Some("IfcSlab".into()),
            psets: Vec::new(),
            source: bim_source(Some(FIXTURE_DOCUMENT_ID), None),
            evidence: BimEvidence::Declared,
        })
        .unwrap();

    let database = bim_database(types, wall(), slab());
    let output = export(&database, &exporter());
    let text = text(&output);
    assert!(text.contains("IFCWALL("));
    assert!(!text.contains("IFCSLABTYPE"));
    assert!(!text.contains("IFCWALLTYPE"));
    assert_eq!(
        output
            .report
            .entries
            .iter()
            .filter(|entry| entry.code == TYPE_NOT_WRITTEN)
            .count(),
        1
    );
}

/// An element naming a storey that is not a level definition stays in the
/// building, reported, never in a storey that was not written. (A validated
/// database cannot hold a dangling reference, so this pins the resolution
/// branch directly.)
#[test]
fn an_unresolved_storey_leaves_the_element_in_the_building() {
    let building = stable_global_id("yacr/building", 0);
    let storeys = Storeys::new();
    let mut counts = Counts::default();
    let container = storey_container(
        Some("level:ifc:doc:missing"),
        &storeys,
        &building,
        &mut counts,
    );
    assert_eq!(container, ObjectRef::Guid(building.encode()));
    assert_eq!(counts.storeys, 1);
    let entries = counts.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].code, STOREY_NOT_WRITTEN);

    // A resolved level is used, and nothing is reported.
    let mut storeys = Storeys::new();
    let storey = stable_global_id("yacr/bim/storey", 1);
    storeys.insert("level:ifc:doc:0".into(), storey);
    let mut counts = Counts::default();
    let container = storey_container(Some("level:ifc:doc:0"), &storeys, &building, &mut counts);
    assert_eq!(container, ObjectRef::Guid(storey.encode()));
    assert!(counts.entries().is_empty());

    // An element with no level at all is in the building without a report.
    let mut counts = Counts::default();
    let container = storey_container(None, &storeys, &building, &mut counts);
    assert_eq!(container, ObjectRef::Guid(building.encode()));
    assert!(counts.entries().is_empty());
}

/// A category outside the declared table falls back to the proxy and says so.
#[test]
fn an_unknown_category_falls_back_to_proxy_and_reports() {
    let mut element = wall();
    element.category = Some("IfcNotARealClass".into());
    let database = bim_database(definitions(), element, slab());
    let out = export(&database, &exporter());
    let text = text(&out);
    assert!(text.contains("IFCBUILDINGELEMENTPROXY"));
    assert!(!text.contains("IFCNOTAREALCLASS"));
    let entry = out
        .report
        .entries
        .iter()
        .find(|entry| entry.code == CATEGORY_NOT_WRITTEN)
        .expect("the fallback is reported");
    assert_eq!(entry.status, SupportStatus::Partial);
}

/// Units are converted, never relabelled: 10 ft is 3.048 m; a unit this
/// writer cannot convert is not written at all.
#[test]
fn a_real_value_converts_its_unit_and_refuses_an_unknown_one() {
    let mut counts = Counts::default();
    let converted = real_value(10.0, Some("IfcLengthMeasure"), Some("ft"), &mut counts)
        .expect("feet are convertible");
    match converted {
        PropertyValue::Measure {
            measure_type,
            value,
            ..
        } => {
            assert_eq!(measure_type, "IfcLengthMeasure");
            assert!((value - 3.048).abs() < 1e-12, "{value}");
        }
        other => panic!("expected a length measure, got {other:?}"),
    }
    assert_eq!(counts.properties, 0);

    let unknown = real_value(5.0, Some("IfcForceMeasure"), Some("kN"), &mut counts);
    assert!(unknown.is_none(), "an unconvertible unit is not written");
    assert_eq!(counts.properties, 1);

    // A positive measure with a non-positive value keeps the number but loses
    // the measure type, and says so.
    let negative = real_value(-2.0, Some("IfcPositiveLengthMeasure"), None, &mut counts);
    assert!(matches!(negative, Some(PropertyValue::Real(value)) if value == -2.0));
    assert_eq!(counts.properties, 2);
}

/// A null property is not invented into `$`: it is skipped and counted, and a
/// set left empty is not written.
#[test]
fn a_null_property_is_skipped_and_reported() {
    let mut counts = Counts::default();
    let sets = convert_psets(
        &[BimPropertySet {
            name: "Pset_NullOnly".into(),
            properties: vec![BimProperty {
                name: "Empty".into(),
                value: BimValue::Null,
            }],
        }],
        BimEvidence::Declared,
        BimExportPolicy::default(),
        &mut counts,
    );
    assert!(sets.is_empty(), "an empty property set is not written");
    assert_eq!(counts.properties, 1);
    let entries = counts.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].code, PROPERTY_NOT_WRITTEN);
}

/// The writable tables are internally consistent: every category is a canonical
/// `Ifc*` token, every type row points at a declared category, and the type
/// derivation is one-to-one for a category.
#[test]
fn every_writable_category_is_canonical_and_has_a_known_shape() {
    for category in BIM_CATEGORIES {
        assert!(category.starts_with("Ifc"), "{category}");
        assert_eq!(
            category_token(&category.to_ascii_uppercase()),
            Some(*category)
        );
    }
    for (type_class, category) in BIM_TYPE_CLASSES {
        assert!(category_token(category).is_some(), "{category}");
        assert_eq!(type_class_for(category), Some(*type_class));
    }
    assert!(BIM_CATEGORIES.contains(&"IfcSpace"));
}
