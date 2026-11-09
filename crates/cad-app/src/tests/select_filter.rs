//! Contract tests for the 2026-11-07 selection/filter/find/visibility batch.
//!
//! Pinned here: exact match sets, ASCII case-insensitivity, the named refusals,
//! the read-only guarantee (no revision bump, no undo entry) for every
//! selection/session command, and the one-transaction rule for `FIND` replace.
//! The picking/render half of the hidden set is pinned through the shared
//! visibility predicate and the renderer's `FLAG_HIDDEN` resolution.

use super::*;
use cad_db::{
    DbEntity, DbObject, DrawingDatabaseBuilder, EntityColor, EntityLineWeight,
    EntityRenderAttributes, EntityTransparency, Layer,
};
use cad_domain::{MLeaderPathType, MLeaderStyleValues, MultiLeaderGeometry, StyleId};

/// The fixture ids, in insertion order.
const LINE_RED: EntityId = EntityId(10);
const LINE_PLAIN: EntityId = EntityId(11);
const CIRCLE: EntityId = EntityId(12);
const TEXT: EntityId = EntityId(13);
const MTEXT: EntityId = EntityId(14);
const LEADER: EntityId = EntityId(15);

fn entity(id: EntityId, type_key: &str, layer: LayerId, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id.0),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id,
        layer,
        space: SpaceId::Model,
        geometry,
        draw_order: 0,
    }
}

fn line(start: (f64, f64), end: (f64, f64)) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: point(start.0, start.1),
        end: point(end.0, end.1),
    }
}

fn text(value: &str, x: f64) -> SemanticGeometry {
    SemanticGeometry::Text {
        text: value.into(),
        position: point(x, 0.0),
        style: StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: None,
    }
}

fn leader_style() -> MLeaderStyleValues {
    MLeaderStyleValues {
        path_type: MLeaderPathType::StraightLineSegments,
        enable_landing: true,
        enable_dogleg: true,
        landing_distance: 2.0,
        landing_gap: 0.1,
        arrowhead_size: 0.5,
        text_height: 0.2,
        text_style: StyleId(0),
        text_style_name: "Standard".into(),
        text_font: Some("arial.ttf".into()),
        scale_factor: 1.0,
    }
}

/// Two lines (one red, one ByLayer), a circle, a TEXT, an MTEXT and a
/// text-bearing MULTILEADER. Layers `0` and `WALLS` come from the shared
/// fixture.
fn filter_app() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_entity(entity(
        LINE_RED,
        "AcDbLine",
        LayerId(3),
        line((0.0, 0.0), (1.0, 0.0)),
    ))
    .unwrap();
    b.set_entity_render_attributes(
        LINE_RED,
        EntityRenderAttributes {
            color: EntityColor::Explicit([255, 0, 0]),
            lineweight: EntityLineWeight::Explicit(0.50),
            transparency: EntityTransparency::Explicit(0.75),
            linetype_scale: 2.0,
            ..EntityRenderAttributes::default()
        },
    )
    .unwrap();
    b.insert_entity(entity(
        LINE_PLAIN,
        "AcDbLine",
        LayerId(0),
        line((0.0, 1.0), (1.0, 1.0)),
    ))
    .unwrap();
    b.set_entity_render_attributes(
        LINE_PLAIN,
        EntityRenderAttributes {
            lineweight: EntityLineWeight::Explicit(0.25),
            transparency: EntityTransparency::Explicit(1.0),
            ..EntityRenderAttributes::default()
        },
    )
    .unwrap();
    b.insert_entity(entity(
        CIRCLE,
        "AcDbCircle",
        LayerId(3),
        SemanticGeometry::Circle {
            center: point(5.0, 5.0),
            radius: 2.0,
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        },
    ))
    .unwrap();
    b.set_entity_render_attributes(
        CIRCLE,
        EntityRenderAttributes {
            lineweight: EntityLineWeight::Explicit(0.50),
            transparency: EntityTransparency::Explicit(0.5),
            ..EntityRenderAttributes::default()
        },
    )
    .unwrap();
    b.insert_entity(entity(
        TEXT,
        "AcDbText",
        LayerId(0),
        text("Hello World", 0.0),
    ))
    .unwrap();
    b.insert_entity(entity(
        MTEXT,
        "AcDbMText",
        LayerId(0),
        text("hello \\Pworld", 10.0),
    ))
    .unwrap();
    let leader = MultiLeaderGeometry::created(
        leader_style(),
        "Standard".into(),
        2,
        point(0.0, 5.0),
        point(10.0, 8.0),
        "Hello Leader".into(),
    )
    .expect("a valid created leader");
    b.insert_entity(entity(
        LEADER,
        "AcDbMultiLeader",
        LayerId(0),
        SemanticGeometry::MultiLeader(Box::new(leader)),
    ))
    .unwrap();
    let drawing = b.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

