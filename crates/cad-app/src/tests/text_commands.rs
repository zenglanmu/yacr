//! The three selection-based text commands (2026-11-07):
//! `TORIENT` (Most Readable / absolute), `TCASE` (five modes with MTEXT escape
//! preservation) and `TXT2MTXT` (top-down/left-right join into one `AcDbMText`).
//!
//! The common contract is pinned here: only the documented field changes, every
//! refusal writes nothing (no revision bump, no undo entry), a no-change outcome
//! reports `*.unchanged` and opens no transaction, and a real change is exactly
//! one transaction / one undo step.

use super::*;
use crate::TextCaseMode;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn text_entity(
    id: u128,
    text: &str,
    position: Point3,
    rotation: f64,
    order: i64,
    type_key: &str,
) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(7),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Text {
            text: text.into(),
            position,
            style: StyleId(0),
            height: 2.0,
            rotation,
            font: Some("arial.ttf".into()),
            h_align: TextAlignH::Center,
            v_align: TextAlignV::Middle,
            wrap_width: None,
        },
        draw_order: order,
    }
}

fn line_entity(id: u128, order: i64) -> DbEntity {
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
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        draw_order: order,
    }
}

fn app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
    build(entities, AppMode::Work)
}

fn build(entities: Vec<DbEntity>, mode: AppMode) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "NOTES")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    builder
        .insert_layout(cad_db::Layout {
            id: LayoutId(2),
            name: "Paper".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([71u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), mode))
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        Command {
            schema_version: 1,
            id,
            document: DocumentId(1),
            viewport: ViewportId(1),
            payload,
        },
    )
}

