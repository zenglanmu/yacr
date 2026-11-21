//! Contract tests for the DGN tag → BIM mapping (ADR-0042 D9).
//!
//! The boundary under test is [`crate::bim`]: it consumes the
//! `cadkit_core::Entity::attributes` records that `cadkit-dgn` 0.5.4 decodes and
//! the mapper stores their result on the entity row. Nothing here re-tests the
//! reader's own tag decoding, and **no real V8i/CONNECT file with Item Types is
//! available**: the end-to-end cases below feed a synthetic
//! `cadkit_core::Document`, exactly like the rest of this crate's degradation
//! tests. Real DGN BIM semantics are NOT VERIFIED.
//!
//! The committed `test_dgnv8.dgn` fixture does carry one tag, but the reader can
//! only name it `tag_0` (its tag-set definition is missing) and it sits on an
//! unmodelled element, so it must stay a `import.dgn.tags_not_stored` report and
//! must never become a property with a fabricated name.

use cad_db::{BimValue, DrawingDatabase, BIM_MAX_PROPERTIES_PER_SET, BIM_MAX_PSETS_PER_ELEMENT};
use cad_domain::DatabaseId;
use cad_import_core::{compute_identity, ImportLimits, ImportReport, Importer};
use cadkit_core::{
    Attribute, Entity, EntityKind, Layer as KitLayer, LengthUnit, Model, ModelKind,
    Point3 as KitPoint3, Units, Value,
};

use crate::bim::{bim_for_entity, EntityBim, DGN_TAGS_SET};
use crate::map::Mapper;
use crate::{
    BIM_BUDGET_EXCEEDED, BIM_CATEGORY_FROM_TAG, BIM_NO_CATEGORY, BIM_TAGS_STORED, BIM_UNVERIFIED,
    TAGS_NOT_STORED,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tag(set: Option<&str>, name: &str, value: Value) -> Attribute {
    let attribute = Attribute::new(name, value);
    match set {
        Some(set) => attribute.with_set(set.to_string()),
        None => attribute,
    }
}

fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

fn line_with(attributes: Vec<Attribute>) -> Entity {
    Entity::new(EntityKind::Line {
        start: KitPoint3::ZERO,
        end: KitPoint3::new(1.0, 0.0, 0.0),
    })
    .with_attributes(attributes)
}

#[allow(clippy::field_reassign_with_default)]
fn synthetic(entities: Vec<Entity>) -> cadkit_core::Document {
    let mut document = cadkit_core::Document::default();
    document.units = Units::from_insunits(LengthUnit::Meter.to_insunits());
    let mut model = Model::default();
    model.name = "Model".into();
    model.kind = ModelKind::Model;
    model.entities = entities;
    model.props.insert(
        "dgn.uor_per_master".into(),
        cadkit_core::Value::Float(10_000.0),
    );
    document.models.push(model);
    let mut layer = KitLayer::default();
    layer.name = "0".into();
    layer.visible = true;
    document.layers.push(layer);
    document
}

fn map(document: &cadkit_core::Document) -> (DrawingDatabase, ImportReport) {
    let cancelled = || false;
    let mut mapper = Mapper::new(
        DatabaseId(1),
        document,
        &ImportLimits::default(),
        &cancelled,
    );
    mapper.map_tables().expect("tables");
    mapper.map_blocks().expect("blocks");
    mapper.map_primary_model().expect("model");
    let (database, _, report) = mapper.finish(compute_identity(&[]), 0.0).expect("finish");
    (database, report)
}

fn codes(report: &ImportReport) -> Vec<&str> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn property<'a>(outcome: &'a EntityBim, set: &str, name: &str) -> &'a BimValue {
    let element = outcome.element.as_ref().expect("a BIM block was stored");
    let set = element
        .psets
        .iter()
        .find(|pset| pset.name == set)
        .unwrap_or_else(|| panic!("no property set {set:?}"));
    &set.properties
        .iter()
        .find(|property| property.name == name)
        .unwrap_or_else(|| panic!("no property {name:?} in {set:?}"))
        .value
}

// ---------------------------------------------------------------------------
// Tag → property set mapping
// ---------------------------------------------------------------------------

