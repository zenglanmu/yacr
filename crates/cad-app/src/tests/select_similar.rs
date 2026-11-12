//! `SELECTSIMILAR` contract tests.
//!
//! Pinned here: the union with the existing selection, the AutoCAD 2026 default
//! similarity set (**type + layer only**, with colour/linetype/lineweight/name/
//! object style opt-in and transparency absent), each Settings dimension toggled
//! independently, object-style matching for TEXT, the model-space and paper-space
//! catalogues (matching only within the active space), the typed-payload
//! contract, the non-empty-selection requirement, and the fact that the command
//! writes nothing (no revision bump, no undo entry).

use super::*;
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, EntityColor, EntityLineType,
    EntityLineWeight, EntityRenderAttributes, EntityTransparency, Layer, Layout, LinetypePattern,
    Style,
};

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn model_ref(entity: EntityId) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity,
        instance: InstancePath::default(),
        sub_element: None,
    }
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

fn model_ids(app: &Application) -> Vec<(EntityId, &'static str)> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .map(|entity| {
            let kind = match &entity.geometry {
                SemanticGeometry::Line { .. } => "line",
                SemanticGeometry::Circle { .. } => "circle",
                _ => "other",
            };
            (entity.id, kind)
        })
        .collect()
}

fn two_lines_and_a_circle() -> (Application, SessionState, Vec<EntityId>, EntityId) {
    let (mut app, mut session) = application_with_document();
    for (start, end) in [((0.0, 0.0), (1.0, 0.0)), ((0.0, 1.0), (1.0, 1.0))] {
        execute(
            &mut app,
            &mut session,
            CommandId::CreateLine,
            CommandPayload::Points(vec![point(start.0, start.1), point(end.0, end.1)]),
        )
        .expect("LINE is created");
    }
    execute(
        &mut app,
        &mut session,
        CommandId::CreateCircle,
        CommandPayload::Points(vec![point(5.0, 5.0), point(7.0, 5.0)]),
    )
    .expect("CIRCLE is created");

    let ids = model_ids(&app);
    let lines: Vec<EntityId> = ids
        .iter()
        .filter(|(_, kind)| *kind == "line")
        .map(|(id, _)| *id)
        .collect();
    let circle = ids
        .iter()
        .find(|(_, kind)| *kind == "circle")
        .map(|(id, _)| *id)
        .expect("one circle");
    assert_eq!(lines.len(), 2, "two model-space LINE entities");
    (app, session, lines, circle)
}

// ---------------------------------------------------------------------------
// A fixture with one deliberate difference per similarity dimension.
// ---------------------------------------------------------------------------

fn base_attributes() -> EntityRenderAttributes {
    EntityRenderAttributes {
        clip: None,
        transparency: EntityTransparency::Explicit(0.5),
        color: EntityColor::Explicit([255, 0, 0]),
        lineweight: EntityLineWeight::Explicit(0.35),
        linetype: EntityLineType::Explicit {
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([0.5, -0.25]),
            scale: 1.0,
        },
        linetype_scale: 1.0,
        ..EntityRenderAttributes::default()
    }
}

fn line_entity(id: u128, layer: LayerId, space: SpaceId, y: f64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer,
        space,
        geometry: SemanticGeometry::Line {
            start: point(0.0, y),
            end: point(10.0, y),
        },
        draw_order: id as i64,
    }
}

fn circle_entity(id: u128, layer: LayerId, space: SpaceId) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbCircle".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer,
        space,
        geometry: SemanticGeometry::Circle {
            center: point(5.0, 5.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 2.0,
        },
        draw_order: id as i64,
    }
}

fn text_entity(id: u128, layer: LayerId, style: StyleId, x: f64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbText".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer,
        space: SpaceId::Model,
        geometry: SemanticGeometry::Text {
            text: format!("T{id}"),
            position: point(x, 20.0),
            style,
            height: 2.5,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            wrap_width: None,
            width_factor: 1.0,
        },
        draw_order: id as i64,
    }
}

