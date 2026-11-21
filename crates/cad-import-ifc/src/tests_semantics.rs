//! IFC BIM semantics contract tests (ADR-0042 D7).
//!
//! Every assertion here is about the *contract*: which file fields reach
//! `cad_db::BimElement` / `cad_db::BimDefinitions`, and which unsupported shapes
//! are reported instead of fabricated. These are synthetic fixtures, not vendor
//! compatibility evidence.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::*;
use cad_db::{BimDefinition, BimElement, BimEvidence, BimKey, BimValue, BIM_MAX_PSETS_PER_ELEMENT};
use cad_domain::{Completeness, DatabaseId, DocumentId};
use cad_import_core::{ImportLimits, ImportRequest, ImportedDrawing};

fn request_bytes(bytes: Vec<u8>) -> ImportRequest {
    ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    }
}

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/ifc")
        .join(name);
    std::fs::read(path).expect("fixture is committed")
}

fn import_fixture(name: &str) -> ImportedDrawing {
    import_bytes(fixture(name))
}

fn import_bytes(bytes: Vec<u8>) -> ImportedDrawing {
    IfcImporter::new()
        .import(&request_bytes(bytes), &|| false)
        .unwrap_or_else(|error| panic!("fixture must import: {error:?}"))
}

fn diagnostic_codes(drawing: &ImportedDrawing) -> Vec<&str> {
    drawing
        .report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn element_by_source_id<'a>(drawing: &'a ImportedDrawing, id: &str) -> &'a BimElement {
    drawing
        .database
        .bim_elements()
        .values()
        .find(|bim| bim.source.id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no BIM element with source id {id}"))
}

