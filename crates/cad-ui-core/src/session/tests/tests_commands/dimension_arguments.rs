//! Typed-argument and vocabulary contracts for the dimension commands
//! (`DIMSPACE` plus the 2026-11-07 editing batch `DIMSTYLE`/`-DIMSTYLE`,
//! `DIMTEDIT`, `DIMEDIT`, `DIMOVERRIDE`, `DIMJOGLINE`, and the argument-free
//! `DIMREGEN`/`DIMDISASSOCIATE`/`QDIM`/`DIMBREAK`).

use super::super::super::*;
use super::super::test_support::*;
use cad_app::{DimEditCommand, DimJogLineCommand, DimStyleCommand, DimTextEditCommand};
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer, Layout};
use cad_domain::{
    DatabaseId, DimStyleId, DimensionDefinition, DimensionGeometry, DimensionKind, DimensionPoints,
    LayerId, LayoutId, ObjectId, Revision, SpaceId, StyleId,
};

use crate::session::commands::{
    ARGUMENT_SELECTORS, DIMEDIT_SELECTOR, DIMJOGLINE_SELECTOR, DIMOVERRIDE_SELECTOR,
    DIMSPACE_SELECTOR, DIMSTYLE_SELECTOR, DIMTEDIT_SELECTOR,
};
use crate::session::dim_arguments::{
    invalid_dimspace_argument_reason, parse_dimedit_argument, parse_dimjogline_argument,
    parse_dimoverride_argument, parse_dimspace_argument, parse_dimstyle_argument,
    parse_dimtedit_argument,
};

#[test]
fn every_argument_selector_is_admitted() {
    for selector in [
        DIMSPACE_SELECTOR,
        DIMSTYLE_SELECTOR,
        DIMTEDIT_SELECTOR,
        DIMEDIT_SELECTOR,
        DIMOVERRIDE_SELECTOR,
        DIMJOGLINE_SELECTOR,
    ] {
        assert!(
            ARGUMENT_SELECTORS.contains(&selector),
            "{selector} must be an argument selector"
        );
    }
}

#[test]
fn dash_dimstyle_is_the_same_command_path() {
    assert_eq!(normalize_command("DIMSTYLE"), Some(CommandId::DimStyle));
    assert_eq!(normalize_command("-DIMSTYLE"), Some(CommandId::DimStyle));
    assert_eq!(command_name(CommandId::DimStyle), Some("DIMSTYLE"));
    assert!(admissible_names().contains(&"-DIMSTYLE"));
}

#[test]
fn dimension_batch_selectors_normalize() {
    for (name, id) in [
        ("DIMSTYLE", CommandId::DimStyle),
        ("DIMTEDIT", CommandId::DimTextEdit),
        ("DIMEDIT", CommandId::DimEdit),
        ("DIMOVERRIDE", CommandId::DimOverride),
        ("DIMREGEN", CommandId::DimRegen),
        ("DIMDISASSOCIATE", CommandId::DimDisassociate),
        ("QDIM", CommandId::QDim),
        ("DIMJOGLINE", CommandId::DimJogLine),
        ("DIMBREAK", CommandId::DimBreak),
    ] {
        assert_eq!(normalize_command(name), Some(id), "{name}");
        assert_eq!(canonical_command_name(name), Some(name), "{name}");
    }
}

#[test]
fn only_the_four_writing_edits_require_work_mode() {
    for id in [
        CommandId::DimTextEdit,
        CommandId::DimEdit,
        CommandId::DimOverride,
        CommandId::QDim,
    ] {
        assert!(id.requires_work_mode(), "{id:?} writes the drawing");
    }
    for id in [
        CommandId::DimStyle,
        CommandId::DimRegen,
        CommandId::DimDisassociate,
        CommandId::DimJogLine,
        CommandId::DimBreak,
    ] {
        assert!(!id.requires_work_mode(), "{id:?} cannot write");
    }
}

#[test]
fn dimstyle_grammar_is_list_current_or_set_name() {
    assert_eq!(parse_dimstyle_argument("LIST"), Some(DimStyleCommand::List));
    assert_eq!(
        parse_dimstyle_argument(" current "),
        Some(DimStyleCommand::Current)
    );
    assert_eq!(
        parse_dimstyle_argument("SET Standard"),
        Some(DimStyleCommand::Set("Standard".into()))
    );
    assert_eq!(
        parse_dimstyle_argument("set My Style"),
        Some(DimStyleCommand::Set("My Style".into()))
    );
    for bad in ["", "LIST extra", "SET", "SET   ", "SHOW"] {
        assert_eq!(parse_dimstyle_argument(bad), None, "{bad:?}");
    }
}

