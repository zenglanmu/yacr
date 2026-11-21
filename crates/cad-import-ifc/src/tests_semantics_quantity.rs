//! IFC element-quantity contract tests (ADR-0042 Phase 3).
//!
//! Every assertion is about the *contract*: an `IfcElementQuantity` becomes one
//! `cad_db::BimPropertySet` under its own source name, a number keeps the unit
//! and type the file declared, and an unreadable quantity is reported instead of
//! being invented. These are synthetic fixtures, not vendor compatibility
//! evidence.

use std::sync::Arc;

use crate::*;
use cad_db::{BimElement, BimPropertySet, BimValue, BIM_MAX_PROPERTIES_PER_SET};
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

fn the_wall(drawing: &ImportedDrawing) -> &BimElement {
    drawing
        .database
        .bim_elements()
        .values()
        .next()
        .expect("the wall has a BIM block")
}

/// One `IfcWall` with a single triangle and one `IfcElementQuantity`.
///
/// The project declares millimetre lengths, square-metre areas, cubic-metre
/// volumes and kilogram masses, so a stored unit symbol is the file's own — no
/// test depends on a guessed default.
fn import_quantity_set(
    set_name: &str,
    quantity_refs: &str,
    quantity_entities: &str,
) -> ImportedDrawing {
    let data = format!(
        "ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('ViewDefinition [CoordinationView]'),'2;1');
FILE_NAME('synthetic-quantities.ifc','2026-11-21T00:00:00',('yacr'),('yacr'),'yacr synthetic fixture','yacr','');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCPROJECT('2O2Fr$t4X7Zf8NOew3FLQA',$,'Quantity project',$,$,$,$,(#2),#9);
#2=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#3,$);
#3=IFCAXIS2PLACEMENT3D(#4,#5,#6);
#4=IFCCARTESIANPOINT((0.,0.,0.));
#5=IFCDIRECTION((0.,0.,1.));
#6=IFCDIRECTION((1.,0.,0.));
#7=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#8=IFCSIUNIT(*,.AREAUNIT.,$,.SQUARE_METRE.);
#17=IFCSIUNIT(*,.VOLUMEUNIT.,$,.CUBIC_METRE.);
#18=IFCSIUNIT(*,.MASSUNIT.,.KILO.,.GRAM.);
#9=IFCUNITASSIGNMENT((#7,#8,#17,#18));
#10=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(1.,0.,0.),(0.,1.,0.)),$);
#11=IFCTRIANGULATEDFACESET(#10,$,.T.,((1,2,3)),$);
#12=IFCSHAPEREPRESENTATION(#2,'Body','Tessellation',(#11));
#13=IFCPRODUCTDEFINITIONSHAPE($,$,(#12));
#14=IFCLOCALPLACEMENT($,#15);
#15=IFCAXIS2PLACEMENT3D(#4,$,$);
#16=IFCWALL('2O2Fr$t4X7Zf8NOew3FLQB',$,'Quantity wall',$,$,#14,#13,'5',.SOLIDWALL.);
#20=IFCELEMENTQUANTITY('2O2Fr$t4X7Zf8NOew3FLQC',$,'{set_name}',$,$,({quantity_refs}));
{quantity_entities}
#90=IFCRELDEFINESBYPROPERTIES('2O2Fr$t4X7Zf8NOew3FLQD',$,$,$,(#16),#20);
ENDSEC;
END-ISO-10303-21;
"
    );
    import_bytes(data.into_bytes())
}

fn the_only_set(drawing: &ImportedDrawing) -> &BimPropertySet {
    let psets = &the_wall(drawing).psets;
    assert_eq!(psets.len(), 1, "one quantity set, one property set");
    &psets[0]
}

#[test]
fn an_element_quantity_set_is_stored_with_its_source_name() {
    let drawing = import_quantity_set(
        "Qto_WallBaseQuantities",
        "#21,#22,#23",
        "#21=IFCQUANTITYLENGTH('Length',$, $, 3000., $);
#22=IFCQUANTITYAREA('NetSideArea',$, $, 7.5, $);
#23=IFCQUANTITYVOLUME('NetVolume',$, $, 1.25, $);",
    );
    let set = the_only_set(&drawing);

    // The source's own set name is kept: no `Pset_`/`Qto_` rewriting and no
    // prefix that would hide where the row came from.
    assert_eq!(set.name, "Qto_WallBaseQuantities");
    let names: Vec<&str> = set
        .properties
        .iter()
        .map(|property| property.name.as_str())
        .collect();
    assert_eq!(names, vec!["Length", "NetSideArea", "NetVolume"]);

    let codes = diagnostic_codes(&drawing);
    assert!(
        codes.contains(&DIAG_BIM_QUANTITY_SET_STORED),
        "the stored set is reported as a lossless note: {:?}",
        drawing.report.diagnostics
    );
    assert!(
        !codes.contains(&DIAG_BIM_QUANTITY_SET_NOT_STORED),
        "a stored quantity set must not be reported as unreadable: {:?}",
        drawing.report.diagnostics
    );
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn a_length_quantity_keeps_its_unit_and_type() {
    let drawing = import_quantity_set(
        "Qto_WallBaseQuantities",
        "#21,#22",
        "#21=IFCQUANTITYLENGTH('Length',$, $, 3000., $);
#22=IFCQUANTITYWEIGHT('GrossWeight',$, $, 120.5, $);",
    );
    let set = the_only_set(&drawing);

    // The number is the file's number: 3000 mm is not silently rewritten to
    // 3 m. The unit symbol is what carries the scale.
    assert_eq!(
        set.properties[0],
        cad_db::BimProperty {
            name: "Length".into(),
            value: BimValue::Real {
                value: 3000.0,
                ifc_type: Some("IfcQuantityLength".into()),
                unit: Some("mm".into()),
            },
        }
    );
    // A weight resolves through the project's mass unit, not the length unit.
    assert_eq!(
        set.properties[1],
        cad_db::BimProperty {
            name: "GrossWeight".into(),
            value: BimValue::Real {
                value: 120.5,
                ifc_type: Some("IfcQuantityWeight".into()),
                unit: Some("kg".into()),
            },
        }
    );
}

#[test]
fn a_count_quantity_is_an_integer() {
    let drawing = import_quantity_set(
        "Qto_WallBaseQuantities",
        "#21",
        "#21=IFCQUANTITYCOUNT('Count',$, $, 3, $);",
    );
    let set = the_only_set(&drawing);
    assert_eq!(
        set.properties,
        vec![cad_db::BimProperty {
            name: "Count".into(),
            value: BimValue::Integer(3),
        }]
    );
}

#[test]
fn an_unresolvable_unit_leaves_unit_none_not_metres() {
    // `#999` is not declared: the quantity's own `Unit` override cannot be
    // resolved. Falling back to the project default (mm) or to "m" would
    // relabel a number whose scale is unknown.
    let drawing = import_quantity_set(
        "Qto_WallBaseQuantities",
        "#21",
        "#21=IFCQUANTITYLENGTH('Length',$,#999,3000.,$);",
    );
    let set = the_only_set(&drawing);
    assert_eq!(
        set.properties,
        vec![cad_db::BimProperty {
            name: "Length".into(),
            value: BimValue::Real {
                value: 3000.0,
                ifc_type: Some("IfcQuantityLength".into()),
                unit: None,
            },
        }]
    );
}

#[test]
fn a_quantity_over_budget_is_dropped_and_reported() {
    let mut entities = String::new();
    let mut refs = Vec::new();
    for index in 0..=BIM_MAX_PROPERTIES_PER_SET {
        let id = 200 + index;
        entities.push_str(&format!(
            "#{id}=IFCQUANTITYLENGTH('Quantity_{index}',$, $, {}.0, $);\n",
            index + 1
        ));
        refs.push(format!("#{id}"));
    }
    let drawing = import_quantity_set("Qto_WallBaseQuantities", &refs.join(","), &entities);
    let set = the_only_set(&drawing);
    assert_eq!(
        set.properties.len(),
        BIM_MAX_PROPERTIES_PER_SET,
        "the first {BIM_MAX_PROPERTIES_PER_SET} quantities are kept; the surplus is dropped whole"
    );
    assert_eq!(set.properties[0].name, "Quantity_0");
    assert_eq!(
        set.properties[BIM_MAX_PROPERTIES_PER_SET - 1].name,
        format!("Quantity_{}", BIM_MAX_PROPERTIES_PER_SET - 1)
    );

    let codes = diagnostic_codes(&drawing);
    assert!(
        codes.contains(&DIAG_BIM_BUDGET_EXCEEDED),
        "diagnostics: {:?}",
        drawing.report.diagnostics
    );
}

#[test]
fn a_complex_quantity_is_reported_not_fabricated() {
    // `IfcPhysicalComplexQuantity` aggregates other quantities; it has no single
    // value, so the set is reported and left out rather than flattened.
    let drawing = import_quantity_set(
        "Qto_WallBaseQuantities",
        "#21",
        "#21=IFCPHYSICALCOMPLEXQUANTITY('Layers',$,(#22),'layer',$,$);
#22=IFCQUANTITYLENGTH('Inner',$, $, 1., $);",
    );
    assert!(
        the_wall(&drawing).psets.is_empty(),
        "no fabricated quantity set: {:?}",
        the_wall(&drawing).psets
    );
    let codes = diagnostic_codes(&drawing);
    assert!(
        codes.contains(&DIAG_BIM_QUANTITY_SET_NOT_STORED),
        "diagnostics: {:?}",
        drawing.report.diagnostics
    );
    assert!(
        !codes.contains(&DIAG_BIM_QUANTITY_SET_STORED),
        "nothing was stored, so the lossless note must not claim otherwise: {:?}",
        drawing.report.diagnostics
    );
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}
