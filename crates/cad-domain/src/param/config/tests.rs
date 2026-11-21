use super::*;
use crate::param::ParamKind;

fn seq(value: u64) -> ParamSeq {
    ParamSeq(value)
}

fn base() -> ParamTable {
    ParamTable::from_rows([
        ParamRow::new(seq(1), "thickness", ParamKind::Length, Expr::num(10.0)).unwrap(),
        ParamRow::new(seq(2), "hole", ParamKind::Length, Expr::num(5.0)).unwrap(),
    ])
    .unwrap()
}

fn config(name: &str) -> ParamConfig {
    ParamConfig::new(name).unwrap()
}

#[test]
fn an_override_replaces_the_expression_and_marks_the_status() {
    let base = base();
    let mut thin = config("thin");
    thin.set_override(seq(1), Expr::num(4.0)).unwrap();
    let mut configs = ParamConfigTable::new();
    configs.insert(thin).unwrap();

    let (effective, status) = resolve_active(&base, &configs, Some("thin")).unwrap();
    assert_eq!(effective.get(seq(1)).unwrap().expr, Expr::num(4.0));
    assert_eq!(effective.get(seq(1)).unwrap().name, "thickness");
    assert_eq!(status.overridden, BTreeSet::from([seq(1)]));
    assert_eq!(status.inherited, BTreeSet::from([seq(2)]));
    assert_eq!(status.name.as_deref(), Some("thin"));
    // The base table is untouched (overlays are derived).
    assert_eq!(base.get(seq(1)).unwrap().expr, Expr::num(10.0));
}

#[test]
fn a_child_wins_over_its_parent() {
    let base = base();
    let mut parent = config("parent");
    parent.set_override(seq(1), Expr::num(8.0)).unwrap();
    let mut child = config("child");
    child.set_override(seq(1), Expr::num(2.0)).unwrap();
    child.parent = Some("parent".to_string());
    let mut configs = ParamConfigTable::new();
    configs.insert(parent).unwrap();
    configs.insert(child).unwrap();

    let (effective, _) = resolve_active(&base, &configs, Some("child")).unwrap();
    assert_eq!(effective.get(seq(1)).unwrap().expr, Expr::num(2.0));
}

#[test]
fn suppression_removes_the_row_and_marks_the_status() {
    let base = base();
    let mut configs = ParamConfigTable::new();
    let mut without_hole = config("without_hole");
    without_hole.suppress(seq(2));
    configs.insert(without_hole).unwrap();

    let (effective, status) = resolve_active(&base, &configs, Some("without_hole")).unwrap();
    assert!(effective.get(seq(2)).is_none());
    assert_eq!(status.suppressed, BTreeSet::from([seq(2)]));
}

#[test]
fn suppressing_a_referenced_parameter_is_refused() {
    let base = ParamTable::from_rows([
        ParamRow::new(seq(1), "a", ParamKind::Length, Expr::num(10.0)).unwrap(),
        ParamRow::new(seq(2), "b", ParamKind::Length, Expr::reference(seq(1))).unwrap(),
    ])
    .unwrap();
    let mut configs = ParamConfigTable::new();
    let mut broken = config("broken");
    broken.suppress(seq(1));
    configs.insert(broken).unwrap();

    let error = resolve_active(&base, &configs, Some("broken")).unwrap_err();
    assert_eq!(error.code(), super::super::PARAM_UNKNOWN_REFERENCE);
}

#[test]
fn a_parent_cycle_is_refused_even_when_unused() {
    let base = base();
    let mut a = config("a");
    a.parent = Some("b".to_string());
    let mut b = config("b");
    b.parent = Some("a".to_string());
    let mut configs = ParamConfigTable::new();
    configs.insert(a).unwrap();
    configs.insert(b).unwrap();
    let error = configs.validate(&base).unwrap_err();
    assert_eq!(error.code(), PARAM_CONFIG_CYCLE);
}

#[test]
fn an_unknown_parent_or_configuration_is_refused() {
    let base = base();
    let mut configs = ParamConfigTable::new();
    let mut child = config("child");
    child.parent = Some("missing".to_string());
    configs.insert(child).unwrap();
    assert_eq!(
        configs.validate(&base).unwrap_err().code(),
        PARAM_UNKNOWN_CONFIG
    );
    assert_eq!(
        resolve_active(&base, &configs, Some("nope"))
            .unwrap_err()
            .code(),
        PARAM_UNKNOWN_CONFIG
    );
}

#[test]
fn an_override_of_a_missing_parameter_is_refused() {
    let base = base();
    let mut configs = ParamConfigTable::new();
    let mut broken = config("broken");
    broken.set_override(seq(9), Expr::num(1.0)).unwrap();
    configs.insert(broken).unwrap();
    assert_eq!(
        configs.validate(&base).unwrap_err().code(),
        super::super::PARAM_UNKNOWN_REFERENCE
    );
}

#[test]
fn a_child_override_re_enables_a_parent_suppression() {
    let base = base();
    let mut parent = config("parent");
    parent.suppress(seq(1));
    let mut child = config("child");
    child.set_override(seq(1), Expr::num(3.0)).unwrap();
    child.parent = Some("parent".to_string());
    let mut configs = ParamConfigTable::new();
    configs.insert(parent).unwrap();
    configs.insert(child).unwrap();

    let (effective, status) = resolve_active(&base, &configs, Some("child")).unwrap();
    assert_eq!(effective.get(seq(1)).unwrap().expr, Expr::num(3.0));
    assert_eq!(status.overridden, BTreeSet::from([seq(1)]));
    assert!(!status.suppressed.contains(&seq(1)));
}

#[test]
fn a_child_suppression_wins_over_a_parent_override() {
    let base = base();
    let mut parent = config("parent");
    parent.set_override(seq(1), Expr::num(8.0)).unwrap();
    let mut child = config("child");
    child.suppress(seq(1));
    child.parent = Some("parent".to_string());
    let mut configs = ParamConfigTable::new();
    configs.insert(parent).unwrap();
    configs.insert(child).unwrap();

    let (effective, status) = resolve_active(&base, &configs, Some("child")).unwrap();
    assert!(effective.get(seq(1)).is_none());
    assert_eq!(status.suppressed, BTreeSet::from([seq(1)]));
    assert!(!status.overridden.contains(&seq(1)));
}

#[test]
fn no_active_configuration_returns_the_base_table() {
    let base = base();
    let configs = ParamConfigTable::new();
    let (effective, status) = resolve_active(&base, &configs, None).unwrap();
    assert_eq!(effective, base);
    assert_eq!(status, ConfigStatus::default());
}

#[test]
fn an_override_must_still_match_the_base_kind() {
    let base = ParamTable::from_rows([
        ParamRow::new(seq(1), "thickness", ParamKind::Length, Expr::num(10.0)).unwrap(),
        ParamRow::new(seq(3), "angle", ParamKind::Angle, Expr::num(1.0)).unwrap(),
    ])
    .unwrap();
    let mut configs = ParamConfigTable::new();
    let mut wrong = config("wrong");
    wrong.set_override(seq(1), Expr::reference(seq(3))).unwrap();
    configs.insert(wrong).unwrap();
    let error = resolve_active(&base, &configs, Some("wrong")).unwrap_err();
    assert_eq!(error.code(), super::super::PARAM_UNIT_MISMATCH);
}
