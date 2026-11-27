//! `MATERIAL <name> RENAME|COLOR|TRANSPARENCY|DELETE` grammar and dispatch.
//!
//! The command layer resolves the row and validates values; this module only
//! pins the typed grammar and the dispatch path.

use super::super::*;
use super::test_support::*;
use crate::session::commands::parse_material_argument;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_material_grammar_covers_rename_color_transparency_and_delete() {
    assert_eq!(
        parse_material_argument("Red RENAME \"Dark Red\""),
        Some(cad_app::MaterialCommand::Rename {
            name: "Red".into(),
            new_name: "Dark Red".into(),
        })
    );
    assert_eq!(
        parse_material_argument("Red COLOR 10 20 30"),
        Some(cad_app::MaterialCommand::Color {
            name: "Red".into(),
            color: [10, 20, 30],
        })
    );
    assert_eq!(
        parse_material_argument("Red TRANSPARENCY 0.4"),
        Some(cad_app::MaterialCommand::Transparency {
            name: "Red".into(),
            transparency: 0.4,
        })
    );
    assert_eq!(
        parse_material_argument("Red DELETE"),
        Some(cad_app::MaterialCommand::Delete { name: "Red".into() })
    );
    for refused in [
        "Red",
        "Red RENAME",
        "Red COLOR 1 2",
        "Red COLOR 1 2 999",
        "Red TRANSPARENCY 2",
        "Red DELETE NOW",
    ] {
        assert_eq!(parse_material_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn material_edits_reach_the_command_layer_and_are_work_only() {
    let session = fresh_session();
    let text = match session.command("MATERIAL Red COLOR 1 2 3") {
        Err(text) => text,
        Ok(()) => panic!("the synthetic session has no document material"),
    };
    assert!(
        text.contains("document not open") || text.contains("material.unknown"),
        "{text}"
    );
    assert_eq!(normalize_command("MATERIAL"), Some(CommandId::Material));
    assert_eq!(canonical_command_name("MATERIAL"), Some("MATERIAL"));
}
