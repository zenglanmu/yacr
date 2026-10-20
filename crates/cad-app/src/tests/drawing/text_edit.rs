//! TEXTEDIT contract tests (ADR 0020 amendment options A + B).
//!
//! Pins the write path the ADR mandates: re-read the addressed entity, replace
//! **only** its string through one `update_entity`, one transaction / one undo
//! step, preserving style/height/rotation/position/alignment. Every refusal is
//! named and writes nothing (no revision bump, no undo entry) — including a
//! target that is not a directly addressable `Text` and a stale ref.

use super::*;

use cad_domain::{StyleId, TextAlignH, TextAlignV};

const ROTATION: f64 = 0.25;
const HEIGHT: f64 = 3.5;

fn text_geometry(text: &str) -> SemanticGeometry {
    SemanticGeometry::Text {
        text: text.into(),
        position: p3(1.0, 2.0, 0.0),
        style: StyleId(3),
        height: HEIGHT,
        rotation: ROTATION,
        font: Some("arial.ttf".into()),
        h_align: TextAlignH::Right,
        v_align: TextAlignV::Top,
    }
}

/// A drawing with layer 0, a `Text` entity (id 7), a `Line` (id 8) and a
/// `Compound` (id 9) — the wrong-type targets.
fn text_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(7, text_geometry("OLD"), 0)).unwrap();
    b.insert_entity(ent(
        8,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(5.0, 0.0),
        },
        1,
    ))
    .unwrap();
    b.insert_entity(ent(
        9,
        SemanticGeometry::Compound(vec![SemanticGeometry::Line {
            start: point(0.0, 1.0),
            end: point(1.0, 1.0),
        }]),
        2,
    ))
    .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
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

fn stored_entity(app: &Application, id: u128) -> DbEntity {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .cloned()
        .expect("the entity")
}

fn stored_text(app: &Application) -> String {
    match stored_entity(app, 7).geometry {
        SemanticGeometry::Text { text, .. } => text,
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn textedit_replaces_only_the_string_in_one_transaction_and_one_undo_step() {
    let (mut app, mut session) = text_app();
    let before_revision = revision(&app);
    let before = stored_entity(&app, 7);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TextEdit,
        CommandPayload::TextEdit {
            target: ref_of(7),
            text: "NEW".into(),
        },
    )
    .unwrap();

    assert_eq!(outcome.objects, vec![ObjectId(7)]);
    let changes = outcome.changes.expect("change set");
    assert_eq!(changes.before, before_revision);
    assert_eq!(
        changes.after.0,
        before_revision.0 + 1,
        "exactly one revision bump"
    );
    assert_eq!(changes.changes.len(), 1, "one transaction");

    // The text changed …
    assert_eq!(stored_text(&app), "NEW");
    // … and nothing else did.
    let after = stored_entity(&app, 7);
    match after.geometry {
        SemanticGeometry::Text {
            position,
            style,
            height,
            rotation,
            font,
            h_align,
            v_align,
            ..
        } => {
            assert_eq!(position, p3(1.0, 2.0, 0.0));
            assert_eq!(style, StyleId(3));
            assert_eq!(height, HEIGHT);
            assert_eq!(rotation, ROTATION);
            assert_eq!(font.as_deref(), Some("arial.ttf"));
            assert_eq!(h_align, TextAlignH::Right);
            assert_eq!(v_align, TextAlignV::Top);
        }
        other => panic!("expected text, got {other:?}"),
    }
    // Id, layer, space, type key and draw order are untouched too.
    assert_eq!(after.id, before.id);
    assert_eq!(after.object.type_key, before.object.type_key);
    assert_eq!(after.layer, before.layer);
    assert_eq!(after.space, before.space);
    assert_eq!(after.draw_order, before.draw_order);

    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(stored_text(&app), "OLD", "undo restores the old string");
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(stored_text(&app), "NEW", "redo reapplies the edit");
}

#[test]
fn textedit_refuses_non_text_and_stale_targets_by_name_without_writing() {
    let (mut app, mut session) = text_app();
    let rev = revision(&app);

    // A line, a compound and a missing entity are all named refusals.
    for (target, expected) in [
        (ref_of(8), "textedit.target_not_text"),
        (ref_of(9), "textedit.target_not_text"),
        (ref_of(999), "textedit.target_missing"),
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::TextEdit,
            CommandPayload::TextEdit {
                target,
                text: "NEW".into(),
            },
        )
        .unwrap_err();
        let message = format!("{error:?}");
        assert!(message.contains(expected), "{expected}: {message}");
        assert_eq!(revision(&app), rev, "{expected}: no write");
    }

    // A block-instance member / sub-element is not a directly addressable
    // entity: editing must never reach into block internals.
    for target in [
        SelectionRef {
            instance: InstancePath(vec![EntityId(8)]),
            ..ref_of(7)
        },
        SelectionRef {
            sub_element: Some(cad_domain::SubElementId {
                source_key: "face0".into(),
                topology_revision: Revision(0),
            }),
            ..ref_of(7)
        },
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::TextEdit,
            CommandPayload::TextEdit {
                target,
                text: "NEW".into(),
            },
        )
        .unwrap_err();
        assert!(
            format!("{error:?}").contains("textedit.target_not_text"),
            "{error:?}"
        );
        assert_eq!(revision(&app), rev);
    }

    // Empty/whitespace-only and over-long text use the TEXT channel's names.
    for (text, expected) in [
        (String::new(), "tool.text_empty"),
        ("  \t\n".into(), "tool.text_empty"),
        (
            "a".repeat(crate::MAX_TOOL_TEXT_CHARS + 1),
            "tool.text_too_long",
        ),
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::TextEdit,
            CommandPayload::TextEdit {
                target: ref_of(7),
                text,
            },
        )
        .unwrap_err();
        assert!(
            format!("{error:?}").contains(expected),
            "{expected}: {error:?}"
        );
        assert_eq!(revision(&app), rev, "{expected}: no write");
    }

    // A mismatched payload is refused, never a silent no-op.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextEdit,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");

    assert_eq!(
        stored_text(&app),
        "OLD",
        "every refusal left the text alone"
    );
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)), "no transaction was opened");
}

