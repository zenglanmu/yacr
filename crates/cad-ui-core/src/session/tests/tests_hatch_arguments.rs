//! `HATCH PATTERN <name>` / `HATCH PATTERNFILE <path.pat> [<name>]` (`-HATCH`
//! included): grammar, dispatch payload and the pre-IPC refusals.
//!
//! The behaviour of the created hatch (stored name/type/scale/angle, one undo
//! step) is pinned in `cad-app`; these tests pin what the session decides —
//! whether the line is admitted, which payload it produces and that every refusal
//! happens **before** IPC (nothing dispatched, nothing read).

use super::super::*;
use super::test_support::*;

use cad_app::command_id::hatch::HatchPatternRequest;
use cad_app::{AppMode, CommandPayload, PatternFileText};
use cad_host::HostResources;
use cad_platform::NamedFileError;
use std::path::{Path, PathBuf};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

/// A host resource strategy that serves a **real** filesystem (the desktop
/// behaviour, without a render/GPU host).
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
        "yacr-hatch-patternfile-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn selection() -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn select_one(session: &CadSession) {
    session
        .core
        .controller
        .borrow_mut()
        .set_selection(vec![selection()])
        .expect("the selection");
}

/// The recorded command-log length: a pre-dispatch refusal echoes nothing.
fn logged(session: &CadSession) -> usize {
    session.snapshot(None, None, 0).command_log.len()
}

#[test]
fn the_hatch_grammar_is_the_documented_one() {
    assert_eq!(
        parse_hatch_argument("PATTERN ANSI31"),
        Ok(HatchPatternArgument::Pattern("ANSI31".into()))
    );
    assert_eq!(
        parse_hatch_argument("pattern ansi31"),
        Ok(HatchPatternArgument::Pattern("ansi31".into()))
    );
    assert_eq!(
        parse_hatch_argument("PATTERNFILE /tmp/acad.pat"),
        Ok(HatchPatternArgument::File {
            path: "/tmp/acad.pat".into(),
            name: None,
        })
    );
    assert_eq!(
        parse_hatch_argument("patternfile C:/data/acadiso.pat ladder"),
        Ok(HatchPatternArgument::File {
            path: "C:/data/acadiso.pat".into(),
            name: Some("ladder".into()),
        })
    );
    // The two words that name a **different fill kind** parse into their own
    // named refusal rather than a malformed line.
    assert_eq!(
        parse_hatch_argument("SOLID"),
        Err(HatchArgumentError::FillNotPattern("SOLID"))
    );
    assert_eq!(
        parse_hatch_argument("gradient"),
        Err(HatchArgumentError::FillNotPattern("GRADIENT"))
    );
    // A bare PATTERN has its own named refusal, not a malformed line.
    assert_eq!(
        parse_hatch_argument("PATTERN"),
        Err(HatchArgumentError::MissingPatternName)
    );
    // An unknown built-in name is answered with the implemented set.
    assert_eq!(
        parse_hatch_argument("PATTERN NOPE"),
        Err(HatchArgumentError::UnknownPattern("NOPE".into()))
    );
    // A spaced path is name-refused (the grammar cannot tell it from the name).
    assert_eq!(
        parse_hatch_argument("PATTERNFILE a.pat dir/name"),
        Err(HatchArgumentError::PatternFilePath)
    );
    // Everything else is malformed: no option stack, no partial word, no third
    // token, no value after SOLID/GRADIENT.
    for refused in [
        "",
        "ANSI31",
        "PATTERN a b",
        "PATTERNFILE",
        "PATTERNFILE a.pat b c",
        "SOLID 1",
        "GRADIENT ON",
        "PAT ANSI31",
        "PATTERNTYPE 2",
    ] {
        assert_eq!(
            parse_hatch_argument(refused),
            Err(HatchArgumentError::Malformed),
            "{refused:?}"
        );
    }
}

#[test]
fn every_named_refusal_answers_with_its_stable_code() {
    for (error, code) in [
        (
            HatchArgumentError::MissingPatternName,
            "hatch.needs_pattern_name",
        ),
        (
            HatchArgumentError::UnknownPattern("NOPE".into()),
            "hatch.unknown_pattern",
        ),
        (
            HatchArgumentError::PatternFilePath,
            "hatch.pattern_file_path",
        ),
        (
            HatchArgumentError::FillNotPattern("SOLID"),
            "hatch.fill_not_pattern",
        ),
        (
            HatchArgumentError::FileNotRead,
            "hatch.pattern_file_not_read",
        ),
    ] {
        let text = hatch_argument_refusal(error.clone(), "typed line");
        assert!(text.starts_with(code), "{error:?}: {text}");
    }
    // The implemented built-in table is the one name authority of the message.
    let names = hatch_unknown_pattern_reason("NOPE");
    for name in ["ANSI31", "ANSI32", "ANSI37"] {
        assert!(names.contains(name), "{names}");
    }
}

