//! MTEXT interactive-tool host contracts (2026-10-26).
//!
//! The tool captures the two opposite corners of the wrap rectangle and then the
//! terminal typed text. The **host** is the one place the payload's two-corner
//! model is applied: `width = |opposite.x − first.x|` (finite, `> 0`), height from
//! [`ToolParameter::TextHeight`], rotation from
//! [`ToolParameter::TextRotation`] (degrees → radians, converted once) and the
//! attachment from [`ToolParameter::MTextAttachment`] (AutoCAD's `1..=9` codes →
//! the domain alignment pair; the untouched defaults are `0` rad and top-left).
//! A degenerate or off-plane pair, and a value the closed sets do not allow, are
//! explicit refusals that open no transaction.

use super::*;

use crate::{DrawIntent, ToolParameter, ToolValue};

fn fixture() -> HostController {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    controller
}

fn entity_count(controller: &HostController) -> usize {
    controller
        .drawing()
        .expect("a document")
        .model_space()
        .len()
}

/// One stored `AcDbMText`, projected to the fields creation must pin.
struct StoredMText {
    text: String,
    position: Point3,
    height: f64,
    rotation: f64,
    h_align: TextAlignH,
    v_align: TextAlignV,
    width: Option<f64>,
}

fn mtexts(controller: &HostController) -> Vec<StoredMText> {
    let drawing = controller.drawing().expect("a document");
    drawing
        .model_space()
        .iter()
        .filter(|entity| entity.object.type_key == "AcDbMText")
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Text {
                text,
                position,
                height,
                rotation,
                h_align,
                v_align,
                wrap_width,
                ..
            } => Some(StoredMText {
                text: text.clone(),
                position: *position,
                height: *height,
                rotation: *rotation,
                h_align: *h_align,
                v_align: *v_align,
                width: *wrap_width,
            }),
            _ => None,
        })
        .collect()
}

/// Run the whole capture and return the traced confirm.
fn capture_and_confirm(
    controller: &mut HostController,
    first: Point3,
    opposite: Point3,
    text: &str,
) -> CadResult<crate::TracedCommandOutcome> {
    controller.begin_draw_tool(DrawToolKind::MText)?;
    controller.capture_tool_point(first)?;
    controller.capture_tool_point(opposite)?;
    controller.session.set_tool_text(text)?;
    controller.confirm_draw_traced()
}

#[test]
fn mtext_captures_two_corners_and_writes_one_acdbmtext() {
    let mut controller = fixture();
    let before = entity_count(&controller);

    controller.begin_draw_tool(DrawToolKind::MText).unwrap();
    let preview = controller.session.draw_preview().expect("the tool");
    assert_eq!(preview.kind, DrawToolKind::MText);
    assert_eq!(preview.remaining_points(), Some(2));
    assert!(!preview.awaiting_text);

    controller.capture_tool_point(p(100.0, 50.0)).unwrap();
    let preview = controller.session.draw_preview().expect("the tool");
    assert_eq!(preview.remaining_points(), Some(1));
    assert!(!preview.awaiting_text);

    controller.capture_tool_point(p(160.0, 90.0)).unwrap();
    let preview = controller.session.draw_preview().expect("the tool");
    assert!(preview.awaiting_text, "the terminal text step is open");
    assert!(!preview.can_confirm(), "text is terminal, not confirmable");

    // The same terminal text channel TEXT uses; newlines are legal because the
    // target is an `AcDbMText` and the shared policy stores them as `\P`.
    controller.session.set_tool_text("LINE1\r\nLINE2").unwrap();
    let traced = controller.confirm_draw_traced().expect("the create runs");
    assert!(traced.result.is_ok(), "{:?}", traced.result);
    assert_eq!(
        controller
            .session
            .draw_preview()
            .map(|preview| preview.kind),
        None,
        "the tool completed"
    );

    let created = mtexts(&controller);
    assert_eq!(created.len(), 1, "exactly one MTEXT entity");
    let stored = &created[0];
    assert_eq!(
        stored.text, "LINE1\\PLINE2",
        "newlines use the one shared policy"
    );
    assert_eq!(
        stored.position,
        p(100.0, 50.0),
        "the first corner is the position"
    );
    assert_eq!(
        stored.height,
        crate::draw_tool::TEXT_DEFAULT_HEIGHT,
        "the documented TEXT default, never invented per invocation"
    );
    assert_eq!(
        stored.rotation, 0.0,
        "the untouched `textRotation` default is 0, exactly as before"
    );
    assert_eq!(
        stored.h_align,
        TextAlignH::Left,
        "the untouched `mtextAttachment` default is 1 (left)"
    );
    assert_eq!(
        stored.v_align,
        TextAlignV::Top,
        "the untouched `mtextAttachment` default is 1 (top)"
    );
    assert_eq!(
        stored.width,
        Some(60.0),
        "width = |opposite.x − first.x|; the vertical span is not stored"
    );
    assert_eq!(entity_count(&controller), before + 1);
    assert!(controller.history_availability().can_undo);
}