#[test]
fn ifc_wall_globalid_storey_type_material_pset_are_stored() {
    let drawing = import_fixture("synthetic-wall-semantics.ifc");
    let wall_a = element_by_source_id(&drawing, "2O2Fr$t4X7Zf8NOew3FLSJ");

    // The BIM pass does not disturb the geometry lane: two products, two
    // 12-triangle boxes.
    assert_eq!(drawing.database.entity_count(), 2);
    let triangles: usize = drawing
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            cad_domain::SemanticGeometry::Mesh(mesh) => Some(mesh.triangles.len()),
            _ => None,
        })
        .sum();
    assert_eq!(triangles, 24);

    // Identity: the file's own GlobalId / Tag / Name and the project's GlobalId
    // are the document id, not an invented identity.
    assert_eq!(
        wall_a.source.connector, "ifc",
        "the source connector is the IFC lane"
    );
    assert_eq!(
        wall_a.source.document_id.as_deref(),
        Some("2O2Fr$t4X7Zf8NOew3FLSE"),
        "the document id is the project's GlobalId"
    );
    assert_eq!(wall_a.source.id.as_deref(), Some("2O2Fr$t4X7Zf8NOew3FLSJ"));
    assert_eq!(wall_a.source.native_id.as_deref(), Some("22"));
    assert_eq!(wall_a.name.as_deref(), Some("Wall A"));
    assert_eq!(wall_a.category.as_deref(), Some("IfcWall"));
    assert_eq!(wall_a.evidence, BimEvidence::Declared);

    // Level: the contained storey, elevation converted from millimetres to
    // metres (the document's stored unit).
    match drawing
        .database
        .bim_definition(wall_a.level.as_deref().expect("Wall A has a level"))
        .expect("the level definition is stored")
    {
        BimDefinition::Level {
            name, elevation, ..
        } => {
            assert_eq!(name, "Level 0");
            assert!(
                elevation.expect("Level 0 declares an elevation").abs() < 1e-9,
                "0 mm is 0 m"
            );
        }
        other => panic!("expected a level definition, got {other:?}"),
    }

    // The second wall reaches its storey through IfcRelAggregates (not
    // containment), and its 3000 mm elevation must be stored as 3 m.
    let wall_b = element_by_source_id(&drawing, "2O2Fr$t4X7Zf8NOew3FLSK");
    match drawing
        .database
        .bim_definition(wall_b.level.as_deref().expect("Wall B has a level"))
        .expect("the second level definition is stored")
    {
        BimDefinition::Level {
            name, elevation, ..
        } => {
            assert_eq!(name, "Level 1");
            assert!(
                (elevation.expect("Level 1 declares an elevation") - 3.0).abs() < 1e-9,
                "3000 mm must be stored as 3 m, not 3000"
            );
        }
        other => panic!("expected a level definition, got {other:?}"),
    }

    // Type: class is the type object's own IFC class; category is the
    // occurrence's class. The type PSet is part of the definition.
    match drawing
        .database
        .bim_definition(wall_a.element_type.as_deref().expect("Wall A is typed"))
        .expect("the type definition is stored")
    {
        BimDefinition::Type {
            name,
            class,
            category,
            psets,
            ..
        } => {
            assert_eq!(name, "Basic Wall 200mm");
            assert_eq!(class.as_deref(), Some("IfcWallType"));
            assert_eq!(category.as_deref(), Some("IfcWall"));
            let type_pset = psets
                .iter()
                .find(|set| set.name == "Pset_WallCommon_Type")
                .expect("the type PSet is stored on the type definition");
            assert_eq!(
                type_pset.properties,
                vec![cad_db::BimProperty {
                    name: "TypeMark".into(),
                    value: BimValue::Text("W-200".into()),
                }]
            );
        }
        other => panic!("expected a type definition, got {other:?}"),
    }

    // Material.
    match drawing
        .database
        .bim_definition(wall_a.material.as_deref().expect("Wall A has material"))
        .expect("the material definition is stored")
    {
        BimDefinition::Material { name, .. } => assert_eq!(name, "Concrete"),
        other => panic!("expected a material definition, got {other:?}"),
    }

    // Occurrence PSets: only the two single values are stored; the
    // IfcComplexProperty is not flattened into a string.
    assert_eq!(wall_a.psets.len(), 1, "one occurrence property set");
    let pset = &wall_a.psets[0];
    assert_eq!(pset.name, "Pset_WallCommon");
    assert_eq!(
        pset.properties,
        vec![
            cad_db::BimProperty {
                name: "IsExternal".into(),
                value: BimValue::Bool(true),
            },
            cad_db::BimProperty {
                name: "Reference".into(),
                value: BimValue::Text("W-200-A".into()),
            },
        ]
    );

    // The relation reading is a Partial ceiling (ADR-0042 §3), and the import
    // says why: complex values are unsupported and BIM data has no vendor
    // cross-validation.
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
    let codes = diagnostic_codes(&drawing);
    assert!(
        codes.contains(&DIAG_BIM_PROPERTY_VALUE_UNSUPPORTED),
        "diagnostics: {:?}",
        drawing.report.diagnostics
    );
    let capability = &drawing.report.capabilities[0];
    assert_eq!(
        capability.semantic,
        cad_domain::SupportStatus::Partial,
        "BIM semantics must not upgrade the capability row"
    );
}

