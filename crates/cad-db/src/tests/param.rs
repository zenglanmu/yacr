//! The merged parametric write path (`DrawingDatabase::apply_parametric_update`).
//!
//! What must hold (ADR-0043 D6/D8):
//!
//! 1. static failures refuse the whole batch — the database, the side table and
//!    the revision stay untouched;
//! 2. one batch is **one** revision and **one** `ChangeSet`;
//! 3. the scope names `TableKind::Parameters` **and** every shard whose row was
//!    rewritten. `copyable_shard` in `cad-store-zip` copies any shard it was not
//!    told about, so a missing `element()` would save the old `params`/geometry
//!    bytes on the second save while reporting success;
//! 4. a geometry failure keeps last-good geometry and publishes
//!    `param.blocked`; it never refuses the parameter commit;
//! 5. an empty batch is a no-op.

use std::collections::BTreeSet;

use cad_domain::param::{
    BinaryOp, Expr, ParamSeq, ParamTable, PARAM_GEOMETRY_FAILED, PARAM_UNKNOWN_REFERENCE,
    PARAM_UNKNOWN_SLOT,
};
use cad_domain::ymodel::{shard_of, ModelKey};
use cad_domain::{
    DatabaseId, DocumentIdentity, EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry,
    TransactionId, UnitContext,
};
use serde_json::{json, Value};

use super::*;

#[test]
fn one_commit_is_one_revision_and_one_changeset() {
    let mut db = bound_db();
    let before = db.revision();
    let old = row(7, "r", Expr::num(10.0));
    let new = row(7, "r", Expr::num(20.0));
    let patch = ParamPatch {
        seq: ParamSeq(7),
        before: Some(old),
        after: Some(new),
    };

    let outcome = db
        .apply_parametric_update(
            "edit parameter",
            TransactionId(1),
            std::slice::from_ref(&patch),
            &[],
        )
        .expect("apply");

    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert_eq!(outcome.changes.before, before);
    assert_eq!(outcome.changes.after, db.revision());
    assert_eq!(
        outcome.changes.changes,
        vec![ObjectChange::Update(ObjectId(1), ChangeMask::GEOMETRY)]
    );
    assert!(
        outcome
            .changes
            .scope
            .tables
            .contains(&TableKind::Parameters),
        "the table row changed"
    );
    assert!(
        outcome
            .changes
            .scope
            .shards
            .contains(&(ModelKey::ModelSpace, shard_of(1))),
        "the rewritten element row dirties its shard"
    );
    assert!(
        !outcome.changes.scope.header,
        "parameter identity is table-local: no header counter moved"
    );
    assert_eq!(outcome.updated, vec![EntityId(1)]);
    assert_eq!(outcome.evaluated, BTreeSet::from([EntityId(1)]));
    assert!(outcome.diagnostics.is_empty());
    assert_eq!(outcome.values.get(ParamSeq(7)).expect("value").value, 20.0);
    assert_eq!(radius_of(&db), 20.0, "the derived geometry was replaced");

    // A second identical apply changes nothing: same table, same geometry.
    let applied = db.clone();
    let again = db
        .apply_parametric_update("edit parameter", TransactionId(2), &[patch], &[])
        .expect("re-apply");
    assert_eq!(db.revision(), applied.revision(), "no content changed");
    assert!(again.changes.changes.is_empty());
}

#[test]
fn an_empty_batch_is_a_noop() {
    let mut db = bound_db();
    let before = db.revision();
    let outcome = db
        .apply_parametric_update("noop", TransactionId(1), &[], &[])
        .expect("apply");
    assert_eq!(db.revision(), before);
    assert!(outcome.changes.is_empty());
    assert!(outcome.changes.scope.is_empty());
    assert!(outcome.updated.is_empty());
    assert!(outcome.diagnostics.is_empty());
}