fn ref_of(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn select(session: &mut SessionState, entities: &[u128]) {
    session
        .selection
        .replace(entities.iter().map(|id| ref_of(*id)).collect::<Vec<_>>());
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

fn entity(app: &Application, id: u128) -> Option<DbEntity> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .cloned()
}

fn text_of(app: &Application, id: u128) -> String {
    match &entity(app, id).expect("entity exists").geometry {
        SemanticGeometry::Text { text, .. } => text.clone(),
        other => panic!("expected Text, got {other:?}"),
    }
}

fn rotation_of(app: &Application, id: u128) -> f64 {
    match &entity(app, id).expect("entity exists").geometry {
        SemanticGeometry::Text { rotation, .. } => *rotation,
        other => panic!("expected Text, got {other:?}"),
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn assert_invalid(error: CadError, code: &str) {
    match error {
        CadError::InvalidInput(message) => assert!(
            message.starts_with(code),
            "expected refusal starting with {code}, got {message}"
        ),
        other => panic!("expected InvalidInput {code}, got {other:?}"),
    }
}

fn assert_unsupported(error: CadError, needle: &str) {
    match error {
        CadError::Unsupported(message) => assert!(
            message.contains(needle),
            "expected Unsupported containing {needle}, got {message}"
        ),
        other => panic!("expected Unsupported containing {needle}, got {other:?}"),
    }
}

// --- TORIENT -----------------------------------------------------------------

#[test]
fn most_readable_folds_every_quadrant_into_the_upright_half_open_range() {
    // 0/45 stay; 135 -> -45; 225 -> 45; 315 -> -45; the exact ±90 boundary is
    // the included end of (-π/2, π/2].
    for (degrees, expected) in [
        (0.0_f64, 0.0),
        (45.0, 45.0),
        (135.0, -45.0),
        (225.0, 45.0),
        (315.0, -45.0),
        (90.0, 90.0),
        (-90.0, 90.0),
        (180.0, 0.0),
    ] {
        let actual = crate::app_drawing::most_readable_rotation(degrees.to_radians()).to_degrees();
        assert!(
            near(actual, expected),
            "{degrees}° should fold to {expected}°, got {actual}°"
        );
    }
    // Non-finite input is returned unchanged rather than fabricated.
    assert!(crate::app_drawing::most_readable_rotation(f64::NAN).is_nan());
}

#[test]
fn torient_most_readable_changes_only_rotation_and_is_one_undo_step() {
    let source = text_entity(
        1,
        "hello",
        p3(3.0, 4.0, 0.0),
        135f64.to_radians(),
        4,
        "AcDbText",
    );
    let before = source.clone();
    let (mut app, mut session) = app_with(vec![source]);
    select(&mut session, &[1]);
    let before_revision = revision(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Torient,
        CommandPayload::None,
    )
    .expect("most readable TORIENT");
    assert_eq!(
        outcome.objects,
        vec![ObjectId(1)],
        "the same entity is updated"
    );
    assert!(near(rotation_of(&app, 1).to_degrees(), -45.0));
    let after = entity(&app, 1).unwrap();
    let SemanticGeometry::Text {
        text,
        position,
        style,
        height,
        rotation,
        font,
        h_align,
        v_align,
        wrap_width,
    } = &after.geometry
    else {
        panic!("expected Text");
    };
    let SemanticGeometry::Text {
        text: before_text,
        position: before_position,
        style: before_style,
        height: before_height,
        rotation: before_rotation,
        font: before_font,
        h_align: before_h,
        v_align: before_v,
        wrap_width: before_wrap,
    } = &before.geometry
    else {
        panic!("expected Text");
    };
    assert_eq!(text, before_text, "content is untouched");
    assert_eq!(position, before_position, "location is untouched");
    assert_eq!(style, before_style);
    assert_eq!(height, before_height, "height is untouched");
    assert!(!near(*rotation, *before_rotation));
    assert_eq!(font, before_font);
    assert_eq!(h_align, before_h);
    assert_eq!(v_align, before_v);
    assert_eq!(wrap_width, before_wrap);
    assert_eq!(after.layer, LayerId(7), "layer is preserved");
    assert_eq!(after.draw_order, 4, "draw order is preserved");
    assert_eq!(after.object.type_key, "AcDbText");
    assert_eq!(revision(&app).0, before_revision.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert!(near(rotation_of(&app, 1).to_degrees(), 135.0));
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn torient_absolute_angle_sets_that_rotation_without_moving_the_text() {
    let source = text_entity(1, "abc", p3(1.0, 2.0, 0.0), 0.5, 0, "AcDbMText");
    let (mut app, mut session) = app_with(vec![source]);
    select(&mut session, &[1]);

    execute(
        &mut app,
        &mut session,
        CommandId::Torient,
        CommandPayload::Torient {
            angle_degrees: Some(30.0),
        },
    )
    .expect("absolute TORIENT");
    assert!(near(rotation_of(&app, 1).to_degrees(), 30.0));
    assert_eq!(text_of(&app, 1), "abc");
    assert_eq!(entity(&app, 1).unwrap().object.type_key, "AcDbMText");
    // Moving from 0.5 rad to 30° is a real change -> one undo step.
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn torient_reverse_adds_180_degrees_and_undo_restores_the_original() {
    let source = text_entity(
        1,
        "dir",
        p3(1.0, 2.0, 0.0),
        30f64.to_radians(),
        0,
        "AcDbMText",
    );
    let (mut app, mut session) = app_with(vec![source]);
    select(&mut session, &[1]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Torient,
        CommandPayload::TorientReverse,
    )
    .expect("reverse TORIENT");
    assert_eq!(outcome.objects, vec![ObjectId(1)]);
    assert!(near(rotation_of(&app, 1).to_degrees(), 210.0));
    assert_eq!(text_of(&app, 1), "dir", "content is untouched");
    assert_eq!(undo_depth(&app), 1, "one undo step");
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert!(near(rotation_of(&app, 1).to_degrees(), 30.0));
}

#[test]
fn torient_skip_report_names_the_entity_kind_and_is_stable() {
    let (mut app, mut session) = app_with(vec![line_entity(2, 0)]);
    select(&mut session, &[2]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Torient,
        CommandPayload::TorientReverse,
    )
    .expect("a non-text selection is reported, not fatal");
    let skipped: Vec<&str> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "torient.not_reset")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert_eq!(skipped.len(), 1);
    assert!(skipped[0].contains("entity 2 (LINE)"), "{}", skipped[0]);
    assert!(
        skipped[0].contains("TORIENT resets Text/MText rotations only"),
        "{}",
        skipped[0]
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "torient.unchanged"));
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn torient_reports_non_text_targets_but_still_rewrites_the_text() {
    let (mut app, mut session) = app_with(vec![
        text_entity(
            1,
            "hi",
            p3(0.0, 0.0, 0.0),
            225f64.to_radians(),
            0,
            "AcDbText",
        ),
        line_entity(2, 1),
    ]);
    select(&mut session, &[1, 2]);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Torient,
        CommandPayload::None,
    )
    .expect("a non-text neighbour never fails the command");
    assert_eq!(outcome.objects, vec![ObjectId(1)]);
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "torient.not_reset"),
        "the LINE is reported: {:?}",
        outcome.diagnostics
    );
    assert!(near(rotation_of(&app, 1).to_degrees(), 45.0));
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn torient_with_only_non_text_or_upright_text_is_unchanged_and_writes_nothing() {
    // Only a LINE: no Text can change.
    let (mut app, mut session) = app_with(vec![line_entity(2, 0)]);
    select(&mut session, &[2]);
    let before_revision = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Torient,
        CommandPayload::None,
    )
    .expect("nothing to orient is a successful no-op");
    assert!(outcome.objects.is_empty());
    assert!(outcome.changes.is_none());
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "torient.unchanged"));
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);

    // An already-upright Text is the same no-op.
    let (mut app, mut session) = app_with(vec![text_entity(
        1,
        "up",
        p3(0.0, 0.0, 0.0),
        0.25,
        0,
        "AcDbText",
    )]);
    select(&mut session, &[1]);
    let before_revision = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Torient,
        CommandPayload::None,
    )
    .expect("an upright text is unchanged");
    assert!(outcome.objects.is_empty());
    assert_eq!(outcome.diagnostics[0].code, "torient.unchanged");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
    assert!(near(rotation_of(&app, 1), 0.25));
}