/// One `IfcWall` whose Body representation has two independently textured
/// `IfcTriangulatedFaceSet` items: ifc-lite emits one sub-mesh per textured face
/// set, so the adapter creates two geometry rows for one IFC product.
const SPLIT_TEXTURED_WALL: &str = r#"ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('ViewDefinition [CoordinationView]'),'2;1');
FILE_NAME('synthetic-split-textured-wall.ifc','2026-11-21T00:00:00',('yacr'),('yacr'),'yacr synthetic fixture','yacr','');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCPROJECT('2O2Fr$t4X7Zf8NOew3FLTA',$,'Split submesh project',$,$,$,$,(#2),#9);
#2=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#3,$);
#3=IFCAXIS2PLACEMENT3D(#4,#5,#6);
#4=IFCCARTESIANPOINT((0.,0.,0.));
#5=IFCDIRECTION((0.,0.,1.));
#6=IFCDIRECTION((1.,0.,0.));
#7=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#8=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);
#9=IFCUNITASSIGNMENT((#7,#8));
#10=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(1.,0.,0.),(1.,1.,0.),(0.,1.,0.)),$);
#11=IFCTRIANGULATEDFACESET(#10,$,.T.,((1,2,3),(1,3,4)),$);
#12=IFCCARTESIANPOINTLIST3D(((2.,0.,0.),(3.,0.,0.),(3.,1.,0.),(2.,1.,0.)),$);
#13=IFCTRIANGULATEDFACESET(#12,$,.T.,((1,2,3),(1,3,4)),$);
#14=IFCSHAPEREPRESENTATION(#2,'Body','Tessellation',(#11,#13));
#15=IFCPRODUCTDEFINITIONSHAPE($,$,(#14));
#16=IFCLOCALPLACEMENT($,#17);
#17=IFCAXIS2PLACEMENT3D(#4,$,$);
#18=IFCWALL('2O2Fr$t4X7Zf8NOew3FLTB',$,'Split wall',$,$,#16,#15,'7',.SOLIDWALL.);
#20=IFCPIXELTEXTURE(.T.,.T.,'MODULATE',$,$,2,1,3,("0FF0000","000FF00"));
#21=IFCTEXTUREVERTEXLIST(((0.,0.),(1.,0.),(1.,1.),(0.,1.)));
#22=IFCINDEXEDTRIANGLETEXTUREMAP((#20),#11,#21,((1,2,3),(1,3,4)));
#30=IFCPIXELTEXTURE(.T.,.T.,'MODULATE',$,$,2,1,3,("0FF0000","000FF00"));
#31=IFCTEXTUREVERTEXLIST(((0.,0.),(1.,0.),(1.,1.),(0.,1.)));
#32=IFCINDEXEDTRIANGLETEXTUREMAP((#30),#13,#31,((1,2,3),(1,3,4)));
ENDSEC;
END-ISO-10303-21;
"#;

#[test]
fn one_ifc_product_stays_one_bim_element() {
    let drawing = import_bytes(SPLIT_TEXTURED_WALL.as_bytes().to_vec());
    assert_eq!(
        drawing.database.entity_count(),
        2,
        "two textured sub-meshes become two geometry rows"
    );
    assert_eq!(
        drawing.database.bim_elements().len(),
        2,
        "both rows carry the product's BIM block"
    );
    let elements: Vec<&BimElement> = drawing.database.bim_elements().values().collect();
    assert_eq!(
        elements[0].source, elements[1].source,
        "one IFC product has one source identity"
    );
    assert_eq!(elements[0].category.as_deref(), Some("IfcWall"));
    assert_eq!(
        elements[0].source.id.as_deref(),
        Some("2O2Fr$t4X7Zf8NOew3FLTB")
    );
    assert_eq!(elements[0].source.native_id.as_deref(), Some("7"));

    // The logical component is the source equivalence class, not the geometry
    // row: two rows, one `BimKey`.
    let keys: BTreeSet<BimKey> = drawing
        .database
        .bim_elements()
        .keys()
        .map(|entity| drawing.database.bim_key(*entity).expect("BIM key"))
        .collect();
    assert_eq!(keys.len(), 1, "one product must not become two components");
}

#[test]
fn complex_property_is_reported_not_fabricated() {
    let drawing = import_fixture("synthetic-wall-semantics.ifc");
    let wall_a = element_by_source_id(&drawing, "2O2Fr$t4X7Zf8NOew3FLSJ");
    let pset = &wall_a.psets[0];
    assert_eq!(pset.properties.len(), 2, "only the two single values");
    for name in ["AcousticRating", "Layer"] {
        assert!(
            !pset.properties.iter().any(|property| property.name == name),
            "{name} must not be fabricated from the complex property"
        );
    }
    let codes = diagnostic_codes(&drawing);
    assert!(
        codes.contains(&DIAG_BIM_PROPERTY_VALUE_UNSUPPORTED),
        "diagnostics: {:?}",
        drawing.report.diagnostics
    );
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn a_product_without_relations_has_source_and_category_only() {
    let drawing = import_fixture("synthetic-wall.ifc");
    assert_eq!(drawing.database.bim_elements().len(), 1);
    let bim = drawing
        .database
        .bim_elements()
        .values()
        .next()
        .expect("the wall has a BIM block");

    assert_eq!(
        bim.source.document_id.as_deref(),
        Some("2O2Fr$t4X7Zf8NOew3FLOA")
    );
    assert_eq!(bim.source.id.as_deref(), Some("2O2Fr$t4X7Zf8NOew3FLOB"));
    assert_eq!(bim.source.native_id, None, "the fixture's Tag is unset");
    assert_eq!(bim.name.as_deref(), Some("Wall"));
    assert_eq!(bim.category.as_deref(), Some("IfcWall"));
    assert_eq!(bim.evidence, BimEvidence::Declared);
    assert!(bim.level.is_none(), "no spatial relation was declared");
    assert!(bim.element_type.is_none(), "no type relation was declared");
    assert!(bim.material.is_none(), "no material relation was declared");
    assert!(bim.psets.is_empty(), "no property set was declared");
    assert!(drawing.database.bim_definitions().is_empty());
}

#[test]
fn bim_budget_exceeded_keeps_no_dangling_reference() {
    // One wall with 65 property sets (one over the 64-per-element budget) and a
    // type whose name is over the 4 KiB string budget. The surplus PSet and the
    // type row must be dropped, the element must not reference either, and the
    // import must say so.
    let mut data = String::from(
        "ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('ViewDefinition [CoordinationView]'),'2;1');
FILE_NAME('synthetic-bim-budget.ifc','2026-11-21T00:00:00',('yacr'),('yacr'),'yacr synthetic fixture','yacr','');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCPROJECT('2O2Fr$t4X7Zf8NOew3FLXZ',$,'Budget project',$,$,$,$,(#2),#9);
#2=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#3,$);
#3=IFCAXIS2PLACEMENT3D(#4,#5,#6);
#4=IFCCARTESIANPOINT((0.,0.,0.));
#5=IFCDIRECTION((0.,0.,1.));
#6=IFCDIRECTION((1.,0.,0.));
#7=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#8=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);
#9=IFCUNITASSIGNMENT((#7,#8));
#10=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(1.,0.,0.),(0.,1.,0.)),$);
#11=IFCTRIANGULATEDFACESET(#10,$,.T.,((1,2,3)),$);
#12=IFCSHAPEREPRESENTATION(#2,'Body','Tessellation',(#11));
#13=IFCPRODUCTDEFINITIONSHAPE($,$,(#12));
#14=IFCLOCALPLACEMENT($,#15);
#15=IFCAXIS2PLACEMENT3D(#4,$,$);
#16=IFCWALL('2O2Fr$t4X7Zf8NOew3FLZY',$,'Budget wall',$,$,#14,#13,'9',.SOLIDWALL.);
#17=IFCWALLTYPE('2O2Fr$t4X7Zf8NOew3FLZZ',$,'",
    );
    data.push_str(&"A".repeat(5000));
    data.push_str(
        "',$,$,$,$,$,$,.STANDARD.);
#18=IFCRELDEFINESBYTYPE('2O2Fr$t4X7Zf8NOew3FLZY1',$,$,$,(#16),#17);
",
    );
    for index in 0..(BIM_MAX_PSETS_PER_ELEMENT + 1) {
        let set = 100 + index * 3;
        let property = set + 1;
        let relation = set + 2;
        data.push_str(&format!(
            "#{set}=IFCPROPERTYSET('2O2Fr$t4X7Zf8NOew3F{index:02}',$,'Pset_Budget_{index}',$,(#{property}));\n\
             #{property}=IFCPROPERTYSINGLEVALUE('Budget_{index}',$,IFCLABEL('v'),$);\n\
             #{relation}=IFCRELDEFINESBYPROPERTIES('2O2Fr$t4X7Zf8NOew3G{index:02}',$,$,$,(#16),#{set});\n"
        ));
    }
    data.push_str("ENDSEC;\nEND-ISO-10303-21;\n");

    let drawing = import_bytes(data.into_bytes());
    let bim = element_by_source_id(&drawing, "2O2Fr$t4X7Zf8NOew3FLZY");
    assert_eq!(
        bim.psets.len(),
        BIM_MAX_PSETS_PER_ELEMENT,
        "the first 64 property sets are kept; the 65th is dropped whole"
    );
    assert!(
        bim.element_type.is_none(),
        "an over-budget type name must not be referenced"
    );

    // Nothing in the document points at a definition that was not inserted.
    for element in drawing.database.bim_elements().values() {
        for reference in [
            element.level.as_deref(),
            element.element_type.as_deref(),
            element.material.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            assert!(
                drawing.database.bim_definition(reference).is_some(),
                "dangling BIM reference {reference}"
            );
        }
    }

    let codes = diagnostic_codes(&drawing);
    assert!(
        codes.contains(&DIAG_BIM_BUDGET_EXCEEDED),
        "diagnostics: {:?}",
        drawing.report.diagnostics
    );
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

/// One `IfcSpace` with a single tessellated triangle: a spatial element is an
/// ordinary drawable product, not a storey, and its category is its own class.
const IFC_SPACE: &str = r#"ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('ViewDefinition [CoordinationView]'),'2;1');
FILE_NAME('synthetic-space.ifc','2026-11-21T00:00:00',('yacr'),('yacr'),'yacr synthetic fixture','yacr','');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCPROJECT('2O2Fr$t4X7Zf8NOew3FLSA',$,'Space project',$,$,$,$,(#2),#9);
#2=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#3,$);
#3=IFCAXIS2PLACEMENT3D(#4,#5,#6);
#4=IFCCARTESIANPOINT((0.,0.,0.));
#5=IFCDIRECTION((0.,0.,1.));
#6=IFCDIRECTION((1.,0.,0.));
#7=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#8=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);
#9=IFCUNITASSIGNMENT((#7,#8));
#10=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(1.,0.,0.),(0.,1.,0.)),$);
#11=IFCTRIANGULATEDFACESET(#10,$,.T.,((1,2,3)),$);
#12=IFCSHAPEREPRESENTATION(#2,'Body','Tessellation',(#11));
#13=IFCPRODUCTDEFINITIONSHAPE($,$,(#12));
#14=IFCLOCALPLACEMENT($,#15);
#15=IFCAXIS2PLACEMENT3D(#4,$,$);
#16=IFCSPACE('2O2Fr$t4X7Zf8NOew3FLSB',$,'Room',$,$,#14,#13,$,.ELEMENT.,.INTERNAL.,$);
ENDSEC;
END-ISO-10303-21;
"#;

#[test]
fn an_ifc_space_is_an_ordinary_element_with_its_category() {
    let drawing = import_bytes(IFC_SPACE.as_bytes().to_vec());
    assert_eq!(drawing.database.entity_count(), 1);
    let bim = drawing
        .database
        .bim_elements()
        .values()
        .next()
        .expect("the space has a BIM block");
    assert_eq!(bim.category.as_deref(), Some("IfcSpace"));
    assert_eq!(bim.source.id.as_deref(), Some("2O2Fr$t4X7Zf8NOew3FLSB"));
    assert_eq!(bim.name.as_deref(), Some("Room"));
    assert_eq!(
        bim.source.native_id, None,
        "IfcSpace is not an IfcElement, so attribute 7 is LongName, not Tag"
    );
    assert!(bim.level.is_none());
    assert!(bim.element_type.is_none());
    assert!(bim.material.is_none());
    assert!(bim.psets.is_empty());
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

#[test]
fn import_without_bim_relations_stays_unchanged() {
    // The pre-ADR fixtures declare no spatial/type/material/property relations.
    // They gain the product's own source/category/name and nothing else, and
    // their existing completeness verdict is untouched.
    for name in [
        "synthetic-wall-opening.ifc",
        "synthetic-two-walls-ifc2x3.ifc",
    ] {
        let drawing = import_fixture(name);
        assert!(
            drawing.database.bim_definitions().is_empty(),
            "{name} declares no shared BIM definition"
        );
        assert!(!drawing.database.bim_elements().is_empty());
        for bim in drawing.database.bim_elements().values() {
            assert!(bim.level.is_none(), "{name}");
            assert!(bim.element_type.is_none(), "{name}");
            assert!(bim.material.is_none(), "{name}");
            assert!(bim.psets.is_empty(), "{name}");
            assert_eq!(bim.evidence, BimEvidence::Declared, "{name}");
        }
        assert!(
            !diagnostic_codes(&drawing)
                .iter()
                .any(|code| code.starts_with("import.ifc.bim.")),
            "{name} must not gain a BIM diagnostic: {:?}",
            drawing.report.diagnostics
        );
    }

    let clean = import_fixture("synthetic-wall.ifc");
    assert_eq!(
        clean.report.completeness,
        Completeness::Complete,
        "a geometry-only wall stays Complete: diagnostics: {:?}",
        clean.report.diagnostics
    );
}
