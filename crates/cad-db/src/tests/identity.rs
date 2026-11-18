//! The nine persisted identifier high-water marks and the revision seed
//! (ADR-0037 D2/D5).
//!
//! Two invariants are pinned here:
//!
//! * **ids are never reused** — a create → delete → create hands out a fresh id,
//!   and a load adopts the persisted counters instead of rebuilding them from
//!   `max(row id) + 1`;
//! * **a loaded revision is the saved revision** — the first write after a load
//!   publishes `before == saved_revision`, so an edited-then-reopened document
//!   cannot pass as unmodified (the L0 passthrough predicate).

use cad_domain::ymodel::{IdCounters, MAX_SEQ};

use super::*;

fn linetype(id: u128, name: &str) -> LineType {
    LineType {
        id: LinetypeId(id),
        name: name.into(),
        pattern: LinetypePattern::from_elements([0.5, -0.25]),
        complex: false,
    }
}

fn layer(id: u128, name: &str) -> Layer {
    Layer {
        id: LayerId(id),
        name: name.into(),
        visible: true,
    }
}

/// Counters for a database whose row maxima are all at or below these values.
///
/// The pinned `Standard` DIMSTYLE/MLEADERSTYLE rows make the style floors 2.
fn seeded_counters() -> IdCounters {
    IdCounters {
        entity: 40,
        block: 8,
        layer: 50,
        layout: 9,
        style: 6,
        dim_style: 7,
        mleader_style: 8,
        linetype: 60,
        scale: 5,
    }
}

#[test]
fn a_deleted_linetype_id_is_never_reused() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(layer(0, "0")).unwrap();
    // The import path rebuilds from the row maximum, so the first create gets 2.
    b.insert_linetype(linetype(1, "DASHED")).unwrap();
    let mut db = b.finish().unwrap();

    let (first, create) = db
        .create_linetype("CENTER", &[2.0, -0.5], TransactionId(1), "create")
        .expect("fresh name");
    assert_eq!(first, LinetypeId(2));
    // A newly created row moves the persisted high-water mark, so the document
    // header must be rewritten with the row.
    assert!(create.scope.header, "a new linetype dirties document.json");

    db.delete_linetype(first, TransactionId(2), "delete")
        .expect("nothing references CENTER");
    let (second, _) = db
        .create_linetype("PHANTOM", &[3.0, -1.0], TransactionId(3), "create again")
        .expect("fresh name");
    assert!(
        second.0 > first.0,
        "the deleted id {} must not come back (got {})",
        first.0,
        second.0
    );
    assert_eq!(second, LinetypeId(3));
}

#[test]
fn seeded_counters_win_over_the_row_maxima() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(layer(0, "0")).unwrap();
    b.insert_layer(layer(1, "WALLS")).unwrap();
    b.insert_layout(crate::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: Vec::new(),
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(line_entity(1, point(0.0, 0.0), point(1.0, 0.0)))
        .unwrap();

    let counters = seeded_counters();
    b.seed_identifiers(counters);
    let mut db = b.finish().expect("the seeds are above every row maximum");

    assert_eq!(db.id_counters(), counters, "the seeds are adopted verbatim");
    assert_eq!(
        db.allocate_entity_id(),
        EntityId(40),
        "the entity allocator used the seed, not max(row) + 1"
    );
    let (created, _) = db
        .create_layer("DOORS", TransactionId(1), "create")
        .expect("fresh layer name");
    assert_eq!(
        created,
        LayerId(50),
        "the layer allocator used the seed, not max(row) + 1"
    );
}

#[test]
fn a_seeded_counter_below_the_row_maximum_is_refused() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(layer(5, "WALLS")).unwrap();
    // Layer 5 exists, so a counter of 1 would hand 5 out again.
    b.seed_identifiers(IdCounters {
        layer: 1,
        ..seed_counters_with_floor_2()
    });
    let error = b.finish().expect_err("the layer counter reuses a live id");
    assert!(
        error
            .to_string()
            .contains("identity.counter_below_high_water"),
        "{error}"
    );
}

