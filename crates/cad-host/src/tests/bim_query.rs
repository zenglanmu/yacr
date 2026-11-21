//! BIM query/summary host reads (ADR-0042 D11).
//!
//! The host entry binds the backend index to the open document; these contracts
//! prove the binding and the pass-through, not the backend's filter algebra
//! (that is `cad-query`'s own `bim` tests):
//!
//! 1. a category/level filter and a property predicate reach the backend and
//!    select the expected components;
//! 2. paging is the `QueryPage` contract, `limit = 0` included;
//! 3. the summary counts components and facets;
//! 4. one source element split into several geometry rows is **one** component;
//! 5. no document is an empty success with no database identity, never a panic.

use super::*;

use cad_app::cad_query::bim::{BimFilter, BimPropertyFilter, BimPropertyOp, BimScalar, BimSummary};
use cad_db::{
    BimDefinition, BimDefinitions, BimElement, BimEvidence, BimKey, BimProperty, BimPropertySet,
    BimSource, BimValue, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer,
};
use cad_domain::{DatabaseId, EntityId, LayerId, ObjectId, Revision, SemanticGeometry, SpaceId};

const LEVEL_1: &str = "level:ifc:doc:L1";
const LEVEL_2: &str = "level:ifc:doc:L2";
const TYPE_1: &str = "type:ifc:doc:T1";
const MATERIAL_1: &str = "material:ifc:doc:M1";

fn line(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".to_string(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: p(0.0, 0.0),
            end: p(1.0, 0.0),
        },
        draw_order: 0,
    }
}

fn source(id: &str) -> BimSource {
    BimSource {
        connector: "ifc".to_string(),
        document_id: Some("doc".to_string()),
        id: Some(id.to_string()),
        native_id: None,
    }
}

fn property_set(name: &str, properties: Vec<(&str, BimValue)>) -> BimPropertySet {
    BimPropertySet {
        name: name.to_string(),
        properties: properties
            .into_iter()
            .map(|(name, value)| BimProperty {
                name: name.to_string(),
                value,
            })
            .collect(),
    }
}

fn element(
    source_id: &str,
    category: &str,
    name: &str,
    level: Option<&str>,
    psets: Vec<BimPropertySet>,
) -> BimElement {
    BimElement {
        category: Some(category.to_string()),
        name: Some(name.to_string()),
        source: source(source_id),
        level: level.map(str::to_string),
        element_type: level.map(|_| TYPE_1.to_string()),
        material: level.map(|_| MATERIAL_1.to_string()),
        psets,
        evidence: BimEvidence::Declared,
    }
}

fn definitions() -> BimDefinitions {
    let mut definitions = BimDefinitions::new();
    for (id, name) in [(LEVEL_1, "Level 1"), (LEVEL_2, "Level 2")] {
        definitions
            .insert(BimDefinition::Level {
                id: id.to_string(),
                name: name.to_string(),
                elevation: Some(0.0),
                source: source("storey"),
                evidence: BimEvidence::Declared,
            })
            .expect("level");
    }
    definitions
        .insert(BimDefinition::Type {
            id: TYPE_1.to_string(),
            name: "Basic Wall:Generic 200mm".to_string(),
            class: Some("IfcWallType".to_string()),
            category: None,
            psets: Vec::new(),
            source: source("type"),
            evidence: BimEvidence::Declared,
        })
        .expect("type");
    definitions
        .insert(BimDefinition::Material {
            id: MATERIAL_1.to_string(),
            name: "Concrete".to_string(),
            source: source("material"),
            evidence: BimEvidence::Declared,
        })
        .expect("material");
    definitions
}

