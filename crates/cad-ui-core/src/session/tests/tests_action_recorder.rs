//! Action-recorder contracts (ADR-0056): recording, macro files, playback,
//! refusals and the browser/no-file-bridge boundary.
//!
//! Everything here is a session contract: the recorder state is not document
//! content, no `.ymodel` entry moves, and playback re-enters the ordinary typed
//! command funnel (so its transaction/undo behaviour is the command's own).

use super::super::*;
use super::test_support::*;

use cad_host::{HostCore, HostResources};
use cad_platform::NamedFileError;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

struct NativeFiles;

impl HostResources for NativeFiles {
    fn install_fonts(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Ok(())
    }
    fn install_images(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Ok(())
    }
    fn read_named_file(&self, path: &Path) -> Result<Arc<[u8]>, NamedFileError> {
        cad_platform::read_named_file(path)
    }
    fn write_named_file(&self, path: &Path, bytes: &[u8]) -> Result<(), NamedFileError> {
        cad_platform::write_named_file(path, bytes)
    }
    fn supports_named_files(&self) -> bool {
        true
    }
}

fn recorder_session() -> CadSession {
    let mut host_platform = platform(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    host_platform.resources = Rc::new(NativeFiles);
    session_with_platform(host_platform)
}

fn temp_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "yacr-action-recorder-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn pdmode(session: &CadSession) -> i16 {
    session
        .core
        .controller
        .borrow()
        .drawing()
        .expect("a document")
        .point_display_mode()
}

#[test]
fn a_recording_captures_success_and_ignores_refusals_and_controls() {
    let session = recorder_session();
    session.command("ACTRECORD").expect("start");
    session
        .command("PDMODE 3")
        .expect("a successful typed write");
    assert!(
        session.command("BOGUS").is_err(),
        "an unknown command refuses"
    );
    let snapshot = session.action_recorder().expect("state");
    let recording = snapshot.recording.expect("still recording");
    assert_eq!(recording.name, "macro");
    assert_eq!(
        recording.steps, 1,
        "only the successful command is captured"
    );
    assert_eq!(snapshot.macros, Vec::new());
    // A second ACTRECORD is refused and is never captured.
    assert!(session.command("ACTRECORD again").is_err());
    assert_eq!(
        session
            .action_recorder()
            .expect("state")
            .recording
            .expect("recording")
            .steps,
        1
    );
}

#[test]
fn stop_writes_the_versioned_macro_file_and_lists_it() {
    let session = recorder_session();
    let dir = temp_dir("save");
    let path = dir.join("Mixed Case.actm.json");
    let path_text = path.to_string_lossy().to_string();
    session
        .command(&format!("ACTRECORD {path_text}"))
        .expect("start with a typed path");
    session.command("PDMODE 3").expect("a command");
    session
        .command("ACTUSERINPUT pick the point")
        .expect("input");
    session
        .command("ACTUSERMESSAGE place the block")
        .expect("message");
    let snapshot = session.action_recorder().expect("state");
    assert_eq!(snapshot.recording.expect("recording").steps, 3);
    session.command("ACTSTOP").expect("stop and save");

    let bytes = std::fs::read(&path).expect("the macro file exists");
    let text = String::from_utf8(bytes).expect("UTF-8");
    let macro_file = crate::decode_action_macro(&text).expect("a valid macro file");
    assert_eq!(macro_file.name, "Mixed Case");
    assert_eq!(macro_file.steps.len(), 3);
    assert_eq!(
        macro_file.steps[0],
        crate::MacroStep::Command {
            command: "PDMODE".into(),
            argument: Some("3".into()),
        }
    );
    assert!(matches!(
        macro_file.steps[1],
        crate::MacroStep::Input { .. }
    ));
    assert!(matches!(
        macro_file.steps[2],
        crate::MacroStep::Message { .. }
    ));

    let snapshot = session.action_recorder().expect("state");
    assert!(snapshot.recording.is_none());
    assert_eq!(snapshot.macros.len(), 1);
    assert_eq!(snapshot.macros[0].name, "Mixed Case");
    assert_eq!(snapshot.macros[0].steps, 3);
    assert_eq!(snapshot.last_macro.as_deref(), Some("Mixed Case"));
}

#[test]
fn a_missing_destination_is_refused_without_writing_or_losing_the_recording() {
    let session = recorder_session();
    session.command("ACTRECORD").expect("start");
    session.command("PDMODE 3").expect("a command");
    let error = session.command("ACTSTOP").expect_err("no path");
    assert!(error.starts_with("actstop.path:"), "{error}");
    let snapshot = session.action_recorder().expect("state");
    assert_eq!(snapshot.recording.expect("still recording").steps, 1);
    assert!(snapshot.macros.is_empty());
    let error = session
        .command("ACTSTOP not-a-json.txt")
        .expect_err("bad extension");
    assert!(error.starts_with("actstop.path:"), "{error}");
}

#[test]
fn playback_runs_each_step_through_the_normal_funnel() {
    let session = recorder_session();
    session.command("ACTRECORD").expect("start");
    session.command("PDMODE 3").expect("first write");
    session.command("PDMODE 4").expect("second write");
    session.command("ACTSTOP").expect_err("no typed path yet");
    // The failed stop kept the recording; give it a destination.
    let path = temp_dir("play").join("m.json");
    session
        .command(&format!("ACTSTOP {}", path.to_string_lossy()))
        .expect("save");
    session.command("PDMODE 5").expect("move away");
    assert_eq!(pdmode(&session), 5);
    session.command("ACTMANAGER PLAY m").expect("play");
    assert_eq!(pdmode(&session), 4, "the last recorded write ran last");
}

