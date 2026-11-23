//! `EXPORT <TOKEN>` (ADR-0052): the typed form of the one `EXPORT` selector.
//!
//! The grammar is pinned without a host; the admission/dispatch half runs against
//! a recording save platform that opens the picker and never resolves it, which is
//! exactly what a desktop dialog does while the user chooses a destination.

use super::super::super::*;
use super::super::test_support::*;

use crate::session::commands::export_argument::{
    parse_export_argument, EXPORT_FORMAT_REASON_PREFIX,
};
use crate::session::commands::{ARGUMENT_SELECTORS, EXPORT_SELECTOR};

/// A save platform that records the picker's suggested name and never resolves
/// it (the target the user would choose arrives on a later tick).
struct PickerFiles {
    calls: RefCell<Vec<(SavePurpose, String)>>,
}

impl PickerFiles {
    fn new() -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl PlatformFiles for PickerFiles {
    fn begin_open(&self) -> CadResult<()> {
        Ok(())
    }
    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Ok(())
    }
    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        self.calls
            .borrow_mut()
            .push((purpose, default_name.to_string()));
        Ok(())
    }
    fn write_atomic(&self, _target: &FileTarget, _bytes: &[u8]) -> CadResult<()> {
        Err(CadError::Unsupported(
            "this fixture never resolves a target".into(),
        ))
    }
    fn set_source(&self, _target: Option<&FileTarget>) {}
    fn poll(&self) -> Vec<FileEvent> {
        Vec::new()
    }
}

/// A session with the save capability enabled over a picker that stays open.
fn picker_session(files: Rc<PickerFiles>) -> CadSession {
    let mut host_platform = platform(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    host_platform.files = files;
    host_platform.capabilities.save = true;
    session_with_platform(host_platform)
}

#[test]
fn the_export_selector_is_an_admitted_argument_form() {
    assert!(
        ARGUMENT_SELECTORS.contains(&EXPORT_SELECTOR),
        "EXPORT <TOKEN> must be an admitted argument form"
    );
    let spec = parse_export_argument("glb").expect("a case-insensitive token");
    assert_eq!((spec.format_id, spec.extension), ("glb", "glb"));
    assert_eq!(
        parse_export_argument(" coff ").map(|spec| spec.extension),
        Some("coff"),
        "the token is trimmed and COFF keeps its own `.coff` spelling"
    );
    // A missing, multi-word or unknown token is not an argument.
    assert!(parse_export_argument("").is_none());
    assert!(parse_export_argument("GLB extra").is_none());
    assert!(parse_export_argument("ZIP").is_none());
}

#[test]
fn typed_export_tokens_fix_the_writer_and_the_suggested_name() {
    // The demo document's hint is `yacr-demo`, so the suggested name is that
    // stem with the token's own extension.
    for (line, expected) in [
        ("EXPORT GLB", "yacr-demo.glb"),
        ("EXPORT gltf", "yacr-demo.gltf"),
        ("EXPORT COFF", "yacr-demo.coff"),
        ("EXPORT DWG", "yacr-demo.dwg"),
    ] {
        let files = Rc::new(PickerFiles::new());
        let session = picker_session(files.clone());
        session.command(line).expect("an admitted typed export");
        let calls = files.calls.borrow();
        assert_eq!(calls.len(), 1, "{line}: exactly one picker");
        assert_eq!(calls[0].0, SavePurpose::SaveExport, "{line}");
        assert_eq!(calls[0].1, expected, "{line}: suggested name");
    }
}

#[test]
fn an_unknown_export_token_is_refused_before_the_host() {
    let files = Rc::new(PickerFiles::new());
    let session = picker_session(files.clone());
    let error = session.command("EXPORT ZIP").expect_err("ZIP is refused");
    assert!(
        error.starts_with(EXPORT_FORMAT_REASON_PREFIX),
        "the stable marker leads the reason: {error}"
    );
    assert!(
        files.calls.borrow().is_empty(),
        "a refused token never opens the picker"
    );
    assert!(
        !session.core.save_pending(),
        "a refused token leaves no save interaction pending"
    );
    assert_eq!(*session.error.borrow(), Some(error));
}

#[test]
fn a_bare_export_still_opens_the_picker() {
    let files = Rc::new(PickerFiles::new());
    let session = picker_session(files.clone());
    session
        .command("EXPORT")
        .expect("the bare command dispatches");
    let calls = files.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, SavePurpose::SaveExport);
    // The demo document never came from a file, so the source-derived default
    // falls back to `.dxf`.
    assert_eq!(calls[0].1, "yacr-demo.dxf");
}