#[test]
fn textedit_revalidates_the_target_at_apply_time() {
    // The pick resolved while the entity existed; by apply time it is gone. The
    // command must refuse with the missing-target name and write nothing.
    let (mut app, mut session) = text_app();
    execute(
        &mut app,
        &mut session,
        CommandId::TextEdit,
        CommandPayload::TextEdit {
            target: ref_of(7),
            text: "FIRST".into(),
        },
    )
    .unwrap();
    assert_eq!(stored_text(&app), "FIRST");
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(7)]),
    )
    .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::EraseEntities,
        CommandPayload::None,
    )
    .unwrap();
    let rev = revision(&app);
    let depth = app.history[&DocumentId(1)].undo_depth();

    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextEdit,
        CommandPayload::TextEdit {
            target: ref_of(7),
            text: "SECOND".into(),
        },
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("textedit.target_missing"),
        "{error:?}"
    );
    assert_eq!(revision(&app), rev);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), depth);
}

#[test]
fn textedit_is_work_only() {
    assert!(CommandId::TextEdit.requires_work_mode());
    let (mut app, session) = text_app();
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    let rev = revision(&app);
    let error = execute(
        &mut app,
        &mut viewer,
        CommandId::TextEdit,
        CommandPayload::TextEdit {
            target: ref_of(7),
            text: "NEW".into(),
        },
    )
    .unwrap_err();
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(revision(&app), rev);
    // The Work session is unrelated state; nothing mutated it.
    assert_eq!(session.document, DocumentId(1));
}

// ---------------------------------------------------------------------------
// ADR-0021 D4: MTEXT / MULTILEADER text, the newline policy and the
// display-consistency invariant.
// ---------------------------------------------------------------------------

use cad_domain::{
    MLeaderContent, MLeaderLine, MLeaderPathType, MLeaderReserved, MLeaderRoot, MLeaderStyleValues,
    MLeaderText, MultiLeaderGeometry, MultiLeaderParts, PlaneFrame,
};

/// A drawing whose entity 7 is an MTEXT (`Text` geometry + `AcDbMText` type key).
fn mtext_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let mut entity = ent(7, text_geometry("OLD"), 0);
    entity.object.type_key = "AcDbMText".into();
    b.insert_entity(entity).unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
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

