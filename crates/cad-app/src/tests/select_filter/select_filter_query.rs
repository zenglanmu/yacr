//! Contract tests for the structured QSELECT / FILTER dialog semantics.
//!
//! Split out of `tests/select_filter.rs` to keep both files under the code-size
//! limit. Every operator/apply/scope combination asserts the exact selected id
//! set, nested block members are proven not to be candidates, and the
//! read-only (no revision/undo) guarantee is pinned again here.

use super::*;
use crate::command_payload::{
    FilterExpression, FilterLogic, FilterRequest, QSelectRequest, SelectionApply,
    SelectionOperator, SelectionPredicate, SelectionProperty, SelectionScope,
};
use crate::SelectionToolPayload;
use cad_db::BlockDefinition;

fn predicate(
    property: SelectionProperty,
    operator: SelectionOperator,
    value: &str,
) -> SelectionPredicate {
    SelectionPredicate {
        property,
        operator,
        value: value.into(),
    }
}

fn qselect_payload(
    object_type: Option<&str>,
    predicate: SelectionPredicate,
    scope: SelectionScope,
    apply: SelectionApply,
) -> CommandPayload {
    CommandPayload::SelectionTool(Box::new(SelectionToolPayload::QSelect(Box::new(
        QSelectRequest {
            object_type: object_type.map(str::to_string),
            predicate,
            scope,
            apply,
        },
    ))))
}

fn filter_payload(
    expression: FilterExpression,
    scope: SelectionScope,
    apply: SelectionApply,
) -> CommandPayload {
    CommandPayload::SelectionTool(Box::new(SelectionToolPayload::Filter(Box::new(
        FilterRequest {
            expression,
            scope,
            apply,
        },
    ))))
}

fn leaf(property: SelectionProperty, operator: SelectionOperator, value: &str) -> FilterExpression {
    FilterExpression::Predicate(predicate(property, operator, value))
}

/// A drawing with one top-level model LINE (id 2), one block (id 1) whose
/// member is a block-space LINE (id 100), and an INSERT (id 1) of that block.
fn nested_block_app() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: cad_domain::BlockId(1),
            entities: vec![EntityId(100)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(100),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(100),
            layer: LayerId(0),
            space: SpaceId::Block(cad_domain::BlockId(1)),
            geometry: line((0.0, 0.0), (1.0, 0.0)),
            draw_order: 0,
        })
        .unwrap();
    builder
        .insert_entity(entity(
            EntityId(2),
            "AcDbLine",
            LayerId(0),
            line((10.0, 0.0), (11.0, 0.0)),
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            EntityId(1),
            "AcDbBlockReference",
            LayerId(0),
            SemanticGeometry::Insert {
                block: cad_domain::BlockId(1),
                transform: Transform3::identity(),
            },
        ))
        .unwrap();
    let drawing = builder.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

