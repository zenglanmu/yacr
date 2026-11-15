//! `HATCHEDIT PATTERNFILE <path.pat> [<name>]` — the **host read** half.
//!
//! The command never touches the filesystem: the typed line is parsed in this
//! session, the content is requested from the host resource strategy
//! (`cad_host::HostResources::read_named_file`), and the payload carries the
//! text. These tests pin every read outcome — a real file, a missing path, a
//! non-UTF-8 file, a runtime without a filesystem, and the Viewer gate that runs
//! **before** any read — and that a refusal never dispatches a command.

use super::super::super::*;
use super::super::test_support::*;

use cad_app::AppMode;
use cad_host::HostResources;
use cad_platform::NamedFileError;
use std::path::{Path, PathBuf};

/// A host resource strategy that serves a **real** filesystem through the
/// platform carrier (the desktop behaviour, without a render/GPU host).
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
}

fn session_with_native_files() -> CadSession {
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
        "yacr-hatchedit-patternfile-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// The session's recorded command-log length, i.e. whether the selector was
/// echoed (a pre-dispatch refusal echoes nothing).
fn logged(session: &CadSession) -> usize {
    session.snapshot(None, None, 0).command_log.len()
}

#[test]
fn a_real_pat_file_is_read_by_the_host_and_dispatched() {
    let dir = temp_dir("ok");
    let path = dir.join("acad.pat");
    std::fs::write(&path, "*MINE, one\n0, 0,0, 0,1\n").expect("write fixture");
    let session = session_with_native_files();
    // The selection is empty, so the command refuses *after* the read: the
    // refusal proves the read succeeded (a read failure has its own code) and
    // that the payload reached the dispatcher.
    let error = session
        .command(&format!("HATCHEDIT PATTERNFILE {}", path.display()))
        .expect_err("no selection");
    assert!(error.contains("hatchedit.selection"), "{error}");
    assert!(
        !error.contains("pattern_file"),
        "the file was read: {error}"
    );
    assert!(logged(&session) > 0, "the selector was echoed");
}

#[test]
fn a_missing_file_is_refused_by_name_before_dispatch() {
    let dir = temp_dir("missing");
    let path = dir.join("absent.pat");
    let session = session_with_native_files();
    let error = session
        .command(&format!("HATCHEDIT PATTERNFILE {}", path.display()))
        .expect_err("a missing file is refused");
    assert!(
        error.starts_with("hatchedit.pattern_file_missing:"),
        "{error}"
    );
    assert!(error.contains("absent.pat"), "the path is named: {error}");
    assert_eq!(logged(&session), 0, "nothing was dispatched");
}

#[test]
fn a_directory_is_refused_as_unreadable() {
    let dir = temp_dir("directory");
    let session = session_with_native_files();
    let error = session
        .command(&format!("HATCHEDIT PATTERNFILE {}", dir.display()))
        .expect_err("a directory is not a pattern file");
    assert!(
        error.starts_with("hatchedit.pattern_file_unreadable:"),
        "{error}"
    );
    assert_eq!(logged(&session), 0, "nothing was dispatched");
}

#[test]
fn a_non_utf8_file_is_refused_as_encoding() {
    let dir = temp_dir("encoding");
    let path = dir.join("binary.pat");
    std::fs::write(&path, [0xff_u8, 0xfe, 0x00, 0x9f]).expect("write fixture");
    let session = session_with_native_files();
    let error = session
        .command(&format!("HATCHEDIT PATTERNFILE {}", path.display()))
        .expect_err("binary is not a pattern library");
    assert!(
        error.starts_with("hatchedit.pattern_file_encoding:"),
        "{error}"
    );
    assert_eq!(logged(&session), 0, "nothing was dispatched");
}

#[test]
fn a_host_without_a_filesystem_refuses_with_the_stable_marker() {
    // `FakeResources` inherits the trait default: the browser's answer.
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let error = session
        .command("HATCHEDIT PATTERNFILE /tmp/acad.pat")
        .expect_err("no filesystem");
    assert!(
        error.starts_with("hatchedit.pattern_file_unavailable:"),
        "{error}"
    );
    assert!(
        error.contains("unsupported-runtime: pattern-file"),
        "the stable capability marker is present: {error}"
    );
    assert_eq!(logged(&session), 0, "nothing was dispatched");
}

#[test]
fn viewer_refuses_before_the_host_read() {
    // The path does not exist: if the read ran first, the error would be
    // `pattern_file_missing`. The Viewer gate must win, so a command that cannot
    // write performs no host I/O.
    let session = session_with_native_files();
    session
        .core
        .controller
        .borrow_mut()
        .session
        .switch_mode(AppMode::Viewer)
        .expect("switch to Viewer");
    let error = session
        .command("HATCHEDIT PATTERNFILE /nonexistent/acad.pat")
        .expect_err("Viewer refuses");
    assert!(!error.contains("pattern_file"), "{error}");
    assert_eq!(logged(&session), 0, "nothing was dispatched");
}

#[test]
fn a_malformed_patternfile_line_is_refused_by_the_grammar() {
    let session = session_with_native_files();
    for line in ["HATCHEDIT PATTERNFILE", "HATCHEDIT PATTERNFILE a.pat b c"] {
        let error = session.command(line).expect_err("malformed");
        assert!(error.contains("HATCHEDIT needs STYLE"), "{line}: {error}");
    }
    // A path with a space cannot be told from the pattern name. A second token
    // that is itself a path is refused by name (`pattern_file_path`) instead of
    // reading the first word as a path and the rest as a name.
    let error = session
        .command("HATCHEDIT PATTERNFILE /tmp/my patterns/acad.pat")
        .expect_err("a spaced path is refused by name");
    assert!(error.starts_with("hatchedit.pattern_file_path:"), "{error}");
    assert!(
        error.contains("cannot be split from the name"),
        "the rule is stated: {error}"
    );
    assert_eq!(logged(&session), 0, "nothing was dispatched");
}