/// ids:
/// * 1 — seed LINE on layer 0, base attributes;
/// * 2 — identical LINE (the only default match);
/// * 3 — LINE on layer 1, otherwise identical;
/// * 4 — LINE on layer 0 with a different colour;
/// * 5 — LINE on layer 0 with a different lineweight;
/// * 6 — CIRCLE on layer 0, otherwise identical;
/// * 7 — LINE on layer 0 with a different linetype;
/// * 8 — LINE on layer 0 with a different transparency;
/// * 9/10 — two identical paper-space LINEs (layout 1);
/// * 11 — paper-space CIRCLE;
/// * 12/13 — two TEXT objects on layer 0 with different text styles.
fn similar_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(1), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    for (id, name) in [(StyleId(1), "NOTE"), (StyleId(2), "TITLE")] {
        b.insert_style(Style {
            id,
            name: name.into(),
            resource_keys: Vec::new(),
        })
        .unwrap();
    }
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Sheet1".into(),
        viewports: Vec::new(),
    })
    .unwrap();

    let paper = SpaceId::Paper(LayoutId(1));
    for (entity, attributes) in [
        (
            line_entity(1, LayerId(0), SpaceId::Model, 0.0),
            base_attributes(),
        ),
        (
            line_entity(2, LayerId(0), SpaceId::Model, 1.0),
            base_attributes(),
        ),
        (
            line_entity(3, LayerId(1), SpaceId::Model, 2.0),
            base_attributes(),
        ),
        (
            line_entity(4, LayerId(0), SpaceId::Model, 3.0),
            EntityRenderAttributes {
                clip: None,
                color: EntityColor::Explicit([0, 0, 255]),
                ..base_attributes()
            },
        ),
        (
            line_entity(5, LayerId(0), SpaceId::Model, 4.0),
            EntityRenderAttributes {
                clip: None,
                lineweight: EntityLineWeight::Explicit(0.70),
                ..base_attributes()
            },
        ),
        (
            circle_entity(6, LayerId(0), SpaceId::Model),
            base_attributes(),
        ),
        (
            line_entity(7, LayerId(0), SpaceId::Model, 5.0),
            EntityRenderAttributes {
                clip: None,
                linetype: EntityLineType::Explicit {
                    name: "CONTINUOUS".into(),
                    pattern: LinetypePattern::continuous(),
                    scale: 1.0,
                },
                ..base_attributes()
            },
        ),
        (
            line_entity(8, LayerId(0), SpaceId::Model, 6.0),
            EntityRenderAttributes {
                clip: None,
                transparency: EntityTransparency::ByLayer,
                ..base_attributes()
            },
        ),
        (
            line_entity(9, LayerId(0), paper.clone(), 0.0),
            base_attributes(),
        ),
        (
            line_entity(10, LayerId(0), paper.clone(), 1.0),
            base_attributes(),
        ),
        (circle_entity(11, LayerId(0), paper), base_attributes()),
        (
            text_entity(12, LayerId(0), StyleId(1), 0.0),
            base_attributes(),
        ),
        (
            text_entity(13, LayerId(0), StyleId(2), 20.0),
            base_attributes(),
        ),
    ] {
        let id = entity.id;
        b.insert_entity(entity).unwrap();
        b.set_entity_render_attributes(id, attributes).unwrap();
    }

    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

fn select(app: &mut Application, session: &mut SessionState, id: u128) {
    execute(
        app,
        session,
        CommandId::Select,
        CommandPayload::Selection(vec![model_ref(EntityId(id))]),
    )
    .expect("SELECT stores the seed");
}

fn settings_payload(settings: SelectSimilarSettings) -> CommandPayload {
    CommandPayload::SelectionTool(Box::new(SelectionToolPayload::SelectSimilar(settings)))
}

fn selected_ids(session: &SessionState) -> Vec<u128> {
    let mut ids: Vec<u128> = session
        .selection
        .refs()
        .iter()
        .map(|reference| reference.entity.0)
        .collect();
    ids.sort_unstable();
    ids
}

#[test]
fn adding_similar_objects_keeps_the_selection_and_is_read_only() {
    let (mut app, mut session, lines, circle) = two_lines_and_a_circle();
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![model_ref(lines[0])]),
    )
    .expect("SELECT accepts one line");

    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::SelectSimilar,
        CommandPayload::None,
    )
    .expect("SELECTSIMILAR runs");

    assert_eq!(session.selection.len(), 2, "the other LINE is added");
    assert!(session.selection.contains(&model_ref(lines[0])));
    assert!(session.selection.contains(&model_ref(lines[1])));
    assert!(
        !session.selection.contains(&model_ref(circle)),
        "a CIRCLE is not similar to a LINE"
    );
    assert!(matches!(session.tool, ToolState::Selecting));
    assert!(outcome.objects.is_empty());
    assert!(outcome.changes.is_none());
    assert_eq!(outcome.diagnostics.len(), 2);
    let result = &outcome.diagnostics[0];
    assert_eq!(result.code, "selectsimilar.result");
    assert!(result.message.is_ascii());
    assert!(result.message.contains('1'));
    let settings = &outcome.diagnostics[1];
    assert_eq!(settings.code, "selectsimilar.settings");
    assert!(
        settings.message.contains("type, layer"),
        "the default set is type + layer only: {}",
        settings.message
    );
    assert!(
        !settings.message.contains("color")
            && !settings.message.contains("transparency")
            && !settings.message.contains("object style"),
        "no other dimension is on by default: {}",
        settings.message
    );
    assert_eq!(undo_depth(&app), before_undo, "no undo entry");
    assert_eq!(revision(&app), before_revision, "no database revision bump");
}

