//! TEXTEDIT host-path contracts (ADR 0020 amendment options A + B).
//!
//! The interactive path resolves the entity pick **at capture time**, fills the
//! UI prefill with the current string and re-validates the entity at apply time;
//! the panel path resolves the **current single selection** itself (no id on the
//! wire). Both end in the same `TextEdit` command: one `update_entity`, one
//! transaction / one undo step.

use super::*;

fn text_fixture() -> HostController {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(201),
            type_key: "AcDbText".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(201),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Text {
            text: "OLD".into(),
            position: p(100.0, 0.0),
            style: StyleId(0),
            height: 20.0,
            rotation: 0.0,
            font: Some("arial.ttf".into()),
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
        },
        draw_order: 0,
    })
    .unwrap();
    // A wrong-type entity far from the text, so a pick cannot confuse them.
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(202),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(202),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: p(-200.0, -100.0),
            end: p(-100.0, -100.0),
        },
        draw_order: 1,
    })
    .unwrap();
    fixture_with(b.finish().unwrap())
}

fn fixture_with(drawing: DrawingDatabase) -> HostController {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller
        .application
        .workspace
        .documents
        .get_mut(&controller.document_id)
        .unwrap()
        .drawing = Arc::new(drawing);
    controller.set_snap_enabled(false);
    controller
}

