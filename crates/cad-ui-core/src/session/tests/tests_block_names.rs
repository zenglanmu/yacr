//! `INSERT <name>` / `BLOCK <name>` typed lines.
//!
//! The typed form supplies the block name and starts the same point-then-name
//! capture the bare selector starts; the capture applies the name after its
//! point. Contract level: no GUI, no GPU.

use super::super::*;
use super::test_support::*;
use crate::session::commands::parse_block_name_argument;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_block_name_grammar_is_one_trimmed_single_line_name() {
    assert_eq!(parse_block_name_argument("Widget"), Some("Widget".into()));
    assert_eq!(parse_block_name_argument("  Cabin  "), Some("Cabin".into()));
    for refused in ["", "   ", "A\nB", "A\rB"] {
        assert_eq!(parse_block_name_argument(refused), None, "{refused:?}");
    }
    assert_eq!(
        typed_command_argument("INSERT Widget"),
        Some(("INSERT", "Widget"))
    );
    assert_eq!(
        typed_command_argument("BLOCK Cabin"),
        Some(("BLOCK", "Cabin"))
    );
}

#[test]
fn insert_with_a_name_starts_the_insert_capture() {
    let session = fresh_session();
    session
        .command("INSERT Widget")
        .expect("INSERT <name> is admitted");
    let tool = session.snapshot(None, None, 0).tool.expect("a tool");
    assert_eq!(tool.key, "draw.insertblock");
    assert_eq!(tool.points, 0);
    assert!(!tool.ready);
}

#[test]
fn block_with_a_name_reaches_the_define_capture_and_names_its_selection_rule() {
    let session = fresh_session();
    // The typed line starts the same base-point capture the bare selector does;
    // the selection requirement is re-validated when the capture confirms (the
    // create command's own refusal).
    session
        .command("BLOCK Cabin")
        .expect("BLOCK <name> is admitted");
    let tool = session.snapshot(None, None, 0).tool.expect("a tool");
    assert_eq!(tool.key, "draw.blockdefine");
    // A trailing-space line is the bare selector (name-only funnel), not a
    // blank argument: it starts the same capture and the name is typed later.
    assert_eq!(typed_command_argument("INSERT   "), None);
    session
        .command("INSERT   ")
        .expect("the bare selector is admitted");
    assert_eq!(
        session.snapshot(None, None, 0).tool.expect("a tool").key,
        "draw.insertblock"
    );
}
