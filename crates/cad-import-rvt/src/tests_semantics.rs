//! Contract tests for the RVT BIM mapping (ADR-0042 D8).
//!
//! Every test here builds an [`IfcModel`] by hand and asserts the mapping field
//! by field. The model is what `rvt::ifc::run_export` returns, so this is the
//! exact input [`map_model`] sees in production.
//!
//! The env-gated test at the bottom is **not** a substitute for a real-project
//! verification: an unset `YACR_TEST_RVT` returns early, and that skip is not
//! evidence. A local sample can be supplied explicitly, for example
//! `YACR_TEST_RVT=~/下载/rmeadvancedsampleproject.rvt`, but no path is committed
//! or hard-coded here.

use rvt::ifc::entities::{IfcEntity, Property, PropertySet, PropertyValue};
use rvt::ifc::{IfcModel, MaterialInfo, Storey};

use cad_db::BimDefinitionKind;
use cad_import_core::{ImportLimits, Importer};

use super::{map_model, normalize_ifc_category};
use crate::tests::{fixture, glb_parts, rebuild_glb, request};
use crate::{read_glb, RvtImporter};

/// A minimal `BuildingElement` with the fields this lane reads; every other
/// field is `None`.
fn building_element(
    ifc_type: &str,
    name: &str,
    type_guid: Option<&str>,
    storey_index: Option<usize>,
    material_index: Option<usize>,
    property_set: Option<PropertySet>,
) -> IfcEntity {
    IfcEntity::BuildingElement {
        ifc_type: ifc_type.to_string(),
        name: name.to_string(),
        type_guid: type_guid.map(str::to_string),
        predefined_type: None,
        storey_index,
        material_index,
        property_set,
        location_feet: None,
        rotation_radians: None,
        extrusion: None,
        host_element_index: None,
        material_layer_set_index: None,
        material_profile_set_index: None,
        solid_shape: None,
        representation_map_index: None,
    }
}

fn wall_property_set() -> PropertySet {
    PropertySet {
        name: "Pset_WallCommon".into(),
        properties: vec![
            Property {
                name: "IsExternal".into(),
                value: PropertyValue::Boolean(true),
            },
            Property {
                name: "Reference".into(),
                value: PropertyValue::PositiveLengthFeet(0.2),
            },
        ],
    }
}

/// One wall with a storey, a type, a material and one property set — the case
/// every field of the mapping is exercised on.
fn full_model() -> IfcModel {
    let mut model = IfcModel {
        building_storeys: vec![Storey {
            name: "Level 1".into(),
            elevation_feet: 10.0,
        }],
        materials: vec![MaterialInfo {
            name: "Concrete".into(),
            color_packed: None,
            transparency: None,
        }],
        entities: vec![
            building_element(
                "IFCWALL",
                "Basic Wall:Generic 200mm",
                Some("100"),
                Some(0),
                Some(0),
                Some(wall_property_set()),
            ),
            // A further declared set of the same element, written as its own
            // record after the elements.
            IfcEntity::ElementPropertySet {
                element: 0,
                set: PropertySet {
                    name: "Pset_Revit_Other".into(),
                    properties: vec![Property {
                        name: "Category".into(),
                        value: PropertyValue::Label("Walls".into()),
                    }],
                },
            },
        ],
        ..IfcModel::default()
    };
    // ElementId 100 is the element's Tag (`type_guid`); its type's ElementId is 7.
    model.element_type_ids.insert(100, 7);
    model.type_property_sets.insert(
        7,
        vec![PropertySet {
            name: "Pset_Revit_Type".into(),
            properties: vec![Property {
                name: "TypeName".into(),
                value: PropertyValue::Text("Generic 200mm".into()),
            }],
        }],
    );
    model.global_ids.elements.insert(0, "3vB2GlobalId".into());
    model.global_ids.storeys.insert(0, "storeyGlobalId".into());
    model
}

