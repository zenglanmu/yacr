//! The clipboard cluster's Work/Viewer boundary, split out of
//! `command_vocabulary.rs` to keep that file inside the code-size gate.

use super::super::super::*;

#[test]
fn clipboard_cluster_selectors_declare_their_required_mode() {
    // The clipboard cluster's Work-only boundary (AutoCAD-faithful):
    // * `COPYCLIP` and `COPYBASE` are read-only captures, so Viewer mode may run
    //   them;
    // * `CUTCLIP`/`CUTBASE` delete and both pastes write, so they are Work-only.
    // A Viewer that tried to cut or paste is refused by `SessionState::authorize`
    // before anything is captured or written.
    assert!(!CommandId::CopyToClipboard.requires_work_mode());
    assert!(!CommandId::CopyToClipboardBase.requires_work_mode());
    assert!(CommandId::CutToClipboard.requires_work_mode());
    assert!(CommandId::CutToClipboardBase.requires_work_mode());
    assert!(CommandId::PasteFromClipboard.requires_work_mode());
    assert!(CommandId::PasteOriginalCoordinates.requires_work_mode());
    assert!(CommandId::PasteBlock.requires_work_mode());

    // The three new selectors are real vocabulary entries, not aliases of an
    // existing command.
    for name in ["COPYBASE", "CUTBASE", "PASTEORIG"] {
        assert!(
            admissible_names().contains(&name),
            "{name} is not admissible"
        );
        assert_eq!(canonical_command_name(name), Some(name), "{name}");
        assert!(
            !COMMAND_ALIASES.iter().any(|(alias, _)| *alias == name),
            "{name} must be a canonical selector, not an alias"
        );
    }
}
