//! Host-target contract tests for the browser download platform.

use cad_domain::CadError;
use cad_platform::{FileEvent, FileTarget, PlatformFiles, SavePurpose};

use super::*;
use crate::platform::UNSUPPORTED_FILESYSTEM;

fn files() -> BrowserFiles<RecordingSink> {
    BrowserFiles::new(RecordingSink::new())
}

fn target(name: &str) -> FileTarget {
    FileTarget {
        display_name: name.to_string(),
        opaque: format!("download://{name}"),
    }
}

#[test]
fn begin_save_synthesizes_one_download_target_per_call() {
    let files = files();
    files
        .begin_save(SavePurpose::SaveDxf, "drawing.dxf")
        .expect("the browser synthesizes the save target");
    files
        .begin_save(SavePurpose::Plot, "drawing.svg")
        .expect("the browser synthesizes the plot target");

    let events = files.poll();
    assert_eq!(events.len(), 2, "exactly one event per begin_save");
    match &events[0] {
        FileEvent::SaveTarget { purpose, target } => {
            assert_eq!(*purpose, SavePurpose::SaveDxf);
            assert_eq!(target.display_name, "drawing.dxf");
            assert_eq!(target.opaque, "download://drawing.dxf");
        }
        other => panic!("expected a SaveTarget, got {other:?}"),
    }
    match &events[1] {
        FileEvent::SaveTarget { purpose, target } => {
            assert_eq!(*purpose, SavePurpose::Plot);
            assert_eq!(target.display_name, "drawing.svg");
            assert_eq!(target.opaque, "download://drawing.svg");
        }
        other => panic!("expected a SaveTarget, got {other:?}"),
    }
    // The queue is drained exactly once: nothing is replayed to the next poll.
    assert!(files.poll().is_empty());
}

#[test]
fn begin_save_refuses_a_blank_name_instead_of_inventing_one() {
    for blank in ["", "   ", "\t\n"] {
        let files = files();
        let error = files
            .begin_save(SavePurpose::SaveDxf, blank)
            .expect_err("a blank download name is refused");
        match error {
            CadError::InvalidInput(reason) => {
                assert!(reason.contains("default_name"), "{reason}");
                assert!(reason.is_ascii(), "technical reasons stay ASCII: {reason}");
            }
            other => panic!("a blank name is an explicit InvalidInput: {other:?}"),
        }
        assert!(
            files.poll().is_empty(),
            "a refused begin_save queues no fabricated target"
        );
    }
}

#[test]
fn write_atomic_delivers_the_exact_bytes_and_name_to_the_sink() {
    const BYTES: &[u8] = b"0\nSECTION\n0\nEOF\n";
    let files = files();
    let target = target("drawing.dxf");
    files
        .write_atomic(&target, BYTES)
        .expect("the recording sink accepts the write");

    assert_eq!(
        files.sink().published(),
        vec![("drawing.dxf".to_string(), BYTES.to_vec())]
    );
    assert_eq!(
        files.downloads(),
        vec![DownloadRecord {
            name: "drawing.dxf".to_string(),
            len: BYTES.len(),
        }]
    );
}

#[test]
fn a_failing_sink_is_a_real_error_and_records_no_download() {
    struct RefusingSink;
    impl DownloadSink for RefusingSink {
        fn publish(&self, _name: &str, _bytes: &[u8]) -> CadResult<()> {
            Err(CadError::Unsupported("disk-full".to_string()))
        }
    }

    let files = BrowserFiles::new(RefusingSink);
    let error = files
        .write_atomic(&target("drawing.dxf"), b"x")
        .expect_err("a sink refusal must not become a success");
    assert_eq!(error, CadError::Unsupported("disk-full".to_string()));
    assert!(
        files.downloads().is_empty(),
        "a failed publish records nothing"
    );
}

#[test]
fn open_still_refuses_with_the_stable_filesystem_prefix() {
    let files = files();
    let refusals = [
        files.begin_open().unwrap_err(),
        files.begin_open_target(&target("drawing.dwg")).unwrap_err(),
    ];
    for error in refusals {
        let CadError::Unsupported(reason) = error else {
            panic!("a browser open must be an explicit Unsupported refusal");
        };
        assert!(
            reason.starts_with(UNSUPPORTED_FILESYSTEM),
            "the reason must carry the stable prefix: {reason}"
        );
        assert!(
            reason.contains("open_bytes") && reason.contains("picker"),
            "the refusal names the page picker and the bytes entry: {reason}"
        );
        assert!(reason.is_ascii(), "technical reasons stay ASCII: {reason}");
    }
    assert!(
        files.poll().is_empty(),
        "a refused open queues no interaction"
    );
}

#[test]
fn the_source_overwrite_guard_answers_false_and_still_remembers_the_source() {
    let files = files();
    let source = target("drawing.dwg");
    assert!(!files.refuses_source_overwrite(&source));
    files.set_source(Some(&source));
    assert_eq!(files.source(), Some(source.clone()));
    assert!(!files.refuses_source_overwrite(&source));
    files.set_source(None);
    assert_eq!(files.source(), None);
}

#[test]
fn the_download_mime_comes_from_the_extension() {
    assert_eq!(download_mime("drawing.dxf"), "application/dxf");
    assert_eq!(download_mime("PLOT.SVG"), "image/svg+xml");
    assert_eq!(download_mime("drawing.pdf"), "application/pdf");
    assert_eq!(download_mime("drawing.unknown"), "application/octet-stream");
    assert_eq!(download_mime("drawing"), "application/octet-stream");
}