#[test]
fn qselect_operators_select_exact_id_sets() {
    let (mut app, mut session) = filter_app();
    let all_lines = vec![LINE_RED, LINE_PLAIN];

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        qselect_payload(
            None,
            predicate(SelectionProperty::Type, SelectionOperator::Equals, "LINE"),
            SelectionScope::EntireDrawing,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert_eq!(ids(&session), all_lines, "Equals TYPE LINE");

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        qselect_payload(
            None,
            predicate(SelectionProperty::Type, SelectionOperator::NotEqual, "LINE"),
            SelectionScope::EntireDrawing,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert_eq!(
        ids(&session),
        vec![CIRCLE, TEXT, MTEXT, LEADER],
        "Not Equal To TYPE LINE"
    );

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        qselect_payload(
            None,
            predicate(
                SelectionProperty::Lineweight,
                SelectionOperator::GreaterThan,
                "0.3",
            ),
            SelectionScope::EntireDrawing,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert_eq!(
        ids(&session),
        vec![LINE_RED, CIRCLE],
        "Greater Than linetype weight 0.3 mm"
    );

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        qselect_payload(
            None,
            predicate(
                SelectionProperty::Transparency,
                SelectionOperator::LessThanOrEqual,
                "25",
            ),
            SelectionScope::EntireDrawing,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert_eq!(
        ids(&session),
        vec![LINE_RED, LINE_PLAIN],
        "<= 25 percent transparent (symbolic ByLayer is not numeric)"
    );

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        qselect_payload(
            None,
            predicate(
                SelectionProperty::LinetypeScale,
                SelectionOperator::LessThan,
                "2",
            ),
            SelectionScope::EntireDrawing,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert_eq!(
        ids(&session),
        vec![LINE_PLAIN, CIRCLE, TEXT, MTEXT, LEADER],
        "Less Than linetype scale 2"
    );

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        qselect_payload(
            None,
            predicate(
                SelectionProperty::LinetypeScale,
                SelectionOperator::GreaterThanOrEqual,
                "2",
            ),
            SelectionScope::EntireDrawing,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert_eq!(
        ids(&session),
        vec![LINE_RED],
        "Greater Than Or Equal linetype scale 2"
    );

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        qselect_payload(
            None,
            predicate(
                SelectionProperty::Text,
                SelectionOperator::WildcardMatch,
                "Hello*",
            ),
            SelectionScope::EntireDrawing,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert_eq!(
        ids(&session),
        vec![TEXT, MTEXT, LEADER],
        "*Wildcard Match* on the text content, ASCII case-insensitive"
    );

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        qselect_payload(
            Some("LINE"),
            predicate(SelectionProperty::Type, SelectionOperator::SelectAll, ""),
            SelectionScope::EntireDrawing,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert_eq!(ids(&session), all_lines, "Select All for object type LINE");
}

#[test]
fn qselect_apply_modes_exclude_and_append_exact_sets() {
    let (mut app, mut session) = filter_app();
    session.selection.replace([model_ref(LINE_PLAIN)]);

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        qselect_payload(
            None,
            predicate(SelectionProperty::Type, SelectionOperator::Equals, "CIRCLE"),
            SelectionScope::EntireDrawing,
            SelectionApply::Append,
        ),
    )
    .unwrap();
    assert_eq!(
        ids(&session),
        vec![LINE_PLAIN, CIRCLE],
        "Append to Current Selection Set keeps the existing selection"
    );

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        qselect_payload(
            None,
            predicate(SelectionProperty::Layer, SelectionOperator::Equals, "WALLS"),
            SelectionScope::EntireDrawing,
            SelectionApply::ExcludeNew,
        ),
    )
    .unwrap();
    assert_eq!(
        ids(&session),
        vec![LINE_PLAIN, TEXT, MTEXT, LEADER],
        "Exclude from New Selection Set removes only the WALLS matches"
    );
}

#[test]
fn qselect_current_selection_scope_filters_the_selection_only() {
    let (mut app, mut session) = filter_app();
    session
        .selection
        .replace([model_ref(CIRCLE), model_ref(TEXT)]);

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        qselect_payload(
            None,
            predicate(SelectionProperty::Type, SelectionOperator::NotEqual, "LINE"),
            SelectionScope::CurrentSelection,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert_eq!(
        ids(&session),
        vec![CIRCLE, TEXT],
        "both selected candidates match Not Equal LINE"
    );

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        qselect_payload(
            None,
            predicate(SelectionProperty::Type, SelectionOperator::Equals, "CIRCLE"),
            SelectionScope::CurrentSelection,
            SelectionApply::ExcludeNew,
        ),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![TEXT], "current selection minus CIRCLE");
}

#[test]
fn qselect_nested_block_objects_are_never_selected_or_counted() {
    let (mut app, mut session) = nested_block_app();
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    let outcome = run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        qselect_payload(
            None,
            predicate(SelectionProperty::Type, SelectionOperator::Equals, "LINE"),
            SelectionScope::EntireDrawing,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert_eq!(
        ids(&session),
        vec![EntityId(2)],
        "only the top-level model LINE is a candidate"
    );
    assert!(
        !session
            .selection
            .refs()
            .iter()
            .any(|r| r.entity == EntityId(100)),
        "the block-definition member is not selected"
    );
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|d| d.code == "qselect.result")
        .expect("a result diagnostic");
    assert!(
        diagnostic.message.contains("1 matching object(s)"),
        "the nested member is not counted: {}",
        diagnostic.message
    );

    // A current-selection ref that addresses the block member is dropped too.
    session.selection.replace([SelectionRef {
        document: DocumentId(1),
        entity: EntityId(100),
        instance: InstancePath(vec![EntityId(1)]),
        sub_element: None,
    }]);
    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        qselect_payload(
            None,
            predicate(SelectionProperty::Type, SelectionOperator::Equals, "LINE"),
            SelectionScope::CurrentSelection,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert!(
        session.selection.is_empty(),
        "an instance-path ref is not addressable by QSELECT"
    );
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
}

#[test]
fn filter_logical_operators_select_exact_id_sets() {
    let (mut app, mut session) = filter_app();

    run(
        &mut app,
        &mut session,
        CommandId::FilterSelect,
        filter_payload(
            FilterExpression::Group {
                logic: FilterLogic::And,
                members: vec![
                    leaf(SelectionProperty::Type, SelectionOperator::Equals, "LINE"),
                    leaf(SelectionProperty::Layer, SelectionOperator::Equals, "WALLS"),
                ],
            },
            SelectionScope::EntireDrawing,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![LINE_RED], "AND");

    run(
        &mut app,
        &mut session,
        CommandId::FilterSelect,
        filter_payload(
            FilterExpression::Group {
                logic: FilterLogic::Or,
                members: vec![
                    leaf(SelectionProperty::Type, SelectionOperator::Equals, "CIRCLE"),
                    leaf(SelectionProperty::Type, SelectionOperator::Equals, "MTEXT"),
                ],
            },
            SelectionScope::EntireDrawing,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![CIRCLE, MTEXT], "OR");

    run(
        &mut app,
        &mut session,
        CommandId::FilterSelect,
        filter_payload(
            FilterExpression::Group {
                logic: FilterLogic::Xor,
                members: vec![
                    leaf(SelectionProperty::Layer, SelectionOperator::Equals, "WALLS"),
                    leaf(SelectionProperty::Type, SelectionOperator::Equals, "LINE"),
                ],
            },
            SelectionScope::EntireDrawing,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert_eq!(
        ids(&session),
        vec![LINE_PLAIN, CIRCLE],
        "XOR is the symmetric difference"
    );

    run(
        &mut app,
        &mut session,
        CommandId::FilterSelect,
        filter_payload(
            FilterExpression::Group {
                logic: FilterLogic::Not,
                members: vec![leaf(
                    SelectionProperty::Layer,
                    SelectionOperator::Equals,
                    "WALLS",
                )],
            },
            SelectionScope::EntireDrawing,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![LINE_PLAIN, TEXT, MTEXT, LEADER], "NOT");
}

#[test]
fn filter_nested_expression_is_balanced_and_read_only() {
    let (mut app, mut session) = filter_app();
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);
    // AND(OR(LINE, CIRCLE), NOT(WALLS)) => only the layer-0 LINE.
    let expression = FilterExpression::Group {
        logic: FilterLogic::And,
        members: vec![
            FilterExpression::Group {
                logic: FilterLogic::Or,
                members: vec![
                    leaf(SelectionProperty::Type, SelectionOperator::Equals, "LINE"),
                    leaf(SelectionProperty::Type, SelectionOperator::Equals, "CIRCLE"),
                ],
            },
            FilterExpression::Group {
                logic: FilterLogic::Not,
                members: vec![leaf(
                    SelectionProperty::Layer,
                    SelectionOperator::Equals,
                    "WALLS",
                )],
            },
        ],
    };
    run(
        &mut app,
        &mut session,
        CommandId::FilterSelect,
        filter_payload(
            expression,
            SelectionScope::EntireDrawing,
            SelectionApply::IncludeNew,
        ),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![LINE_PLAIN]);
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
}

#[test]
fn filter_refuses_invalid_logical_arity_and_relative_types_by_name() {
    let (mut app, mut session) = filter_app();
    session.selection.replace([model_ref(CIRCLE)]);
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    let cases: Vec<(FilterExpression, &str)> = vec![
        (
            FilterExpression::Group {
                logic: FilterLogic::Xor,
                members: vec![leaf(
                    SelectionProperty::Type,
                    SelectionOperator::Equals,
                    "LINE",
                )],
            },
            "filter.xor_arity",
        ),
        (
            FilterExpression::Group {
                logic: FilterLogic::Not,
                members: vec![
                    leaf(SelectionProperty::Type, SelectionOperator::Equals, "LINE"),
                    leaf(SelectionProperty::Type, SelectionOperator::Equals, "CIRCLE"),
                ],
            },
            "filter.not_arity",
        ),
        (
            FilterExpression::Group {
                logic: FilterLogic::And,
                members: Vec::new(),
            },
            "filter.group_arity",
        ),
        (
            leaf(
                SelectionProperty::Layer,
                SelectionOperator::GreaterThan,
                "1",
            ),
            "filter.relative_non_numeric",
        ),
        (
            leaf(
                SelectionProperty::Lineweight,
                SelectionOperator::LessThan,
                "heavy",
            ),
            "filter.relative_non_numeric",
        ),
    ];
    for (expression, code) in cases {
        let error = run(
            &mut app,
            &mut session,
            CommandId::FilterSelect,
            filter_payload(
                expression,
                SelectionScope::EntireDrawing,
                SelectionApply::IncludeNew,
            ),
        )
        .unwrap_err();
        assert!(
            matches!(error, CadError::InvalidInput(ref message) if message.contains(code)),
            "expected {code}, got {error:?}"
        );
    }
    assert_eq!(ids(&session), vec![CIRCLE], "refusals change nothing");
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
}

#[test]
fn qselect_and_filter_refuse_named_nfl_filters_by_name() {
    let (mut app, mut session) = filter_app();
    let error = run(
        &mut app,
        &mut session,
        CommandId::FilterSelect,
        filter("NAMED", "myfilter.nfl", SelectionMode::Add),
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::Unsupported(ref m) if m.contains("filter.named_unsupported")),
        "{error:?}"
    );
}
