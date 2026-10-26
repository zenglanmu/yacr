//! AutoCAD's "layer `0` inside a block" resolution rule.
//!
//! Pins the three properties the display and pick paths rely on: layer `0` is
//! found **by name** (never as `LayerId(0)`), a drawing without layer `0` has no
//! inheritance at all, and the rule chains through nested `INSERT`s by resolving
//! one level at a time against that level's effective layer.

use super::*;
use crate::LayerAttributes;
use cad_domain::{DatabaseId, LayerId};

fn db_with_layers(rows: &[(u128, &str)]) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in rows {
        builder
            .insert_layer(Layer {
                id: LayerId(*id),
                name: (*name).into(),
                visible: true,
            })
            .unwrap();
    }
    builder.finish().unwrap()
}

#[test]
fn layer_zero_is_found_by_name_not_by_id() {
    // Layer `0` is id 7 here, and `LayerId(0)` is a different layer.
    let db = db_with_layers(&[(0, "MODEL"), (7, "0")]);
    assert_eq!(db.layer_zero_id(), Some(LayerId(7)));
    assert_eq!(db.layer_id_by_name("MODEL"), Some(LayerId(0)));
    assert_eq!(db.layer_id_by_name("0"), Some(LayerId(7)));
    // Unknown names and case variants are not guessed.
    assert_eq!(db.layer_id_by_name("model"), None);
    assert_eq!(db.layer_id_by_name("missing"), None);
}

#[test]
fn the_zero_layer_row_keeps_its_stored_name_and_attributes() {
    // The lookup is read-only: resolving layer `0` must not rename a row or
    // write attributes (the drawing stays a read-only-after-import database).
    let db = db_with_layers(&[(0, "MODEL"), (7, "0")]);
    let zero = db.layer(LayerId(7)).unwrap();
    assert_eq!(zero.name, "0");
    assert_eq!(db.layer_attributes(LayerId(7)), LayerAttributes::default());
    assert_eq!(db.revision(), Revision(0));
}

#[test]
fn a_drawing_without_layer_zero_has_no_inheritance() {
    let db = db_with_layers(&[(0, "MODEL"), (1, "WALLS")]);
    assert_eq!(db.layer_zero_id(), None);
    let scope = db.root_layer_scope();
    assert_eq!(scope.effective_layer(LayerId(0)), LayerId(0));
    assert_eq!(
        scope.child_scope(LayerId(1)).effective_layer(LayerId(0)),
        LayerId(0),
        "no layer `0` row means the member keeps its own layer"
    );
}

#[test]
fn the_rule_is_own_layer_unless_it_is_layer_zero() {
    let db = db_with_layers(&[(0, "0"), (1, "WALLS")]);
    let child = db.root_layer_scope().child_scope(LayerId(1));
    assert_eq!(
        child.effective_layer(LayerId(0)),
        LayerId(1),
        "a layer-`0` member takes the containing INSERT's layer"
    );
    // A member already on the INSERT's layer is unchanged by the rule.
    assert_eq!(child.effective_layer(LayerId(1)), LayerId(1));
    // A layer-`0` entity at the model root has no INSERT to inherit from.
    assert_eq!(
        db.root_layer_scope().effective_layer(LayerId(0)),
        LayerId(0)
    );
}

#[test]
fn the_rule_chains_through_nested_inserts() {
    let db = db_with_layers(&[(0, "0"), (1, "RED"), (2, "BLUE")]);
    let root = db.root_layer_scope();
    // An inner INSERT that is itself on layer `0`, placed inside an outer INSERT
    // on RED, resolves to RED.
    let inner = root.child_scope(LayerId(1)).effective_layer(LayerId(0));
    assert_eq!(inner, LayerId(1), "a layer-`0` INSERT resolves to RED");
    let nested = root.child_scope(LayerId(1)).child_scope(inner);
    // A layer-`0` member of that inner INSERT follows the INSERT's effective
    // layer (RED), not layer `0` again.
    assert_eq!(nested.effective_layer(LayerId(0)), LayerId(1));
    // A member on BLUE inside the same inner INSERT keeps BLUE.
    assert_eq!(nested.effective_layer(LayerId(2)), LayerId(2));
}
