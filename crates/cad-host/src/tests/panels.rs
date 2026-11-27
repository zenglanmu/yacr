//! Match Photo / Overlays host reads and their panel-action refusals.
//!
//! Match Photo is a capability read (`modeled: false`), Overlays projects the
//! renderer-owned channels; both actions are named refusals and write nothing.
use super::*;

#[test]
fn match_photo_reports_the_unmodeled_capability() {
    let core = core();
    let status = core.match_photo();
    assert!(!status.modeled);
    assert!(
        status.reason.contains("matchphoto.not_implemented"),
        "{}",
        status.reason
    );
    let revision = core.controller.borrow().drawing().unwrap().revision();
    let error = core
        .command(CommandId::MatchPhoto, CommandPayload::None)
        .expect_err("the action refuses");
    assert!(
        matches!(error, CadError::Unsupported(ref text) if text.contains("matchphoto.not_implemented")),
        "{error:?}"
    );
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision(),
        revision,
        "the refusal writes nothing"
    );
}

#[test]
fn overlays_lists_the_renderer_owned_channels() {
    let core = core();
    let list = core.overlays();
    let keys: Vec<&str> = list.rows.iter().map(|row| row.key).collect();
    assert_eq!(keys, crate::OVERLAY_ROW_KEYS.to_vec());
    let active = |key: &str| {
        list.rows
            .iter()
            .find(|row| row.key == key)
            .map(|row| row.active)
            .unwrap_or(false)
    };
    assert!(active("axes") && active("grid"));
    assert!(active("selectionHighlight") && active("snapHints"));
    assert!(!active("selectionBox"), "no gesture is running");
    assert!(!active("measurement") && !active("drawPreview"));

    let error = core
        .command(CommandId::Overlays, CommandPayload::None)
        .expect_err("the management action refuses");
    assert!(
        matches!(error, CadError::Unsupported(ref text) if text.contains("overlays.not_implemented")),
        "{error:?}"
    );
}