#[test]
fn tags_become_one_property_set_per_item_type() {
    let attributes = vec![
        tag(Some("Wall"), "FireRating", text("2h")),
        tag(Some("Wall"), "IsExternal", Value::Bool(true)),
        tag(Some("Wall"), "Thickness", Value::Float(0.2)),
        tag(Some("Wall"), "Zone", Value::Int(7)),
        tag(Some("Wall"), "Note", Value::Null),
        // No tag-set / Item Type name: the fallback set name keeps the value.
        tag(None, "Unassigned", text("loose tag")),
    ];
    let outcome = bim_for_entity(Some("1A"), &attributes);
    assert_eq!(outcome.stored, 6, "every decoded tag became a property");
    assert_eq!(outcome.undecodable, 0);
    assert_eq!(outcome.budget_dropped, 0);

    let element = outcome.element.as_ref().expect("a BIM block was stored");
    assert_eq!(element.psets.len(), 2, "one set per Item Type: {element:?}");
    // `BTreeMap` order keeps the writer deterministic.
    assert_eq!(element.psets[0].name, DGN_TAGS_SET);
    assert_eq!(element.psets[1].name, "Wall");
    assert_eq!(element.psets[1].properties.len(), 5);

    // Types are preserved: nothing is stringified, floats stay floats.
    assert_eq!(
        property(&outcome, "Wall", "FireRating"),
        &BimValue::Text("2h".into())
    );
    assert_eq!(
        property(&outcome, "Wall", "IsExternal"),
        &BimValue::Bool(true)
    );
    assert_eq!(
        property(&outcome, "Wall", "Thickness"),
        &BimValue::Real {
            value: 0.2,
            ifc_type: None,
            unit: None,
        }
    );
    assert_eq!(property(&outcome, "Wall", "Zone"), &BimValue::Integer(7));
    assert_eq!(property(&outcome, "Wall", "Note"), &BimValue::Null);
    assert_eq!(
        property(&outcome, DGN_TAGS_SET, "Unassigned"),
        &BimValue::Text("loose tag".into())
    );

    // Declared source identity, reusing the adapter's handle spelling.
    assert_eq!(element.source.connector, "dgn");
    assert_eq!(element.source.document_id, None);
    assert_eq!(element.source.id.as_deref(), Some("1A"));
    assert_eq!(element.source.native_id.as_deref(), Some("1A"));
    assert_eq!(element.evidence, cad_db::BimEvidence::Declared);
    // D9: no level/type/material and no display name from tags.
    assert_eq!(element.level, None);
    assert_eq!(element.element_type, None);
    assert_eq!(element.material, None);
    assert_eq!(element.name, None);
    assert_eq!(element.category, None);
    assert_eq!(outcome.psets, 2);
}

#[test]
fn the_reader_tag_fallback_name_is_never_a_property_name() {
    // `cadkit-dgn` names a tag `tag_<n>` when the tag-set definition is missing;
    // that string is decoder output, not file data.
    let attributes = vec![
        tag(None, "tag_0", Value::Int(0)),
        tag(Some("Wall"), "Height", Value::Float(2.5)),
    ];
    let outcome = bim_for_entity(Some("2"), &attributes);
    assert_eq!(outcome.undecodable, 1, "the fallback name is not stored");
    assert_eq!(outcome.stored, 1);
    let element = outcome.element.as_ref().expect("the named tag is stored");
    assert_eq!(element.psets.len(), 1);
    assert_eq!(element.psets[0].name, "Wall");
    assert_eq!(element.psets[0].properties[0].name, "Height");

    // Only undecodable tags: nothing to store, so no block is invented.
    let only_fallback = bim_for_entity(None, &[tag(None, "tag_3", Value::Int(1))]);
    assert!(only_fallback.element.is_none());
    assert_eq!(only_fallback.undecodable, 1);
}

// ---------------------------------------------------------------------------
// Category: explicit tags only
// ---------------------------------------------------------------------------

