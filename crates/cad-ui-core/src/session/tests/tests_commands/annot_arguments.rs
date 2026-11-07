//! Typed-argument and vocabulary contracts for the 2026-11-07 annotation/group
//! batch (`GROUP`/`-GROUP`, `UNGROUP`, `GROUPEDIT`, `QLEADER`, `LEADER`,
//! `TEXTFIT`, `HATCHSETORIGIN`, and the argument-free `HATCHSETBOUNDARY` /
//! `XDLIST`).

use super::super::super::*;
use cad_app::{GroupCommand, GroupEditCommand, LeaderCommand, TextFitCommand};

use crate::session::annot_arguments::{
    GROUPEDIT_SELECTOR, GROUP_SELECTOR, HATCHSETORIGIN_SELECTOR, LEADER_SELECTOR, QLEADER_SELECTOR,
    TEXTFIT_SELECTOR, UNGROUP_SELECTOR,
};
use crate::session::commands::ARGUMENT_SELECTORS;
use crate::session::{
    parse_group_argument, parse_groupedit_argument, parse_hatchsetorigin_argument,
    parse_leader_argument, parse_qleader_argument, parse_textfit_argument, parse_ungroup_argument,
};

#[test]
fn every_annot_argument_selector_is_admitted() {
    for selector in [
        GROUP_SELECTOR,
        UNGROUP_SELECTOR,
        GROUPEDIT_SELECTOR,
        QLEADER_SELECTOR,
        LEADER_SELECTOR,
        TEXTFIT_SELECTOR,
        HATCHSETORIGIN_SELECTOR,
    ] {
        assert!(
            ARGUMENT_SELECTORS.contains(&selector),
            "{selector} must be an argument selector"
        );
    }
}

#[test]
fn dash_group_is_the_same_command_path() {
    assert_eq!(normalize_command("GROUP"), Some(CommandId::Group));
    assert_eq!(normalize_command("-GROUP"), Some(CommandId::Group));
    assert_eq!(command_name(CommandId::Group), Some("GROUP"));
    assert!(admissible_names().contains(&"-GROUP"));
}

#[test]
fn annotation_batch_selectors_normalize() {
    for (name, id) in [
        ("GROUP", CommandId::Group),
        ("UNGROUP", CommandId::Ungroup),
        ("GROUPEDIT", CommandId::GroupEdit),
        ("QLEADER", CommandId::QLeader),
        ("LEADER", CommandId::Leader),
        ("TEXTFIT", CommandId::TextFit),
        ("HATCHSETBOUNDARY", CommandId::HatchSetBoundary),
        ("HATCHSETORIGIN", CommandId::HatchSetOrigin),
        ("XDLIST", CommandId::XdList),
    ] {
        assert_eq!(normalize_command(name), Some(id), "{name}");
        assert_eq!(canonical_command_name(name), Some(name), "{name}");
    }
}

#[test]
fn only_the_writing_commands_require_work_mode() {
    for id in [
        CommandId::QLeader,
        CommandId::Leader,
        CommandId::TextFit,
        CommandId::HatchSetBoundary,
        CommandId::HatchSetOrigin,
    ] {
        assert!(id.requires_work_mode(), "{id:?} writes/declares a write");
    }
    for id in [
        CommandId::Group,
        CommandId::Ungroup,
        CommandId::GroupEdit,
        CommandId::XdList,
    ] {
        assert!(!id.requires_work_mode(), "{id:?} cannot write the drawing");
    }
}

#[test]
fn group_grammar_is_create_list_or_delete() {
    assert_eq!(parse_group_argument("LIST"), Some(GroupCommand::List));
    assert_eq!(
        parse_group_argument(" create walls "),
        Some(GroupCommand::Create("walls".into()))
    );
    assert_eq!(
        parse_group_argument("DELETE Walls"),
        Some(GroupCommand::Delete("Walls".into()))
    );
    for bad in ["", "LIST extra", "CREATE", "CREATE two words", "DELETE"] {
        assert_eq!(parse_group_argument(bad), None, "{bad:?}");
    }
}

#[test]
fn ungroup_grammar_is_one_name() {
    assert_eq!(parse_ungroup_argument("walls"), Some("walls".into()));
    for bad in ["", "two words", "  "] {
        assert_eq!(parse_ungroup_argument(bad), None, "{bad:?}");
    }
}

#[test]
fn groupedit_grammar_is_name_verb_arg() {
    assert_eq!(
        parse_groupedit_argument("walls ADD 12,13"),
        Some(GroupEditCommand::Add {
            name: "walls".into(),
            entities: vec![12, 13]
        })
    );
    assert_eq!(
        parse_groupedit_argument("walls ADD"),
        Some(GroupEditCommand::Add {
            name: "walls".into(),
            entities: Vec::new()
        })
    );
    assert_eq!(
        parse_groupedit_argument("walls REMOVE 12 13"),
        Some(GroupEditCommand::Remove {
            name: "walls".into(),
            entities: vec![12, 13]
        })
    );
    assert_eq!(
        parse_groupedit_argument("walls RENAME WALLS"),
        Some(GroupEditCommand::Rename {
            name: "walls".into(),
            new_name: "WALLS".into()
        })
    );
    for bad in [
        "",
        "walls",
        "walls ADD x",
        "walls RENAME",
        "walls RENAME two words",
        "walls NOPE 12",
    ] {
        assert_eq!(parse_groupedit_argument(bad), None, "{bad:?}");
    }
}

#[test]
fn leader_grammar_is_two_pairs_and_bounded_text() {
    let parsed = parse_qleader_argument("1,2 3,4 Hello world").expect("QLEADER");
    assert_eq!(
        parsed,
        LeaderCommand {
            arrow: cad_domain::Point3 {
                x: 1.0,
                y: 2.0,
                z: 0.0
            },
            landing: cad_domain::Point3 {
                x: 3.0,
                y: 4.0,
                z: 0.0
            },
            text: "Hello world".into(),
        }
    );
    assert_eq!(
        parse_leader_argument("1,2 3,4 Note").map(|leader| leader.text),
        Some("Note".into())
    );
    for bad in [
        "",
        "1,2",
        "1,2 3,4",
        "1,2 3,4   ",
        "1 2 3,4 text",
        "1,2,3 3,4 text",
    ] {
        assert_eq!(parse_qleader_argument(bad), None, "{bad:?}");
    }
}

#[test]
fn textfit_grammar_is_one_signed_decimal() {
    assert_eq!(
        parse_textfit_argument("12.5"),
        Some(TextFitCommand { target_width: 12.5 })
    );
    assert_eq!(
        parse_textfit_argument("-3"),
        Some(TextFitCommand { target_width: -3.0 })
    );
    for bad in ["", "12 5", "abc"] {
        assert_eq!(parse_textfit_argument(bad), None, "{bad:?}");
    }
}

#[test]
fn hatchsetorigin_grammar_is_two_decimals() {
    let parsed = parse_hatchsetorigin_argument("1.5 -2").expect("origin");
    assert_eq!(parsed.x, 1.5);
    assert_eq!(parsed.y, -2.0);
    for bad in ["", "1", "1 2 3", "x 2"] {
        assert_eq!(parse_hatchsetorigin_argument(bad), None, "{bad:?}");
    }
}
