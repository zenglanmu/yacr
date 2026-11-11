//! The real shell settings selectors: `NAVBAR`, `NAVVCUBE`,
//! `NAVVCUBEDISPLAY`, `DISPLAYVIEWCUBEIN2D`, `DISPLAYVIEWCUBEIN3D`.
//!
//! They are settings writes, not `CommandId` dispatches, so these contracts
//! prove they are admitted by the vocabulary **without** a command id, that every
//! write is persisted through the one host settings path, and that every refusal
//! is explicit and changes nothing.

use super::super::super::*;
use super::super::test_support::*;

/// A config store that records every persisted preference document, so a write's
/// root leaf can be asserted without a filesystem.
struct RecordingConfig {
    persisted: RefCell<Vec<String>>,
}

impl ConfigStore for RecordingConfig {
    fn load_config(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }
    fn load_preference(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }
    fn persist_preference(&self, text: &str) -> CadResult<()> {
        self.persisted.borrow_mut().push(text.to_string());
        Ok(())
    }
}

fn recording_session() -> (CadSession, Rc<RecordingConfig>) {
    let config = Rc::new(RecordingConfig {
        persisted: RefCell::new(Vec::new()),
    });
    (
        session_with_config(config.clone() as Rc<dyn ConfigStore>),
        config,
    )
}

fn last_persisted(config: &RecordingConfig) -> String {
    config
        .persisted
        .borrow()
        .last()
        .cloned()
        .expect("a settings write persists")
}

#[test]
fn the_five_shell_selectors_are_admissible_without_a_command_id() {
    for name in [
        "NAVBAR",
        "NAVVCUBE",
        "NAVVCUBEDISPLAY",
        "DISPLAYVIEWCUBEIN2D",
        "DISPLAYVIEWCUBEIN3D",
    ] {
        assert_eq!(canonical_command_name(name), Some(name), "{name}");
        assert_eq!(
            normalize_command(name),
            None,
            "{name} must carry no CommandId (it is a settings write)"
        );
        assert!(admissible_names().contains(&name), "{name}");
        assert!(
            !COMMAND_TABLE
                .iter()
                .any(|(candidate, _)| *candidate == name),
            "{name} must not be a command-table row"
        );
        assert!(
            !REFUSAL_TABLE
                .iter()
                .any(|(candidate, _)| *candidate == name),
            "{name} is real now, not a named refusal"
        );
        // It is not an argument selector either: the shell intercept resolves
        // the raw line before the ordinary argument path.
        assert_eq!(typed_command_argument(&format!("{name} ON")), None);
    }
}

