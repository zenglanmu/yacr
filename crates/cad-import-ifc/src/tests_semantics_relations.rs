//! IFC structural-relation contract tests (ADR-0042 D3 tail / D7).
//!
//! Each test is about the contract between the file's own `IfcRel*` records and
//! [`cad_db::BimRelations`]: which edges are stored, how endpoints are keyed, and
//! which losses are reported instead of invented. Every fixture is an inline
//! synthetic IFC string, not vendor compatibility evidence.

use std::sync::Arc;

use cad_db::{BimKey, BimRelation, BimRelationKind};
use cad_domain::{Completeness, DatabaseId, DocumentId};
use cad_connector_core::{ImportLimits, ImportRequest, ImportedDrawing, Importer};
use ifc_lite_core::{build_entity_index, EntityDecoder};

use super::{collect_with_budget, Scan};
use crate::IfcImporter;

const PROJECT: &str = "2O2Fr$t4X7Zf8NOew3FLP0";
const WALL: &str = "2O2Fr$t4X7Zf8NOew3FLWA";
const AGG_CHILD_A: &str = "2O2Fr$t4X7Zf8NOew3FLAA";
const AGG_CHILD_B: &str = "2O2Fr$t4X7Zf8NOew3FLAB";
const NEST_CHILD: &str = "2O2Fr$t4X7Zf8NOew3FLNA";
const PROXY: &str = "2O2Fr$t4X7Zf8NOew3FLPY";
const STOREY: &str = "2O2Fr$t4X7Zf8NOew3FLST";
const GROUP: &str = "2O2Fr$t4X7Zf8NOew3FLGR";
const REL_AGG_1: &str = "2O2Fr$t4X7Zf8NOew3FLR1";
const REL_AGG_2: &str = "2O2Fr$t4X7Zf8NOew3FLR2";
const REL_NEST: &str = "2O2Fr$t4X7Zf8NOew3FLR3";
const REL_SPATIAL: &str = "2O2Fr$t4X7Zf8NOew3FLR4";
const REL_GROUP: &str = "2O2Fr$t4X7Zf8NOew3FLR5";
const REL_CONN: &str = "2O2Fr$t4X7Zf8NOew3FLR6";

/// One drawable `IfcWall` plus `<extra>` records appended to the same DATA section.
fn fixture(extra: &str) -> Vec<u8> {
    format!(
        r#"ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('ViewDefinition [CoordinationView]'),'2;1');
FILE_NAME('synthetic-relations.ifc','2026-11-21T00:00:00',('yacr'),('yacr'),'yacr synthetic fixture','yacr','');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCPROJECT('{PROJECT}',$,'Relations project',$,$,$,$,(#2),#9);
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
#16=IFCWALL('{WALL}',$,'Wall',$,$,#14,#13,$,.SOLIDWALL.);
{extra}
ENDSEC;
END-ISO-10303-21;
"#
    )
    .into_bytes()
}

fn import(bytes: Vec<u8>) -> ImportedDrawing {
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    IfcImporter::new()
        .import(&request, &|| false)
        .unwrap_or_else(|error| panic!("fixture must import: {error:?}"))
}

fn source(id: &str) -> BimKey {
    BimKey::Source {
        connector: "ifc".to_string(),
        document_id: PROJECT.to_string(),
        id: id.to_string(),
    }
}

fn relation<'a>(
    drawing: &'a ImportedDrawing,
    kind: BimRelationKind,
    parent: &BimKey,
    child: &BimKey,
) -> Option<&'a BimRelation> {
    drawing.database.bim_relations().get(&kind, parent, child)
}