#[test]
fn an_explicit_category_tag_sets_the_category() {
    for (tag_name, token) in [
        ("Category", "IfcWall"),
        ("IfcType", "IfcDoor"),
        ("IFCENTITY", "IfcSpace"),
        ("category", "IfcSlab"),
        ("iFcTyPe", "IfcBeam"),
    ] {
        let attributes = vec![
            tag(Some("ItemType"), tag_name, text(token)),
            tag(Some("ItemType"), "Height", Value::Float(2.5)),
        ];
        let outcome = bim_for_entity(Some("3"), &attributes);
        let element = outcome.element.as_ref().expect("a BIM block was stored");
        assert_eq!(
            element.category.as_deref(),
            Some(token),
            "tag {tag_name:?} must declare the category"
        );
        assert_eq!(
            outcome.category.as_deref(),
            Some(token),
            "the category is reported for the tag diagnostic"
        );
        // The category tag is still a property; nothing is moved out of the set.
        assert_eq!(
            property(&outcome, "ItemType", tag_name),
            &BimValue::Text(token.into())
        );
    }

    // Surrounding whitespace is trimmed; the token itself is kept as written.
    let padded = bim_for_entity(None, &[tag(None, "Category", text("  IfcWall  "))]);
    assert_eq!(
        padded.element.expect("block").category.as_deref(),
        Some("IfcWall")
    );
}

#[test]
fn no_category_tag_leaves_category_none_and_reports() {
    let document = synthetic(vec![line_with(vec![tag(
        Some("Material"),
        "Grade",
        text("C30"),
    )])]);
    let (database, report) = map(&document);
    let id = database.entities().next().expect("one row").id;
    let element = database.bim_element(id).expect("tags became a BIM block");
    assert_eq!(element.category, None);
    assert_eq!(element.psets.len(), 1);
    assert_eq!(element.psets[0].name, "Material");

    let codes = codes(&report);
    assert!(codes.contains(&BIM_NO_CATEGORY), "{codes:?}");
    assert!(!codes.contains(&BIM_CATEGORY_FROM_TAG), "{codes:?}");
    assert!(codes.contains(&BIM_TAGS_STORED), "{codes:?}");
    // The interpretation itself is always declared unverified (ADR-0042 D9).
    assert!(codes.contains(&BIM_UNVERIFIED), "{codes:?}");
}

#[test]
fn a_level_name_or_cell_name_is_never_a_category() {
    // D9: levels are not storeys and shared-cell names are not categories. The
    // values stay ordinary properties; no heuristic reads them as a category.
    let attributes = vec![
        tag(None, "Level", text("Level 1")),
        tag(None, "Cell", text("Named definition")),
        // An explicit category tag whose value is prose, not a token.
        tag(None, "Category", text("load bearing wall")),
    ];
    let outcome = bim_for_entity(Some("4"), &attributes);
    let element = outcome
        .element
        .as_ref()
        .expect("the values are still stored");
    assert_eq!(element.category, None, "no heuristic category");
    assert_eq!(element.psets.len(), 1);
    assert_eq!(element.psets[0].properties.len(), 3);
    assert_eq!(
        property(&outcome, DGN_TAGS_SET, "Level"),
        &BimValue::Text("Level 1".into())
    );
    assert_eq!(
        property(&outcome, DGN_TAGS_SET, "Cell"),
        &BimValue::Text("Named definition".into())
    );
    assert!(crate::bim::dgn_bim_category(&attributes).is_none());

    // Non-text category values and empty tokens never declare a category.
    for value in [Value::Int(1), Value::Bool(true), text(""), text("   ")] {
        assert!(
            crate::bim::dgn_bim_category(&[tag(None, "IfcType", value)]).is_none(),
            "a non-token category value must not become a category"
        );
    }
    // The tag name must match exactly (case-insensitively), not by substring.
    assert!(crate::bim::dgn_bim_category(&[tag(None, "SubCategory", text("IfcWall"))]).is_none());
}

// ---------------------------------------------------------------------------
// Budgets
// ---------------------------------------------------------------------------

