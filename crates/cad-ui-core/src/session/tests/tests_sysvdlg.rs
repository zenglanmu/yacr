//! `SYSVDLG` command-line contracts: read one value, edit, and the versioned
//! save/restore file. The file is written through the host named-file bridge
//! exactly like the action recorder; the values come from the one `cad-app`
//! reader.

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

fn sysvdlg_session() -> CadSession {
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
        "yacr-sysvdlg-{label}-{}-{nanos}",
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
fn sysvdlg_reads_and_edits_through_the_one_command_path() {
    let session = sysvdlg_session();
    session
        .command("SYSVDLG PDMODE 3")
        .expect("a typed edit is admitted");
    assert_eq!(pdmode(&session), 3);
    session
        .command("SYSVDLG SNAPUNIT 25")
        .expect("session value");
    assert_eq!(session.core.controller.borrow().snap_spacing(), 25.0);
    // A read never changes a value.
    session.command("SYSVDLG PDMODE").expect("read one value");
    assert_eq!(pdmode(&session), 3);
    // An unmodelled name is refused by name.
    let error = session
        .command("SYSVDLG PLOTSTYLE")
        .expect_err("an unmodelled name is refused");
    assert!(error.contains("sysvdlg.unknown_variable"), "{error}");
}

#[test]
fn sysvdlg_saves_and_restores_a_versioned_file() {
    let session = sysvdlg_session();
    let dir = temp_dir("roundtrip");
    let path = dir.join("sysvars.json");
    let path_text = path.to_string_lossy().to_string();
    session.command("SYSVDLG PDMODE 3").expect("header edit");
    session
        .command("SYSVDLG SNAPUNIT 25")
        .expect("session edit");
    session
        .command(&format!("SYSVDLG SAVE {path_text}"))
        .expect("save");
    let text = std::fs::read_to_string(&path).expect("the settings file exists");
    let file: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    assert_eq!(file["format"], "yacr.sysvar");
    assert_eq!(file["version"], 1);
    assert_eq!(file["variables"]["PDMODE"], "3");
    assert_eq!(file["variables"]["SNAPUNIT"], "25");

    session.command("SYSVDLG PDMODE 0").expect("change header");
    session
        .command("SYSVDLG SNAPUNIT 10")
        .expect("change session");
    session
        .command(&format!("SYSVDLG RESTORE {path_text}"))
        .expect("restore");
    assert_eq!(pdmode(&session), 3);
    assert_eq!(session.core.controller.borrow().snap_spacing(), 25.0);
}

#[test]
fn sysvdlg_refuses_unknown_file_names_before_any_write() {
    let session = sysvdlg_session();
    let dir = temp_dir("unknown");
    let path = dir.join("sysvars.json");
    std::fs::write(
        &path,
        r#"{"format":"yacr.sysvar","version":1,"variables":{"PDMODE":"3","PLOTSTYLE":"x"}}"#,
    )
    .expect("write fixture");
    let error = session
        .command(&format!("SYSVDLG RESTORE {}", path.to_string_lossy()))
        .expect_err("an unknown variable refuses the whole restore");
    assert!(error.contains("sysvdlg.unknown_variable"), "{error}");
    assert_eq!(pdmode(&session), 0, "nothing was written");
}

#[test]
fn sysvdlg_refuses_a_non_json_target_and_a_missing_file() {
    let session = sysvdlg_session();
    let dir = temp_dir("refusals");
    let svf = dir.join("settings.svf");
    let error = session
        .command(&format!("SYSVDLG SAVE {}", svf.to_string_lossy()))
        .expect_err("a .svf target is refused");
    assert!(error.contains("sysvdlg.file_extension"), "{error}");
    assert!(!svf.exists(), "no file was written");

    let missing = dir.join("missing.json");
    let error = session
        .command(&format!("SYSVDLG RESTORE {}", missing.to_string_lossy()))
        .expect_err("a missing file is refused");
    assert!(error.contains("sysvdlg.file_missing"), "{error}");

    let bad = dir.join("bad.json");
    std::fs::write(&bad, "not json").expect("write fixture");
    let error = session
        .command(&format!("SYSVDLG RESTORE {}", bad.to_string_lossy()))
        .expect_err("malformed JSON is refused");
    assert!(error.contains("sysvdlg.invalid_file"), "{error}");
    assert_eq!(pdmode(&session), 0);
}