#[test]
fn every_optional_similarity_dimension_is_independently_toggled() {
    type Toggle = fn(&mut SelectSimilarSettings);
    // Seed 1 is a LINE on layer 0 with the base attributes; the default set is
    // type + layer, so ids 1/2/4/5/7/8 (all layer-0 lines) match and the
    // layer-1 line and every non-line stay out.
    let cases: [(Toggle, Vec<u128>); 4] = [
        (|settings| settings.color = true, vec![1, 2, 5, 7, 8]),
        (|settings| settings.linetype = true, vec![1, 2, 4, 5, 8]),
        (|settings| settings.lineweight = true, vec![1, 2, 4, 7, 8]),
        // Name/Object style are not comparable for a LINE, so enabling them
        // leaves the default match unchanged.
        (|settings| settings.name = true, vec![1, 2, 4, 5, 7, 8]),
    ];

    for (toggle, expected) in cases {
        let (mut app, mut session) = similar_app();
        select(&mut app, &mut session, 1);
        let before_revision = revision(&app);
        let before_undo = undo_depth(&app);
        let mut settings = SelectSimilarSettings::default();
        toggle(&mut settings);
        execute(
            &mut app,
            &mut session,
            CommandId::SelectSimilar,
            settings_payload(settings),
        )
        .expect("SELECTSIMILAR runs with the subset");

        assert_eq!(selected_ids(&session), expected);
        assert_eq!(revision(&app), before_revision, "read-only");
        assert_eq!(undo_depth(&app), before_undo, "read-only");
    }
}

#[test]
fn object_style_matches_text_by_style_name_only_when_enabled() {
    let (mut app, mut session) = similar_app();
    select(&mut app, &mut session, 12);

    // Default: type + layer, so both TEXTs match even though their styles differ.
    execute(
        &mut app,
        &mut session,
        CommandId::SelectSimilar,
        CommandPayload::None,
    )
    .expect("default SELECTSIMILAR runs");
    assert_eq!(selected_ids(&session), vec![12, 13]);

    // Opt in to Object style: only the same text style matches.
    let (mut app, mut session) = similar_app();
    select(&mut app, &mut session, 12);
    let settings = SelectSimilarSettings {
        object_style: true,
        ..SelectSimilarSettings::default()
    };
    execute(
        &mut app,
        &mut session,
        CommandId::SelectSimilar,
        settings_payload(settings),
    )
    .expect("object-style SELECTSIMILAR runs");
    assert_eq!(selected_ids(&session), vec![12]);
}

#[test]
fn type_only_matches_every_line_including_other_layers() {
    let (mut app, mut session) = similar_app();
    select(&mut app, &mut session, 1);
    let settings = SelectSimilarSettings {
        layer: false,
        color: false,
        linetype: false,
        lineweight: false,
        name: false,
        object_style: false,
    };
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::SelectSimilar,
        settings_payload(settings),
    )
    .expect("type-only similarity runs");

    assert_eq!(selected_ids(&session), vec![1, 2, 3, 4, 5, 7, 8]);
    assert!(
        !session.selection.contains(&model_ref(EntityId(6))),
        "a CIRCLE is still a different type"
    );
    assert_eq!(
        outcome.diagnostics[1].message,
        "SELECTSIMILAR: matching on type"
    );
}

#[test]
fn paper_space_seeds_match_only_paper_space_objects() {
    let (mut app, mut session) = similar_app();
    session.active_space = SpaceId::Paper(LayoutId(1));
    select(&mut app, &mut session, 9);

    execute(
        &mut app,
        &mut session,
        CommandId::SelectSimilar,
        CommandPayload::None,
    )
    .expect("paper-space SELECTSIMILAR runs");

    assert_eq!(selected_ids(&session), vec![9, 10]);
    assert!(
        !session.selection.contains(&model_ref(EntityId(2))),
        "a model-space entity is not in the paper-space catalogue"
    );
}