#[test]
fn navbar_toggles_and_sets_the_root_preference() {
    let (session, config) = recording_session();
    assert!(session.settings().navbar_visible, "the default is visible");

    session.command("NAVBAR").expect("bare NAVBAR toggles");
    assert!(!session.settings().navbar_visible);
    assert!(last_persisted(&config).contains(r#""navbarVisible":false"#));
    assert!(!session.snapshot(None, None, 0).navigation.navbar_visible);

    session.command("NAVBAR OFF").expect("NAVBAR OFF sets off");
    assert!(!session.settings().navbar_visible);
    session.command("navBar On").expect("NAVBAR ON sets on");
    assert!(session.settings().navbar_visible);
    assert!(last_persisted(&config).contains(r#""navbarVisible":true"#));

    // The selector is recorded like any other admitted command.
    let log = session.snapshot(None, None, 0).command_log;
    let last = log
        .iter()
        .rev()
        .find(|entry| entry.kind == CadCommandLogKind::Command)
        .expect("the selector is echoed");
    assert_eq!(last.command.as_deref(), Some("NAVBAR"));
}

#[test]
fn navvcube_toggles_the_current_view_class_bit() {
    let (session, config) = recording_session();
    assert_eq!(session.settings().navvcube_display, 3);

    // A 2D view class: bit 2.
    session.command("NAVVCUBE OFF").expect("NAVVCUBE OFF");
    assert_eq!(session.settings().navvcube_display, 1);
    assert!(last_persisted(&config).contains(r#""navvcubeDisplay":1"#));
    session
        .command("NAVVCUBE")
        .expect("bare NAVVCUBE toggles bit 2");
    assert_eq!(session.settings().navvcube_display, 3);

    // A 3D view class: bit 1.
    session
        .command("SWITCH2D3D")
        .expect("the fixture switches to 3D");
    assert_eq!(
        session.snapshot(None, None, 0).view.map(|view| view.mode),
        Some("3d".to_string()),
        "the session is really in the 3D class"
    );
    session.command("NAVVCUBE OFF").expect("NAVVCUBE OFF in 3D");
    assert_eq!(session.settings().navvcube_display, 2);
    session.command("NAVVCUBE ON").expect("NAVVCUBE ON in 3D");
    assert_eq!(session.settings().navvcube_display, 3);
}

#[test]
fn navvcubedisplay_writes_the_mask_and_refuses_the_bare_form() {
    let (session, config) = recording_session();
    let before = config.persisted.borrow().len();
    let error = session
        .command("NAVVCUBEDISPLAY")
        .expect_err("the bare form needs a value");
    assert!(error.contains("navvcubedisplay.needs_value"), "{error}");
    assert_eq!(
        config.persisted.borrow().len(),
        before,
        "a refusal writes nothing"
    );

    for display in [0, 1, 2, 3] {
        session
            .command(&format!("NAVVCUBEDISPLAY {display}"))
            .unwrap_or_else(|error| panic!("{display}: {error}"));
        assert_eq!(session.settings().navvcube_display, display);
    }
    let error = session
        .command("NAVVCUBEDISPLAY 4")
        .expect_err("an out-of-range mask is refused");
    assert!(error.contains("navvcubedisplay.needs_value"), "{error}");
    assert_eq!(
        session.settings().navvcube_display,
        3,
        "a refusal changes nothing"
    );
    let _ = config;
}

#[test]
fn displayviewcubein2d_and_in3d_set_the_flags() {
    let (session, config) = recording_session();
    for selector in ["DISPLAYVIEWCUBEIN2D", "DISPLAYVIEWCUBEIN3D"] {
        let error = session
            .command(selector)
            .expect_err("the bare form needs a value");
        assert!(
            error.contains(&format!("{}.needs_value", selector.to_ascii_lowercase())),
            "{error}"
        );
    }
    session
        .command("DISPLAYVIEWCUBEIN2D OFF")
        .expect("DISPLAYVIEWCUBEIN2D OFF");
    assert!(!session.settings().view_cube_in_2d);
    assert!(session.settings().view_cube_in_3d);
    session
        .command("DISPLAYVIEWCUBEIN3D 0")
        .expect("DISPLAYVIEWCUBEIN3D 0");
    assert!(!session.settings().view_cube_in_3d);
    let written = last_persisted(&config);
    assert!(written.contains(r#""viewCubeIn2d":false"#), "{written}");
    assert!(written.contains(r#""viewCubeIn3d":false"#), "{written}");
}

#[test]
fn navvcube_properties_is_refused_and_changes_nothing() {
    let (session, config) = recording_session();
    session
        .command("NAVVCUBE ON")
        .expect("a real NAVVCUBE write");
    let before = session.settings().navvcube_display;
    let writes = config.persisted.borrow().len();
    let error = session
        .command("NAVVCUBE PROPERTIES")
        .expect_err("no ViewCube settings dialog exists");
    assert!(
        error.contains("navvcube.properties_not_implemented"),
        "{error}"
    );
    assert_eq!(session.settings().navvcube_display, before);
    assert_eq!(config.persisted.borrow().len(), writes);
}

#[test]
fn invalid_shell_arguments_are_refused_without_writing() {
    let (session, config) = recording_session();
    let before = session.settings();
    let writes = config.persisted.borrow().len();
    for (line, code) in [
        ("NAVBAR MAYBE", "navbar.invalid_argument"),
        ("NAVVCUBE MAYBE", "navvcube.invalid_argument"),
        ("NAVVCUBEDISPLAY x", "navvcubedisplay.needs_value"),
        (
            "DISPLAYVIEWCUBEIN2D MAYBE",
            "displayviewcubein2d.needs_value",
        ),
        (
            "DISPLAYVIEWCUBEIN3D MAYBE",
            "displayviewcubein3d.needs_value",
        ),
    ] {
        let error = session.command(line).expect_err(line);
        assert!(error.contains(code), "{line}: {error}");
        assert_eq!(
            session.snapshot(None, None, 0).error.as_deref(),
            Some(error.as_str()),
            "{line} reaches cad_status.error"
        );
    }
    assert_eq!(session.settings(), before);
    assert_eq!(config.persisted.borrow().len(), writes);
}

#[test]
fn setvar_writes_the_three_write_only_shell_settings() {
    let (session, config) = recording_session();

    session
        .command("SETVAR NAVVCUBEDISPLAY 1")
        .expect("SETVAR writes the mask");
    assert_eq!(session.settings().navvcube_display, 1);
    assert!(last_persisted(&config).contains(r#""navvcubeDisplay":1"#));

    session
        .command("SETVAR DISPLAYVIEWCUBEIN2D OFF")
        .expect("SETVAR writes the 2D flag");
    assert!(!session.settings().view_cube_in_2d);
    session
        .command("SETVAR DISPLAYVIEWCUBEIN3D 1")
        .expect("SETVAR writes the 3D flag");
    assert!(session.settings().view_cube_in_3d);

    // The read form stays explicitly unmodelled: no fabricated value.
    let error = session
        .command("SETVAR NAVVCUBEDISPLAY")
        .expect_err("the report form is not modelled");
    assert!(error.contains("setvar.write_only_variable"), "{error}");
    assert_eq!(session.settings().navvcube_display, 1);

    let error = session
        .command("SETVAR NAVVCUBEDISPLAY 9")
        .expect_err("an out-of-range value is refused");
    assert!(error.contains("invalid SETVAR NAVVCUBEDISPLAY"), "{error}");
    assert_eq!(session.settings().navvcube_display, 1);
}

#[test]
fn the_wire_gate_admits_the_five_selectors_with_their_typed_arguments() {
    use crate::shell::check_command;

    // The gate runs before the queue exists: a refusal here means the typed line
    // never reaches `CadSession::command`, so these are the assertions a real
    // host's command line and ribbon buttons depend on. They caught a gate that
    // admitted only the drawing argument selectors, which made every sysvar
    // write below work in the session unit tests and fail on a real host with
    // `unsupported CAD command: <line>`.
    for (line, normalized) in [
        ("NAVVCUBEDISPLAY 0", "NAVVCUBEDISPLAY 0"),
        ("navvcubedisplay 3", "NAVVCUBEDISPLAY 3"),
        ("NAVVCUBE ON", "NAVVCUBE ON"),
        ("DISPLAYVIEWCUBEIN2D OFF", "DISPLAYVIEWCUBEIN2D OFF"),
        ("DISPLAYVIEWCUBEIN3D 1", "DISPLAYVIEWCUBEIN3D 1"),
    ] {
        assert_eq!(
            check_command(true, false, 0, line).as_deref(),
            Ok(normalized),
            "{line} must be admitted by the wire gate"
        );
    }

    // A bare selector stays the name-only line; the *session* refuses the missing
    // value, so the gate must not pre-empt it with an unknown-name refusal.
    for name in [
        "NAVBAR",
        "NAVVCUBE",
        "NAVVCUBEDISPLAY",
        "DISPLAYVIEWCUBEIN2D",
        "DISPLAYVIEWCUBEIN3D",
    ] {
        assert_eq!(
            check_command(true, false, 0, name).as_deref(),
            Ok(name),
            "{name} must be admitted as a name-only line"
        );
    }

    // The gate admits the shape only: the value is the session's call.
    assert_eq!(
        check_command(true, false, 0, "NAVVCUBEDISPLAY 9").as_deref(),
        Ok("NAVVCUBEDISPLAY 9"),
        "an out-of-range value is refused by the session, not the gate"
    );
    // A drawing selector without an argument form stays an unsupported name.
    assert_eq!(
        check_command(true, false, 0, "LINE 2").unwrap_err(),
        "unsupported CAD command: LINE 2"
    );
}
