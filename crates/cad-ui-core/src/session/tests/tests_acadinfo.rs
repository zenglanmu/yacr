//! `ACADINFO <file.json>`: the versioned information file through the host
//! named-file bridge. The facts are the same `cad-app` snapshot the bare report
//! prints; a host without a sink refuses by name instead of claiming a file.

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

fn acadinfo_session() -> CadSession {
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
        "yacr-acadinfo-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn acadinfo_writes_the_versioned_information_file() {
    let session = acadinfo_session();
    let dir = temp_dir("write");
    let path = dir.join("acadinfo.json");
    session
        .command(&format!("ACADINFO {}", path.to_string_lossy()))
        .expect("ACADINFO writes the information file");
    let text = std::fs::read_to_string(&path).expect("the file exists");
    let file: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    assert_eq!(file["format"], "yacr.acadinfo");
    assert_eq!(file["version"], 1);
    assert_eq!(file["product"]["name"], "yacr");
    assert_eq!(file["product"]["component"], "cad-app");
    assert_eq!(file["document"]["units"], "drawing units");
    assert_eq!(file["document"]["decimalPlaces"], 3);
    assert_eq!(file["notCollected"][0], "licence");
    assert_eq!(file["notCollected"][1], "hardware");
    assert_eq!(file["notCollected"][2], "installedModules");
    assert!(
        file["document"].get("createdAt").is_none(),
        "no fabricated creation timestamp"
    );
}

#[test]
fn acadinfo_refuses_a_non_json_target_and_a_host_without_a_sink() {
    let session = acadinfo_session();
    let dir = temp_dir("refusals");
    let txt = dir.join("acadinfo.txt");
    let error = session
        .command(&format!("ACADINFO {}", txt.to_string_lossy()))
        .expect_err("a non-.json target is refused");
    assert!(error.contains("acadinfo.path"), "{error}");
    assert!(!txt.exists(), "no file was written");

    let unsupported = session_with_platform(platform(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    })));
    let path = dir.join("unsupported.json");
    let error = unsupported
        .command(&format!("ACADINFO {}", path.to_string_lossy()))
        .expect_err("a host without a named-file sink refuses");
    assert!(error.contains("acadinfo.write_unavailable"), "{error}");
    assert!(error.contains("unsupported-runtime"), "{error}");
    assert!(!path.exists(), "no file was written");
}