#[test]
fn mtext_height_comes_from_the_shared_text_height_parameter() {
    let mut controller = fixture();
    controller
        .session
        .set_tool_parameter(ToolParameter::TextHeight, ToolValue::Number(5.0))
        .expect("a positive height is accepted");
    let traced = capture_and_confirm(&mut controller, p(0.0, 0.0), p(40.0, 20.0), "HEIGHT")
        .expect("the create runs");
    assert!(traced.result.is_ok(), "{:?}", traced.result);
    let created = mtexts(&controller);
    assert_eq!(created.len(), 1);
    assert_eq!(
        created[0].height, 5.0,
        "the session height is the glyph height"
    );
}

#[test]
fn mtext_refuses_a_zero_width_rectangle_without_writing() {
    let mut controller = fixture();
    let before = entity_count(&controller);
    let revision = controller.drawing().unwrap().revision();

    controller.begin_draw_tool(DrawToolKind::MText).unwrap();
    controller.capture_tool_point(p(10.0, 10.0)).unwrap();
    controller.capture_tool_point(p(10.0, 50.0)).unwrap();
    controller.session.set_tool_text("ZERO").unwrap();

    let error = controller
        .confirm_draw_traced()
        .expect_err("a zero-width rectangle is refused");
    let message = error.to_string();
    assert!(message.contains("tool.mtext_zero_rect"), "{message}");

    let drawing = controller.drawing().unwrap();
    assert_eq!(drawing.revision(), revision, "no transaction ran");
    assert_eq!(entity_count(&controller), before, "no entity was written");
    assert!(
        controller.session.draw_preview().is_some(),
        "the refused capture is kept so the user can correct it"
    );
}

#[test]
fn mtext_refuses_a_zero_height_rectangle_too() {
    let mut controller = fixture();
    let before = entity_count(&controller);
    let error = capture_and_confirm(&mut controller, p(10.0, 10.0), p(50.0, 10.0), "FLAT")
        .expect_err("a zero-height rectangle is refused");
    assert!(
        error.to_string().contains("tool.mtext_zero_rect"),
        "{error}"
    );
    assert_eq!(entity_count(&controller), before);
}

#[test]
fn mtext_refuses_an_off_plane_corner_pair_without_writing() {
    let mut controller = fixture();
    let before = entity_count(&controller);
    let error = capture_and_confirm(
        &mut controller,
        p(0.0, 0.0),
        Point3 {
            x: 40.0,
            y: 20.0,
            z: 5.0,
        },
        "PLANE",
    )
    .expect_err("an off-plane pair is refused");
    assert!(error.to_string().contains("one +Z plane"), "{error}");
    assert_eq!(entity_count(&controller), before, "no entity was written");
}

#[test]
fn mtext_refuses_a_non_finite_corner_at_the_intent_mapping() {
    // The capture layer refuses a non-finite point, so the intent mapping's own
    // guard is exercised directly (it is the same rule RECTANG applies).
    let controller = fixture();
    let error = controller
        .command_for_draw_intent(
            DrawIntent::MText {
                first: Point3 {
                    x: f64::NAN,
                    y: 0.0,
                    z: 0.0,
                },
                opposite: p(10.0, 10.0),
                text: "BAD".into(),
            },
            &[],
        )
        .err()
        .expect("a non-finite corner cannot become a payload");
    let message = match error {
        CadError::InvalidInput(message) => message,
        other => panic!("expected an InvalidInput refusal, got {other:?}"),
    };
    assert!(message.contains("tool.mtext_zero_rect"), "{message}");
    assert!(message.contains("finite"), "{message}");
}

#[test]
fn mtext_confirm_is_refused_while_the_text_prompt_is_open() {
    let mut controller = fixture();
    let before = entity_count(&controller);
    controller.begin_draw_tool(DrawToolKind::MText).unwrap();
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();
    controller.capture_tool_point(p(30.0, 15.0)).unwrap();
    // No text yet: the tool's own intent refuses it by name.
    let error = controller
        .confirm_draw_traced()
        .expect_err("no text, no commit");
    assert!(error.to_string().contains("tool.text_required"), "{error}");
    assert_eq!(entity_count(&controller), before, "nothing was written");
    assert_eq!(
        controller
            .session
            .draw_preview()
            .expect("still active")
            .kind,
        DrawToolKind::MText
    );
}