#[test]
fn a_midway_refusal_names_the_step_and_does_not_report_success() {
    let session = recorder_session();
    let path = temp_dir("refuse").join("bad.json");
    let macro_file = crate::ActionMacro::new(
        "bad",
        vec![
            crate::MacroStep::Command {
                command: "PDMODE".into(),
                argument: Some("3".into()),
            },
            crate::MacroStep::Command {
                command: "PDMODE".into(),
                argument: Some("3.5".into()),
            },
            crate::MacroStep::Command {
                command: "PDMODE".into(),
                argument: Some("4".into()),
            },
        ],
    );
    std::fs::write(
        &path,
        crate::encode_action_macro(&macro_file).expect("encode"),
    )
    .expect("write fixture");
    session.command("PDMODE 5").expect("baseline");
    let error = session
        .command(&format!("ACTMANAGER PLAY {}", path.to_string_lossy()))
        .expect_err("the second step refuses");
    assert!(
        error.starts_with("actmanager.step_failed: step 2"),
        "{error}"
    );
    assert_eq!(pdmode(&session), 3, "the first step really ran");
    // The playback stopped: a later CONTINUE has nothing to resume.
    assert!(session.command("ACTMANAGER CONTINUE").is_err());
    assert_eq!(pdmode(&session), 3, "the third step never ran");
}

#[test]
fn input_steps_pause_and_continue() {
    let session = recorder_session();
    let path = temp_dir("pause").join("pause.json");
    let macro_file = crate::ActionMacro::new(
        "pause",
        vec![
            crate::MacroStep::Command {
                command: "PDMODE".into(),
                argument: Some("3".into()),
            },
            crate::MacroStep::Input {
                message: Some("pick".into()),
            },
            crate::MacroStep::Command {
                command: "PDMODE".into(),
                argument: Some("4".into()),
            },
        ],
    );
    std::fs::write(
        &path,
        crate::encode_action_macro(&macro_file).expect("encode"),
    )
    .expect("write fixture");
    session.command("PDMODE 5").expect("baseline");
    session
        .command(&format!("ACTMANAGER PLAY {}", path.to_string_lossy()))
        .expect("pause");
    assert_eq!(pdmode(&session), 3, "the step before the pause ran");
    session.command("ACTMANAGER CONTINUE").expect("resume");
    assert_eq!(pdmode(&session), 4, "the step after the pause ran");
}

#[test]
fn delete_removes_only_the_session_entry() {
    let session = recorder_session();
    let dir = temp_dir("delete");
    let path = dir.join("gone.json");
    session
        .command(&format!("ACTRECORD {}", path.to_string_lossy()))
        .expect("start");
    session.command("PDMODE 3").expect("a command");
    session.command("ACTSTOP").expect("save");
    assert!(path.exists());
    session.command("ACTMANAGER DELETE gone").expect("delete");
    let snapshot = session.action_recorder().expect("state");
    assert!(snapshot.macros.is_empty());
    assert_eq!(snapshot.last_macro, None);
    assert!(
        path.exists(),
        "delete is a session entry, not a file unlink"
    );
    assert!(session.command("ACTMANAGER DELETE gone").is_err());
}

#[test]
fn a_runtime_without_a_file_bridge_refuses_the_whole_recorder() {
    // The shared test platform's resource strategy leaves
    // `supports_named_files` at its honest default (`false`), standing in for
    // the browser/wasm carrier.
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    for command in [
        "ACTRECORD m",
        "ACTSTOP",
        "-ACTSTOP m.json",
        "ACTUSERINPUT",
        "ACTUSERMESSAGE hi",
        "-ACTUSERMESSAGE hi",
        "ACTMANAGER",
    ] {
        let error = session.command(command).expect_err(command);
        assert_eq!(
            error, "unsupported-runtime: action-recorder",
            "{command} must carry the stable marker"
        );
    }
    assert_eq!(
        session.action_recorder().expect_err("no read"),
        "unsupported-runtime: action-recorder"
    );
}

#[test]
fn the_error_codes_are_stable() {
    let session = recorder_session();
    assert!(session
        .command("ACTSTOP")
        .expect_err("not recording")
        .starts_with("actstop.not_recording:"));
    session.command("ACTRECORD").expect("start");
    assert!(session
        .command("ACTRECORD x")
        .expect_err("already recording")
        .starts_with("actrecord.already_recording:"));
    assert!(session
        .command("ACTUSERMESSAGE")
        .expect_err("empty message")
        .starts_with("actusermessage.message:"));
    assert!(session
        .command("ACTMANAGER WAT")
        .expect_err("bad option")
        .starts_with("actmanager.option:"));
    assert!(session
        .command("ACTMANAGER PLAY nope")
        .expect_err("recording blocks playback")
        .starts_with("actmanager.recording:"));
    let path = temp_dir("codes").join("m.json");
    session
        .command(&format!("ACTSTOP {}", path.to_string_lossy()))
        .expect("save");
    assert!(session
        .command("ACTMANAGER PLAY nope")
        .expect_err("unknown macro")
        .starts_with("actmanager.unknown_macro:"));
}
