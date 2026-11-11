//! The typed `-LAYER <option> …` command-line layer editor.
//!
//! Pinned here: the one-line grammar (keywords, documented short aliases,
//! case-insensitivity, comma-separated name lists, required operands), the
//! selector admission (`-LAYER` is an argument form of its existing selector, not
//! a new command), the wire-gate admission (`shell::check_command`), the dispatch
//! to the real command layer, and the Viewer split:
//!
//! * `-LAYER` and `-LAYER ?` are the **read-only report** of the extended
//!   `DashLayer` command, so they stay Viewer-legal;
//! * every write option rides the Work-only `CommandId::SetLayerProperty`
//!   carrier, so a Viewer session is refused by the same authorisation every
//!   layer-table write already uses.
//!
//! The command semantics (atomicity, one undo step, the active-layer rule) are
//! pinned in `cad-app`'s `dash_layer_writes` tests; the payload nothing is
//! re-verified here beyond the dispatch reaching that command.

use super::super::super::*;
use super::super::test_support::*;

use crate::session::commands::{
    parse_dash_layer_argument, ARGUMENT_SELECTORS, DASH_LAYER_SELECTOR,
};
use cad_app::{DashLayerOps, ExtendedCommandId};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

fn names(layers: &[&str]) -> Vec<String> {
    layers.iter().map(|name| (*name).to_string()).collect()
}

#[test]
fn every_documented_option_parses_with_its_short_alias() {
    // Lists, case-insensitive keyword, surrounding whitespace ignored.
    assert_eq!(
        parse_dash_layer_argument("NEW WALLS,DIMS"),
        Some(DashLayerOps::New(names(&["WALLS", "DIMS"])))
    );
    assert_eq!(
        parse_dash_layer_argument("  n  walls , dims  "),
        Some(DashLayerOps::New(names(&["walls", "dims"])))
    );
    assert_eq!(
        parse_dash_layer_argument("D WALLS"),
        Some(DashLayerOps::Delete(names(&["WALLS"])))
    );
    for (keyword, alias, build) in [
        (
            "ON",
            "ON",
            DashLayerOps::On as fn(Vec<String>) -> DashLayerOps,
        ),
        ("OFF", "OFF", DashLayerOps::Off),
        ("FREEZE", "F", DashLayerOps::Freeze),
        ("THAW", "T", DashLayerOps::Thaw),
        ("LOCK", "LO", DashLayerOps::Lock),
        ("UNLOCK", "U", DashLayerOps::Unlock),
    ] {
        let expected = Some(build(names(&["WALLS", "DIMS"])));
        assert_eq!(
            parse_dash_layer_argument(&format!("{keyword} WALLS,DIMS")),
            expected,
            "{keyword}"
        );
        assert_eq!(
            parse_dash_layer_argument(alias),
            None,
            "{alias} still needs a name list"
        );
        assert_eq!(
            parse_dash_layer_argument(&format!("{alias} WALLS,DIMS")),
            expected,
            "{alias}"
        );
    }

    // One-name forms: the trimmed remainder, spaces allowed.
    assert_eq!(
        parse_dash_layer_argument("MAKE A B"),
        Some(DashLayerOps::Make("A B".into()))
    );
    assert_eq!(
        parse_dash_layer_argument("m walls"),
        Some(DashLayerOps::Make("walls".into()))
    );
    assert_eq!(
        parse_dash_layer_argument("SET WALLS"),
        Some(DashLayerOps::Set("WALLS".into()))
    );
    assert_eq!(
        parse_dash_layer_argument("s  a b  "),
        Some(DashLayerOps::Set("a b".into()))
    );
    assert_eq!(
        parse_dash_layer_argument("SET \"A B\""),
        Some(DashLayerOps::Set("A B".into())),
        "one fully quoted name is unquoted, not stored with quotes"
    );

    // RENAME: first token is the old name, the trimmed remainder the new one.
    assert_eq!(
        parse_dash_layer_argument("RENAME OLD NEW"),
        Some(DashLayerOps::Rename {
            old: "OLD".into(),
            new: "NEW".into(),
        })
    );
    assert_eq!(
        parse_dash_layer_argument("r  old  new walls "),
        Some(DashLayerOps::Rename {
            old: "old".into(),
            new: "new walls".into(),
        })
    );
    assert_eq!(
        parse_dash_layer_argument("RENAME \"OLD NAME\" \"NEW NAME\""),
        Some(DashLayerOps::Rename {
            old: "OLD NAME".into(),
            new: "NEW NAME".into(),
        }),
        "quoted names keep their spaces"
    );

    // Value + list forms.
    assert_eq!(
        parse_dash_layer_argument("COLOR 3 WALLS,DIMS"),
        Some(DashLayerOps::Color {
            value: "3".into(),
            layers: names(&["WALLS", "DIMS"]),
        })
    );
    // A true-colour triple is one comma-joined *value* token.
    assert_eq!(
        parse_dash_layer_argument("c 255,0,0 WALLS"),
        Some(DashLayerOps::Color {
            value: "255,0,0".into(),
            layers: names(&["WALLS"]),
        })
    );
    assert_eq!(
        parse_dash_layer_argument("LTYPE DASHED WALLS"),
        Some(DashLayerOps::Linetype {
            name: "DASHED".into(),
            layers: names(&["WALLS"]),
        })
    );
    assert_eq!(
        parse_dash_layer_argument("l dashed walls,dims"),
        Some(DashLayerOps::Linetype {
            name: "dashed".into(),
            layers: names(&["walls", "dims"]),
        })
    );
    assert_eq!(
        parse_dash_layer_argument("LTYPE \"My Style\" WALLS,DIMS"),
        Some(DashLayerOps::Linetype {
            name: "My Style".into(),
            layers: names(&["WALLS", "DIMS"]),
        }),
        "a quoted value may contain spaces"
    );
    assert_eq!(
        parse_dash_layer_argument("LWEIGHT 35 WALLS"),
        Some(DashLayerOps::Lineweight {
            value: "35".into(),
            layers: names(&["WALLS"]),
        })
    );
    assert_eq!(
        parse_dash_layer_argument("lw ByLayer WALLS"),
        Some(DashLayerOps::Lineweight {
            value: "ByLayer".into(),
            layers: names(&["WALLS"]),
        })
    );
    for (value, plottable) in [
        ("Y", true),
        ("y", true),
        ("TRUE", true),
        ("1", true),
        ("N", false),
        ("n", false),
        ("FALSE", false),
        ("0", false),
    ] {
        assert_eq!(
            parse_dash_layer_argument(&format!("PLOT {value} WALLS")),
            Some(DashLayerOps::Plot {
                plottable,
                layers: names(&["WALLS"]),
            }),
            "PLOT {value}"
        );
    }
    assert_eq!(
        parse_dash_layer_argument("p n walls,dims"),
        Some(DashLayerOps::Plot {
            plottable: false,
            layers: names(&["walls", "dims"]),
        })
    );
}