#[test]
fn the_typed_line_dispatches_the_exact_hatch_pattern_payload() {
    let refs = vec![selection()];
    // `PATTERN <name>`: a built-in request on the HatchPattern payload.
    let payload = hatch_pattern_payload(
        HatchPatternArgument::Pattern("ANSI31".into()),
        refs.clone(),
        None,
    )
    .expect("a named request needs no file");
    let CommandPayload::HatchPattern {
        refs: sent,
        request,
    } = payload
    else {
        panic!("HATCH PATTERN must dispatch HatchPattern");
    };
    assert_eq!(sent, refs);
    assert_eq!(request, HatchPatternRequest::Named("ANSI31".into()));
    // `PATTERNFILE`: the host-read text travels in the request, so the command
    // never opens a file itself.
    let text = PatternFileText::new("*MINE, one\n0, 0,0, 0,1\n");
    let payload = hatch_pattern_payload(
        HatchPatternArgument::File {
            path: "/tmp/acad.pat".into(),
            name: Some("MINE".into()),
        },
        refs.clone(),
        Some(text),
    )
    .expect("a file request with content");
    let CommandPayload::HatchPattern {
        refs: sent,
        request,
    } = payload
    else {
        panic!("HATCH PATTERNFILE must dispatch HatchPattern");
    };
    assert_eq!(sent, refs);
    let HatchPatternRequest::File { path, name, text } = request else {
        panic!("the request must be the file form");
    };
    assert_eq!(path, "/tmp/acad.pat");
    assert_eq!(name.as_deref(), Some("MINE"));
    assert_eq!(text.as_str(), "*MINE, one\n0, 0,0, 0,1\n");
    // A file request without the host text is refused, never an empty success.
    assert_eq!(
        hatch_pattern_payload(
            HatchPatternArgument::File {
                path: "/tmp/acad.pat".into(),
                name: None,
            },
            refs,
            None,
        )
        .err(),
        Some(HatchArgumentError::FileNotRead)
    );
}

#[test]
fn a_bare_pattern_name_line_reaches_the_dispatcher_and_echoes_the_selector() {
    let session = fresh_session();
    select_one(&session);
    // The session has no open document, so the command layer refuses the
    // *document* (not the line): the grammar, the authorize gate, the selection
    // and the payload were all admitted first.
    // The command layer ran with the payload and refused the *boundary* the
    // fixture selects (a single open LINE is not a closed loop), so the line,
    // the authorize gate, the selection and the payload were all admitted.
    let error = session
        .command("HATCH PATTERN ANSI31")
        .expect_err("the fixture boundary is open");
    assert!(error.contains("boundary"), "{error}");
    assert!(!error.contains("hatch.pattern_"), "{error}");
    assert!(!error.contains("hatch.unknown_pattern"), "{error}");
    assert!(logged(&session) > 0, "the selector was echoed");

    // An unknown name never dispatches.
    let session = fresh_session();
    select_one(&session);
    let error = session
        .command("HATCH PATTERN NOPE")
        .expect_err("unknown name");
    assert!(error.starts_with("hatch.unknown_pattern:"), "{error}");
    assert_eq!(logged(&session), 0, "nothing was dispatched");

    // A spaced path, a missing name and the fill-kind words are named refusals
    // too, each before IPC.
    for (line, code) in [
        ("HATCH PATTERN", "hatch.needs_pattern_name"),
        (
            "HATCH PATTERNFILE a.pat dir/name",
            "hatch.pattern_file_path",
        ),
        ("HATCH SOLID", "hatch.fill_not_pattern"),
        ("HATCH GRADIENT", "hatch.fill_not_pattern"),
        ("HATCH ANSI31", "unsupported CAD command"),
    ] {
        let session = fresh_session();
        select_one(&session);
        let error = session.command(line).expect_err("refused");
        assert!(error.starts_with(code), "{line}: {error}");
        assert_eq!(logged(&session), 0, "{line}: nothing was dispatched");
    }
}