#[test]
fn torient_refuses_an_empty_selection_and_viewer_mode() {
    let (mut app, mut session) = app_with(vec![]);
    let before_revision = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Torient,
        CommandPayload::None,
    )
    .expect_err("empty selection");
    assert_invalid(error, "torient.needs_selection");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);

    let (mut app, mut viewer) = build(vec![line_entity(1, 0)], AppMode::Viewer);
    let error = execute(
        &mut app,
        &mut viewer,
        CommandId::Torient,
        CommandPayload::None,
    )
    .expect_err("Viewer cannot write");
    assert!(matches!(error, CadError::PermissionDenied));
}

// --- TCASE -------------------------------------------------------------------

#[test]
fn tcase_applies_all_five_modes_and_undo_restores_the_string() {
    for (mode, expected) in [
        (TextCaseMode::Upper, "HELLO WORLD"),
        (TextCaseMode::Lower, "hello world"),
        (TextCaseMode::Title, "Hello World"),
        (TextCaseMode::Sentence, "Hello world"),
        (TextCaseMode::Toggle, "HellO world"),
    ] {
        let (mut app, mut session) = app_with(vec![text_entity(
            1,
            "hELLo WORLD",
            p3(0.0, 0.0, 0.0),
            0.0,
            0,
            "AcDbText",
        )]);
        select(&mut session, &[1]);
        execute(
            &mut app,
            &mut session,
            CommandId::TCase,
            CommandPayload::TCase { mode },
        )
        .unwrap_or_else(|error| panic!("{mode:?} refused: {error:?}"));
        assert_eq!(text_of(&app, 1), expected, "{mode:?}");
        assert_eq!(undo_depth(&app), 1, "{mode:?}: one undo step");
        execute(
            &mut app,
            &mut session,
            CommandId::Undo,
            CommandPayload::None,
        )
        .expect("undo");
        assert_eq!(text_of(&app, 1), "hELLo WORLD", "{mode:?}: undo restores");
    }
}

#[test]
fn tcase_bare_defaults_to_title() {
    let (mut app, mut session) = app_with(vec![text_entity(
        1,
        "the quick BROWN fox",
        p3(0.0, 0.0, 0.0),
        0.0,
        0,
        "AcDbText",
    )]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TCase,
        CommandPayload::None,
    )
    .expect("bare TCASE");
    assert_eq!(text_of(&app, 1), "The Quick Brown Fox");
}

#[test]
fn tcase_preserves_every_mtext_escape_sequence_verbatim() {
    let source = r"{\fArial|b0|i0;Hello \H2.5x;WORLD}\Pnext \C1;LINE \~and \\ done";
    let expected = r"{\fArial|b0|i0;Hello \H2.5x;World}\PNext \C1;Line \~And \\ Done";
    let (mut app, mut session) = app_with(vec![text_entity(
        1,
        source,
        p3(0.0, 0.0, 0.0),
        0.0,
        0,
        "AcDbMText",
    )]);
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::TCase,
        CommandPayload::TCase {
            mode: TextCaseMode::Title,
        },
    )
    .expect("MTEXT title case");
    assert_eq!(text_of(&app, 1), expected);
    // The stored type key stays MTEXT: this is a string edit, not a retype.
    assert_eq!(entity(&app, 1).unwrap().object.type_key, "AcDbMText");
}

