//! TEXT text channel: selector routing, `awaiting_text`, terminal submission and
//! the named refusal surface (ADR 0019 decisions A.1/A.2, P7a).

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

/// Click at a canvas-logical pixel (a press + its release), the shape a real DOM
/// click produces.
fn click(session: &CadSession, pixel: [f64; 2], sequence: u64) {
    session
        .handle_input(&pointer_at(InputKind::PointerDown, sequence, pixel, None))
        .expect("the press is admitted");
    session
        .handle_input(&pointer_at(InputKind::PointerUp, sequence + 1, pixel, None))
        .expect("the release is admitted");
}

fn text_entities(session: &CadSession) -> Vec<(String, Point3)> {
    let controller = session.core.controller.borrow();
    let drawing = controller.drawing().expect("a document");
    let entities = drawing.model_space();
    entities
        .iter()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Text { text, position, .. } => Some((text.clone(), *position)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_text_selector_starts_the_text_kind_with_the_documented_height() {
    let session = fresh_session();
    session.command("TEXT").expect("TEXT is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("TEXT starts a tool");
    assert_eq!(tool.key, "draw.text");
    assert_eq!(tool.points, 0);
    assert!(!tool.ready, "no insertion point yet");
    assert!(
        !tool.awaiting_text,
        "the prompt opens only once the point is complete"
    );
    assert_eq!(
        tool.params.text_height,
        Some(cad_app::draw_tool::TEXT_DEFAULT_HEIGHT),
        "the session channel carries the documented default"
    );
}

#[test]
fn awaiting_text_is_projected_and_confirm_is_refused_by_name() {
    let session = fresh_session();
    session.command("TEXT").expect("TEXT is admitted");
    click(&session, [400.0, 300.0], 1);
    let tool = session.snapshot(None, None, 0).tool.expect("the tool");
    assert_eq!(tool.points, 1);
    assert!(
        tool.awaiting_text,
        "one point completes the complement, so the prompt is open"
    );
    assert!(
        !tool.ready,
        "`ready` is false because nothing can be confirmed until the text arrives"
    );

    // `Confirm` while awaiting text must never complete the command.
    let error = session.command("CONFIRM").unwrap_err();
    assert!(error.contains("tool.awaiting_text"), "{error}");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("the refused confirm keeps the tool");
    assert!(tool.awaiting_text);
    assert!(text_entities(&session).is_empty(), "nothing was written");

    // The first Esc only dismisses focus: the tool stays open and awaiting.
    session
        .dismiss_text_focus()
        .expect("the first Esc is legal");
    let tool = session.snapshot(None, None, 0).tool.expect("still active");
    assert!(tool.awaiting_text, "the tool still awaits its text");

    // The second Esc is the ordinary CANCEL and resets the tool (no transaction).
    session.command("CANCEL").expect("CANCEL");
    assert!(session.snapshot(None, None, 0).tool.is_none());
    assert!(text_entities(&session).is_empty(), "cancel writes nothing");
}

#[test]
fn set_tool_text_is_terminal_and_completes_without_a_second_confirm() {
    let session = fresh_session();
    let before = {
        let controller = session.core.controller.borrow();
        controller.drawing().expect("a document").entity_count()
    };
    session.command("TEXT").expect("TEXT is admitted");
    click(&session, [400.0, 300.0], 1);
    session
        .set_tool_text("Hello")
        .expect("the text is accepted");

    let tool = session.snapshot(None, None, 0).tool;
    assert!(
        tool.is_none(),
        "the committed text completes the tool immediately"
    );
    let texts = text_entities(&session);
    assert_eq!(texts.len(), 1, "exactly one TEXT entity");
    assert_eq!(texts[0].0, "Hello");
    let controller = session.core.controller.borrow();
    let drawing = controller.drawing().expect("a document");
    assert_eq!(
        drawing.entity_count(),
        before + 1,
        "one entity, not a second one"
    );
    assert!(controller.application.can_undo(&controller.document_id));
}

#[test]
fn the_text_refusals_are_named_and_write_nothing() {
    // No tool at all.
    let session = fresh_session();
    let error = session.set_tool_text("Hello").unwrap_err();
    assert!(error.contains("tool.text_not_awaiting"), "{error}");

    // TEXT active but no insertion point yet.
    let session = fresh_session();
    session.command("TEXT").expect("TEXT is admitted");
    let error = session.set_tool_text("Hello").unwrap_err();
    assert!(error.contains("tool.text_points_incomplete"), "{error}");

    // Empty / whitespace-only and over the 1024-character bound.
    for (text, expected) in [
        ("".to_string(), "tool.text_empty"),
        ("   ".to_string(), "tool.text_empty"),
        (
            "a".repeat(cad_app::MAX_TOOL_TEXT_CHARS + 1),
            "tool.text_too_long",
        ),
    ] {
        let session = fresh_session();
        session.command("TEXT").expect("TEXT is admitted");
        click(&session, [400.0, 300.0], 1);
        let error = session.set_tool_text(&text).unwrap_err();
        assert!(error.contains(expected), "expected {expected}, got {error}");
        // The refused text left the prompt open and wrote nothing.
        let tool = session.snapshot(None, None, 0).tool.expect("still active");
        assert!(tool.awaiting_text, "{expected}: the prompt stays open");
        assert!(text_entities(&session).is_empty(), "{expected}: no write");
    }

    // A non-text tool refuses text.
    let session = fresh_session();
    session.command("LINE").expect("LINE is admitted");
    let error = session.set_tool_text("Hello").unwrap_err();
    assert!(error.contains("tool.text_not_awaiting"), "{error}");
}

#[test]
fn the_text_height_parameter_is_present_only_for_the_text_tools() {
    let session = fresh_session();
    session.command("TEXT").expect("TEXT is admitted");
    assert_eq!(
        session
            .snapshot(None, None, 0)
            .tool
            .expect("the tool")
            .params
            .text_height,
        Some(2.5)
    );
    session
        .set_tool_parameter(ToolParameter::TextHeight, ToolValue::Number(5.0))
        .expect("a positive height is accepted");
    assert_eq!(
        session
            .snapshot(None, None, 0)
            .tool
            .expect("the tool")
            .params
            .text_height,
        Some(5.0)
    );
    // MTEXT creates its glyph with the same parameter, so it projects it too.
    session.command("MTEXT").expect("MTEXT is admitted");
    let tool = session.snapshot(None, None, 0).tool.expect("the tool");
    assert_eq!(tool.key, "draw.mtext");
    assert_eq!(
        tool.params.text_height,
        Some(5.0),
        "the shared channel keeps the value across the tool switch"
    );
    // A non-text tool reports `null`, never a stale height.
    session.command("LINE").expect("LINE is admitted");
    assert_eq!(
        session
            .snapshot(None, None, 0)
            .tool
            .expect("the tool")
            .params
            .text_height,
        None
    );
    // The channel refuses non-positive / non-finite values explicitly.
    assert!(session
        .set_tool_parameter(ToolParameter::TextHeight, ToolValue::Number(0.0))
        .is_err());
}

/// The session's created MTEXT entities: stored string, insertion point and the
/// stored wrap width (only `AcDbMText` entities are considered, so a TEXT with
/// the same `Text` geometry is never counted).
fn mtext_entities(session: &CadSession) -> Vec<(String, Point3, Option<f64>)> {
    let controller = session.core.controller.borrow();
    let drawing = controller.drawing().expect("a document");
    drawing
        .model_space()
        .iter()
        .filter(|entity| entity.object.type_key == "AcDbMText")
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Text {
                text,
                position,
                wrap_width,
                ..
            } => Some((text.clone(), *position, *wrap_width)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_mtext_selector_starts_the_two_corner_tool_with_the_documented_height() {
    let session = fresh_session();
    session.command("MTEXT").expect("MTEXT is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("MTEXT starts a tool");
    assert_eq!(tool.key, "draw.mtext");
    assert_eq!(tool.points, 0, "no corner captured yet");
    assert!(!tool.ready);
    assert!(
        !tool.awaiting_text,
        "the text prompt opens only once both corners are captured"
    );
    assert_eq!(
        tool.params.text_height,
        Some(cad_app::draw_tool::TEXT_DEFAULT_HEIGHT),
        "the same documented default TEXT uses"
    );
}

#[test]
fn mtext_two_corners_then_text_commit_one_acdbmtext() {
    let session = fresh_session();
    let before = {
        let controller = session.core.controller.borrow();
        controller.drawing().expect("a document").entity_count()
    };
    session.command("MTEXT").expect("MTEXT is admitted");
    click(&session, [400.0, 300.0], 1);
    let tool = session.snapshot(None, None, 0).tool.expect("the tool");
    assert_eq!(tool.points, 1);
    assert!(
        !tool.awaiting_text,
        "one corner is not the point complement"
    );

    click(&session, [520.0, 360.0], 3);
    let tool = session.snapshot(None, None, 0).tool.expect("the tool");
    assert_eq!(tool.points, 2);
    assert!(tool.awaiting_text, "the terminal text step is open");
    assert!(
        !tool.ready,
        "`ready` is false because nothing can be confirmed until the text arrives"
    );

    // `Confirm` while awaiting text must never complete the command.
    let error = session.command("CONFIRM").unwrap_err();
    assert!(error.contains("tool.awaiting_text"), "{error}");
    assert!(mtext_entities(&session).is_empty(), "nothing was written");

    // The same terminal text channel TEXT uses; MTEXT accepts a line break.
    session.set_tool_text("A\nB").expect("the text is accepted");
    assert!(
        session.snapshot(None, None, 0).tool.is_none(),
        "the committed text completes the tool immediately"
    );
    let created = mtext_entities(&session);
    assert_eq!(created.len(), 1, "exactly one MTEXT entity");
    let (text, _position, width) = created[0].clone();
    assert_eq!(text, "A\\PB", "the one shared newline policy stores `\\P`");
    let width = width.expect("the two-corner model always stores a width");
    assert!(width.is_finite() && width > 0.0, "{width}");
    let controller = session.core.controller.borrow();
    let drawing = controller.drawing().expect("a document");
    assert_eq!(
        drawing.entity_count(),
        before + 1,
        "one entity, not a second"
    );
    assert!(controller.application.can_undo(&controller.document_id));
}

#[test]
fn mtext_refuses_a_zero_width_rectangle_without_writing() {
    // Two clicks on the same canvas column produce a zero horizontal span: the
    // width is never fabricated, so the host refuses explicitly and the capture
    // stays active for a corrected second corner.
    let session = fresh_session();
    let before = {
        let controller = session.core.controller.borrow();
        (
            controller.drawing().expect("a document").entity_count(),
            controller.drawing().expect("a document").revision(),
        )
    };
    session.command("MTEXT").expect("MTEXT is admitted");
    click(&session, [400.0, 300.0], 1);
    click(&session, [400.0, 360.0], 3);
    // The text step is **terminal**: submitting it commits immediately, so the
    // zero-width refusal surfaces from `cad_set_tool_text` itself.
    let error = session.set_tool_text("ZERO").unwrap_err();
    assert!(error.contains("tool.mtext_zero_rect"), "{error}");
    let controller = session.core.controller.borrow();
    let drawing = controller.drawing().expect("a document");
    assert_eq!((drawing.entity_count(), drawing.revision()), before);
    assert!(!controller.application.can_undo(&controller.document_id));
    drop(controller);
    // The capture is kept so the user can correct the opposite corner.
    assert_eq!(
        session
            .snapshot(None, None, 0)
            .tool
            .expect("still active")
            .key,
        "draw.mtext"
    );
}

/// The single selected `Text` of the demo session, once the test created one.
fn the_created_text(session: &CadSession) -> SelectionRef {
    let controller = session.core.controller.borrow();
    let drawing = controller.drawing().expect("a document");
    let entity = drawing
        .model_space()
        .iter()
        .find(|entity| matches!(entity.geometry, SemanticGeometry::Text { .. }))
        .map(|entity| entity.id)
        .expect("a text entity");
    SelectionRef {
        document: controller.document_id,
        entity,
        instance: InstancePath::default(),
        sub_element: None,
    }
}

#[test]
fn textedit_selector_starts_the_edit_tool_with_no_prefill() {
    let session = fresh_session();
    session.command("TEXTEDIT").expect("TEXTEDIT is admitted");
    let tool = session.snapshot(None, None, 0).tool.expect("the tool");
    assert_eq!(tool.key, "draw.textedit");
    assert_eq!(tool.points, 0);
    assert!(!tool.awaiting_text);
    assert_eq!(
        tool.text_prefill, None,
        "nothing is prefilled before the entity pick resolves"
    );
}

#[test]
fn the_panel_channel_reports_the_round_trippable_text_and_edits_the_selection() {
    let session = fresh_session();
    // Create the entity under test through the ordinary TEXT channel.
    session.command("TEXT").expect("TEXT is admitted");
    click(&session, [400.0, 300.0], 1);
    session.set_tool_text("OLD").expect("the text");
    let reference = the_created_text(&session);
    session
        .core
        .controller
        .borrow_mut()
        .set_selection(vec![reference])
        .expect("the selection");

    // `selection.text` is the round-trippable string, not a display row.
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.selection.count, 1);
    assert_eq!(status.selection.text.as_deref(), Some("OLD"));

    // The panel channel replaces it and the next status carries the new string.
    session.set_selected_text("NEW").expect("the edit");
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.selection.text.as_deref(), Some("NEW"));
    let controller = session.core.controller.borrow();
    let drawing = controller.drawing().expect("a document");
    let texts: Vec<String> = drawing
        .model_space()
        .iter()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["NEW".to_string()], "one edited entity");
}

#[test]
fn the_panel_channel_refuses_an_empty_or_multi_selection_by_name() {
    // Nothing selected.
    let session = fresh_session();
    let error = session.set_selected_text("X").unwrap_err();
    assert!(error.contains("textedit.selection_empty"), "{error}");

    // Two selected entities (the created text plus an existing demo entity).
    let session = fresh_session();
    session.command("TEXT").expect("TEXT is admitted");
    click(&session, [400.0, 300.0], 1);
    session.set_tool_text("OLD").expect("the text");
    let text = the_created_text(&session);
    let other = {
        let controller = session.core.controller.borrow();
        let drawing = controller.drawing().expect("a document");
        drawing
            .model_space()
            .iter()
            .find(|entity| entity.id != text.entity)
            .map(|entity| SelectionRef {
                document: controller.document_id,
                entity: entity.id,
                instance: InstancePath::default(),
                sub_element: None,
            })
            .expect("another entity")
    };
    session
        .core
        .controller
        .borrow_mut()
        .set_selection(vec![text, other])
        .expect("the selection");
    let error = session.set_selected_text("X").unwrap_err();
    assert!(error.contains("textedit.selection_multiple"), "{error}");
}

#[test]
fn selected_text_edit_reports_only_a_single_editable_text_selection() {
    let session = fresh_session();
    // Nothing selected: no popup payload (the popup host names the refusal).
    assert_eq!(
        session.selected_text_edit().expect("no document error"),
        None
    );

    session.command("TEXT").expect("TEXT is admitted");
    click(&session, [400.0, 300.0], 1);
    session.set_tool_text("OLD").expect("the text");
    let reference = the_created_text(&session);
    session
        .core
        .controller
        .borrow_mut()
        .set_selection(vec![reference])
        .expect("the selection");

    // The host-derived payload: the raw string plus the source class's
    // multiline flag (TEXT is single-line).
    assert_eq!(
        session.selected_text_edit().expect("the selection reads"),
        Some(("OLD".to_string(), false))
    );
}

#[test]
fn the_status_selection_block_projects_text_only_for_a_single_text() {
    let row = |text: Option<&str>, multiline: bool, count: usize| {
        let selection = CadSelection::from_properties(&cad_app::SelectionProperties {
            count,
            rows: Vec::new(),
            mixed_keys: Vec::new(),
            text: text.map(str::to_string),
            text_multiline: multiline,
            empty: count == 0,
        });
        (selection.text, selection.text_multiline)
    };
    assert_eq!(row(Some("Hello"), false, 1), (Some("Hello".into()), false));
    assert_eq!(row(Some("A\nB"), true, 1), (Some("A\nB".into()), true));
    // A count mismatch (multi-selection) never carries a single object's text,
    // so it never carries a text capability either — even a stale flag is dropped.
    assert_eq!(row(Some("A\nB"), true, 2), (None, false));
    assert_eq!(row(None, false, 1), (None, false));
    assert_eq!(row(None, false, 0), (None, false));
    // `text: null` always forces `text_multiline: false`.
    assert_eq!(row(None, true, 1), (None, false));
}

/// Collect the top-level wire field names a `Serialize` impl emits.
///
/// `cad-ui-core` deliberately carries no `serde_json`; this recorder only needs
/// the struct's field names (each value is skipped), which is exactly what
/// proves that a `serde(rename)` reached the wire rather than the camelCase
/// convention silently winning.
pub(super) fn wire_field_names<T: serde::Serialize>(value: &T) -> Vec<String> {
    use serde::ser::{Impossible, SerializeStruct, Serializer};

    #[derive(Debug)]
    struct WireError;

    impl std::fmt::Display for WireError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("wire field recorder: unsupported value")
        }
    }
    impl std::error::Error for WireError {}
    impl serde::ser::Error for WireError {
        fn custom<T: std::fmt::Display>(_message: T) -> Self {
            WireError
        }
    }

    struct Recorder(Vec<String>);
    struct Fields<'a>(&'a mut Recorder);

    impl SerializeStruct for Fields<'_> {
        type Ok = ();
        type Error = WireError;

        fn serialize_field<T: ?Sized + serde::Serialize>(
            &mut self,
            key: &'static str,
            _value: &T,
        ) -> Result<(), WireError> {
            self.0 .0.push(key.to_string());
            Ok(())
        }

        fn end(self) -> Result<(), WireError> {
            Ok(())
        }
    }

    fn unsupported<T>() -> Result<T, WireError> {
        Err(WireError)
    }

    impl<'a> Serializer for &'a mut Recorder {
        type Ok = ();
        type Error = WireError;
        type SerializeSeq = Impossible<(), WireError>;
        type SerializeTuple = Impossible<(), WireError>;
        type SerializeTupleStruct = Impossible<(), WireError>;
        type SerializeTupleVariant = Impossible<(), WireError>;
        type SerializeMap = Impossible<(), WireError>;
        type SerializeStruct = Fields<'a>;
        type SerializeStructVariant = Impossible<(), WireError>;

        fn serialize_bool(self, _value: bool) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_i8(self, _value: i8) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_i16(self, _value: i16) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_i32(self, _value: i32) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_i64(self, _value: i64) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_i128(self, _value: i128) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_u8(self, _value: u8) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_u16(self, _value: u16) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_u32(self, _value: u32) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_u64(self, _value: u64) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_u128(self, _value: u128) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_f32(self, _value: f32) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_f64(self, _value: f64) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_char(self, _value: char) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_str(self, _value: &str) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_bytes(self, _value: &[u8]) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_none(self) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_some<T: ?Sized + serde::Serialize>(self, _value: &T) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_unit(self) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_unit_struct(self, _name: &'static str) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_unit_variant(
            self,
            _name: &'static str,
            _variant_index: u32,
            _variant: &'static str,
        ) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_newtype_struct<T: ?Sized + serde::Serialize>(
            self,
            _name: &'static str,
            _value: &T,
        ) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_newtype_variant<T: ?Sized + serde::Serialize>(
            self,
            _name: &'static str,
            _variant_index: u32,
            _variant: &'static str,
            _value: &T,
        ) -> Result<(), WireError> {
            unsupported()
        }
        fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, WireError> {
            unsupported()
        }
        fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, WireError> {
            unsupported()
        }
        fn serialize_tuple_struct(
            self,
            _name: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeTupleStruct, WireError> {
            unsupported()
        }
        fn serialize_tuple_variant(
            self,
            _name: &'static str,
            _variant_index: u32,
            _variant: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeTupleVariant, WireError> {
            unsupported()
        }
        fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, WireError> {
            unsupported()
        }
        fn serialize_struct(
            self,
            _name: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeStruct, WireError> {
            Ok(Fields(self))
        }
        fn serialize_struct_variant(
            self,
            _name: &'static str,
            _variant_index: u32,
            _variant: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeStructVariant, WireError> {
            unsupported()
        }
    }

    let mut recorder = Recorder(Vec::new());
    value
        .serialize(&mut recorder)
        .expect("the selection block serializes as a struct");
    recorder.0
}

#[test]
fn the_selection_wire_name_is_text_multiline_not_camel_case() {
    let selection = CadSelection::from_properties(&cad_app::SelectionProperties {
        count: 1,
        rows: Vec::new(),
        mixed_keys: Vec::new(),
        text: Some("Hello".into()),
        text_multiline: true,
        empty: false,
    });
    assert!(selection.text_multiline);

    let names = wire_field_names(&selection);
    assert!(
        names.iter().any(|name| name == "text_multiline"),
        "the wire name must stay snake_case: {names:?}"
    );
    assert!(
        !names.iter().any(|name| name == "textMultiline"),
        "a silent camelCase rename would make the panel misbehave: {names:?}"
    );
    // Every pre-existing key keeps its documented wire name.
    assert!(names.iter().any(|name| name == "text"), "{names:?}");
    assert!(names.iter().any(|name| name == "mixedKeys"), "{names:?}");

    // The honest empty block advertises the same key with `false`.
    let empty = CadSelection::empty();
    assert!(!empty.text_multiline);
    let names = wire_field_names(&empty);
    assert!(
        names.iter().any(|name| name == "text_multiline"),
        "{names:?}"
    );
    assert!(names.iter().any(|name| name == "text"), "{names:?}");
}