#[test]
fn a_seeded_counter_above_the_sequence_ceiling_is_refused() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(layer(0, "0")).unwrap();
    b.seed_identifiers(IdCounters {
        layer: MAX_SEQ + 2,
        ..IdCounters::default()
    });
    let error = b.finish().expect_err("the counter cannot be stored");
    assert!(
        error.to_string().contains("identity.counter_out_of_range"),
        "{error}"
    );
}

#[test]
fn a_seeded_revision_is_the_first_writes_before() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(layer(0, "0")).unwrap();
    b.set_revision(Revision(7));
    let mut db = b.finish().expect("built");

    assert_eq!(db.revision(), Revision(7));
    let (_, change_set) = db
        .create_layer("WALLS", TransactionId(1), "create")
        .expect("fresh layer name");
    assert_eq!(change_set.before, Revision(7));
    assert_eq!(change_set.after, Revision(8));
    assert_eq!(db.revision(), Revision(8));
}

#[test]
fn counters_round_trip_and_adoption_drives_the_next_create() {
    let mut db = writable_db();
    let counters = db.id_counters();
    assert_eq!(counters.entity, 1);
    assert_eq!(counters.block, 1);
    assert_eq!(counters.layer, 1);
    assert_eq!(counters.layout, 2);
    assert_eq!(counters.style, 1);
    assert_eq!(counters.dim_style, 2, "the pinned Standard row is id 1");
    assert_eq!(counters.mleader_style, 2);
    assert_eq!(counters.linetype, 1);
    assert_eq!(counters.scale, 1);

    // Every insert moves exactly the mark its allocator reports.
    assert_eq!(db.allocate_entity_id(), EntityId(1));
    let (created_layer, _) = db
        .create_layer("WALLS", TransactionId(1), "create")
        .expect("fresh layer name");
    assert_eq!(created_layer, LayerId(1));
    let (created_linetype, _) = db
        .create_linetype("CENTER", &[2.0, -0.5], TransactionId(2), "create")
        .expect("fresh name");
    assert_eq!(created_linetype, LinetypeId(1));
    let after_inserts = db.id_counters();
    assert_eq!(after_inserts.entity, 2);
    assert_eq!(after_inserts.layer, 2);
    assert_eq!(after_inserts.linetype, 2);

    // A valid adopted set is used verbatim, and the next create starts there.
    let adopted = seeded_counters();
    db.adopt_id_counters(&adopted)
        .expect("all seeds are above the rows");
    assert_eq!(db.id_counters(), adopted);
    assert_eq!(db.allocate_entity_id(), EntityId(40));
    assert_eq!(db.id_counters().entity, 41);
}

#[test]
fn adoption_refuses_a_counter_that_would_reuse_a_live_id() {
    let mut db = writable_db();
    let (created, _) = db
        .create_layer("WALLS", TransactionId(1), "create")
        .expect("fresh layer name");
    assert_eq!(created, LayerId(1));
    let before = db.id_counters();

    let error = db
        .adopt_id_counters(&IdCounters {
            layer: 1,
            ..seeded_counters()
        })
        .expect_err("layer 1 already exists");
    assert!(
        error
            .to_string()
            .contains("identity.counter_below_high_water"),
        "{error}"
    );
    // A refused adoption leaves every counter where it was.
    assert_eq!(db.id_counters(), before);
}

/// The smallest counters a database with one layer row (id 0) and the pinned
/// style rows accepts.
fn seed_counters_with_floor_2() -> IdCounters {
    IdCounters {
        entity: 1,
        block: 1,
        layer: 1,
        layout: 1,
        style: 1,
        dim_style: 2,
        mleader_style: 2,
        linetype: 1,
        scale: 1,
    }
}
