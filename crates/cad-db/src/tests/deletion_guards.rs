//! Save/open symmetry guards for entity deletion (ADR-0043 P1/P2/P3).
//!
//! Deleting an element must not leave a parameter binding, feature row or
//! relation endpoint that the writer would save and the reader refuse.

use cad_domain::param::{Expr, ParamBindings, ParamKind, ParamRow, ParamSeq, ParamTable};
use cad_domain::relation::{InContextState, RelationReference};
use cad_domain::{DatabaseId, LayerId, ObjectId, Point3, SpaceId};
use cad_domain::{EntityId, Revision, SemanticGeometry, TransactionId};

use super::*;
use crate::relation::InContextRelation;
use crate::{
    FeatureDefinition, FeatureDefinitions, FeatureOperation, InContextRelations, ParamPatch,
};

fn line(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: Point3::default(),
            end: Point3 {
                x: 1.0,
                ..Point3::default()
            },
        },
        draw_order: 0,
    }
}

fn circle(id: u128, radius: f64) -> DbEntity {
    let mut entity = line(id);
    entity.object.type_key = "AcDbCircle".into();
    entity.geometry = SemanticGeometry::Circle {
        center: Point3::default(),
        normal: Point3::default(),
        radius,
    };
    entity
}

fn parameters() -> ParamTable {
    let mut table = ParamTable::new();
    table
        .insert(ParamRow::new(ParamSeq(7), "r", ParamKind::Length, Expr::num(5.0)).expect("row"))
        .expect("insert");
    table
}

fn feature() -> FeatureDefinition {
    FeatureDefinition {
        id: EntityId(1),
        order: 0,
        operation: FeatureOperation::Extrude,
        inputs: vec![EntityId(2)],
        parameters: ParamBindings::from_entries([(
            "radius".to_string(),
            Expr::reference(ParamSeq(7)),
        )])
        .expect("bindings"),
    }
}

fn relation() -> InContextRelation {
    InContextRelation {
        source: EntityId(1),
        target: EntityId(3),
        reference: RelationReference::Face {
            key: "f1".to_string(),
        },
        state: InContextState::InContext,
        label: None,
    }
}

fn guarded_db() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(crate::tables::Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .expect("layer");
    b.insert_entity(circle(1, 10.0)).expect("feature entity");
    b.insert_entity(line(2)).expect("input entity");
    b.insert_entity(line(3)).expect("relation target");
    b.set_parameters(parameters()).expect("parameters");
    let mut features = FeatureDefinitions::new();
    features.insert(feature()).expect("feature");
    b.set_features(features).expect("features");
    let mut relations = InContextRelations::new();
    relations.insert(relation()).expect("relation");
    b.set_relations(relations).expect("relations");
    b.finish().expect("finish")
}

fn delete(db: &mut DrawingDatabase, id: EntityId) -> CadResult<ChangeSet> {
    db.apply_drawing_changes("delete", TransactionId(9), vec![(id, None)])
}

#[test]
fn deleting_a_feature_element_or_input_is_refused() {
    let mut db = guarded_db();
    let before = db.revision();

    let error = delete(&mut db, EntityId(1)).expect_err("feature definition target");
    assert!(format!("{error}").contains("feature.referenced"), "{error}");
    assert_eq!(db.revision(), before);
    assert!(db.entities.contains_key(&EntityId(1)), "nothing mutated");

    let error = delete(&mut db, EntityId(2)).expect_err("feature input");
    assert!(format!("{error}").contains("feature.referenced"), "{error}");
    assert!(db.entities.contains_key(&EntityId(2)), "nothing mutated");
}

#[test]
fn deleting_a_relation_endpoint_is_refused() {
    let mut db = guarded_db();
    let before = db.revision();
    let error = delete(&mut db, EntityId(3)).expect_err("relation target");
    assert!(
        format!("{error}").contains("rel.endpoint_missing"),
        "{error}"
    );
    assert_eq!(db.revision(), before);
    assert!(db.entities.contains_key(&EntityId(3)), "nothing mutated");
}

#[test]
fn deleting_a_parameter_a_feature_binds_is_refused() {
    let mut db = guarded_db();
    let before = db.revision();
    let patch = ParamPatch {
        seq: ParamSeq(7),
        before: Some(
            ParamRow::new(ParamSeq(7), "r", ParamKind::Length, Expr::num(5.0)).expect("row"),
        ),
        after: None,
    };
    let error = db
        .apply_parametric_update("delete bound param", TransactionId(2), &[patch], &[])
        .expect_err("a feature still binds it");
    assert!(
        format!("{error}").contains(cad_domain::param::PARAM_UNKNOWN_REFERENCE),
        "{error}"
    );
    assert_eq!(db.revision(), before);
    assert!(db.parameters().get(ParamSeq(7)).is_some());
}

#[test]
fn deleting_a_bound_element_prunes_its_parameter_rows() {
    let mut parameters = ParamTable::new();
    parameters
        .insert(ParamRow::new(ParamSeq(7), "r", ParamKind::Length, Expr::num(10.0)).expect("row"))
        .expect("insert");
    let mut configs = cad_domain::param::ParamConfigTable::new();
    let mut config = cad_domain::param::ParamConfig::new("thin").expect("config");
    config
        .set_override(ParamSeq(7), Expr::num(4.0))
        .expect("override");
    configs.insert(config).expect("config");

    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(crate::tables::Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .expect("layer");
    b.insert_entity(circle(1, 10.0)).expect("entity");
    b.set_parameters(parameters).expect("parameters");
    b.set_param_configs(configs).expect("configs");
    b.set_param_bindings(
        EntityId(1),
        ParamBindings::from_entries([("radius".to_string(), Expr::reference(ParamSeq(7)))])
            .expect("bindings"),
    )
    .expect("bindings");
    let mut db = b.finish().expect("finish");
    assert!(db.param_bindings(EntityId(1)).is_some());

    delete(&mut db, EntityId(1)).expect("an unguarded element deletes");
    assert!(
        db.param_bindings(EntityId(1)).is_none(),
        "the orphan binding is pruned"
    );

    // Selecting a configuration after the delete must not index the removed row.
    let outcome = db
        .set_active_configuration(Some("thin".into()), "select", TransactionId(3))
        .expect("no orphan invariant");
    assert!(outcome.updated.is_empty());
    assert_eq!(db.active_configuration(), Some("thin"));
}
