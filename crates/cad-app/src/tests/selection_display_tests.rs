//! The four display-attribute `PropertyRow`s: emission, canonical values, kinds,
//! options and the multi-selection intersection/mixed behaviour.
//!
//! These rows are the properties panel's editable display block, so their
//! `value` must be the **write grammar** itself (it round-trips unchanged), not a
//! presentation string. The tests therefore parse every emitted value back
//! through [`crate::property_value::parse`] — the same function the write path
//! uses to accept or refuse a `SETENTITYPROPERTY` value.

use super::*;
use cad_db::{
    DbEntity, DbObject, DrawingDatabaseBuilder, EntityColor, EntityLineType, EntityLineWeight,
    EntityRenderAttributes, EntityTransparency, Layer, LineType, LinetypePattern,
};
use cad_domain::{DatabaseId, DocumentId, EntityId, LayerId, LinetypeId, ObjectId, Revision};

const DISPLAY_KEYS: [&str; 4] = ["color", "linetype", "lineweight", "transparency"];

fn entity(id: u128, layer: LayerId) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: Some(format!("{id:X}")),
        },
        id: EntityId(id),
        layer,
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
        draw_order: id as i64,
    }
}

fn dashdot() -> LinetypePattern {
    LinetypePattern::from_elements([0.5, -0.25])
}

/// Explicit display attributes that every write grammar accepts: ACI red, the
/// `DASHDOT` table entry, the canonical `0.35` mm weight and 0.4 opacity
/// (= 60 % transparent).
fn explicit_attributes(color: [u8; 3]) -> EntityRenderAttributes {
    EntityRenderAttributes {
        transparency: EntityTransparency::Explicit(0.4),
        color: EntityColor::Explicit(color),
        lineweight: EntityLineWeight::Explicit(0.35),
        linetype: EntityLineType::Explicit {
            name: "DASHDOT".into(),
            pattern: dashdot(),
            scale: 1.0,
        },
        ..EntityRenderAttributes::default()
    }
}

/// A drawing with one layer, the `DASHDOT` linetype and two line entities whose
/// colour is given by the caller (their other attributes are identical).
fn attribute_database(colors: [[u8; 3]; 2]) -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(7));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_linetype(LineType {
        id: LinetypeId(1),
        name: "DASHDOT".into(),
        pattern: dashdot(),
        complex: false,
    })
    .unwrap();
    for (index, color) in colors.into_iter().enumerate() {
        let id = EntityId(index as u128 + 1);
        b.insert_entity(entity(index as u128 + 1, LayerId(0)))
            .unwrap();
        b.set_entity_render_attributes(id, explicit_attributes(color))
            .unwrap();
    }
    b.finish().unwrap()
}

fn row<'a>(rows: &'a [PropertyRow], key: &str) -> &'a PropertyRow {
    rows.iter()
        .find(|row| row.key == key)
        .unwrap_or_else(|| panic!("no '{key}' row in {rows:?}"))
}

