//! The `cad_set_selected_property` admission contracts.
//!
//! The IPC command itself only takes a lock and forwards two strings; everything
//! that needs live state (the current selection, the drawing's tables, the value
//! grammar) is the host session's authority on drain. These tests therefore pin
//! the two things this layer owns: the stable-code wire parsing and the gates
//! (modal popup, probe mode, bound).

use super::super::commands::{
    admit_selected_property, parse_property_write, CAD_SELECTED_PROPERTY_CAP,
};
use cad_ui_core::shell::POPUP_MODAL_OPEN;

#[test]
fn the_property_wire_parses_the_five_supported_keys_and_refuses_everything_else() {
    for key in ["layer", "color", "linetype", "lineweight", "transparency"] {
        assert_eq!(
            parse_property_write(key, "ByLayer").expect("a supported key"),
            (key.to_string(), "ByLayer".to_string())
        );
    }
    // `text` has its own command, every other key has no write path: the refusal
    // is the stable code only (no human-readable message on this wire).
    for key in ["text", "id", "type", "space", "", "COLOR", " layer"] {
        assert_eq!(
            parse_property_write(key, "ByLayer").unwrap_err(),
            "property.unsupported_key",
            "'{key}'"
        );
    }
    // The value is carried verbatim: its grammar is validated against the
    // drawing's tables on the owner thread, never guessed here.
    assert_eq!(
        parse_property_write("layer", "  WALLS  ").expect("the key is supported"),
        ("layer".to_string(), "  WALLS  ".to_string())
    );
}

#[test]
fn property_admission_is_gated_bounded_and_carries_the_pair_verbatim() {
    let mut queue: Vec<(String, String)> = Vec::new();

    // The modal popup gate comes first, like every other CAD mutation: while a
    // popup owns input the write is refused by name.
    assert_eq!(
        admit_selected_property(&mut queue, true, true, "layer".to_string(), "0".to_string())
            .unwrap_err(),
        POPUP_MODAL_OPEN
    );
    assert!(queue.is_empty(), "a gated refusal queues nothing");

    // Probe mode (no CAD session) is refused explicitly.
    assert_eq!(
        admit_selected_property(
            &mut queue,
            false,
            false,
            "layer".to_string(),
            "0".to_string()
        )
        .unwrap_err(),
        "CAD is disabled in probe mode"
    );
    assert!(queue.is_empty(), "a probe-mode refusal queues nothing");

    // Admitted verbatim: no entity id, no trimming, no clamping.
    admit_selected_property(
        &mut queue,
        true,
        false,
        "transparency".to_string(),
        "60".to_string(),
    )
    .expect("admitted");
    assert_eq!(queue, vec![("transparency".to_string(), "60".to_string())]);

    // Bounded: filling to capacity then one more is an explicit refusal.
    queue.clear();
    for _ in 0..CAD_SELECTED_PROPERTY_CAP {
        admit_selected_property(
            &mut queue,
            true,
            false,
            "layer".to_string(),
            "0".to_string(),
        )
        .expect("within capacity");
    }
    assert_eq!(queue.len(), CAD_SELECTED_PROPERTY_CAP);
    assert_eq!(
        admit_selected_property(
            &mut queue,
            true,
            false,
            "layer".to_string(),
            "0".to_string()
        )
        .unwrap_err(),
        "CAD selected-property queue is full"
    );
}
