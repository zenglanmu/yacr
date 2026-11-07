//! Inquiry selectors are read-only and Viewer-legal, split out of
//! `command_vocabulary.rs` to keep that file inside the code-size gate.

use super::super::super::*;

#[test]
fn inquiry_commands_are_read_only_and_viewer_legal() {
    // LIST/STATUS/FS are report-only: no database write, revision bump or undo
    // entry. FS mutates only the session selection. All three therefore stay out
    // of `requires_work_mode` and are admitted in Viewer and Work.
    for (selector, id) in [
        ("LIST", CommandId::List),
        ("STATUS", CommandId::Status),
        ("FS", CommandId::Fs),
        // MASSPROP (AutoCAD 2026) reports selected closed planar regions.
        ("MASSPROP", CommandId::MassProp),
        // MEASUREGEOM's RADIUS/report paths are read-only; the tool options start
        // the existing measurement requests and write nothing either.
        ("MEASUREGEOM", CommandId::MeasureGeom),
    ] {
        assert_eq!(normalize_command(selector), Some(id), "{selector}");
        assert_eq!(canonical_command_name(selector), Some(selector));
        assert!(admissible_names().contains(&selector), "{selector}");
        assert!(
            !id.requires_work_mode(),
            "{selector} is read-only and must be Viewer-legal"
        );
    }
}