#[test]
fn a_seed_from_another_space_is_refused_by_name() {
    let (mut app, mut session) = similar_app();
    session.active_space = SpaceId::Paper(LayoutId(1));
    select(&mut app, &mut session, 1);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::SelectSimilar,
        CommandPayload::None,
    )
    .expect_err("model-space seed while paper is active");
    match error {
        CadError::Unsupported(message) => assert!(
            message.starts_with("selectsimilar.space"),
            "stable refusal prefix, got {message:?}"
        ),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn empty_selection_is_refused_by_name() {
    let (mut app, mut session, _, _) = two_lines_and_a_circle();
    let before = session.selection.clone();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::SelectSimilar,
        CommandPayload::None,
    )
    .expect_err("an empty selection cannot produce a similarity match");
    match error {
        CadError::InvalidInput(message) => assert!(
            message.starts_with("selectsimilar.selection_required"),
            "stable refusal prefix, got {message:?}"
        ),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    assert_eq!(session.selection, before);
    assert!(matches!(session.tool, ToolState::Idle));
}

#[test]
fn a_foreign_payload_is_refused() {
    let (mut app, mut session, lines, _) = two_lines_and_a_circle();
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![model_ref(lines[0])]),
    )
    .expect("SELECT accepts one line");
    let before = session.selection.clone();
    for payload in [
        CommandPayload::Selection(Vec::new()),
        CommandPayload::SelectionTool(Box::new(SelectionToolPayload::MatchProp(
            MatchPropSettings::default(),
        ))),
    ] {
        let error = execute(&mut app, &mut session, CommandId::SelectSimilar, payload)
            .expect_err("only SELECTSIMILAR's own settings are accepted");
        assert!(matches!(error, CadError::InvalidInput(_)));
        assert_eq!(session.selection, before);
    }
}

#[test]
fn a_selected_ref_outside_the_catalogue_is_stale() {
    let (mut app, mut session, _, _) = two_lines_and_a_circle();
    let stale = EntityId(9_999);
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![model_ref(stale)]),
    )
    .expect("SELECT stores the ref");
    let error = execute(
        &mut app,
        &mut session,
        CommandId::SelectSimilar,
        CommandPayload::None,
    )
    .expect_err("a ref with no entity cannot be matched");
    assert!(matches!(error, CadError::StaleResult));
}

// ---------------------------------------------------------------------------
// `Name`: members expanded from block references.
// ---------------------------------------------------------------------------

fn block_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for (block, name, member) in [(1u128, "A", 100u128), (2u128, "B", 200u128)] {
        b.insert_block(BlockDefinition {
            id: BlockId(block),
            entities: vec![EntityId(member)],
            dynamic_visibility: None,
        })
        .unwrap();
        b.set_block_metadata(BlockId(block), name, point(0.0, 0.0))
            .unwrap();
        b.insert_entity(line_entity(
            member,
            LayerId(0),
            SpaceId::Block(BlockId(block)),
            0.0,
        ))
        .unwrap();
    }
    for (id, block, x) in [(10u128, 1u128, 0.0), (20u128, 2u128, 10.0)] {
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbBlockReference".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Insert {
                block: BlockId(block),
                transform: Transform3::translation(point(x, 0.0)),
            },
            draw_order: id as i64,
        })
        .unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

fn member_ref(entity: u128, insert: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath(vec![EntityId(insert)]),
        sub_element: None,
    }
}

#[test]
fn name_matches_only_members_of_the_same_root_block_reference() {
    // Default set (type + layer): both members are similar.
    let (mut app, mut session) = block_app();
    session.selection.insert(member_ref(100, 10));
    execute(
        &mut app,
        &mut session,
        CommandId::SelectSimilar,
        CommandPayload::None,
    )
    .expect("the default similarity runs");
    assert_eq!(selected_ids(&session), vec![100, 200]);

    // Name enabled: only the member of the same block reference matches.
    let (mut app, mut session) = block_app();
    session.selection.insert(member_ref(100, 10));
    let settings = SelectSimilarSettings {
        name: true,
        ..SelectSimilarSettings::default()
    };
    execute(
        &mut app,
        &mut session,
        CommandId::SelectSimilar,
        settings_payload(settings),
    )
    .expect("name similarity runs");
    assert_eq!(
        selected_ids(&session),
        vec![100],
        "member 100 belongs to block reference 10 (block A), 200 to reference 20 (block B)"
    );
}