#[test]
fn mtext_rotation_comes_from_the_text_rotation_parameter_in_radians() {
    // The channel carries degrees (AutoCAD's prompt unit) and the host converts
    // once into the radians the domain stores: 30° is π/6.
    let mut controller = fixture();
    controller
        .session
        .set_tool_parameter(ToolParameter::TextRotation, ToolValue::Number(30.0))
        .expect("30 degrees is finite");
    let traced = capture_and_confirm(&mut controller, p(0.0, 0.0), p(40.0, 20.0), "ROT")
        .expect("the create runs");
    assert!(traced.result.is_ok(), "{:?}", traced.result);
    let created = mtexts(&controller);
    assert_eq!(created.len(), 1);
    let stored = created[0].rotation;
    assert_eq!(
        stored,
        30.0_f64.to_radians(),
        "the channel's degrees are converted once with `.to_radians()`"
    );
    assert!(
        (stored - std::f64::consts::FRAC_PI_6).abs() < 1e-15,
        "30 degrees is π/6 radians, got {stored}"
    );
    // The default attachment is untouched by a rotation change.
    assert_eq!(created[0].h_align, TextAlignH::Left);
    assert_eq!(created[0].v_align, TextAlignV::Top);
}

#[test]
fn mtext_stores_a_negative_rotation_verbatim_without_wrapping() {
    let mut controller = fixture();
    controller
        .session
        .set_tool_parameter(ToolParameter::TextRotation, ToolValue::Number(-30.0))
        .expect("-30 degrees is finite");
    let traced = capture_and_confirm(&mut controller, p(0.0, 0.0), p(40.0, 20.0), "NEG")
        .expect("the create runs");
    assert!(traced.result.is_ok(), "{:?}", traced.result);
    assert_eq!(
        mtexts(&controller)[0].rotation,
        -30.0_f64.to_radians(),
        "no wrap and no clamp: a negative angle stays negative"
    );
}

#[test]
fn mtext_attachment_parameter_maps_the_nine_codes_onto_the_domain_grid() {
    use TextAlignH::{Center, Left, Right};
    use TextAlignV::{Bottom, Middle, Top};
    for (code, h_align, v_align) in [
        (1.0, Left, Top),
        (2.0, Center, Top),
        (3.0, Right, Top),
        (4.0, Left, Middle),
        (5.0, Center, Middle),
        (6.0, Right, Middle),
        (7.0, Left, Bottom),
        (8.0, Center, Bottom),
        (9.0, Right, Bottom),
    ] {
        let mut controller = fixture();
        controller
            .session
            .set_tool_parameter(ToolParameter::MTextAttachment, ToolValue::Number(code))
            .expect("a code in the closed set is accepted");
        let traced = capture_and_confirm(&mut controller, p(0.0, 0.0), p(40.0, 20.0), "ATT")
            .expect("the create runs");
        assert!(traced.result.is_ok(), "{code}: {:?}", traced.result);
        let created = mtexts(&controller);
        assert_eq!(created.len(), 1, "{code}: one entity");
        assert_eq!(
            (created[0].h_align, created[0].v_align),
            (h_align, v_align),
            "attachment code {code}"
        );
        assert_ne!(
            created[0].v_align,
            TextAlignV::Baseline,
            "{code}: no attachment code names a baseline"
        );
    }
}

#[test]
fn mtext_refuses_an_attachment_code_outside_the_closed_set_instead_of_guessing() {
    // The channel refuses 0/10/1.5/NaN, so this forced store value reaches the
    // intent mapping's own guard: it must refuse by name and write nothing.
    for refused in [0.0, 10.0, 1.5, f64::NAN] {
        let mut controller = fixture();
        let before = entity_count(&controller);
        controller
            .session
            .force_tool_parameter(ToolParameter::MTextAttachment, ToolValue::Number(refused));
        let error = controller
            .command_for_draw_intent(
                DrawIntent::MText {
                    first: p(0.0, 0.0),
                    opposite: p(40.0, 20.0),
                    text: "BAD".into(),
                },
                &[],
            )
            .err()
            .expect("an out-of-set attachment cannot become a payload");
        let message = match error {
            CadError::InvalidInput(message) => message,
            other => panic!("expected an InvalidInput refusal, got {other:?}"),
        };
        assert!(message.contains("mtextAttachment"), "{message}");
        assert!(message.contains("between 1 and 9"), "{message}");
        assert_eq!(entity_count(&controller), before, "{refused}: no write");
    }
}

#[test]
fn mtext_refuses_a_forced_non_finite_rotation_at_the_intent_mapping() {
    let mut controller = fixture();
    controller
        .session
        .force_tool_parameter(ToolParameter::TextRotation, ToolValue::Number(f64::NAN));
    let error = controller
        .command_for_draw_intent(
            DrawIntent::MText {
                first: p(0.0, 0.0),
                opposite: p(40.0, 20.0),
                text: "BAD".into(),
            },
            &[],
        )
        .err()
        .expect("a non-finite rotation cannot become a payload");
    assert!(error.to_string().contains("textRotation"), "{error}");
}
