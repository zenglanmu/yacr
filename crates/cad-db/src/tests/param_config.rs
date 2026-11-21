//! Configuration overlays in the database (ADR-0043 P1).
//!
//! The base table stays the authority: activating a configuration derives
//! geometry from an overlay, never from stored state, and a named selection is
//! runtime-only.

use std::collections::BTreeSet;

use cad_domain::param::{
    Expr, ParamConfig, ParamConfigTable, ParamSeq, PARAM_SUPPRESSED, PARAM_UNKNOWN_CONFIG,
};
use cad_domain::ymodel::{shard_of, ModelKey};
use cad_domain::{EntityId, ObjectId, SemanticGeometry, TransactionId};

use super::*;

// ------------------------------------------------------ configurations ---

fn override_config(name: &str, seq: u64, value: f64) -> ParamConfig {
    let mut config = ParamConfig::new(name).expect("config");
    config
        .set_override(ParamSeq(seq), Expr::num(value))
        .expect("override");
    config
}

#[test]
fn param_values_reflect_the_active_configuration_without_touching_the_base() {
    let mut parameters = ParamTable::new();
    parameters
        .insert(row(7, "r", Expr::num(10.0)))
        .expect("row");
    parameters
        .insert(row(8, "s", Expr::num(20.0)))
        .expect("row");
    let mut config = override_config("thin", 7, 4.0);
    config.suppress(ParamSeq(8));
    let mut configs = ParamConfigTable::new();
    configs.insert(config).expect("config");

    let mut b = base(10.0);
    b.set_parameters(parameters.clone()).expect("parameters");
    b.set_param_configs(configs).expect("configs");
    b.set_param_bindings(
        EntityId(1),
        bindings("radius", Expr::reference(ParamSeq(7))),
    )
    .expect("bindings");
    let mut db = b.finish().expect("finish");
    let before = db.revision();

    let outcome = db
        .set_active_configuration(Some("thin".into()), "select", TransactionId(1))
        .expect("select");
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert_eq!(db.active_configuration(), Some("thin"));
    assert_eq!(outcome.updated, vec![EntityId(1)]);
    assert_eq!(radius_of(&db), 4.0);
    assert_eq!(outcome.values.get(ParamSeq(7)).expect("value").value, 4.0);
    assert!(
        outcome.values.get(ParamSeq(8)).is_none(),
        "a suppressed parameter is absent from the effective evaluation"
    );
    let status = db.param_config_status().expect("status");
    assert_eq!(status.name.as_deref(), Some("thin"));
    assert_eq!(status.overridden, BTreeSet::from([ParamSeq(7)]));
    assert_eq!(status.suppressed, BTreeSet::from([ParamSeq(8)]));
    assert!(status.inherited.is_empty());
    assert_eq!(
        db.parameters(),
        &parameters,
        "the base table is never rewritten by a selection"
    );

    // Selecting the same name is a no-op.
    let same = db
        .set_active_configuration(Some("thin".into()), "select", TransactionId(2))
        .expect("same");
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert!(same.changes.is_empty());

    // An unknown name is refused and changes nothing.
    let error = db
        .set_active_configuration(Some("nope".into()), "select", TransactionId(3))
        .expect_err("unknown configuration");
    assert!(format!("{error}").contains(PARAM_UNKNOWN_CONFIG), "{error}");
    assert_eq!(db.active_configuration(), Some("thin"));
    assert_eq!(db.revision(), Revision(before.0 + 1));

    // Clearing the selection re-derives from the base table.
    let cleared = db
        .set_active_configuration(None, "clear", TransactionId(4))
        .expect("clear");
    assert_eq!(db.revision(), Revision(before.0 + 2));
    assert_eq!(db.active_configuration(), None);
    assert_eq!(radius_of(&db), 10.0);
    assert_eq!(cleared.updated, vec![EntityId(1)]);
}

