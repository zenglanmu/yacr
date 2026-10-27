//! The clipboard tool cluster's pure capture tests (`COPYBASE`/`CUTBASE`/
//! `PASTECLIP`).
//!
//! Split out of `draw_tool/tests.rs` to keep that file under the code-size limit.
//! The cluster's capture shape is small but its identity matters: `COPYBASE` and
//! `CUTBASE` share one intent (the command decides whether the captured refs are
//! also deleted), and `PASTECLIP` is the only paste that captures a point at all.

use super::*;

#[test]
fn the_clipboard_kinds_declare_their_arity_and_selection_requirement() {
    // COPYBASE/CUTBASE capture one base point over the selection (a
    // selection-less capture would be an empty success); PASTECLIP captures one
    // insertion point and takes its input from the clipboard, so it needs no
    // selection. None of the three picks an entity step.
    assert_eq!(DrawToolKind::CopyBase.arity(), PointArity::Exact(1));
    assert_eq!(DrawToolKind::CutBase.arity(), PointArity::Exact(1));
    assert_eq!(DrawToolKind::PasteClip.arity(), PointArity::Exact(1));
    assert!(DrawToolKind::CopyBase.requires_selection());
    assert!(DrawToolKind::CutBase.requires_selection());
    assert!(!DrawToolKind::PasteClip.requires_selection());
    for kind in [
        DrawToolKind::CopyBase,
        DrawToolKind::CutBase,
        DrawToolKind::PasteClip,
    ] {
        assert_eq!(kind.picked_step_count(), 0, "{kind:?}");
        assert!(!kind.needs_picked_entities(), "{kind:?}");
        assert!(!kind.arity().is_open_ended(), "{kind:?}");
    }
}

#[test]
fn clipboard_cluster_captures_one_point_and_is_ready_only_when_usable() {
    // COPYBASE: selection snapshot + one base point; the intent is the same shape
    // CUTBASE produces (the command decides whether the refs are also deleted).
    let mut copy_base = DrawTool::new(DrawToolKind::CopyBase, 1);
    assert_eq!(copy_base.remaining(), Some(1));
    assert!(!copy_base.is_ready());
    copy_base.push_point(p(2.0, 3.0)).unwrap();
    assert!(copy_base.is_ready());
    assert_eq!(
        copy_base.intent().unwrap(),
        DrawIntent::CopyBase { base: p(2.0, 3.0) }
    );
    // Without a selection the capture is not confirmable at all.
    let mut no_selection = DrawTool::new(DrawToolKind::CopyBase, 0);
    no_selection.push_point(p(2.0, 3.0)).unwrap();
    assert!(matches!(
        no_selection.intent(),
        Err(CadError::InvalidInput(_))
    ));
    // CUTBASE is the same capture shape.
    let mut cut_base = DrawTool::new(DrawToolKind::CutBase, 1);
    cut_base.push_point(p(2.0, 3.0)).unwrap();
    assert_eq!(
        cut_base.intent().unwrap(),
        DrawIntent::CopyBase { base: p(2.0, 3.0) }
    );
    // PASTECLIP: one insertion point, no selection requirement.
    let mut paste = DrawTool::new(DrawToolKind::PasteClip, 0);
    assert!(!paste.is_ready(), "no insertion point captured yet");
    paste.push_point(p(7.0, -1.0)).unwrap();
    assert!(paste.is_ready());
    assert_eq!(
        paste.intent().unwrap(),
        DrawIntent::PasteClip {
            insertion: p(7.0, -1.0)
        }
    );
}