#[test]
fn display_rows_carry_the_write_grammar_and_round_trip_through_it() {
    let db = attribute_database([[255, 0, 0], [255, 0, 0]]);
    let reference = model_ref(DocumentId(7), EntityId(1));
    let stored = db.entity(EntityId(1)).expect("the entity");
    let set = SelectionSet::from_refs([reference.clone()]);
    let props = SelectionProperties::extract(&db, &set);

    // The canonical projection, one row per attribute, in the write path's own
    // key order (`SUPPORTED_KEYS` minus `layer`).
    let emitted: Vec<&str> = props
        .rows
        .iter()
        .filter(|row| DISPLAY_KEYS.contains(&row.key))
        .map(|row| row.key)
        .collect();
    assert_eq!(emitted, DISPLAY_KEYS);

    for key in DISPLAY_KEYS {
        let value = row(&props.rows, key).value.clone().expect("a real value");
        // The row value is exactly `property_value`'s projection…
        assert_eq!(
            Some(value.clone()),
            crate::property_value::property_value(&db, stored, key)
        );
        // …and the write path accepts it unchanged (the round trip).
        crate::property_value::parse(&db, key, &value)
            .unwrap_or_else(|error| panic!("'{key}={value}' is not writable: {error:?}"));
    }

    // The concrete values of this fixture, and the control kind each one needs.
    let color = row(&props.rows, "color");
    assert_eq!(color.value.as_deref(), Some("aci:1"));
    assert_eq!(color.kind, PropertyKind::Text);
    assert_eq!(color.group.key(), "display");
    assert!(
        color.options.is_empty(),
        "a free-form grammar has no picker"
    );
    assert!(color.editable);

    let linetype = row(&props.rows, "linetype");
    assert_eq!(linetype.value.as_deref(), Some("DASHDOT"));
    assert_eq!(linetype.kind, PropertyKind::Enum);
    assert_eq!(
        linetype.options,
        vec![
            "ByBlock".to_string(),
            "ByLayer".to_string(),
            "DASHDOT".to_string()
        ],
        "the drawing's linetype names plus the symbolic values, sorted"
    );
    assert!(linetype.editable);

    let lineweight = row(&props.rows, "lineweight");
    assert_eq!(lineweight.value.as_deref(), Some("0.35"));
    assert_eq!(lineweight.kind, PropertyKind::Enum);
    assert!(lineweight.editable);
    // One shared source: the picker lists exactly the parser's accepted set.
    let mut expected = crate::property_value::lineweight_options();
    assert_eq!(expected.len(), 27, "3 symbolic + 24 canonical weights");
    expected.sort();
    assert_eq!(lineweight.options, expected);

    let transparency = row(&props.rows, "transparency");
    assert_eq!(transparency.value.as_deref(), Some("60"));
    assert_eq!(transparency.kind, PropertyKind::Number);
    assert!(transparency.options.is_empty());
    assert!(transparency.editable);

    // Every lineweight option really is writable (no dead entry in the picker).
    for option in &lineweight.options {
        crate::property_value::parse(&db, "lineweight", option).unwrap_or_else(|error| {
            panic!("the picker offers '{option}' but the parser refuses it: {error:?}")
        });
    }
    // Likewise for the linetype picker.
    for option in &linetype.options {
        crate::property_value::parse(&db, "linetype", option)
            .unwrap_or_else(|error| panic!("linetype '{option}' is not writable: {error:?}"));
    }
}

#[test]
fn a_by_layer_color_projects_as_the_symbolic_value() {
    // An entity with no recorded attributes (the default) is `ByLayer`: the row
    // reports the symbolic value the grammar accepts, never an invented colour.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(8));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(entity(3, LayerId(0))).unwrap();
    let plain = b.finish().unwrap();

    let set = SelectionSet::from_refs([model_ref(DocumentId(8), EntityId(3))]);
    let props = SelectionProperties::extract(&plain, &set);
    let color = row(&props.rows, "color");
    assert_eq!(color.value.as_deref(), Some("ByLayer"));
    assert!(!color.mixed);
    assert!(color.editable, "the symbolic value is writable too");
    assert_eq!(
        crate::property_value::parse(&plain, "color", color.value.as_deref().expect("a value"))
            .unwrap(),
        crate::property_value::PropertyValue::Color(cad_db::EntityColor::ByLayer)
    );
    // The other display rows are honest defaults too, not blanks.
    assert_eq!(row(&props.rows, "transparency").value.as_deref(), Some("0"));
    assert_eq!(
        row(&props.rows, "lineweight").value.as_deref(),
        Some("ByLayer")
    );
}