#[test]
fn a_noop_binding_patch_still_marks_its_shard() {
    let mut db = bound_db();
    let before = db.revision();
    let current = bindings("radius", Expr::reference(ParamSeq(7)));
    let patch = ParamBindingPatch {
        id: EntityId(1),
        before: Some(current.clone()),
        after: Some(current),
        before_geometry: None,
        after_geometry: None,
    };
    let outcome = db
        .apply_parametric_update("noop", TransactionId(1), &[], &[patch])
        .expect("apply");
    assert_eq!(db.revision(), before, "an unchanged patch stores nothing");
    assert!(outcome.changes.changes.is_empty());
    assert!(
        outcome
            .changes
            .scope
            .shards
            .contains(&(ModelKey::ModelSpace, shard_of(1))),
        "the addressed row must not be raw-copied on the next save"
    );
}

#[test]
fn a_geometry_failure_keeps_last_good_geometry_and_reports_blocked() {
    let mut db = base(10.0).finish().expect("finish");
    let before = db.revision();
    // A parameter edit and the binding that first uses it, in one batch.
    let param = ParamPatch {
        seq: ParamSeq(8),
        before: None,
        after: Some(row(8, "zero", Expr::num(0.0))),
    };
    let binding = ParamBindingPatch {
        id: EntityId(1),
        before: None,
        after: Some(bindings("radius", Expr::reference(ParamSeq(8)))),
        before_geometry: None,
        after_geometry: None,
    };

    let outcome = db
        .apply_parametric_update(
            "bind a degenerate radius",
            TransactionId(1),
            &[param],
            &[binding],
        )
        .expect("static validation passes; only the geometry is bad");

    assert_eq!(
        db.revision(),
        Revision(before.0 + 1),
        "the parameters commit"
    );
    assert_eq!(
        outcome.values.get(ParamSeq(8)).expect("value").value,
        0.0,
        "a legal zero is stored and evaluated"
    );
    assert!(outcome.updated.is_empty(), "no new geometry was written");
    assert_eq!(radius_of(&db), 10.0, "last-good geometry is preserved");
    assert_eq!(outcome.diagnostics.len(), 1);
    let diagnostic = &outcome.diagnostics[0];
    assert_eq!(diagnostic.entity, Some(EntityId(1)));
    assert_eq!(diagnostic.code, cad_domain::param::PARAM_BLOCKED);
    assert!(
        diagnostic.detail.contains(PARAM_GEOMETRY_FAILED),
        "{}",
        diagnostic.detail
    );
    assert!(outcome.changes.changes.is_empty());
    assert!(
        outcome
            .changes
            .scope
            .shards
            .contains(&(ModelKey::ModelSpace, shard_of(1))),
        "the params block itself was written, so its shard is dirty"
    );
}

#[test]
fn an_unknown_slot_refuses_the_whole_batch() {
    let mut db = bound_db();
    let before = db.revision();
    let patch = ParamBindingPatch {
        id: EntityId(1),
        before: None,
        after: Some(bindings("diameter", Expr::reference(ParamSeq(7)))),
        before_geometry: None,
        after_geometry: None,
    };
    let error = db
        .apply_parametric_update("bad slot", TransactionId(1), &[], &[patch])
        .expect_err("unknown slot is a static failure");
    assert!(format!("{error}").contains(PARAM_UNKNOWN_SLOT), "{error}");
    assert_eq!(db.revision(), before);
    assert!(
        db.param_bindings(EntityId(1)).is_some(),
        "nothing was written"
    );
}

#[test]
fn a_dangling_reference_refuses_the_whole_batch() {
    let mut db = base(10.0).finish().expect("finish");
    let before = db.revision();
    let patch = ParamPatch {
        seq: ParamSeq(9),
        before: None,
        after: Some(row(9, "dangling", Expr::reference(ParamSeq(99)))),
    };
    let error = db
        .apply_parametric_update("dangling", TransactionId(1), &[patch], &[])
        .expect_err("a dangling reference is a static failure");
    assert!(
        format!("{error}").contains(PARAM_UNKNOWN_REFERENCE),
        "{error}"
    );
    assert_eq!(db.revision(), before);
    assert!(db.parameters().is_empty(), "no row was installed");
}

