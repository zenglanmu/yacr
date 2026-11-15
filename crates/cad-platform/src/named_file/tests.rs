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

#[test]
fn a_write_creates_the_file_with_exactly_the_given_bytes() {
    let dir = temp_dir("write");
    let path = dir.join("table.csv");
    write_named_file(&path, b"a,b\r\n").expect("a writable directory");
    assert_eq!(std::fs::read(&path).expect("written file"), b"a,b\r\n");
    // The temporary file the atomic write used is gone.
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("read dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "temporary debris: {leftovers:?}");
}

#[test]
fn a_write_replaces_an_existing_file_atomically() {
    let dir = temp_dir("replace");
    let path = dir.join("table.csv");
    std::fs::write(&path, b"old content that is longer\r\n").expect("write fixture");
    write_named_file(&path, b"new\r\n").expect("overwrite");
    assert_eq!(std::fs::read(&path).expect("replaced"), b"new\r\n");
}

#[test]
fn a_write_never_creates_a_missing_directory() {
    let dir = temp_dir("nodir");
    let path = dir.join("absent").join("table.csv");
    let error = write_named_file(&path, b"x").expect_err("no directory");
    assert!(
        matches!(error, NamedFileError::Missing(_)),
        "expected Missing, got {error:?}"
    );
}

#[test]
fn a_write_to_a_directory_path_is_refused() {
    let dir = temp_dir("isdir");
    let error = write_named_file(&dir, b"x").expect_err("a directory is not a file");
    assert!(
        matches!(error, NamedFileError::Unreadable(_)),
        "expected Unreadable, got {error:?}"
    );
}

#[test]
fn the_write_size_bound_is_refused_by_name() {
    let dir = temp_dir("bound");
    let path = dir.join("huge.csv");
    let bytes = vec![0u8; NAMED_FILE_WRITE_MAX_BYTES as usize + 1];
    let error = write_named_file(&path, &bytes).expect_err("over the bound");
    assert!(
        matches!(error, NamedFileError::Unreadable(ref reason) if reason.contains("limit")),
        "{error:?}"
    );
    assert!(!path.exists(), "a refused write creates no file");
}