#[test]
fn an_empty_selection_is_refused_before_any_dispatch() {
    let session = fresh_session();
    let error = session
        .command("HATCH PATTERN ANSI31")
        .expect_err("no boundary");
    assert!(error.starts_with("hatch.selection:"), "{error}");
    assert_eq!(logged(&session), 0, "nothing was dispatched");
}

#[test]
fn dash_hatch_accepts_the_same_typed_forms() {
    let session = fresh_session();
    select_one(&session);
    let error = session
        .command("-HATCH PATTERN ANSI32")
        .expect_err("the fixture boundary is open");
    assert!(error.contains("boundary"), "{error}");
    assert!(logged(&session) > 0, "the -HATCH selector was echoed");
    // The same grammar refuses the same way.
    let session = fresh_session();
    let error = session
        .command("-HATCH PATTERN NOPE")
        .expect_err("unknown name");
    assert!(error.starts_with("hatch.unknown_pattern:"), "{error}");
}

#[test]
fn a_viewer_session_refuses_the_typed_line_before_the_host_read() {
    let session = fresh_session();
    select_one(&session);
    session
        .core
        .controller
        .borrow_mut()
        .set_mode(AppMode::Viewer)
        .expect("switch to Viewer");
    // The path does not exist: if the read ran, the refusal would be
    // `hatch.pattern_file_missing` instead of the permission refusal.
    let error = session
        .command("HATCH PATTERNFILE /nonexistent/acad.pat")
        .expect_err("Viewer must refuse");
    assert!(!error.contains("pattern_file"), "{error}");
    assert!(
        error.to_ascii_lowercase().contains("viewer")
            || error.to_ascii_lowercase().contains("permission")
            || error.contains("mode"),
        "the command layer's own refusal: {error}"
    );
    assert_eq!(logged(&session), 0, "nothing was dispatched");
}

#[test]
fn a_real_pat_file_is_read_by_the_host_and_dispatched() {
    let dir = temp_dir("ok");
    let path = dir.join("acad.pat");
    std::fs::write(&path, "*MINE, one\n0, 0,0, 0,1\n").expect("write fixture");
    let session = session_with_native_files();
    select_one(&session);
    let error = session
        .command(&format!("HATCH PATTERNFILE {}", path.display()))
        .expect_err("the fixture boundary is open");
    assert!(
        !error.contains("pattern_file"),
        "the file was read: {error}"
    );
    assert!(error.contains("boundary"), "{error}");
    assert!(logged(&session) > 0, "the selector was echoed");
}

#[test]
fn a_missing_file_is_refused_by_name_before_dispatch() {
    let dir = temp_dir("missing");
    let path = dir.join("absent.pat");
    let session = session_with_native_files();
    select_one(&session);
    let error = session
        .command(&format!("HATCH PATTERNFILE {}", path.display()))
        .expect_err("a missing file is refused");
    assert!(error.starts_with("hatch.pattern_file_missing:"), "{error}");
    assert!(error.contains("absent.pat"), "the path is named: {error}");
    assert_eq!(logged(&session), 0, "nothing was dispatched");
}

#[test]
fn a_directory_is_refused_as_unreadable() {
    let dir = temp_dir("directory");
    let session = session_with_native_files();
    select_one(&session);
    let error = session
        .command(&format!("HATCH PATTERNFILE {}", dir.display()))
        .expect_err("a directory is not a pattern file");
    assert!(
        error.starts_with("hatch.pattern_file_unreadable:"),
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
    select_one(&session);
    let error = session
        .command(&format!("HATCH PATTERNFILE {}", path.display()))
        .expect_err("binary is not a pattern library");
    assert!(error.starts_with("hatch.pattern_file_encoding:"), "{error}");
    assert_eq!(logged(&session), 0, "nothing was dispatched");
}

#[test]
fn a_host_without_a_filesystem_refuses_the_file_form_by_name() {
    // The default fixture host has no `read_named_file`, i.e. the browser /
    // wasm-shaped carrier: a stable runtime refusal, not an invented success.
    let session = fresh_session();
    select_one(&session);
    let error = session
        .command("HATCH PATTERNFILE /tmp/acad.pat")
        .expect_err("no filesystem");
    assert!(
        error.starts_with("hatch.pattern_file_unavailable:"),
        "{error}"
    );
    assert!(
        error.contains("unsupported-runtime: pattern-file"),
        "{error}"
    );
    assert_eq!(logged(&session), 0, "nothing was dispatched");
}