#[test]
fn display_rows_join_the_multi_selection_intersection_and_mixed_logic() {
    let doc = DocumentId(7);
    let rows_for = |colors: [[u8; 3]; 2]| {
        let db = attribute_database(colors);
        let set =
            SelectionSet::from_refs([model_ref(doc, EntityId(1)), model_ref(doc, EntityId(2))]);
        SelectionProperties::extract(&db, &set).rows
    };

    // Differing colours: an explicit mixed row with no fabricated value, and the
    // key listed in `mixed_keys`.
    let mixed = rows_for([[255, 0, 0], [0, 255, 0]]);
    let color = row(&mixed, "color");
    assert!(color.mixed);
    assert_eq!(color.value, None);
    assert_eq!(color.group.key(), "display");
    assert_eq!(color.kind, PropertyKind::Text);
    assert!(color.editable, "every selected ref is addressable");
    // The attributes that agree stay one shared value.
    for key in ["linetype", "lineweight", "transparency"] {
        assert!(!row(&mixed, key).mixed, "'{key}' agrees across the refs");
    }

    // Equal colours: one real shared value, never a mixed marker.
    let shared = rows_for([[255, 0, 0], [255, 0, 0]]);
    let color = row(&shared, "color");
    assert!(!color.mixed);
    assert_eq!(color.value.as_deref(), Some("aci:1"));
}

#[test]
fn display_rows_are_not_editable_for_a_block_instance_member_or_sub_element() {
    let db = attribute_database([[255, 0, 0], [255, 0, 0]]);
    let doc = DocumentId(7);
    let reference = SelectionRef {
        document: doc,
        entity: EntityId(1),
        instance: InstancePath(vec![EntityId(9)]),
        sub_element: None,
    };
    let props = SelectionProperties::extract(&db, &SelectionSet::from_refs([reference]));
    for key in DISPLAY_KEYS {
        let row = row(&props.rows, key);
        assert!(
            row.value.is_some(),
            "'{key}' is still the entity's real stored value"
        );
        assert!(
            !row.editable,
            "'{key}' is not writable through a block-instance member"
        );
    }
}

#[test]
fn a_value_no_grammar_can_express_is_shown_but_not_offered_as_editable() {
    // A symbolic `ByBlock` transparency is a real stored value, but the
    // transparency grammar only accepts an integer percentage: the row reports
    // the truth and advertises no write that would have to fail.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(10));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(entity(1, LayerId(0))).unwrap();
    b.set_entity_render_attributes(
        EntityId(1),
        EntityRenderAttributes {
            transparency: EntityTransparency::ByBlock,
            ..EntityRenderAttributes::default()
        },
    )
    .unwrap();
    let db = b.finish().unwrap();
    let set = SelectionSet::from_refs([model_ref(DocumentId(10), EntityId(1))]);
    let props = SelectionProperties::extract(&db, &set);

    let transparency = row(&props.rows, "transparency");
    assert_eq!(transparency.value.as_deref(), Some("ByBlock"));
    assert!(
        !transparency.editable,
        "the write grammar cannot express this stored value"
    );
    assert!(!crate::property_value::accepts(
        &db,
        "transparency",
        "ByBlock"
    ));
    // The attributes that do round-trip stay editable.
    assert!(row(&props.rows, "color").editable);
}

#[test]
fn a_nameless_linetype_pattern_has_no_row_instead_of_a_fabricated_blank() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(9));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(entity(1, LayerId(0))).unwrap();
    b.set_entity_render_attributes(
        EntityId(1),
        EntityRenderAttributes {
            // An imported name-less pattern: no string the grammar accepts.
            linetype: EntityLineType::Explicit {
                name: String::new(),
                pattern: dashdot(),
                scale: 1.0,
            },
            ..EntityRenderAttributes::default()
        },
    )
    .unwrap();
    let db = b.finish().unwrap();
    let set = SelectionSet::from_refs([model_ref(DocumentId(9), EntityId(1))]);
    let props = SelectionProperties::extract(&db, &set);
    assert!(
        props.rows.iter().all(|row| row.key != "linetype"),
        "no writable value means no row"
    );
    // The attributes that do have a value are still emitted.
    assert!(props.rows.iter().any(|row| row.key == "color"));
}