fn diagnostic_codes(drawing: &ImportedDrawing) -> Vec<&str> {
    drawing
        .report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

#[test]
fn aggregates_and_nests_become_relations() {
    let extra = format!(
        r#"#20=IFCGROUP('{AGG_CHILD_A}',$,'Part A',$,$);
#21=IFCGROUP('{AGG_CHILD_B}',$,'Part B',$,$);
#22=IFCRELAGGREGATES('{REL_AGG_1}',$,'Assembly',$,#16,(#20,#21));
#23=IFCGROUP('{NEST_CHILD}',$,'Nested part',$,$);
#24=IFCRELNESTS('{REL_NEST}',$,'Nesting',$,#16,(#23));"#
    );
    let drawing = import(fixture(&extra));
    let wall = source(WALL);

    for child in [AGG_CHILD_A, AGG_CHILD_B] {
        let row = relation(&drawing, BimRelationKind::Aggregates, &wall, &source(child))
            .unwrap_or_else(|| panic!("aggregates {child} must be stored"));
        assert_eq!(row.label.as_deref(), Some("Assembly"));
        assert_eq!(row.source.as_ref(), Some(&source(REL_AGG_1)));
    }

    let nested = relation(&drawing, BimRelationKind::Nests, &wall, &source(NEST_CHILD))
        .expect("nests edge must be stored");
    assert_eq!(nested.label.as_deref(), Some("Nesting"));

    // Storing structural edges is not itself a loss: an otherwise complete
    // import stays Complete.
    assert_eq!(drawing.report.completeness, Completeness::Complete);
}

#[test]
fn containment_links_storey_and_element() {
    let extra = format!(
        r#"#20=IFCBUILDINGSTOREY('{STOREY}',$,'Level 0',$,$,#14,$,$,.ELEMENT.,0.);
#21=IFCRELCONTAINEDINSPATIALSTRUCTURE('{REL_SPATIAL}',$,$,$,(#16),#20);"#
    );
    let drawing = import(fixture(&extra));

    let row = relation(
        &drawing,
        BimRelationKind::Contains,
        &source(STOREY),
        &source(WALL),
    )
    .expect("spatial containment must be stored");
    assert_eq!(row.label, None, "an unnamed relation stores no label");
    assert_eq!(
        row.source.as_ref(),
        Some(&source(REL_SPATIAL)),
        "the relation's own GlobalId is the edge source"
    );
}

#[test]
fn a_group_assignment_is_a_group_relation() {
    let extra = format!(
        r#"#20=IFCGROUP('{GROUP}',$,'Structure group',$,$);
#21=IFCRELASSIGNSTOGROUP('{REL_GROUP}',$,'Assigned',$,(#16),$,#20);"#
    );
    let drawing = import(fixture(&extra));
    assert!(
        relation(
            &drawing,
            BimRelationKind::Group,
            &source(GROUP),
            &source(WALL)
        )
        .is_some(),
        "RelatingGroup is the parent and RelatedObjects the children"
    );
}

#[test]
fn a_connection_is_a_connects_relation() {
    let extra = format!(
        r#"#20=IFCBUILDINGELEMENTPROXY('{PROXY}',$,'Other element',$,$,$,$,$,.NOTDEFINED.);
#21=IFCRELCONNECTSELEMENTS('{REL_CONN}',$,'Connection',$,$,#16,#20);"#
    );
    let drawing = import(fixture(&extra));
    assert!(
        relation(
            &drawing,
            BimRelationKind::Connects,
            &source(WALL),
            &source(PROXY)
        )
        .is_some(),
        "RelatingElement is the parent and RelatedElement the child"
    );
}

#[test]
fn an_relation_endpoint_without_a_global_id_is_skipped_and_reported() {
    let extra = format!(
        r#"#20=IFCBUILDINGELEMENTPROXY($,$,'No global id',$,$,$,$,$,.NOTDEFINED.);
#21=IFCRELAGGREGATES('{REL_AGG_1}',$,$,$,#16,(#20));"#
    );
    let drawing = import(fixture(&extra));

    assert!(
        drawing.database.bim_relations().is_empty(),
        "no edge may be keyed by an invented endpoint identity"
    );
    assert!(
        diagnostic_codes(&drawing).contains(&crate::DIAG_BIM_RELATION_ENDPOINT_MISSING),
        "the skipped edge must be reported under its stable code"
    );
    assert!(
        matches!(drawing.report.completeness, Completeness::Partial(_)),
        "a relation that could not be read marks the import Partial"
    );
}

#[test]
fn a_duplicate_relation_is_stored_once() {
    let extra = format!(
        r#"#20=IFCBUILDINGELEMENTPROXY('{AGG_CHILD_A}',$,'Part A',$,$,$,$,$,.NOTDEFINED.);
#21=IFCRELAGGREGATES('{REL_AGG_1}',$,$,$,#16,(#20));
#22=IFCRELAGGREGATES('{REL_AGG_2}',$,$,$,#16,(#20));"#
    );
    let drawing = import(fixture(&extra));

    // One edge for the repeated `(aggregates, wall, part)` triple; the first
    // relationship record wins deterministically.
    assert_eq!(drawing.database.bim_relations().len(), 1);
    let row = relation(
        &drawing,
        BimRelationKind::Aggregates,
        &source(WALL),
        &source(AGG_CHILD_A),
    )
    .expect("the deduplicated edge is stored");
    assert_eq!(row.source.as_ref(), Some(&source(REL_AGG_1)));
}

#[test]
fn relations_survive_into_the_database() {
    let extra = format!(
        r#"#20=IFCBUILDINGSTOREY('{STOREY}',$,'Level 0',$,$,#14,$,$,.ELEMENT.,0.);
#21=IFCRELCONTAINEDINSPATIALSTRUCTURE('{REL_SPATIAL}',$,$,$,(#16),#20);"#
    );
    let drawing = import(fixture(&extra));

    assert!(
        !drawing.database.bim_relations().is_empty(),
        "the builder must receive the relation table before finish"
    );
    let rows: Vec<&BimRelation> = drawing.database.bim_relations().iter().collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, BimRelationKind::Contains);
    assert_eq!(rows[0].parent, source(STOREY));
    assert_eq!(rows[0].child, source(WALL));
}

#[test]
fn a_relation_over_budget_is_refused_not_truncated() {
    let extra = format!(
        r#"#20=IFCBUILDINGELEMENTPROXY('{AGG_CHILD_A}',$,'Part A',$,$,$,$,$,.NOTDEFINED.);
#21=IFCBUILDINGELEMENTPROXY('{NEST_CHILD}',$,'Nested part',$,$,$,$,$,.NOTDEFINED.);
#22=IFCRELAGGREGATES('{REL_AGG_1}',$,$,$,#16,(#20));
#23=IFCRELNESTS('{REL_NEST}',$,$,$,#16,(#21));"#
    );
    let bytes = fixture(&extra);
    let index = build_entity_index(&bytes);
    let mut decoder = EntityDecoder::with_index(&bytes, index);

    // Two unique edges, a one-row budget: the whole table is refused instead of
    // storing a truncated prefix.
    let Scan {
        relations,
        endpoint_missing,
        budget_exceeded,
    } = collect_with_budget(&bytes, &mut decoder, Some(PROJECT), 1);

    assert!(budget_exceeded, "the row budget must be reported");
    assert!(
        relations.is_empty(),
        "a refused table stores nothing, not the first row"
    );
    assert_eq!(endpoint_missing, 0);
}