fn model_ref(entity: EntityId) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity,
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn filter(property: &str, value: &str, mode: SelectionMode) -> CommandPayload {
    CommandPayload::SelectFilter {
        property: property.into(),
        value: value.into(),
        mode,
    }
}

fn run(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn ids(session: &SessionState) -> Vec<EntityId> {
    let mut ids: Vec<EntityId> = session.selection.refs().iter().map(|r| r.entity).collect();
    ids.sort_by_key(|id| id.0);
    ids
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn entity_in(app: &Application, id: EntityId) -> DbEntity {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(id)
        .cloned()
        .expect("entity exists")
}

fn text_value(entity: &DbEntity) -> Option<&str> {
    match &entity.geometry {
        SemanticGeometry::Text { text, .. } => Some(text),
        SemanticGeometry::MultiLeader(leader) => Some(&leader.text().string),
        _ => None,
    }
}

#[test]
fn qselect_matches_type_layer_color_and_linetype_case_insensitively() {
    let (mut app, mut session) = filter_app();

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        filter("type", "line", SelectionMode::Replace),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![LINE_RED, LINE_PLAIN]);

    // The stored type key is the other accepted spelling.
    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        filter("TYPE", "AcDbLine", SelectionMode::Replace),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![LINE_RED, LINE_PLAIN]);

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        filter("LAYER", "walls", SelectionMode::Replace),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![LINE_RED, CIRCLE]);

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        filter("COLOR", "aci:1", SelectionMode::Replace),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![LINE_RED]);

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        filter("LINETYPE", "ByLayer", SelectionMode::Replace),
    )
    .unwrap();
    assert_eq!(
        ids(&session),
        vec![LINE_RED, LINE_PLAIN, CIRCLE, TEXT, MTEXT, LEADER]
    );

    // TEXT and MTEXT are distinct aliases even though both are `Text` geometry.
    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        filter("TYPE", "MTEXT", SelectionMode::Replace),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![MTEXT]);
}

#[test]
fn qselect_is_selection_only_and_refuses_unknown_property() {
    let (mut app, mut session) = filter_app();
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        filter("TYPE", "CIRCLE", SelectionMode::Replace),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![CIRCLE]);
    assert!(matches!(session.tool, ToolState::Selecting));
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);

    let error = run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        filter("WEIGHT", "2", SelectionMode::Replace),
    )
    .unwrap_err();
    match error {
        CadError::InvalidInput(message) => {
            assert!(message.contains("qselect.unknown_property"), "{message}")
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    // The refusal changes nothing: the previous match is still selected.
    assert_eq!(ids(&session), vec![CIRCLE]);
}

#[test]
fn qselect_zero_matches_is_a_diagnostic_and_an_empty_selection() {
    let (mut app, mut session) = filter_app();
    session.selection.insert(model_ref(CIRCLE));

    let outcome = run(
        &mut app,
        &mut session,
        CommandId::QSelect,
        filter("TYPE", "SPLINE", SelectionMode::Replace),
    )
    .unwrap();
    assert!(session.selection.is_empty());
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "qselect.result"
            && diagnostic.message.contains("no model-space entity")));
}