#[test]
fn a_cycle_refuses_the_whole_batch() {
    let mut db = base(10.0).finish().expect("finish");
    let before = db.revision();
    let a = ParamPatch {
        seq: ParamSeq(1),
        before: None,
        after: Some(row(1, "a", Expr::reference(ParamSeq(2)))),
    };
    let b = ParamPatch {
        seq: ParamSeq(2),
        before: None,
        after: Some(row(2, "b", Expr::reference(ParamSeq(1)))),
    };
    let error = db
        .apply_parametric_update("cycle", TransactionId(1), &[a, b], &[])
        .expect_err("a cycle is a static failure");
    assert!(
        format!("{error}").contains(cad_domain::param::PARAM_CYCLE),
        "{error}"
    );
    assert_eq!(db.revision(), before);
    assert!(db.parameters().is_empty());
}

#[test]
fn a_binding_patch_for_an_unknown_entity_is_refused() {
    let mut db = bound_db();
    let before = db.revision();
    let patch = ParamBindingPatch {
        id: EntityId(99),
        before: None,
        after: Some(bindings("radius", Expr::num(1.0))),
        before_geometry: None,
        after_geometry: None,
    };
    let error = db
        .apply_parametric_update("missing", TransactionId(1), &[], &[patch])
        .expect_err("the target must exist");
    assert!(
        format!("{error}").contains("param.target_missing"),
        "{error}"
    );
    assert_eq!(db.revision(), before);
}

#[test]
fn a_builder_binding_for_an_unknown_entity_is_refused() {
    let mut b = base(10.0);
    let error = b
        .set_param_bindings(EntityId(99), bindings("radius", Expr::num(1.0)))
        .expect_err("the target must exist");
    assert!(
        format!("{error}").contains("param.target_missing"),
        "{error}"
    );
}

#[test]
fn the_same_batch_on_two_identical_databases_is_deterministic() {
    let patch = ParamPatch {
        seq: ParamSeq(7),
        before: Some(row(7, "r", Expr::num(10.0))),
        after: Some(row(
            7,
            "r",
            Expr::binary(BinaryOp::Mul, Expr::num(10.0), Expr::num(3.0)),
        )),
    };
    let mut left = bound_db();
    let mut right = bound_db();
    let a = left
        .apply_parametric_update("same", TransactionId(1), std::slice::from_ref(&patch), &[])
        .expect("apply");
    let b = right
        .apply_parametric_update("same", TransactionId(1), &[patch], &[])
        .expect("apply");
    assert_eq!(a, b, "same input, same outcome");
    assert_eq!(radius_of(&left), 30.0);
    assert_eq!(radius_of(&right), 30.0);
}

// ------------------------------------------------- load-time reconciliation ---

fn snapshot_of(db: &DrawingDatabase) -> DocumentSnapshot {
    db.snapshot(
        UnitContext::drawing_units(),
        DocumentIdentity::Temporary(1),
        Vec::new(),
        db.revision(),
    )
    .expect("snapshot")
}

/// A circle stored with `stored`, bound to a parameter that evaluates to
/// `evaluated`. The builder does not derive geometry, so the two may disagree —
/// that is exactly what an archive written elsewhere looks like.
fn stale_radius_db(stored: f64, evaluated: f64) -> DrawingDatabase {
    let mut parameters = ParamTable::new();
    parameters
        .insert(row(7, "r", Expr::num(evaluated)))
        .expect("row");
    let mut b = base(stored);
    b.set_parameters(parameters).expect("parameters");
    b.set_param_bindings(
        EntityId(1),
        bindings("radius", Expr::reference(ParamSeq(7))),
    )
    .expect("bindings");
    b.finish()
        .expect("the builder validates slots, not geometry")
}