#[test]
fn semantics_map_storeys_types_materials_and_psets() {
    let model = full_model();
    let semantics = map_model(&model);
    let bim = semantics
        .element_for_index(Some(0))
        .expect("the wall has a BIM block");

    assert_eq!(bim.category.as_deref(), Some("IfcWall"));
    assert_eq!(bim.name.as_deref(), Some("Basic Wall:Generic 200mm"));
    // The document GUID changes on every save and is never used as the
    // document id; the element's own GlobalId is its source id.
    assert_eq!(bim.source.connector, "rvt");
    assert_eq!(bim.source.document_id, None);
    assert_eq!(bim.source.id.as_deref(), Some("3vB2GlobalId"));
    assert_eq!(bim.source.native_id.as_deref(), Some("100"));

    let level_id = bim.level.as_deref().expect("the wall has a level");
    assert_eq!(level_id, "level:rvt:storeyGlobalId");
    let Some(cad_db::BimDefinition::Level {
        name,
        elevation,
        evidence,
        ..
    }) = semantics.definitions().get(level_id)
    else {
        panic!("the level definition is a level row");
    };
    assert_eq!(name, "Level 1");
    // Revit stores feet; the database stores metres.
    let elevation = elevation.expect("the storey declares an elevation");
    assert!(
        (elevation - 3.048).abs() < 1e-12,
        "10 ft is 3.048 m, got {elevation}"
    );
    assert_eq!(*evidence, cad_db::BimEvidence::Declared);

    let type_id = bim.element_type.as_deref().expect("the wall has a type");
    assert_eq!(type_id, "type:rvt:7");
    let Some(cad_db::BimDefinition::Type {
        name,
        category,
        psets,
        ..
    }) = semantics.definitions().get(type_id)
    else {
        panic!("the type definition is a type row");
    };
    // `Family:Type` -> the Type half names the type object.
    assert_eq!(name, "Generic 200mm");
    assert_eq!(category.as_deref(), Some("IfcWall"));
    assert_eq!(psets.len(), 1);
    assert_eq!(psets[0].name, "Pset_Revit_Type");
    assert_eq!(psets[0].properties[0].name, "TypeName");

    let material_id = bim.material.as_deref().expect("the wall has a material");
    assert_eq!(material_id, "material:rvt:Concrete");
    assert!(matches!(
        semantics.definitions().get(material_id),
        Some(cad_db::BimDefinition::Material { name, .. }) if name == "Concrete"
    ));

    assert_eq!(bim.psets.len(), 2);
    assert_eq!(bim.psets[0].name, "Pset_WallCommon");
    assert_eq!(bim.psets[0].properties[0].name, "IsExternal");
    assert_eq!(
        bim.psets[0].properties[0].value,
        cad_db::BimValue::Bool(true)
    );
    assert_eq!(bim.psets[0].properties[1].name, "Reference");
    let cad_db::BimValue::Real {
        value,
        ifc_type,
        unit,
    } = &bim.psets[0].properties[1].value
    else {
        panic!("a positive length is a real measure");
    };
    assert!(
        (*value - 0.2 * 0.3048).abs() < 1e-12,
        "0.2 ft is 0.06096 m, got {value}"
    );
    assert_eq!(ifc_type.as_deref(), Some("IfcPositiveLengthMeasure"));
    assert_eq!(unit.as_deref(), Some("m"));
    // The element's `ElementPropertySet` record belongs to the same element.
    assert_eq!(bim.psets[1].name, "Pset_Revit_Other");
    assert_eq!(
        bim.psets[1].properties[0].value,
        cad_db::BimValue::Text("Walls".into())
    );
    assert_eq!(bim.evidence, cad_db::BimEvidence::Declared);

    assert_eq!(*semantics.counts(), Default::default());
}

#[test]
fn one_element_with_several_nodes_shares_one_bim_element() {
    // `rvt` writes one node per body piece; every one names the same
    // `model.entities` index. Both rows must resolve to the same block.
    let (mut json, bin) = glb_parts(&fixture("synthetic-two-node.glb"));
    for node in [0, 2] {
        json["nodes"][node]["extras"] = serde_json::json!({
            "entityIndex": 0,
            "ifcType": "IFCWALL"
        });
    }
    let scene = read_glb(&rebuild_glb(&json, &bin), &ImportLimits::default(), &|| {
        false
    })
    .expect("the patched GLB reads");
    assert_eq!(scene.meshes.len(), 2, "one row per node/mesh instance");

    let semantics = map_model(&full_model());
    let first = semantics
        .element_for_index(scene.meshes[0].entity_index)
        .expect("node 0 names the wall's entity index");
    let second = semantics
        .element_for_index(scene.meshes[1].entity_index)
        .expect("node 2 names the wall's entity index");
    assert_eq!(scene.meshes[0].entity_index, Some(0));
    assert_eq!(scene.meshes[1].entity_index, Some(0));
    assert_eq!(first, second, "both rows carry the same BIM block");

    // A node with no `extras` has no BIM identity, never a guessed one.
    let (json, bin) = glb_parts(&fixture("synthetic-two-node.glb"));
    let scene = read_glb(&rebuild_glb(&json, &bin), &ImportLimits::default(), &|| {
        false
    })
    .expect("the unpatched GLB reads");
    for placed in &scene.meshes {
        assert_eq!(placed.entity_index, None);
        assert!(semantics.element_for_index(placed.entity_index).is_none());
    }
}