#[test]
fn the_pure_case_transformer_handles_grouping_toggles_and_punctuation() {
    // A toggle code has no `;`; a generic "consume to ;" rule would eat the
    // visible text after it. `\L`/`\l` are copied pair-wise.
    assert_eq!(
        crate::app_drawing::apply_text_case(r"\Lunder\l text", TextCaseMode::Title),
        r"\LUnder\l Text"
    );
    // Punctuation does not start a new word; whitespace does.
    assert_eq!(
        crate::app_drawing::apply_text_case(
            "(hello) o'neill state-of-the-art",
            TextCaseMode::Title
        ),
        "(Hello) O'neill State-of-the-art"
    );
    // Sentence case only upper-cases the first alphabetic character overall.
    assert_eq!(
        crate::app_drawing::apply_text_case("  (hello) WORLD", TextCaseMode::Sentence),
        "  (Hello) world"
    );
    // Grouping braces are copied and never count as visible text.
    assert_eq!(
        crate::app_drawing::apply_text_case("{hello} {WORLD}", TextCaseMode::Title),
        "{Hello} {World}"
    );
    // Toggle is per alphabetic character and leaves codes/digits alone.
    assert_eq!(
        crate::app_drawing::apply_text_case(r"aBc \P123 \fArial;xY", TextCaseMode::Toggle),
        r"AbC \P123 \fArial;Xy"
    );
}

#[test]
fn tcase_reports_non_text_targets_and_never_partially_claims_them() {
    let (mut app, mut session) = app_with(vec![
        text_entity(1, "note", p3(0.0, 0.0, 0.0), 0.0, 0, "AcDbText"),
        line_entity(2, 1),
        line_entity(3, 2),
    ]);
    select(&mut session, &[1, 2, 3]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TCase,
        CommandPayload::TCase {
            mode: TextCaseMode::Upper,
        },
    )
    .expect("a non-text neighbour is reported, not fatal");
    assert_eq!(outcome.objects, vec![ObjectId(1)]);
    let skipped: Vec<_> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "tcase.not_reset")
        .collect();
    assert_eq!(skipped.len(), 2, "both LINEs are reported: {skipped:?}");
    assert_eq!(text_of(&app, 1), "NOTE");
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn tcase_without_a_change_is_unchanged_and_opens_no_transaction() {
    let (mut app, mut session) = app_with(vec![text_entity(
        1,
        "ALREADY",
        p3(0.0, 0.0, 0.0),
        0.0,
        0,
        "AcDbText",
    )]);
    select(&mut session, &[1]);
    let before_revision = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TCase,
        CommandPayload::TCase {
            mode: TextCaseMode::Upper,
        },
    )
    .expect("an already-upper text is unchanged");
    assert!(outcome.objects.is_empty());
    assert!(outcome.changes.is_none());
    assert_eq!(outcome.diagnostics[0].code, "tcase.unchanged");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn tcase_refuses_an_empty_selection() {
    let (mut app, mut session) = app_with(vec![]);
    let before_revision = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TCase,
        CommandPayload::None,
    )
    .expect_err("empty selection");
    assert_invalid(error, "tcase.needs_selection");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

// --- TXT2MTXT ----------------------------------------------------------------