#[test]
fn a_configuration_change_re_derives_every_bound_element() {
    let mut configs = ParamConfigTable::new();
    configs
        .insert(override_config("thin", 7, 4.0))
        .expect("config");
    let mut parameters = ParamTable::new();
    parameters
        .insert(row(7, "r", Expr::num(10.0)))
        .expect("row");

    let mut b = base(10.0);
    let mut second = circle(10.0);
    second.id = EntityId(2);
    second.object.id = ObjectId(2);
    b.insert_entity(second).expect("second entity");
    b.set_parameters(parameters).expect("parameters");
    b.set_param_configs(configs).expect("configs");
    for id in [EntityId(1), EntityId(2)] {
        b.set_param_bindings(id, bindings("radius", Expr::reference(ParamSeq(7))))
            .expect("bindings");
    }
    let mut db = b.finish().expect("finish");
    let before = db.revision();

    let outcome = db
        .set_active_configuration(Some("thin".into()), "select", TransactionId(1))
        .expect("select");
    assert_eq!(db.revision(), Revision(before.0 + 1), "one revision");
    assert_eq!(outcome.updated, vec![EntityId(1), EntityId(2)]);
    for id in [EntityId(1), EntityId(2)] {
        match db.entities[&id].geometry {
            SemanticGeometry::Circle { radius, .. } => assert_eq!(radius, 4.0),
            ref other => panic!("expected a circle, got {other:?}"),
        }
    }
    assert!(
        outcome
            .changes
            .scope
            .shards
            .contains(&(ModelKey::ModelSpace, shard_of(1))),
        "every rewritten shard is dirty"
    );
    assert!(
        outcome.changes.scope.tables.is_empty(),
        "the configuration table did not change"
    );
    assert!(
        !outcome.changes.scope.header,
        "the selection is runtime state"
    );
}

#[test]
fn a_base_edit_masked_by_an_override_keeps_geometry_and_marks_the_shard() {
    let mut configs = ParamConfigTable::new();
    configs
        .insert(override_config("thin", 7, 4.0))
        .expect("config");
    let mut parameters = ParamTable::new();
    parameters
        .insert(row(7, "r", Expr::num(10.0)))
        .expect("row");

    let mut b = base(10.0);
    b.set_parameters(parameters).expect("parameters");
    b.set_param_configs(configs).expect("configs");
    b.set_param_bindings(
        EntityId(1),
        bindings("radius", Expr::reference(ParamSeq(7))),
    )
    .expect("bindings");
    let mut db = b.finish().expect("finish");
    db.set_active_configuration(Some("thin".into()), "select", TransactionId(1))
        .expect("select");
    assert_eq!(radius_of(&db), 4.0);
    let before = db.revision();

    // Editing the base row does not change the derived value: the override wins.
    let patch = ParamPatch {
        seq: ParamSeq(7),
        before: Some(row(7, "r", Expr::num(10.0))),
        after: Some(row(7, "r", Expr::num(50.0))),
    };
    let outcome = db
        .apply_parametric_update("edit masked base row", TransactionId(2), &[patch], &[])
        .expect("apply");
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert_eq!(radius_of(&db), 4.0, "the override still masks the base row");
    assert!(outcome.updated.is_empty());
    assert!(
        outcome
            .changes
            .scope
            .tables
            .contains(&TableKind::Parameters),
        "the base table row changed"
    );
    assert!(
        outcome
            .changes
            .scope
            .shards
            .contains(&(ModelKey::ModelSpace, shard_of(1))),
        "the affected element shard is still marked"
    );
    assert_eq!(
        db.parameters().get(ParamSeq(7)).expect("row").expr,
        Expr::num(50.0),
        "phase 3 stores the base row, never the effective overlay"
    );
}