#[test]
fn a_stale_stored_radius_is_diagnosed_but_not_rewritten_on_open() {
    let db = stale_radius_db(10.0, 20.0);
    assert_eq!(radius_of(&db), 10.0, "the builder stored the row as given");
    let revision = db.revision();

    let loaded = DrawingDatabase::from_snapshot(&snapshot_of(&db)).expect("open");
    assert_eq!(
        radius_of(&loaded),
        10.0,
        "load is a diagnosis: the archive stays the authority until an edit"
    );
    assert_eq!(
        loaded.revision(),
        revision,
        "load derivation is not an edit"
    );
    assert_eq!(loaded.param_diagnostics().len(), 1);
    let diagnostic = &loaded.param_diagnostics()[0];
    assert_eq!(diagnostic.entity, Some(EntityId(1)));
    assert_eq!(diagnostic.code, cad_domain::param::PARAM_RECOMPUTED);
    assert!(
        diagnostic.detail.contains("stored geometry disagrees"),
        "{}",
        diagnostic.detail
    );
}

#[test]
fn a_degenerate_binding_keeps_stored_geometry_and_repeats_on_reopen() {
    let db = stale_radius_db(10.0, 0.0);
    let first = DrawingDatabase::from_snapshot(&snapshot_of(&db)).expect("open");
    assert_eq!(radius_of(&first), 10.0, "geometry is never fabricated");
    assert_eq!(first.param_diagnostics().len(), 1);
    assert_eq!(
        first.param_diagnostics()[0].code,
        cad_domain::param::PARAM_BLOCKED
    );
    assert!(
        first.param_diagnostics()[0]
            .detail
            .contains(PARAM_GEOMETRY_FAILED),
        "{}",
        first.param_diagnostics()[0].detail
    );

    // Saving and reopening replays the same pure derivation: the identical
    // diagnostic must come back, not a silent success.
    let second = DrawingDatabase::from_snapshot(&snapshot_of(&first)).expect("reopen");
    assert_eq!(radius_of(&second), 10.0);
    assert_eq!(second.param_diagnostics(), first.param_diagnostics());
}

// ------------------------------------------------ bind / unbind replay ---

fn circle_geometry(radius: f64) -> SemanticGeometry {
    SemanticGeometry::Circle {
        center: Point3::default(),
        normal: Point3::default(),
        radius,
    }
}

#[test]
fn undo_of_a_bind_restores_the_pre_bind_geometry() {
    // A radius-10 circle, a document parameter `r = 20`, no binding yet.
    let mut parameters = ParamTable::new();
    parameters
        .insert(row(7, "r", Expr::num(20.0)))
        .expect("row");
    let mut b = base(10.0);
    b.set_parameters(parameters).expect("parameters");
    let mut db = b.finish().expect("finish");
    let before = db.revision();

    // Forward bind: no explicit geometry, so it is re-derived from `r`.
    let bound = bindings("radius", Expr::reference(ParamSeq(7)));
    let forward = ParamBindingPatch {
        id: EntityId(1),
        before: None,
        after: Some(bound.clone()),
        before_geometry: Some(circle_geometry(10.0)),
        after_geometry: Some(circle_geometry(20.0)),
    };
    let outcome = db
        .apply_parametric_update("bind", TransactionId(1), &[], &[forward])
        .expect("bind");
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert_eq!(outcome.updated, vec![EntityId(1)]);
    assert_eq!(radius_of(&db), 20.0);

    // Undo: the binding is removed and the pre-bind geometry is installed
    // verbatim; the generic "unbind keeps last-good" rule must not win here.
    let undo = ParamBindingPatch {
        id: EntityId(1),
        before: Some(bound),
        after: None,
        before_geometry: Some(circle_geometry(20.0)),
        after_geometry: Some(circle_geometry(10.0)),
    };
    let undo_outcome = db
        .apply_parametric_update("undo bind", TransactionId(2), &[], &[undo])
        .expect("undo");
    assert_eq!(
        db.revision(),
        Revision(before.0 + 2),
        "one undo, one revision"
    );
    assert_eq!(radius_of(&db), 10.0, "the pre-bind radius is restored");
    assert_eq!(undo_outcome.updated, vec![EntityId(1)]);
    assert_eq!(
        undo_outcome.changes.changes,
        vec![ObjectChange::Update(ObjectId(1), ChangeMask::GEOMETRY)],
        "one ChangeSet entry for the restored geometry"
    );
    assert!(db.param_bindings(EntityId(1)).is_none());
}