#[test]
fn filter_adds_and_refuses_empty_value() {
    let (mut app, mut session) = filter_app();
    session.selection.insert(model_ref(LINE_PLAIN));

    run(
        &mut app,
        &mut session,
        CommandId::FilterSelect,
        filter("TYPE", "CIRCLE", SelectionMode::Add),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![LINE_PLAIN, CIRCLE]);

    // ADD to an empty selection is valid (documented divergence from a dialog).
    session.selection.clear();
    run(
        &mut app,
        &mut session,
        CommandId::FilterSelect,
        filter("LAYER", "0", SelectionMode::Add),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![LINE_PLAIN, TEXT, MTEXT, LEADER]);

    let error = run(
        &mut app,
        &mut session,
        CommandId::FilterSelect,
        filter("TYPE", "  ", SelectionMode::Add),
    )
    .unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(ref m) if m.contains("filter.empty_value")));
}

#[test]
fn getsel_accepts_layer_and_type_only() {
    let (mut app, mut session) = filter_app();
    run(
        &mut app,
        &mut session,
        CommandId::GetSelection,
        filter("LAYER", "WALLS", SelectionMode::Replace),
    )
    .unwrap();
    assert_eq!(ids(&session), vec![LINE_RED, CIRCLE]);

    let error = run(
        &mut app,
        &mut session,
        CommandId::GetSelection,
        filter("COLOR", "aci:1", SelectionMode::Replace),
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref m) if m.contains("getsel.unknown_property"))
    );
}

#[test]
fn ssx_unions_by_all_five_properties_and_selectcount_is_read_only() {
    let (mut app, mut session) = filter_app();
    // The red line and the circle share layer WALLS but differ in type/colour,
    // so the template is the red line itself.
    session.selection.insert(model_ref(LINE_RED));
    let undo_before = undo_depth(&app);

    let outcome = run(
        &mut app,
        &mut session,
        CommandId::SelectMatchingProperties,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        ids(&session),
        vec![LINE_RED],
        "no other entity has all five"
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == "ssx.result" && d.message.contains("added 0")));

    // Add a second identical red line: SSX now unions it.
    let (mut app, mut session) = filter_app();
    {
        let document = app.workspace.documents.get_mut(&DocumentId(1)).unwrap();
        let mut candidate = document.drawing.as_ref().clone();
        let mut transaction = candidate
            .begin_drawing_transaction("test duplicate", TransactionId(90))
            .unwrap();
        transaction
            .insert_entity(entity(
                EntityId(20),
                "AcDbLine",
                LayerId(3),
                line((2.0, 2.0), (3.0, 2.0)),
            ))
            .unwrap();
        transaction.commit().unwrap();
        document.drawing = Arc::new(candidate);
    }
    // The duplicate needs the same explicit colour to match the five properties.
    {
        let document = app.workspace.documents.get_mut(&DocumentId(1)).unwrap();
        let mut candidate = document.drawing.as_ref().clone();
        let mut transaction = candidate
            .begin_drawing_transaction("test colour", TransactionId(91))
            .unwrap();
        transaction
            .update_entity_display(
                EntityId(20),
                EntityRenderAttributes {
                    color: EntityColor::Explicit([255, 0, 0]),
                    ..EntityRenderAttributes::default()
                },
            )
            .unwrap();
        transaction.commit().unwrap();
        document.drawing = Arc::new(candidate);
    }
    session.selection.replace([model_ref(LINE_RED)]);
    run(
        &mut app,
        &mut session,
        CommandId::SelectMatchingProperties,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(ids(&session), vec![LINE_RED, EntityId(20)]);

    // SELECTCOUNT reports the same pair without touching the selection.
    let selection_before = session.selection.clone();
    let revision_before = revision(&app);
    let count = run(
        &mut app,
        &mut session,
        CommandId::SelectCount,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(session.selection, selection_before);
    assert_eq!(revision(&app), revision_before);
    assert!(count
        .diagnostics
        .iter()
        .any(|d| d.code == "selectcount.result" && d.message.contains("2 model-space object(s)")));
    assert_eq!(undo_depth(&app), undo_before);
}

#[test]
fn ssx_and_selectcount_refuse_an_empty_selection() {
    let (mut app, mut session) = filter_app();
    let error = run(
        &mut app,
        &mut session,
        CommandId::SelectMatchingProperties,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(ref m) if m.contains("ssx.selection_required")));

    let error = run(
        &mut app,
        &mut session,
        CommandId::SelectCount,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref m) if m.contains("selectcount.selection_required"))
    );
}