/// Four components: `wall-1` spans two geometry rows (one component), `wall-2`
/// is on the second storey, `slab-1` is a second category and `loose` has no
/// source identifier (an entity-keyed component).
fn bim_database() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(7));
    builder.set_revision(Revision(3));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("layer");

    let external = vec![property_set(
        "Pset_WallCommon",
        vec![
            ("IsExternal", BimValue::Bool(true)),
            (
                "Height",
                BimValue::Real {
                    value: 2.5,
                    ifc_type: None,
                    unit: None,
                },
            ),
        ],
    )];
    let rows = vec![
        (
            1_u128,
            element(
                "wall-1",
                "IfcWall",
                "Wall L1 A",
                Some(LEVEL_1),
                external.clone(),
            ),
        ),
        (
            2,
            element(
                "wall-1",
                "IfcWall",
                "Wall L1 A",
                Some(LEVEL_1),
                external.clone(),
            ),
        ),
        (
            3,
            element(
                "wall-2",
                "IfcWall",
                "Wall L2",
                Some(LEVEL_2),
                external.clone(),
            ),
        ),
        (
            4,
            element("slab-1", "IfcSlab", "Slab L1", Some(LEVEL_1), Vec::new()),
        ),
    ];
    for (id, bim) in rows {
        builder.insert_entity(line(id)).expect("entity");
        builder.set_entity_bim(EntityId(id), bim).expect("bim");
    }
    // A component without a source identifier: its own entity is the key.
    builder.insert_entity(line(5)).expect("entity");
    builder
        .set_entity_bim(
            EntityId(5),
            BimElement {
                category: Some("IfcWall".to_string()),
                name: Some("Loose Wall".to_string()),
                source: BimSource {
                    connector: "ifc".to_string(),
                    document_id: None,
                    id: None,
                    native_id: None,
                },
                level: None,
                element_type: None,
                material: None,
                psets: Vec::new(),
                evidence: BimEvidence::Heuristic,
            },
        )
        .expect("bim");
    builder
        .set_bim_definitions(definitions())
        .expect("definitions");
    builder.finish().expect("database")
}

/// Install the BIM fixture as the core's open document.
///
/// The other host contracts install a built database directly too (they are
/// about the host read, not about the import funnel); the query service is the
/// session's own, and it has no cached index yet.
fn bim_core() -> HostCore {
    let core = core();
    {
        let mut controller = core.controller.borrow_mut();
        let document = controller.document_id;
        controller
            .application
            .workspace
            .documents
            .get_mut(&document)
            .expect("the host always has a document")
            .drawing = Arc::new(bim_database());
    }
    core
}

fn categories(terms: &[&str]) -> BimFilter {
    BimFilter {
        categories: terms.iter().map(|term| (*term).to_string()).collect(),
        ..BimFilter::new()
    }
}

#[test]
fn a_category_filter_selects_the_declared_components() {
    let core = bim_core();
    let page = core
        .bim_query(&categories(&["IfcWall"]), 0, 100)
        .expect("the host answers");
    assert_eq!(page.database, Some(DatabaseId(7)));
    assert_eq!(
        page.page.total, 3,
        "wall-1, wall-2 and the entity-keyed wall"
    );
    assert_eq!(page.page.request.revision, Revision(3));
    assert_eq!(page.page.rows.len(), 3);
    assert_eq!(
        page.page
            .rows
            .iter()
            .filter(|row| row.category.as_deref() == Some("IfcWall"))
            .count(),
        3
    );

    let slabs = core
        .bim_query(&categories(&["IfcSlab"]), 0, 100)
        .expect("the host answers");
    assert_eq!(slabs.page.total, 1);
    assert_eq!(slabs.page.rows[0].name.as_deref(), Some("Slab L1"));
}

#[test]
fn one_source_element_across_two_geometry_rows_is_one_component() {
    let core = bim_core();
    let page = core
        .bim_query(&categories(&["IfcWall"]), 0, 100)
        .expect("the host answers");
    let wall = page
        .page
        .rows
        .iter()
        .find(|row| {
            row.object
                == (BimKey::Source {
                    connector: "ifc".to_string(),
                    document_id: "doc".to_string(),
                    id: "wall-1".to_string(),
                })
        })
        .expect("wall-1 is one component");
    assert_eq!(wall.entity_ids, vec![EntityId(1), EntityId(2)]);
    assert_eq!(page.page.total, 3, "the two geometry rows count once");
}