/// A content-less MULTILEADER (the migrated `172 = 0` shape).
fn content_less_leader() -> SemanticGeometry {
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
        content: MLeaderContent::None,
        text: MLeaderText {
            string: String::new(),
            location: p3(10.0, 5.0, 0.0),
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
            attachment_point: 2,
            left_attachment: 2,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: p3(10.0, 5.0, 0.0),
            dogleg_vector: p3(1.0, 0.0, 0.0),
            dogleg_length: 2.0,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![p3(0.0, 0.0, 0.0), p3(8.0, 0.0, 0.0)],
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

fn text_of(app: &Application, id: u128) -> String {
    match stored_entity(app, id).geometry {
        SemanticGeometry::Text { text, .. } => text,
        other => panic!("expected text, got {other:?}"),
    }
}

/// The stored MULTILEADER and its display's text children.
fn leader_state(app: &Application, id: u128) -> (MultiLeaderGeometry, Vec<String>) {
    match stored_entity(app, id).geometry {
        SemanticGeometry::MultiLeader(leader) => {
            let texts = leader
                .display()
                .iter()
                .filter_map(|child| match child {
                    SemanticGeometry::Text { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect();
            (*leader, texts)
        }
        other => panic!("expected a multileader, got {other:?}"),
    }
}

#[test]
fn textedit_on_a_multileader_writes_one_transaction_and_a_consistent_display() {
    let (mut app, mut session) = text_app();
    let created = execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        CommandPayload::MLeader {
            arrow: p3(0.0, 0.0, 0.0),
            landing: p3(10.0, 2.0, 0.0),
            text: "OLD".into(),
        },
    )
    .unwrap();
    let entity = created.objects[0].0;
    let (before, before_texts) = leader_state(&app, entity);
    assert_eq!(before_texts, vec!["OLD".to_string()]);
    let rev = revision(&app);
    let undo = app.history[&DocumentId(1)].undo_depth();

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TextEdit,
        CommandPayload::TextEdit {
            target: ref_of(entity),
            text: "NEW".into(),
        },
    )
    .unwrap();

    assert_eq!(outcome.objects, vec![ObjectId(entity)]);
    let changes = outcome.changes.expect("change set");
    assert_eq!(changes.before, rev);
    assert_eq!(changes.after.0, rev.0 + 1, "exactly one revision bump");
    assert_eq!(changes.changes.len(), 1, "one transaction");
    assert_eq!(
        app.history[&DocumentId(1)].undo_depth(),
        undo + 1,
        "one undo step"
    );

    let (after, after_texts) = leader_state(&app, entity);
    assert_eq!(after.text().string, "NEW");
    // The invariant `update_entity` does not enforce: the stored display was
    // rebuilt, so it shows the new string instead of the old one.
    assert_eq!(after_texts, vec!["NEW".to_string()]);
    assert_eq!(after.display().len(), before.display().len());
    // Nothing else changed.
    assert_eq!(after.override_mask(), before.override_mask());
    assert_eq!(after.style(), before.style());
    assert_eq!(after.style_name(), before.style_name());
    assert_eq!(after.text().width, before.text().width);
    assert_eq!(after.text().location, before.text().location);
    assert_eq!(after.text().rotation, before.text().rotation);
    assert_eq!(after.text().height, before.text().height);
    assert_eq!(
        after.text().attachment_point,
        before.text().attachment_point
    );
    assert_eq!(after.text().left_attachment, before.text().left_attachment);
    assert_eq!(after.roots(), before.roots());
    assert_eq!(after.lines(), before.lines());

    // Undo restores both the string and the derived display.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let (undone, undone_texts) = leader_state(&app, entity);
    assert_eq!(undone.text().string, "OLD");
    assert_eq!(undone_texts, vec!["OLD".to_string()]);
}

#[test]
fn textedit_refuses_a_content_less_multileader_by_name() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(7, content_less_leader(), 0)).unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let rev = revision(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextEdit,
        CommandPayload::TextEdit {
            target: ref_of(7),
            text: "NEW".into(),
        },
    )
    .unwrap_err();
    let message = format!("{error:?}");
    assert!(
        message.contains("textedit.mleader_content_not_text"),
        "{message}"
    );
    assert_eq!(revision(&app), rev, "nothing written");
    assert!(!app.can_undo(&DocumentId(1)), "no transaction opened");
}

#[test]
fn mtext_converts_every_newline_form_to_paragraph_breaks() {
    let (mut app, mut session) = mtext_app();
    execute(
        &mut app,
        &mut session,
        CommandId::TextEdit,
        CommandPayload::TextEdit {
            target: ref_of(7),
            text: "A\r\nB\rC\nD".into(),
        },
    )
    .unwrap();
    assert_eq!(text_of(&app, 7), "A\\PB\\PC\\PD");
    // Undo restores the raw stored string.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(text_of(&app, 7), "OLD");
}

#[test]
fn mtext_passes_every_other_control_code_through_verbatim() {
    let (mut app, mut session) = mtext_app();
    let draft = "{\\fArial|b1;bold} plain \\W0.8;wide \\H2.5;tall 100% \\S1^2;";
    execute(
        &mut app,
        &mut session,
        CommandId::TextEdit,
        CommandPayload::TextEdit {
            target: ref_of(7),
            text: draft.into(),
        },
    )
    .unwrap();
    assert_eq!(text_of(&app, 7), draft);
}

#[test]
fn text_refuses_a_newline_draft_and_writes_nothing() {
    let (mut app, mut session) = text_app();
    let rev = revision(&app);
    for draft in ["A\nB", "A\rB", "A\r\nB"] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::TextEdit,
            CommandPayload::TextEdit {
                target: ref_of(7),
                text: draft.into(),
            },
        )
        .unwrap_err();
        let message = format!("{error:?}");
        assert!(message.contains("textedit.newline_text"), "{message}");
        assert_eq!(revision(&app), rev, "{draft:?}: nothing written");
    }
    assert_eq!(text_of(&app, 7), "OLD");
    assert!(!app.can_undo(&DocumentId(1)), "no transaction was opened");
}

#[test]
fn text_keeps_other_control_codes_verbatim() {
    let (mut app, mut session) = text_app();
    // TEXT does not interpret `\P` either, but the dialog only *writes* the
    // codes the user typed; nothing is rewritten.
    let draft = "{brackets} \\W0.8; \\P literal";
    execute(
        &mut app,
        &mut session,
        CommandId::TextEdit,
        CommandPayload::TextEdit {
            target: ref_of(7),
            text: draft.into(),
        },
    )
    .unwrap();
    assert_eq!(text_of(&app, 7), draft);
}