#[test]
fn dimtedit_grammar_is_move_rotate_or_reset() {
    assert_eq!(
        parse_dimtedit_argument("MOVE 1 2"),
        Some(DimTextEditCommand::Move { x: 1.0, y: 2.0 })
    );
    assert_eq!(
        parse_dimtedit_argument("move -1.5 2.5"),
        Some(DimTextEditCommand::Move { x: -1.5, y: 2.5 })
    );
    assert_eq!(
        parse_dimtedit_argument("ROTATE 30"),
        Some(DimTextEditCommand::Rotate { degrees: 30.0 })
    );
    assert_eq!(
        parse_dimtedit_argument("ROTATE -45.5"),
        Some(DimTextEditCommand::Rotate { degrees: -45.5 })
    );
    assert_eq!(
        parse_dimtedit_argument("RESET"),
        Some(DimTextEditCommand::Reset)
    );
    for bad in ["", "MOVE 1", "MOVE 1 2 3", "ROTATE x", "RESET 1", "HOME"] {
        assert_eq!(parse_dimtedit_argument(bad), None, "{bad:?}");
    }
}

#[test]
fn dimedit_grammar_is_text_override_or_reset() {
    assert_eq!(
        parse_dimedit_argument("TEXT <> mm"),
        Some(DimEditCommand::Text("<> mm".into()))
    );
    assert_eq!(
        parse_dimedit_argument("text done"),
        Some(DimEditCommand::Text("done".into()))
    );
    assert_eq!(parse_dimedit_argument("RESET"), Some(DimEditCommand::Reset));
    for bad in ["", "TEXT", "TEXT   ", "RESET now", "HOME"] {
        assert_eq!(parse_dimedit_argument(bad), None, "{bad:?}");
    }
}

#[test]
fn dimoverride_grammar_is_variable_plus_verbatim_value() {
    let parsed = parse_dimoverride_argument("DIMTXT 5").expect("DIMTXT");
    assert_eq!(parsed.variable, "DIMTXT");
    assert_eq!(parsed.value, "5");
    let parsed = parse_dimoverride_argument("DIMPOST <> mm").expect("DIMPOST");
    assert_eq!(parsed.variable, "DIMPOST");
    assert_eq!(parsed.value, "<> mm");
    for bad in ["", "DIMTXT", "DIMTXT   "] {
        assert_eq!(parse_dimoverride_argument(bad), None, "{bad:?}");
    }
}

#[test]
fn dimjogline_grammar_is_add_point_or_remove() {
    assert_eq!(
        parse_dimjogline_argument("ADD 1 2"),
        Some(DimJogLineCommand::Add { x: 1.0, y: 2.0 })
    );
    assert_eq!(
        parse_dimjogline_argument("remove"),
        Some(DimJogLineCommand::Remove)
    );
    for bad in ["", "ADD 1", "ADD 1 2 3", "REMOVE 1", "TOGGLE"] {
        assert_eq!(parse_dimjogline_argument(bad), None, "{bad:?}");
    }
}

#[test]
fn dimspace_grammar_is_one_positive_spacing() {
    for (argument, expected) in [("5", 5.0), ("2.5", 2.5), ("0.5", 0.5), ("10", 10.0)] {
        assert_eq!(
            parse_dimspace_argument(argument),
            Some(expected),
            "{argument}"
        );
    }
    for bad in [
        "", "-5", "+5", "1e3", "5.", ".5", "5 6", "5mm", "NaN", "inf",
    ] {
        assert_eq!(parse_dimspace_argument(bad), None, "{bad:?}");
    }
    for zero in ["0", "0.0"] {
        assert_eq!(parse_dimspace_argument(zero), Some(0.0), "{zero:?}");
    }
    let reason = invalid_dimspace_argument_reason("DIMSPACE x");
    assert!(reason.contains("non-negative finite spacing"), "{reason}");
    assert!(reason.contains("Auto"), "{reason}");
}

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn dimension_entity(id: u128, first: Point3, second: Point3, line: Point3) -> DbEntity {
    let mut style = cad_db::DimStyle::standard_metric(DimStyleId(0));
    style.dim_line_increment = 4.0;
    let geometry = DimensionGeometry::synthesized(
        DimensionDefinition {
            kind: DimensionKind::Linear { rotation: 0.0 },
            points: DimensionPoints::Linear {
                first,
                second,
                dimension_line: line,
            },
            text_middle: point(0.0, 0.0),
            text_rotation: 0.0,
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        },
        "Standard".to_string(),
        style.resolved("Standard".to_string(), StyleId(0), None),
        None,
    )
    .expect("the fixture dimension synthesizes");
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Dimension(Box::new(geometry)),
        draw_order: id as i64,
    }
}