#[test]
fn the_row_carries_the_definition_display_names() {
    let core = bim_core();
    let page = core
        .bim_query(&categories(&["IfcWall"]), 0, 100)
        .expect("the host answers");
    let wall = page
        .page
        .rows
        .iter()
        .find(|row| row.name.as_deref() == Some("Wall L2"))
        .expect("the storey-2 wall");
    assert_eq!(wall.level_name.as_deref(), Some("Level 2"));
    assert_eq!(wall.type_name.as_deref(), Some("Basic Wall:Generic 200mm"));
    assert_eq!(wall.material_name.as_deref(), Some("Concrete"));
    assert!(wall
        .property_summary
        .iter()
        .any(|set| set.name == "Pset_WallCommon"));
}

/// How many components one level filter selects (`limit = 0`: total only).
fn level_total(core: &HostCore, terms: &[&str]) -> usize {
    let filter = BimFilter {
        levels: terms.iter().map(|term| (*term).to_string()).collect(),
        ..BimFilter::new()
    };
    core.bim_query(&filter, 0, 0)
        .expect("the host answers")
        .page
        .total
}

#[test]
fn a_level_filter_matches_the_definition_id_or_its_name() {
    let core = bim_core();
    assert_eq!(
        level_total(&core, &["Level 1"]),
        2,
        "wall-1 (one component across two geometry rows) + slab-1"
    );
    assert_eq!(
        level_total(&core, &[LEVEL_1]),
        2,
        "the definition id is accepted too"
    );
    assert_eq!(level_total(&core, &["Level 2"]), 1);
}

#[test]
fn a_property_predicate_uses_the_backend_comparison() {
    let core = bim_core();
    let eq = BimFilter {
        property: Some(BimPropertyFilter::new(
            "Pset_WallCommon",
            "IsExternal",
            BimPropertyOp::Eq,
            BimScalar::Bool(true),
        )),
        ..BimFilter::new()
    };
    let page = core.bim_query(&eq, 0, 0).expect("the host answers");
    assert_eq!(
        page.page.total, 2,
        "wall-1 and wall-2 declare IsExternal; the entity-keyed wall declares nothing"
    );

    // A real crosses with an integer numerically: both walls declare
    // `Height = 2.5`; the slab declares the property set at all.
    let real = BimFilter {
        property: Some(BimPropertyFilter::new(
            "Pset_WallCommon",
            "Height",
            BimPropertyOp::Eq,
            BimScalar::real(2.5),
        )),
        ..BimFilter::new()
    };
    let page = core.bim_query(&real, 0, 0).expect("the host answers");
    assert_eq!(page.page.total, 2);

    // A property nobody declares selects nothing even for `Ne`.
    let absent = BimFilter {
        property: Some(BimPropertyFilter::new(
            "Pset_WallCommon",
            "IsCurtainWall",
            BimPropertyOp::Ne,
            BimScalar::Bool(true),
        )),
        ..BimFilter::new()
    };
    let page = core.bim_query(&absent, 0, 0).expect("the host answers");
    assert_eq!(page.page.total, 0);
}

