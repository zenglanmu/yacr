//! `GRADIENT` (AutoCAD 2026) vocabulary: a canonical Work-only selector.
//!
//! GRADIENT is its own selector (not an alias of HATCH) because the fill is
//! forced to a gradient and the boundary must be in the +Z plane. It takes no
//! payload and no argument; the current selection is the input.

use super::super::super::*;
use super::super::test_support::*;

#[test]
fn gradient_is_a_canonical_work_only_selector() {
    assert_eq!(normalize_command("gradient"), Some(CommandId::Gradient));
    assert_eq!(command_name(CommandId::Gradient), Some("GRADIENT"));
    assert!(CommandId::Gradient.requires_work_mode());
    assert_eq!(
        typed_command_argument("GRADIENT 1"),
        None,
        "GRADIENT takes no argument"
    );

    // A bare GRADIENT is admitted by name and reaches the application, which
    // refuses the absent selection (`gradient.selection`).
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let error = session.command("GRADIENT").unwrap_err();
    assert!(error.contains("gradient.selection"), "{error}");
}