#[test]
fn a_budget_excess_drops_the_block_and_reports() {
    // Per-set budget: one property over the ceiling drops the whole set, never
    // the first N (a truncated Item Type would look like the file said so).
    let mut too_many_properties = Vec::new();
    for index in 0..=BIM_MAX_PROPERTIES_PER_SET {
        too_many_properties.push(tag(
            Some("Big"),
            &format!("P{index}"),
            Value::Int(index as i64),
        ));
    }
    let outcome = bim_for_entity(Some("5"), &too_many_properties);
    assert_eq!(outcome.budget_dropped, 1);
    assert_eq!(outcome.stored, 0, "no partial property set is stored");
    assert!(outcome.element.is_none(), "an empty block is not written");

    // Per-element budget: the overflow sets are dropped whole; the kept sets
    // stay complete.
    let mut too_many_sets = Vec::new();
    for index in 0..=BIM_MAX_PSETS_PER_ELEMENT {
        too_many_sets.push(tag(
            Some(&format!("Set{index:03}")),
            "Value",
            Value::Int(index as i64),
        ));
    }
    let outcome = bim_for_entity(Some("6"), &too_many_sets);
    assert_eq!(outcome.psets, BIM_MAX_PSETS_PER_ELEMENT);
    assert_eq!(outcome.budget_dropped, 1);
    assert_eq!(outcome.stored, BIM_MAX_PSETS_PER_ELEMENT);
    for set in &outcome.element.as_ref().expect("kept sets").psets {
        assert_eq!(set.properties.len(), 1, "no set is truncated");
    }

    // Per-string budget.
    let long = "x".repeat(cad_db::BIM_MAX_STRING_BYTES + 1);
    let outcome = bim_for_entity(None, &[tag(None, "Description", Value::Text(long))]);
    assert_eq!(outcome.budget_dropped, 1);
    assert!(outcome.element.is_none());

    // End to end: the row survives, the refused block is reported, and the
    // tag is not double-counted as an undecodable tag.
    let document = synthetic(vec![line_with(too_many_properties)]);
    let (database, report) = map(&document);
    let id = database
        .entities()
        .next()
        .expect("the geometry row stays")
        .id;
    assert!(database.bim_element(id).is_none(), "no incomplete block");
    let codes = codes(&report);
    assert!(codes.contains(&BIM_BUDGET_EXCEEDED), "{codes:?}");
    assert!(!codes.contains(&TAGS_NOT_STORED), "{codes:?}");
    assert!(!codes.contains(&BIM_UNVERIFIED), "nothing was interpreted");
}

// ---------------------------------------------------------------------------
// No tags
// ---------------------------------------------------------------------------

#[test]
fn an_entity_without_tags_has_no_bim_block() {
    let document = synthetic(vec![line_with(Vec::new())]);
    let (database, report) = map(&document);
    assert_eq!(database.entity_count(), 1);
    assert!(
        database.bim_elements().is_empty(),
        "no fabricated BIM block"
    );
    let codes = codes(&report);
    for code in [
        BIM_TAGS_STORED,
        BIM_CATEGORY_FROM_TAG,
        BIM_NO_CATEGORY,
        BIM_UNVERIFIED,
        BIM_BUDGET_EXCEEDED,
        TAGS_NOT_STORED,
    ] {
        assert!(!codes.contains(&code), "{code} must not fire: {codes:?}");
    }

    assert!(bim_for_entity(None, &[]).element.is_none());
}

// ---------------------------------------------------------------------------
// The committed fixture
// ---------------------------------------------------------------------------

#[test]
fn the_v8_fixture_tag_is_reported_and_never_stored() {
    // The fixture's tag has no tag-set definition (`tag_0`) and belongs to an
    // unmodelled element: there is no row and no decoded name, so it must be a
    // report, never a property with an invented name.
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dgn/test_dgnv8.dgn");
    let bytes = std::fs::read(path).expect("fixture is committed");
    let request = cad_import_core::ImportRequest {
        document: cad_domain::DocumentId(1),
        database: DatabaseId(1),
        bytes: std::sync::Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    let drawing = crate::DgnImporter::new()
        .import(&request, &|| false)
        .expect("the fixture imports");
    assert!(
        drawing.database.bim_elements().is_empty(),
        "the fixture's tag has no storage target and must not be fabricated"
    );
    let codes = codes(&drawing.report);
    assert!(codes.contains(&TAGS_NOT_STORED), "{codes:?}");
    assert!(!codes.contains(&BIM_TAGS_STORED), "{codes:?}");
    assert!(!codes.contains(&BIM_UNVERIFIED), "{codes:?}");
}