#[test]
fn paging_is_the_query_page_contract() {
    let core = bim_core();
    let first = core
        .bim_query(&BimFilter::new(), 0, 2)
        .expect("the host answers");
    assert_eq!(first.page.total, 4, "four components in the fixture");
    assert_eq!(first.page.rows.len(), 2);
    assert_eq!(first.page.request.offset, 0);
    assert_eq!(first.page.request.limit, 2);

    let second = core
        .bim_query(&BimFilter::new(), 2, 2)
        .expect("the host answers");
    assert_eq!(second.page.total, 4, "total is every match, not the page");
    assert_eq!(second.page.rows.len(), 2);
    assert_ne!(
        first.page.rows[0].object, second.page.rows[0].object,
        "offset skips real rows"
    );

    let total_only = core
        .bim_query(&BimFilter::new(), 0, 0)
        .expect("the host answers");
    assert_eq!(total_only.page.total, 4);
    assert!(
        total_only.page.rows.is_empty(),
        "limit = 0 requests only the total"
    );

    let past_the_end = core
        .bim_query(&BimFilter::new(), 10, 5)
        .expect("the host answers");
    assert_eq!(past_the_end.page.total, 4);
    assert!(past_the_end.page.rows.is_empty());
}

#[test]
fn the_summary_counts_components_and_facets() {
    let core = bim_core();
    let summary = core
        .bim_summary(&BimFilter::new(), 0, 1)
        .expect("the host answers");
    assert_eq!(summary.database, Some(DatabaseId(7)));
    assert_eq!(summary.page.total, 1, "the summary is one logical row");
    let BimSummary {
        total,
        by_category,
        by_level,
        by_type,
        by_material,
    } = summary.page.rows.first().expect("one summary row").clone();
    assert_eq!(total, 4);
    let facade = |counts: &[cad_app::cad_query::bim::BimFacetCount], name: &str| {
        counts
            .iter()
            .find(|count| count.name == name)
            .map(|count| count.count)
    };
    assert_eq!(facade(&by_category, "IfcWall"), Some(3));
    assert_eq!(facade(&by_category, "IfcSlab"), Some(1));
    assert_eq!(facade(&by_level, "Level 1"), Some(2));
    assert_eq!(facade(&by_level, "Level 2"), Some(1));
    assert_eq!(facade(&by_type, "Basic Wall:Generic 200mm"), Some(3));
    assert_eq!(facade(&by_material, "Concrete"), Some(3));

    let summary = core
        .bim_summary(&BimFilter::new(), 0, 0)
        .expect("the host answers");
    assert!(
        summary.page.rows.is_empty(),
        "limit = 0 answers the total only, summary included"
    );
    assert_eq!(summary.page.total, 1);
}

#[test]
fn an_over_budget_filter_is_the_backends_own_error() {
    let core = bim_core();
    let filter = BimFilter {
        categories: (0..=cad_app::cad_query::bim::BIM_MAX_FILTER_TERMS)
            .map(|index| format!("Ifc{index}"))
            .collect(),
        ..BimFilter::new()
    };
    let error = core
        .bim_query(&filter, 0, 1)
        .expect_err("a filter over the term budget is refused");
    assert!(
        error
            .to_string()
            .contains(cad_app::cad_query::bim::BIM_FILTER_TOO_MANY_TERMS),
        "the backend's own marker travels out: {error}"
    );
}

/// A production-shaped core with no document at startup.
fn no_document_core() -> HostCore {
    let platform = HostPlatform {
        files: Rc::new(FakeFiles::default()),
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    let mut options = options();
    options.document_at_start = StartDocument::Empty;
    HostCore::new(options, platform, RenderSessionHandle::default()).expect("empty host core")
}

#[test]
fn no_document_answers_an_empty_page() {
    let core = no_document_core();
    assert!(
        core.controller.borrow().drawing().is_none(),
        "the fixture really starts with no document"
    );
    let page = core
        .bim_query(&BimFilter::new(), 0, 10)
        .expect("no document is not an error");
    assert_eq!(page.database, None);
    assert_eq!(page.page.total, 0);
    assert!(page.page.rows.is_empty());

    let summary = core
        .bim_summary(&BimFilter::new(), 0, 10)
        .expect("no document is not an error");
    assert_eq!(summary.database, None);
    assert_eq!(
        summary.page.total, 0,
        "no document answers no summary row at all"
    );
    assert!(summary.page.rows.is_empty());
}