#[test]
fn txt2mtxt_sorts_top_down_then_left_right_and_joins_raw_strings() {
    // Y=9: B(0), C(5); Y=5: A(10); Y=2: D(0). The first sorted source is B.
    let b = text_entity(1, r"B\fArial;", p3(0.0, 9.0, 0.0), 0.25, 7, "AcDbText");
    let c = text_entity(2, "C", p3(5.0, 9.0, 0.0), 0.0, 1, "AcDbMText");
    let a = text_entity(3, "A", p3(10.0, 5.0, 0.0), 0.0, 2, "AcDbText");
    let d = text_entity(4, "D", p3(0.0, 2.0, 0.0), 0.0, 3, "AcDbText");
    let (mut app, mut session) = app_with(vec![b.clone(), c, a, d]);
    select(&mut session, &[1, 2, 3, 4]);
    let before_revision = revision(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Txt2MText,
        CommandPayload::None,
    )
    .expect("TXT2MTXT combines the four texts");
    assert_eq!(outcome.objects.len(), 1, "exactly one new MTEXT");
    let new_id = outcome.objects[0].0;
    let combined = entity(&app, new_id).expect("the new entity exists");
    assert_eq!(combined.object.type_key, "AcDbMText");
    let SemanticGeometry::Text {
        text,
        position,
        style,
        height,
        rotation,
        font,
        h_align,
        v_align,
        wrap_width,
    } = &combined.geometry
    else {
        panic!("expected Text geometry");
    };
    assert_eq!(
        text, r"B\fArial;\PC\PA\PD",
        "raw strings are joined with \\P and never re-processed"
    );
    // The first sorted source (B) supplies every style/placement field.
    let SemanticGeometry::Text {
        position: b_position,
        style: b_style,
        height: b_height,
        rotation: b_rotation,
        font: b_font,
        h_align: b_h,
        v_align: b_v,
        wrap_width: b_wrap,
        ..
    } = &b.geometry
    else {
        panic!("expected Text");
    };
    assert_eq!(position, b_position, "position comes from the first source");
    assert_eq!(style, b_style);
    assert_eq!(height, b_height);
    assert_eq!(rotation, b_rotation);
    assert_eq!(font, b_font);
    assert_eq!(h_align, b_h);
    assert_eq!(v_align, b_v);
    assert_eq!(wrap_width, b_wrap, "None stays None; no width is invented");
    assert_eq!(combined.layer, LayerId(7), "first source's layer");
    assert_eq!(combined.draw_order, 7, "first source's draw order");
    // All sources are gone, in the same transaction.
    for id in [1u128, 2, 3, 4] {
        assert!(entity(&app, id).is_none(), "source {id} must be deleted");
    }
    assert_eq!(revision(&app).0, before_revision.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "drawing.txt2mtxt"));

    // One undo restores every source and removes the new MTEXT.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert!(entity(&app, new_id).is_none(), "the new MTEXT is removed");
    for id in [1u128, 2, 3, 4] {
        assert!(entity(&app, id).is_some(), "source {id} is restored");
    }
    assert_eq!(text_of(&app, 1), r"B\fArial;");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn txt2mtxt_keeps_a_first_source_wrap_width() {
    let mut source = text_entity(1, "wide", p3(0.0, 0.0, 0.0), 0.0, 0, "AcDbMText");
    if let SemanticGeometry::Text { wrap_width, .. } = &mut source.geometry {
        *wrap_width = Some(12.5);
    }
    let (mut app, mut session) = app_with(vec![source]);
    select(&mut session, &[1]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Txt2MText,
        CommandPayload::None,
    )
    .expect("a single source still becomes one MTEXT");
    let combined = entity(&app, outcome.objects[0].0).unwrap();
    let SemanticGeometry::Text { wrap_width, .. } = combined.geometry else {
        panic!("expected Text");
    };
    assert_eq!(wrap_width, Some(12.5));
}

#[test]
fn txt2mtxt_refuses_the_whole_selection_for_every_unsupported_target() {
    // Mixed Text + LINE.
    let (mut app, mut session) = app_with(vec![
        text_entity(1, "a", p3(0.0, 0.0, 0.0), 0.0, 0, "AcDbText"),
        line_entity(2, 1),
    ]);
    select(&mut session, &[1, 2]);
    let before_revision = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Txt2MText,
        CommandPayload::None,
    )
    .expect_err("a mixed selection is refused");
    assert_unsupported(error, "txt2mtxt.unsupported");
    assert_eq!(revision(&app), before_revision, "no revision bump");
    assert_eq!(undo_depth(&app), 0, "no transaction");
    assert!(entity(&app, 1).is_some() && entity(&app, 2).is_some());

    // A block-instance member / sub-element ref.
    let (mut app, mut session) = app_with(vec![text_entity(
        1,
        "a",
        p3(0.0, 0.0, 0.0),
        0.0,
        0,
        "AcDbText",
    )]);
    session.selection.replace(vec![SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath(vec![EntityId(9)]),
        sub_element: None,
    }]);
    let before_revision = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Txt2MText,
        CommandPayload::None,
    )
    .expect_err("a block member is refused");
    assert_invalid(error, "txt2mtxt.not_addressable");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);

    // Empty selection.
    let (mut app, mut session) = app_with(vec![]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Txt2MText,
        CommandPayload::None,
    )
    .expect_err("an empty selection is refused");
    assert_invalid(error, "txt2mtxt.needs_selection");

    // Paper space is addressable but outside v1.
    let mut paper = text_entity(1, "p", p3(0.0, 0.0, 0.0), 0.0, 0, "AcDbText");
    paper.space = SpaceId::Paper(LayoutId(2));
    let (mut app, mut session) = app_with(vec![paper]);
    select(&mut session, &[1]);
    let before_revision = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Txt2MText,
        CommandPayload::None,
    )
    .expect_err("paper space is unsupported in v1");
    assert_unsupported(error, "txt2mtxt.unsupported");
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn txt2mtxt_refuses_a_viewer_session() {
    let (mut app, mut viewer) = build(
        vec![text_entity(1, "a", p3(0.0, 0.0, 0.0), 0.0, 0, "AcDbText")],
        AppMode::Viewer,
    );
    let error = execute(
        &mut app,
        &mut viewer,
        CommandId::Txt2MText,
        CommandPayload::None,
    )
    .expect_err("Viewer cannot combine text");
    assert!(matches!(error, CadError::PermissionDenied));
}
