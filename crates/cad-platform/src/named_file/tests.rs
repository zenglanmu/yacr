//! The native user-named-file carrier: a regular file, a missing path, a
//! directory and the size bound.

use super::*;
use std::path::PathBuf;

fn temp_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "yacr-named-file-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn a_regular_file_is_read_verbatim() {
    let dir = temp_dir("ok");
    let path = dir.join("acad.pat");
    let text = "*MINE, a test pattern\n0, 0,0, 0,1\n";
    std::fs::write(&path, text).expect("write fixture");
    let bytes = read_named_file(&path).expect("a regular file is read");
    assert_eq!(&*bytes, text.as_bytes());
}

#[test]
fn a_missing_path_is_missing_not_unreadable() {
    let dir = temp_dir("missing");
    let error = read_named_file(&dir.join("nope.pat")).expect_err("absent file");
    assert!(
        matches!(error, NamedFileError::Missing(_)),
        "expected Missing, got {error:?}"
    );
    assert!(error.reason().contains("no such file"), "{error}");
}

#[test]
fn a_directory_is_not_a_named_file() {
    let dir = temp_dir("directory");
    let error = read_named_file(&dir).expect_err("a directory is not a file");
    assert!(
        matches!(error, NamedFileError::Unreadable(_)),
        "expected Unreadable, got {error:?}"
    );
    assert!(error.reason().contains("not a regular file"), "{error}");
}

#[test]
fn a_file_over_the_bound_is_refused_before_it_is_read() {
    let dir = temp_dir("budget");
    let path = dir.join("huge.pat");
    let bytes = vec![b';'; NAMED_FILE_MAX_BYTES as usize + 1];
    std::fs::write(&path, bytes).expect("write fixture");
    let error = read_named_file(&path).expect_err("over the bound");
    assert!(
        matches!(error, NamedFileError::Unreadable(_)),
        "expected Unreadable, got {error:?}"
    );
    assert!(
        error.reason().contains("bytes; the limit"),
        "the bound is named: {error}"
    );
}