#[test]
fn unbinding_forward_keeps_the_last_evaluated_geometry() {
    let mut db = bound_db();
    // Move the parameter so the last successful evaluation is radius 20.
    let patch = ParamPatch {
        seq: ParamSeq(7),
        before: Some(row(7, "r", Expr::num(10.0))),
        after: Some(row(7, "r", Expr::num(20.0))),
    };
    db.apply_parametric_update("grow", TransactionId(1), &[patch], &[])
        .expect("grow");
    assert_eq!(radius_of(&db), 20.0);
    let before = db.revision();

    // Forward unbind carries no after geometry, so the derived 20 stays.
    let unbind = ParamBindingPatch {
        id: EntityId(1),
        before: Some(bindings("radius", Expr::reference(ParamSeq(7)))),
        after: None,
        before_geometry: Some(circle_geometry(20.0)),
        after_geometry: None,
    };
    let outcome = db
        .apply_parametric_update("unbind", TransactionId(2), &[], &[unbind])
        .expect("unbind");
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert_eq!(radius_of(&db), 20.0, "last evaluated geometry is kept");
    assert!(outcome.updated.is_empty(), "no geometry was replaced");
}

// ---------------------------------------------------- static vs blocked ---

#[test]
fn a_dangling_binding_is_refused_not_downgraded_to_blocked() {
    let mut db = base(10.0).finish().expect("finish");
    let before = db.revision();
    let patch = ParamBindingPatch {
        id: EntityId(1),
        before: None,
        after: Some(bindings("radius", Expr::reference(ParamSeq(99)))),
        before_geometry: None,
        after_geometry: None,
    };
    let error = db
        .apply_parametric_update("dangling binding", TransactionId(1), &[], &[patch])
        .expect_err("a dangling binding is a static failure, not last-good geometry");
    assert!(
        format!("{error}").contains(PARAM_UNKNOWN_REFERENCE),
        "{error}"
    );
    assert_eq!(db.revision(), before);
    assert!(db.param_bindings(EntityId(1)).is_none());
}

// ---------------------------------------------------------- locked layer ---

fn lock_layer_zero(db: &mut DrawingDatabase) {
    db.set_layer_attributes(
        LayerId(0),
        LayerAttributes {
            locked: true,
            ..LayerAttributes::default()
        },
        TransactionId(99),
        "lock",
    )
    .expect("lock layer 0");
}

#[test]
fn a_geometry_write_on_a_locked_layer_is_refused() {
    let mut db = bound_db();
    lock_layer_zero(&mut db);
    let before = db.revision();
    let patch = ParamPatch {
        seq: ParamSeq(7),
        before: Some(row(7, "r", Expr::num(10.0))),
        after: Some(row(7, "r", Expr::num(20.0))),
    };
    let error = db
        .apply_parametric_update("grow", TransactionId(1), &[patch], &[])
        .expect_err("a locked layer refuses the geometry write");
    assert!(format!("{error}").contains("layer.locked"), "{error}");
    assert_eq!(db.revision(), before);
    assert_eq!(radius_of(&db), 10.0, "nothing was mutated");
    assert_eq!(
        db.parameters().get(ParamSeq(7)).expect("row").expr,
        Expr::num(10.0),
        "the table row was not written either"
    );
}