/// A session whose drawing has a linear base (line 10 below its measurement)
/// plus one linear target.
fn dimension_session() -> CadSession {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    builder
        .insert_dim_style(cad_db::DimStyle::standard_metric(DimStyleId(0)))
        .unwrap();
    builder
        .insert_entity(dimension_entity(
            701,
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(5.0, -10.0),
        ))
        .unwrap();
    builder
        .insert_entity(dimension_entity(
            702,
            point(20.0, 5.0),
            point(30.0, 5.0),
            point(25.0, -3.0),
        ))
        .unwrap();
    let drawing = builder.finish().unwrap();
    {
        let mut controller = session.core.controller.borrow_mut();
        let document_id = controller.document_id;
        controller
            .application
            .workspace
            .documents
            .get_mut(&document_id)
            .expect("the session's document")
            .drawing = Arc::new(drawing);
    }
    session
}

fn select(session: &CadSession, ids: &[u128]) {
    let document = session.core.controller.borrow().document_id;
    let references = ids
        .iter()
        .map(|id| SelectionRef {
            document,
            entity: EntityId(*id),
            instance: InstancePath::default(),
            sub_element: None,
        })
        .collect();
    session
        .core
        .controller
        .borrow_mut()
        .set_selection(references)
        .expect("the selection");
}

fn dimension_line(session: &CadSession, id: u128) -> Point3 {
    let controller = session.core.controller.borrow();
    let drawing = controller.drawing().expect("the document");
    let entity = drawing.entity(EntityId(id)).expect("the dimension entity");
    match &entity.geometry {
        SemanticGeometry::Dimension(dimension) => match &dimension.definition().points {
            DimensionPoints::Linear { dimension_line, .. } => *dimension_line,
            other => panic!("expected Linear points, got {other:?}"),
        },
        other => panic!("expected a dimension, got {other:?}"),
    }
}

fn near_point(actual: Point3, expected: Point3) -> bool {
    (actual.x - expected.x).abs() < 1e-9
        && (actual.y - expected.y).abs() < 1e-9
        && (actual.z - expected.z).abs() < 1e-9
}

#[test]
fn typed_dimspace_dispatches_the_explicit_spacing_and_bare_stays_auto() {
    let session = dimension_session();
    select(&session, &[701, 702]);
    session
        .command("DIMSPACE 4")
        .expect("DIMSPACE 4 is admitted");
    assert!(
        near_point(dimension_line(&session, 702), point(5.0, -14.0)),
        "{:?}",
        dimension_line(&session, 702)
    );
    session
        .command("DIMSPACE")
        .expect("bare DIMSPACE stays Auto");
    assert!(
        near_point(dimension_line(&session, 702), point(5.0, -15.0)),
        "Auto uses 2 x the base style text height (2 x 2.5): {:?}",
        dimension_line(&session, 702)
    );
}

#[test]
fn typed_dimspace_zero_aligns_the_targets_on_the_base_line() {
    let session = dimension_session();
    select(&session, &[701, 702]);
    session
        .command("DIMSPACE 0")
        .expect("DIMSPACE 0 aligns, it is not a bad spacing");
    assert!(
        near_point(dimension_line(&session, 702), point(5.0, -10.0)),
        "zero spacing aligns on the base dimension line: {:?}",
        dimension_line(&session, 702)
    );
}

#[test]
fn typed_dimspace_refuses_bad_spacing_by_name_and_changes_nothing() {
    let session = dimension_session();
    select(&session, &[701, 702]);
    for line in [
        "DIMSPACE x",
        "DIMSPACE 5 6",
        "DIMSPACE -2",
        "DIMSPACE +2",
        "DIMSPACE 1e3",
    ] {
        let error = session.command(line).unwrap_err();
        assert!(
            error.contains("DIMSPACE needs one non-negative finite spacing"),
            "{line}: {error}"
        );
        assert!(error.contains("Auto"), "{line}: {error}");
    }
    assert!(
        session.snapshot(None, None, 0).command_log.is_empty(),
        "the refusal happens before the command is recorded"
    );
}
