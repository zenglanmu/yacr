//! Pure contract for the id pass's `FLAG_HIDDEN` suppression (no GPU): the
//! predicate must mirror the colour pass's in-shader discard exactly,
//! including the `flags_ctl.enabled == 0` short-circuit.

use super::*;
use crate::BackendPreference;

/// A renderer whose CPU-side flag mirror is set directly, so the predicate is
/// testable without a device.
fn renderer(flags_enabled: bool, bits: &[(u32, u32)]) -> Renderer {
    let mut renderer = Renderer::new(BackendPreference::Auto);
    renderer.set_selection_flags_enabled(flags_enabled);
    let len = bits
        .iter()
        .map(|(key, _)| *key as usize + 1)
        .max()
        .unwrap_or(0);
    renderer.flags_bits = vec![0; len];
    for (key, value) in bits {
        renderer.flags_bits[*key as usize] = *value;
    }
    renderer
}

#[test]
fn hidden_suppression_matrix_matches_the_colour_shader() {
    let hidden = cad_scene::FLAG_HIDDEN;
    let selected = cad_scene::FLAG_SELECTED;
    let hovered = cad_scene::FLAG_HOVERED;
    let cases = [
        (true, hidden, true, "enabled + HIDDEN"),
        (
            true,
            hidden | selected,
            true,
            "enabled + HIDDEN|SELECTED (inclusion, not equality)",
        ),
        (
            true,
            selected,
            false,
            "enabled + SELECTED must stay pickable",
        ),
        (true, hovered, false, "enabled + HOVERED must stay pickable"),
        (true, 0, false, "enabled + no bits must stay pickable"),
        (
            false,
            hidden,
            false,
            "with the toggle off a HIDDEN bit hides nothing (shader short-circuit)",
        ),
    ];
    for (enabled, bits, expected, what) in cases {
        let renderer = renderer(enabled, &[(1, bits)]);
        assert_eq!(
            renderer.draw_suppressed_by_flags(1),
            expected,
            "{what} (enabled={enabled}, bits={bits:#x})"
        );
    }
}

#[test]
fn keys_without_recorded_bits_are_never_suppressed() {
    let renderer = renderer(true, &[(1, cad_scene::FLAG_HIDDEN)]);
    assert!(
        !renderer.draw_suppressed_by_flags(0),
        "the background key (0) carries no entity"
    );
    assert!(
        !renderer.draw_suppressed_by_flags(99),
        "a key beyond the recorded mirror has no bits and must stay pickable"
    );
    assert_eq!(
        renderer.pick_key_flags(99),
        0,
        "the mirror reads 0 for an unrecorded key"
    );
}