#[test]
fn a_binding_to_a_suppressed_parameter_blocks_instead_of_refusing() {
    let mut parameters = ParamTable::new();
    parameters
        .insert(row(7, "r", Expr::num(10.0)))
        .expect("row");
    parameters
        .insert(row(8, "s", Expr::num(20.0)))
        .expect("row");
    let mut config = ParamConfig::new("thin").expect("config");
    config.suppress(ParamSeq(8));
    let mut configs = ParamConfigTable::new();
    configs.insert(config).expect("config");

    let mut b = base(20.0);
    b.set_parameters(parameters).expect("parameters");
    b.set_param_configs(configs).expect("configs");
    let mut db = b.finish().expect("finish");
    db.set_active_configuration(Some("thin".into()), "select", TransactionId(1))
        .expect("select");
    let before = db.revision();

    let patch = ParamBindingPatch {
        id: EntityId(1),
        before: None,
        after: Some(bindings("radius", Expr::reference(ParamSeq(8)))),
        before_geometry: None,
        after_geometry: None,
    };
    let outcome = db
        .apply_parametric_update("bind suppressed", TransactionId(2), &[], &[patch])
        .expect("a suppressed binding is second tier, not a refusal");
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert_eq!(radius_of(&db), 20.0, "last-good geometry is kept");
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].code, PARAM_SUPPRESSED);
    assert!(
        db.param_bindings(EntityId(1)).is_some(),
        "the binding itself is stored"
    );
}

#[test]
fn the_builder_refuses_a_broken_configuration_table() {
    let parameters = {
        let mut table = ParamTable::new();
        table.insert(row(7, "r", Expr::num(1.0))).expect("row");
        table
    };

    let mut cycle = ParamConfigTable::new();
    cycle
        .insert(
            ParamConfig::new("a")
                .expect("a")
                .with_parent("b")
                .expect("parent"),
        )
        .expect("row");
    cycle
        .insert(
            ParamConfig::new("b")
                .expect("b")
                .with_parent("a")
                .expect("parent"),
        )
        .expect("row");
    let mut b = base(10.0);
    b.set_parameters(parameters.clone()).expect("parameters");
    b.set_param_configs(cycle).expect("configs");
    let error = b
        .finish()
        .expect_err("a parent cycle must refuse the build");
    assert!(
        format!("{error}").contains(cad_domain::param::PARAM_CONFIG_CYCLE),
        "{error}"
    );

    let mut unknown = ParamConfigTable::new();
    unknown
        .insert(
            ParamConfig::new("a")
                .expect("a")
                .with_parent("missing")
                .expect("parent"),
        )
        .expect("row");
    let mut b = base(10.0);
    b.set_parameters(parameters).expect("parameters");
    b.set_param_configs(unknown).expect("configs");
    let error = b
        .finish()
        .expect_err("an unknown parent must refuse the build");
    assert!(format!("{error}").contains(PARAM_UNKNOWN_CONFIG), "{error}");
}

#[test]
fn deleting_a_parameter_an_inactive_configuration_names_is_refused() {
    let mut parameters = cad_domain::param::ParamTable::new();
    parameters
        .insert(row(1, "r", Expr::num(10.0)))
        .expect("row");
    let mut configs = ParamConfigTable::new();
    configs
        .insert(override_config("thin", 1, 4.0))
        .expect("config");
    let mut b = base(10.0);
    b.set_parameters(parameters.clone()).expect("parameters");
    b.set_param_configs(configs).expect("configs");
    let mut db = b.finish().expect("finish");
    let before = db.revision();

    let patch = ParamPatch {
        seq: ParamSeq(1),
        before: Some(row(1, "r", Expr::num(10.0))),
        after: None,
    };
    let error = db
        .apply_parametric_update("delete", TransactionId(2), &[patch], &[])
        .expect_err("an inactive config still names sequence 1");
    assert!(
        format!("{error}").contains(cad_domain::param::PARAM_UNKNOWN_REFERENCE),
        "{error}"
    );
    assert_eq!(db.revision(), before);
    assert_eq!(db.parameters(), &parameters, "base table unchanged");
}