fn text_ref(controller: &HostController) -> SelectionRef {
    SelectionRef {
        document: controller.document_id,
        entity: EntityId(201),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn line_ref(controller: &HostController) -> SelectionRef {
    SelectionRef {
        document: controller.document_id,
        entity: EntityId(202),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

/// A point inside the text's placeholder box (`x 100..160`, `y 0..20`).
fn on_the_text() -> Point3 {
    p(110.0, 5.0)
}

fn on_the_line() -> Point3 {
    p(-150.0, -100.0)
}

fn stored_text(controller: &HostController) -> String {
    let drawing = controller.drawing().expect("a document");
    match &drawing.entity(EntityId(201)).expect("the text").geometry {
        SemanticGeometry::Text { text, .. } => text.clone(),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn textedit_pick_prefills_and_commit_changes_only_the_string() {
    let mut controller = text_fixture();
    let undo_before = controller.history_availability().can_undo;

    controller.begin_draw_tool(DrawToolKind::TextEdit).unwrap();
    let preview = controller.session.draw_preview().expect("the tool");
    assert_eq!(preview.kind, DrawToolKind::TextEdit);
    assert_eq!(preview.kind.picked_step_count(), 1);
    assert_eq!(
        preview.text_prefill, None,
        "no prefill before the pick resolves"
    );

    controller
        .capture_tool_point(on_the_text())
        .expect("the pick resolves");
    let preview = controller.session.draw_preview().expect("the tool");
    assert_eq!(preview.picks, vec![text_ref(&controller)]);
    assert!(preview.awaiting_text);
    assert_eq!(
        preview.text_prefill.as_deref(),
        Some("OLD"),
        "the current string is the prefill"
    );
    assert!(!preview.can_confirm(), "text is the terminal step");

    controller.session.set_tool_text("NEW").expect("the text");
    let traced = controller.confirm_draw_traced().expect("the edit commits");
    assert!(traced.result.is_ok(), "{:?}", traced.result);
    assert_eq!(stored_text(&controller), "NEW");
    assert_eq!(
        controller
            .session
            .draw_preview()
            .map(|preview| preview.kind),
        None,
        "the tool completed"
    );
    assert!(!undo_before && controller.history_availability().can_undo);
}

#[test]
fn textedit_refuses_a_non_text_pick_without_consuming_the_step() {
    let mut controller = text_fixture();
    let revision = controller.drawing().unwrap().revision();

    controller.begin_draw_tool(DrawToolKind::TextEdit).unwrap();
    let error = controller.capture_tool_point(on_the_line()).unwrap_err();
    let message = format!("{error:?}");
    assert!(message.contains("textedit.target_not_text"), "{message}");
    let preview = controller.session.draw_preview().expect("the tool");
    assert!(preview.points.is_empty(), "the step was not consumed");
    assert!(preview.picks.is_empty());
    assert_eq!(preview.text_prefill, None);
    assert_eq!(controller.drawing().unwrap().revision(), revision);

    // The capture is still usable: the correct pick now resolves.
    controller.capture_tool_point(on_the_text()).unwrap();
    assert_eq!(
        controller
            .session
            .draw_preview()
            .unwrap()
            .text_prefill
            .as_deref(),
        Some("OLD")
    );
}

#[test]
fn textedit_refuses_a_target_deleted_between_pick_and_text() {
    let mut controller = text_fixture();
    let rev = controller.drawing().unwrap().revision();

    controller.begin_draw_tool(DrawToolKind::TextEdit).unwrap();
    controller.capture_tool_point(on_the_text()).unwrap();
    // The entity disappears between the pick and the text submission (the
    // command line is not capture-modal). The apply-time revalidation must name
    // the missing target and open no transaction.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let replacement = fixture_with(b.finish().unwrap());
    controller
        .application
        .workspace
        .documents
        .get_mut(&controller.document_id)
        .unwrap()
        .drawing = replacement.drawing().expect("the replacement drawing");

    controller.session.set_tool_text("NEW").expect("the text");
    let traced = controller.confirm_draw_traced().expect("the command ran");
    let error = traced.result.expect_err("the target is gone");
    assert!(
        format!("{error:?}").contains("textedit.target_missing"),
        "{error:?}"
    );
    assert!(!controller.history_availability().can_undo);
    assert_eq!(controller.drawing().unwrap().revision(), rev);
    // The capture is preserved by the failed confirm (no silent success).
    assert!(controller.session.draw_preview().is_some());
}

#[test]
fn panel_edit_targets_the_current_selection_and_refuses_by_name() {
    let mut controller = text_fixture();

    // Empty selection.
    let error = controller.set_selected_text("X").unwrap_err();
    assert!(
        format!("{error:?}").contains("textedit.selection_empty"),
        "{error:?}"
    );

    // Multi-selection.
    controller
        .set_selection(vec![text_ref(&controller), line_ref(&controller)])
        .unwrap();
    let error = controller.set_selected_text("X").unwrap_err();
    assert!(
        format!("{error:?}").contains("textedit.selection_multiple"),
        "{error:?}"
    );

    // Single non-text selection.
    controller
        .set_selection(vec![line_ref(&controller)])
        .unwrap();
    let error = controller.set_selected_text("X").unwrap_err();
    assert!(
        format!("{error:?}").contains("textedit.target_not_text"),
        "{error:?}"
    );

    // A block-instance member is never reached into.
    let member = SelectionRef {
        instance: InstancePath(vec![EntityId(202)]),
        ..text_ref(&controller)
    };
    controller.set_selection(vec![member]).unwrap();
    let error = controller.set_selected_text("X").unwrap_err();
    assert!(
        format!("{error:?}").contains("textedit.target_not_text"),
        "{error:?}"
    );

    // Empty and over-long strings use the text-channel names, with no write.
    controller
        .set_selection(vec![text_ref(&controller)])
        .unwrap();
    let rev = controller.drawing().unwrap().revision();
    for (text, expected) in [
        ("".to_string(), "tool.text_empty"),
        ("  ".to_string(), "tool.text_empty"),
        (
            "a".repeat(crate::MAX_TOOL_TEXT_CHARS + 1),
            "tool.text_too_long",
        ),
    ] {
        let error = controller.set_selected_text(&text).unwrap_err();
        assert!(
            format!("{error:?}").contains(expected),
            "{expected}: {error:?}"
        );
        assert_eq!(controller.drawing().unwrap().revision(), rev);
    }
    assert_eq!(stored_text(&controller), "OLD");

    // The accepted edit changes the selected entity, one undo step.
    controller.set_selected_text("PANEL").unwrap();
    assert_eq!(stored_text(&controller), "PANEL");
    assert!(controller.history_availability().can_undo);
}

#[test]
fn the_status_selection_block_carries_the_round_trippable_text() {
    let mut controller = text_fixture();
    controller
        .set_selection(vec![text_ref(&controller)])
        .unwrap();
    let properties = controller.selection_properties().expect("properties");
    assert_eq!(properties.text.as_deref(), Some("OLD"));
    // The display rows are unchanged (the text row is still there, formatted).
    assert!(properties.rows.iter().any(|row| row.key == "text"));

    controller
        .set_selection(vec![line_ref(&controller)])
        .unwrap();
    assert_eq!(
        controller.selection_properties().unwrap().text,
        None,
        "a non-text selection has no editable text"
    );

    controller.set_selection(Vec::new()).unwrap();
    assert_eq!(controller.selection_properties().unwrap().text, None);
}

// ---------------------------------------------------------------------------
// ADR-0021 D4: the popup payload comes from the real selection, and both
// entry paths share the newline policy.
// ---------------------------------------------------------------------------

fn mtext_geometry(text: &str) -> SemanticGeometry {
    SemanticGeometry::Text {
        text: text.into(),
        position: p(100.0, 0.0),
        style: StyleId(0),
        height: 20.0,
        rotation: 0.0,
        font: Some("arial.ttf".into()),
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
    }
}

fn leader_geometry(text: Option<&str>) -> SemanticGeometry {
    let content = if text.is_some() {
        MLeaderContent::Text
    } else {
        MLeaderContent::None
    };
    let (geometry, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: MLeaderStyleValues {
            path_type: MLeaderPathType::StraightLineSegments,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 2.0,
            landing_gap: 0.1,
            arrowhead_size: 0.5,
            text_height: 0.2,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 1.0,
        },
        style_name: "Standard".into(),
        override_mask: 0,
        content,
        text: MLeaderText {
            string: text.unwrap_or_default().into(),
            location: p(300.0, 0.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
            attachment_point: 2,
            left_attachment: 2,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: p(300.0, 0.0),
            dogleg_vector: p(1.0, 0.0),
            dogleg_length: 2.0,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![p(280.0, -20.0), p(298.0, 0.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    SemanticGeometry::MultiLeader(Box::new(geometry))
}

fn entity(id: u128, type_key: &str, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: order,
    }
}

fn single_ref(controller: &HostController, id: u128) -> SelectionRef {
    SelectionRef {
        document: controller.document_id,
        entity: EntityId(id),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

/// A drawing with one of each editable kind plus the refusal cases:
/// 201 MTEXT, 202 LINE, 203 MULTILEADER text, 204 content-less leader,
/// 205 TEXT.
fn mixed_text_fixture() -> HostController {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(entity(201, "AcDbMText", mtext_geometry("LINE1\\PLINE2"), 0))
        .unwrap();
    b.insert_entity(entity(
        202,
        "AcDbLine",
        SemanticGeometry::Line {
            start: p(-200.0, -100.0),
            end: p(-100.0, -100.0),
        },
        1,
    ))
    .unwrap();
    b.insert_entity(entity(
        203,
        "AcDbMultiLeader",
        leader_geometry(Some("LEADER\\PNOTE")),
        2,
    ))
    .unwrap();
    b.insert_entity(entity(204, "AcDbMultiLeader", leader_geometry(None), 3))
        .unwrap();
    b.insert_entity(entity(
        205,
        "AcDbText",
        SemanticGeometry::Text {
            text: "SINGLE".into(),
            position: p(-400.0, 0.0),
            style: StyleId(0),
            height: 20.0,
            rotation: 0.0,
            font: Some("arial.ttf".into()),
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
        },
        4,
    ))
    .unwrap();
    fixture_with(b.finish().unwrap())
}

fn stored_string(controller: &HostController, id: u128) -> String {
    let drawing = controller.drawing().expect("a document");
    let entity = drawing.entity(EntityId(id)).expect("the entity");
    crate::selection::editable_text_source(entity)
        .map(|(text, _)| text.to_string())
        .unwrap_or_default()
}

#[test]
fn selected_text_edit_derives_the_payload_from_the_real_selection() {
    let mut controller = mixed_text_fixture();

    // TEXT: single-line.
    controller
        .set_selection(vec![single_ref(&controller, 205)])
        .unwrap();
    assert_eq!(
        controller.selected_text_edit().unwrap(),
        Some(("SINGLE".to_string(), false))
    );

    // MTEXT: multiline, raw `\P` preserved (the popup maps it at display time).
    controller
        .set_selection(vec![single_ref(&controller, 201)])
        .unwrap();
    assert_eq!(
        controller.selected_text_edit().unwrap(),
        Some(("LINE1\\PLINE2".to_string(), true))
    );

    // MULTILEADER text: multiline.
    controller
        .set_selection(vec![single_ref(&controller, 203)])
        .unwrap();
    assert_eq!(
        controller.selected_text_edit().unwrap(),
        Some(("LEADER\\PNOTE".to_string(), true))
    );

    // Content-less leader, non-text, multi-selection: no payload.
    for selection in [
        vec![single_ref(&controller, 204)],
        vec![single_ref(&controller, 202)],
        vec![single_ref(&controller, 201), single_ref(&controller, 203)],
    ] {
        controller.set_selection(selection).unwrap();
        assert_eq!(controller.selected_text_edit().unwrap(), None);
    }

    // A block-instance member / sub-element is never reached into.
    controller
        .set_selection(vec![SelectionRef {
            instance: InstancePath(vec![EntityId(202)]),
            ..single_ref(&controller, 201)
        }])
        .unwrap();
    assert_eq!(controller.selected_text_edit().unwrap(), None);
    controller
        .set_selection(vec![SelectionRef {
            sub_element: Some(cad_domain::SubElementId {
                source_key: "face0".into(),
                topology_revision: Revision(0),
            }),
            ..single_ref(&controller, 201)
        }])
        .unwrap();
    assert_eq!(controller.selected_text_edit().unwrap(), None);

    // Empty selection and a missing document are no payload, not a fake one.
    controller.set_selection(Vec::new()).unwrap();
    assert_eq!(controller.selected_text_edit().unwrap(), None);
}

#[test]
fn both_entry_paths_apply_the_same_newline_policy() {
    let mut controller = mixed_text_fixture();

    // Panel path (`SetSelectedText`) on MTEXT: convert, one transaction.
    controller
        .set_selection(vec![single_ref(&controller, 201)])
        .unwrap();
    controller.set_selected_text("A\r\nB\rC\nD").unwrap();
    assert_eq!(stored_string(&controller, 201), "A\\PB\\PC\\PD");

    // Panel path on TEXT: refused by name, nothing written.
    controller
        .set_selection(vec![single_ref(&controller, 205)])
        .unwrap();
    let rev = controller.drawing().unwrap().revision();
    let error = controller.set_selected_text("A\nB").unwrap_err();
    assert!(
        format!("{error:?}").contains("textedit.newline_text"),
        "{error:?}"
    );
    assert_eq!(stored_string(&controller, 205), "SINGLE");
    assert_eq!(controller.drawing().unwrap().revision(), rev);

    // Interactive path (`TEXTEDIT` pick + terminal text): same command, same
    // policy, and the leader prefill is the stored raw string.
    controller
        .set_selection(vec![single_ref(&controller, 203)])
        .unwrap();
    controller.begin_draw_tool(DrawToolKind::TextEdit).unwrap();
    controller.capture_tool_point(p(300.0, 0.0)).unwrap();
    let preview = controller.session.draw_preview().expect("the tool");
    assert_eq!(preview.text_prefill.as_deref(), Some("LEADER\\PNOTE"));
    controller.session.set_tool_text("X\nY").unwrap();
    controller.confirm_draw().unwrap();
    assert_eq!(stored_string(&controller, 203), "X\\PY");
}
