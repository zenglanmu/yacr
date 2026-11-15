//! The user-named-file channel of the host resource strategy.
//!
//! `HATCHEDIT PATTERNFILE <path.pat>` needs a host read of a path the user
//! typed. The command line is parsed in the session (which is filesystem-free),
//! so the read is a [`HostResources`] decision: the native desktop strategy
//! serves it through `cad_platform::read_named_file`, and a runtime without a
//! filesystem inherits the trait default (`Unavailable`) instead of answering
//! "missing file".

use super::*;
use std::path::PathBuf;

/// A strategy that only answers the named-file channel the way a browser does:
/// the trait default. Fonts/images are the unsupported stub.
struct BrowserLike;

impl HostResources for BrowserLike {
    fn install_fonts(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Ok(())
    }

    fn install_images(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Ok(())
    }
}

fn temp_file(label: &str, text: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "yacr-host-named-file-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("acad.pat");
    std::fs::write(&path, text).expect("write fixture");
    path
}

#[test]
fn the_native_strategy_reads_a_user_named_pattern_file() {
    let text = "*MINE, one pattern\n0, 0,0, 0,.25\n";
    let path = temp_file("native", text);
    let bytes = DesktopResources
        .read_named_file(&path)
        .expect("the desktop strategy reads a regular file");
    assert_eq!(&*bytes, text.as_bytes());
    // The same strategy reports a missing path as missing, not empty.
    let missing = path.with_file_name("absent.pat");
    let error = DesktopResources
        .read_named_file(&missing)
        .expect_err("absent path");
    assert!(
        matches!(error, cad_platform::NamedFileError::Missing(_)),
        "{error:?}"
    );
}

#[test]
fn a_strategy_without_a_filesystem_refuses_the_channel_by_name() {
    let path = temp_file("browser", "*MINE\n0, 0,0, 0,1\n");
    let error = BrowserLike
        .read_named_file(&path)
        .expect_err("no filesystem carrier");
    assert_eq!(error, cad_platform::NamedFileError::Unavailable);
    assert!(error.reason().contains("no filesystem"), "{error}");
}