#[test]
fn find_report_is_read_only_and_case_insensitive_across_text_mtext_and_mleader() {
    let (mut app, mut session) = filter_app();
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);
    let selection_before = session.selection.clone();

    for id in [CommandId::FindReport, CommandId::FindReplace] {
        let outcome = run(
            &mut app,
            &mut session,
            id,
            CommandPayload::TextFind {
                needle: "HELLO".into(),
                replacement: None,
            },
        )
        .unwrap();
        let diagnostic = outcome
            .diagnostics
            .iter()
            .find(|d| d.code == "find.result")
            .expect("a find report");
        assert!(
            diagnostic.message.contains("3 match(es)"),
            "TEXT + MTEXT + MULTILEADER: {}",
            diagnostic.message
        );
        assert!(
            diagnostic.message.contains("Hello World"),
            "{}",
            diagnostic.message
        );
    }
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
    assert_eq!(session.selection, selection_before);
}

#[test]
fn find_replace_is_one_transaction_and_one_undo_step() {
    let (mut app, mut session) = filter_app();
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    let outcome = run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        CommandPayload::TextFind {
            needle: "hello".into(),
            replacement: Some("Howdy".into()),
        },
    )
    .unwrap();
    assert!(outcome.changes.is_some());
    assert_eq!(undo_depth(&app), undo_before + 1);
    assert_ne!(revision(&app), revision_before);
    assert_eq!(text_value(&entity_in(&app, TEXT)), Some("Howdy World"));
    assert_eq!(
        text_value(&entity_in(&app, MTEXT)),
        Some("Howdy \\Pworld"),
        "the stored MTEXT paragraph break is untouched"
    );
    assert_eq!(
        text_value(&entity_in(&app, LEADER)),
        Some("Howdy Leader"),
        "the MULTILEADER text and its rebuilt display agree"
    );

    // One undo restores every target.
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
    assert_eq!(text_value(&entity_in(&app, TEXT)), Some("Hello World"));
    assert_eq!(text_value(&entity_in(&app, MTEXT)), Some("hello \\Pworld"));
    assert_eq!(text_value(&entity_in(&app, LEADER)), Some("Hello Leader"));
}

#[test]
fn find_replace_refuses_a_newline_on_single_line_text() {
    let (mut app, mut session) = filter_app();
    let revision_before = revision(&app);

    let error = run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        CommandPayload::TextFind {
            needle: "World".into(),
            replacement: Some("a\nb".into()),
        },
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref m) if m.contains("text.single_line_newline")),
        "{error:?}"
    );
    assert_eq!(revision(&app), revision_before);
    assert_eq!(text_value(&entity_in(&app, TEXT)), Some("Hello World"));
}

#[test]
fn find_refuses_an_empty_needle_and_report_only_replacement() {
    let (mut app, mut session) = filter_app();
    let error = run(
        &mut app,
        &mut session,
        CommandId::FindReport,
        CommandPayload::TextFind {
            needle: String::new(),
            replacement: None,
        },
    )
    .unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(ref m) if m.contains("find.empty_needle")));

    let error = run(
        &mut app,
        &mut session,
        CommandId::FindReport,
        CommandPayload::TextFind {
            needle: "x".into(),
            replacement: Some("y".into()),
        },
    )
    .unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(ref m) if m.contains("find.report_only")));
}