#[test]
fn a_missing_element_global_id_falls_back_to_the_native_id_and_reports() {
    let mut model = full_model();
    model.global_ids.elements.clear();
    // The upstream note: the document GUID changes on every save. It must not
    // become the element's source id just because the element has none.
    model.global_ids.document = Some("a-document-guid-that-changes-every-save".into());

    let semantics = map_model(&model);
    let bim = semantics.element_for_index(Some(0)).expect("BIM block");
    assert_eq!(bim.source.id, None);
    assert_eq!(bim.source.document_id, None);
    assert_eq!(bim.source.native_id.as_deref(), Some("100"));
    assert_eq!(semantics.counts().source_id_missing, 1);
}

#[test]
fn an_out_of_range_storey_index_leaves_level_none_and_reports() {
    let mut model = full_model();
    // The storey index 3 names no storey: `building_storeys` has one.
    model.entities = vec![building_element(
        "IFCWALL",
        "Basic Wall:Generic 200mm",
        Some("100"),
        Some(3),
        Some(0),
        None,
    )];
    let semantics = map_model(&model);
    let bim = semantics.element_for_index(Some(0)).expect("BIM block");
    assert_eq!(bim.level, None, "no storey is invented");
    assert_eq!(
        semantics.counts().storey_unresolved,
        1,
        "the unresolved storey is reported, not hidden"
    );
    assert!(
        semantics
            .definitions()
            .iter()
            .all(|row| row.kind() != BimDefinitionKind::Level),
        "an out-of-range storey creates no level row"
    );
}

#[test]
fn an_aggregate_is_counted_not_grouped() {
    let mut model = full_model();
    model.entities.push(IfcEntity::Aggregate {
        whole: 0,
        parts: vec![1, 2],
    });
    let semantics = map_model(&model);
    // Entities: the wall (0), its further property set (1), the aggregate (2).
    assert!(
        semantics.element_for_index(Some(2)).is_none(),
        "an Aggregate is not itself a building element"
    );
    assert_eq!(semantics.counts().aggregates_not_grouped, 1);
    assert_eq!(semantics.counts().aggregate_parts, 2);
}

#[test]
fn categories_are_normalized_to_ifc_spelling() {
    assert_eq!(
        normalize_ifc_category("IFCWALL").as_deref(),
        Some("IfcWall")
    );
    assert_eq!(
        normalize_ifc_category("IFCCURTAINWALL").as_deref(),
        Some("IfcCurtainWall")
    );
    assert_eq!(
        normalize_ifc_category("IFCBUILDINGELEMENTPROXY").as_deref(),
        Some("IfcBuildingElementProxy")
    );
    // Already normalized input is accepted as-is.
    assert_eq!(
        normalize_ifc_category("IfcWall").as_deref(),
        Some("IfcWall")
    );
    // An unknown STEP name gets a deterministic spelling, not a guessed one.
    assert_eq!(
        normalize_ifc_category("IFCWIDGET").as_deref(),
        Some("IfcWidget")
    );
    assert_eq!(normalize_ifc_category("  "), None);
}

#[test]
#[allow(non_snake_case)]
fn a_real_project_imports_bim_when_YACR_TEST_RVT_is_set() {
    // NOT evidence when skipped: an unset variable returns before any work.
    // Supply a local project explicitly (for example a Revit sample under
    // `~/下载/rmeadvancedsampleproject.rvt`); no path is hard-coded here.
    let Ok(path) = std::env::var("YACR_TEST_RVT") else {
        return;
    };
    let bytes = std::fs::read(&path).expect("YACR_TEST_RVT points at a readable file");
    let drawing = RvtImporter::new()
        .import(&request(bytes), &|| false)
        .expect("a real Revit project imports (preview)");

    let database = &drawing.database;
    let mut elements = 0usize;
    let mut with_category = 0usize;
    let mut with_level = 0usize;
    let mut with_type = 0usize;
    let mut with_material = 0usize;
    let mut element_psets = 0usize;
    for bim in database.bim_elements().values() {
        elements += 1;
        with_category += usize::from(bim.category.is_some());
        with_level += usize::from(bim.level.is_some());
        with_type += usize::from(bim.element_type.is_some());
        with_material += usize::from(bim.material.is_some());
        element_psets += bim.psets.len();
    }
    let mut levels = 0usize;
    let mut types = 0usize;
    let mut materials = 0usize;
    for row in database.bim_definitions() {
        match row.kind() {
            BimDefinitionKind::Level => levels += 1,
            BimDefinitionKind::Type => types += 1,
            BimDefinitionKind::Material => materials += 1,
        }
    }
    eprintln!(
        "real RVT BIM: elements={elements} levels={levels} types={types} \
         materials={materials} element_psets={element_psets} with_category={with_category} \
         with_level={with_level} with_type={with_type} with_material={with_material}"
    );

    assert!(
        elements > 0,
        "a project with bodies must yield BIM elements"
    );
    assert_eq!(
        with_category, elements,
        "every building element gets its IFC category"
    );
    assert!(
        drawing
            .report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == crate::DIAG_BIM_UNVERIFIED),
        "the BIM projection is reported as unverified"
    );
}