#[test]
fn a_binding_only_change_on_a_locked_layer_is_refused() {
    let mut db = bound_db();
    lock_layer_zero(&mut db);
    let before = db.revision();
    // The new binding evaluates to the same radius, so only the `params` block
    // changes — still a row modification.
    let patch = ParamBindingPatch {
        id: EntityId(1),
        before: Some(bindings("radius", Expr::reference(ParamSeq(7)))),
        after: Some(bindings("radius", Expr::num(10.0))),
        before_geometry: None,
        after_geometry: None,
    };
    let error = db
        .apply_parametric_update("rebind", TransactionId(1), &[], &[patch])
        .expect_err("a locked layer refuses the row write");
    assert!(format!("{error}").contains("layer.locked"), "{error}");
    assert_eq!(db.revision(), before);
    assert_eq!(
        db.param_bindings(EntityId(1)),
        Some(&bindings("radius", Expr::reference(ParamSeq(7)))),
        "the binding was not written"
    );
}

// ------------------------------------------------------ Insert + budgets ---

#[test]
fn a_binding_on_an_insert_is_refused_by_the_build() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .expect("layer");
    let mut entity = circle(1.0);
    entity.geometry = SemanticGeometry::Insert {
        block: cad_domain::BlockId(1),
        transform: cad_domain::Transform3::identity(),
    };
    b.insert_entity(entity).expect("entity");
    b.set_param_bindings(EntityId(1), bindings("radius", Expr::num(1.0)))
        .expect("bindings");
    let error = b.finish().expect_err("Insert has no P0 slots");
    let message = format!("{error}");
    // The domain currently maps `cad:Insert` to a key with an empty slot table,
    // so the refusal is `param.unknown_slot`; once the mapping is dropped it is
    // `param.no_evaluator`. Either way the build must refuse.
    assert!(
        message.contains(cad_domain::param::PARAM_NO_EVALUATOR)
            || message.contains(PARAM_UNKNOWN_SLOT),
        "{error}"
    );
}

#[test]
fn an_over_budget_parameter_table_is_refused_on_encode_and_decode() {
    fn tree(nodes: u32) -> Expr {
        if nodes <= 1 {
            Expr::num(1.0)
        } else {
            Expr::binary(BinaryOp::Add, tree(nodes / 2), tree(nodes - 1 - nodes / 2))
        }
    }
    // 255 nodes (the per-expression budget) encode to roughly 5 KiB; enough rows
    // push the table past the 8 MiB budget.
    let expr = tree(255);
    let rows: Vec<Value> = (1..=1800)
        .map(|seq| {
            json!({
                "seq": seq,
                "name": format!("p{seq}"),
                "kind": "length",
                "expr": parameters::encode_expr(&expr),
            })
        })
        .collect();
    let mut tables = crate::snapshot::TableSnapshot::default();
    tables.set(TableKind::Parameters, rows.clone());
    let error = crate::snapshot::parameters::decode_parameters(&tables)
        .expect_err("an over-budget table is refused on decode");
    assert!(
        error
            .detail
            .contains(cad_domain::param::PARAM_BUDGET_EXCEEDED),
        "{}",
        error.detail
    );

    let mut table = ParamTable::new();
    for seq in 1..=1800u64 {
        table
            .insert(row(seq, &format!("p{seq}"), expr.clone()))
            .expect("row");
    }
    let mut b = base(10.0);
    b.set_parameters(table).expect("parameters");
    let db = b.finish().expect("the table itself is legal");
    let error = crate::snapshot::parameters::encode_parameters(&db)
        .expect_err("an over-budget table is refused on encode");
    assert!(
        error
            .detail
            .contains(cad_domain::param::PARAM_BUDGET_EXCEEDED),
        "{}",
        error.detail
    );
}