#[test]
fn hide_isolate_and_unisolate_are_session_only() {
    let (mut app, mut session) = filter_app();
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);

    // HIDEOBJECTS hides the selection and clears it.
    session
        .selection
        .replace([model_ref(LINE_PLAIN), model_ref(CIRCLE)]);
    run(
        &mut app,
        &mut session,
        CommandId::HideObjects,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(session.hidden_entities.len(), 2);
    assert!(session.hidden_entities.contains(&model_ref(LINE_PLAIN)));
    assert!(session.selection.is_empty());
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);

    // UNISOLATEOBJECTS clears it.
    run(
        &mut app,
        &mut session,
        CommandId::UnisolateObjects,
        CommandPayload::None,
    )
    .unwrap();
    assert!(session.hidden_entities.is_empty());

    // ISOLATEOBJECTS hides every model-space entity except the selection.
    session.selection.replace([model_ref(TEXT)]);
    run(
        &mut app,
        &mut session,
        CommandId::IsolateObjects,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(session.hidden_entities.len(), 5);
    assert!(!session.hidden_entities.contains(&model_ref(TEXT)));
    assert_eq!(ids(&session), vec![TEXT], "isolation keeps the selection");
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
}

#[test]
fn hide_and_isolate_refuse_an_empty_selection() {
    let (mut app, mut session) = filter_app();
    let error = run(
        &mut app,
        &mut session,
        CommandId::HideObjects,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref m) if m.contains("hideobjects.selection_required"))
    );

    let error = run(
        &mut app,
        &mut session,
        CommandId::IsolateObjects,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        CadError::InvalidInput(ref m) if m.contains("isolateobjects.selection_required")
    ));
}

#[test]
fn hidden_entities_are_not_pickable_or_rendered_as_visible() {
    let (app, mut session) = filter_app();
    let drawing = app.workspace.documents[&DocumentId(1)].drawing.clone();
    session.hidden_entities.insert(model_ref(CIRCLE));

    let root = drawing.root_layer_scope();
    let empty = crate::layers::LayerOverrideSet::new();
    assert_eq!(
        crate::picking::ref_visibility(
            &drawing,
            &empty,
            &session.hidden_entities,
            root,
            &model_ref(CIRCLE)
        ),
        Some(false),
        "the shared picking predicate hides a hidden ref"
    );
    assert!(!crate::picking::ref_is_visible(
        &drawing,
        &empty,
        &session.hidden_entities,
        &model_ref(CIRCLE)
    ));
    assert!(
        crate::picking::ref_visibility(
            &drawing,
            &empty,
            &session.hidden_entities,
            root,
            &model_ref(TEXT)
        ) == Some(true)
    );

    // The renderer resolves the same set as `FLAG_HIDDEN` (fragment discard).
    let inputs = crate::render_scene::OverlayInputs {
        hidden: session.hidden_entities.clone(),
        ..crate::render_scene::OverlayInputs::default()
    };
    let flags = crate::render_scene::selection_entity_flags(&drawing, &inputs, DocumentId(1));
    assert!(
        flags
            .flags
            .iter()
            .any(|(reference, bits)| reference.entity == CIRCLE
                && bits & cad_scene::FLAG_HIDDEN != 0),
        "hidden ref published as FLAG_HIDDEN"
    );
}

/// A MULTILEADER must rebuild its display when its text changes; the FIND
/// replacement goes through `with_text`, so the derived text geometry follows.
#[test]
fn find_replacement_rebuilds_the_multileader_display() {
    let (mut app, mut session) = filter_app();
    let before = entity_in(&app, LEADER);
    run(
        &mut app,
        &mut session,
        CommandId::FindReplace,
        CommandPayload::TextFind {
            needle: "Leader".into(),
            replacement: Some("Note".into()),
        },
    )
    .unwrap();
    let after = entity_in(&app, LEADER);
    assert_ne!(before.geometry, after.geometry);
    if let SemanticGeometry::MultiLeader(leader) = &after.geometry {
        assert_eq!(leader.text().string, "Hello Note");
        assert!(leader.display().iter().any(
            |item| matches!(item, SemanticGeometry::Text { text, .. } if text.contains("Note"))
        ));
    } else {
        panic!("expected a multileader");
    }
}

mod select_filter_query;