#[test]
fn a_space_belongs_to_a_name_and_only_a_comma_separates_the_list() {
    // Documented rule: a layer name may contain spaces, so `NEW A B` is **one**
    // layer named `A B`; only a comma starts a second entry.
    assert_eq!(
        parse_dash_layer_argument("NEW A B"),
        Some(DashLayerOps::New(names(&["A B"])))
    );
    assert_eq!(
        parse_dash_layer_argument("NEW A B,C D"),
        Some(DashLayerOps::New(names(&["A B", "C D"])))
    );
}

#[test]
fn the_parser_refuses_unknown_options_and_missing_operands() {
    for refused in [
        "",
        "   ",
        "?",
        "? WALLS",
        "BOGUS WALLS",
        "NEW",
        "N ",
        "NEW ,",
        "NEW WALLS,",
        "NEW ,WALLS",
        "NEW WALLS,,DIMS",
        "MAKE",
        "SET",
        "S ",
        "RENAME",
        "RENAME OLD",
        "DELETE",
        "ON",
        "OFF",
        "FREEZE",
        "THAW",
        "LOCK",
        "UNLOCK",
        "COLOR",
        "COLOR 3",
        "LTYPE",
        "LTYPE DASHED",
        "LWEIGHT",
        "LWEIGHT 35",
        "PLOT",
        "PLOT MAYBE WALLS",
        "PLOT Y",
    ] {
        assert_eq!(parse_dash_layer_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn the_reports_spelling_is_not_a_write_option() {
    // `?` is the read-only report and is handled before the parser; the parser
    // itself never turns it into an op.
    assert_eq!(parse_dash_layer_argument("?"), None);
    assert_eq!(parse_dash_layer_argument("? WALLS"), None);
    // The bare selector is not an argument form at all.
    assert_eq!(typed_command_argument("-LAYER"), None);
    assert_eq!(
        typed_command_argument("-LAYER ?"),
        Some(("-LAYER", "?")),
        "`-LAYER ?` is admitted so the session can report"
    );
}

#[test]
fn the_selector_is_admitted_as_an_argument_form_of_the_existing_selector() {
    assert!(ARGUMENT_SELECTORS.contains(&DASH_LAYER_SELECTOR));
    assert_eq!(DASH_LAYER_SELECTOR, "-LAYER");
    assert_eq!(
        typed_command_argument("-LAYER NEW WALLS"),
        Some(("-LAYER", "NEW WALLS"))
    );
    assert_eq!(
        typed_command_argument("  -layer   new  walls  "),
        Some(("-LAYER", "new  walls"))
    );
    // Still one canonical selector: no new command name was invented.
    assert_eq!(
        normalize_command("-LAYER"),
        Some(CommandId::Extended(ExtendedCommandId::DashLayer))
    );
    // The write carrier is Work-only; the report selector stays Viewer-legal.
    assert!(CommandId::SetLayerProperty.requires_work_mode());
    assert!(!ExtendedCommandId::DashLayer.requires_work_mode());
}

#[test]
fn the_wire_gate_admits_a_dash_layer_write_line() {
    use crate::shell::check_command;

    assert_eq!(
        check_command(true, false, 0, "-LAYER NEW x").as_deref(),
        Ok("-LAYER NEW x"),
        "the command line must admit the typed write form"
    );
    assert_eq!(
        check_command(true, false, 0, "  -layer   new  x ").as_deref(),
        Ok("-LAYER new  x"),
        "the gate stores the canonical selector plus the argument verbatim"
    );
    assert_eq!(
        check_command(true, false, 0, "-LAYER ?").as_deref(),
        Ok("-LAYER ?"),
        "the report spelling is admitted too"
    );
    assert_eq!(
        check_command(true, false, 0, "-LAYER").as_deref(),
        Ok("-LAYER"),
        "the bare selector keeps its name-only line"
    );
}

#[test]
fn a_typed_write_reaches_the_command_layer_and_the_report_stays_read_only() {
    let session = fresh_session();
    session
        .command("-LAYER NEW DOORS")
        .expect("the typed NEW creates a layer");

    let layer_names: Vec<String> = {
        let controller = session.core.controller.borrow();
        let drawing = controller.drawing().expect("a drawing is open");
        drawing.layers().map(|row| row.name.clone()).collect()
    };
    assert!(
        layer_names.contains(&"DOORS".to_string()),
        "{layer_names:?}"
    );

    // `-LAYER ?` is a successful report, not a write: the created set does not
    // grow and no error is recorded.
    session.command("-LAYER ?").expect("-LAYER ? reports");
    let layer_names_after: Vec<String> = {
        let controller = session.core.controller.borrow();
        let drawing = controller.drawing().expect("a drawing is open");
        drawing.layers().map(|row| row.name.clone()).collect()
    };
    assert_eq!(layer_names_after, layer_names);

    // A malformed option is refused by name before IPC, with the grammar spelled
    // out and nothing changed.
    let error = session.command("-LAYER BOGUS WALLS").unwrap_err();
    assert!(error.contains("-LAYER takes one option"), "{error}");
    let layer_names_after_refusal: Vec<String> = {
        let controller = session.core.controller.borrow();
        let drawing = controller.drawing().expect("a drawing is open");
        drawing.layers().map(|row| row.name.clone()).collect()
    };
    assert_eq!(layer_names_after_refusal, layer_names);
}

#[test]
fn a_typed_write_is_refused_in_a_viewer_session() {
    let session = fresh_session();
    session
        .core
        .controller
        .borrow_mut()
        .set_mode(cad_app::AppMode::Viewer)
        .expect("switch to Viewer");

    let error = session
        .command("-LAYER NEW DOORS")
        .expect_err("Viewer mode refuses a layer-table write");
    assert!(error.contains("PermissionDenied"), "{error}");

    // The report is still admitted in Viewer mode.
    session
        .command("-LAYER ?")
        .expect("-LAYER ? reports in Viewer");
}
