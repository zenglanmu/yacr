//! Lexical path policy for external references (XREFs).
//!
//! Spec v2.0 §7.3, §16.2: a crafted drawing must not be able to direct the
//! application outside the drawing directory and the explicitly granted roots.
//! This module owns the **lexical** decision only: it never touches the
//! filesystem, never decodes a URL and never canonicalises. Hosts remain the
//! only layer allowed to perform I/O.
//!
//! The policy is deliberately conservative:
//!
//! * Any `..` segment is refused, even when joining it would still stay inside
//!   a granted root. A drawing cannot use ascent to probe sibling layouts, and
//!   the rule is independent of the current root list.
//! * URL schemes (including `file:`) and UNC paths are refused before any
//!   folding, so a platform-specific interpretation cannot sneak them past the
//!   lexical checks. URL-encoded spellings are not decoded: `%2e%2e` is a file
//!   name, not traversal, and the containment check still applies.
//! * Windows drive specifications are only meaningful on Windows hosts.
//!
//! # Symlinks
//!
//! These checks are purely lexical. After a host resolves an allowed path it
//! **must** canonicalise it and re-check that the real path is still under the
//! drawing directory or a granted root. Only that second check can stop a
//! symlink inside an allowed directory from escaping it. Never canonicalise in
//! this module: that would require I/O and would make the policy host-bound.

use std::path::{Path, PathBuf};

/// Stable reason code: the reference is empty (or only quotes/`unnamed`).
pub const EMPTY: &str = "xref.path.empty";
/// Stable reason code: the reference contains a NUL or another control character.
pub const CONTROL: &str = "xref.path.control";
/// Stable reason code: the reference starts with a URL scheme (including `file:`).
pub const URL: &str = "xref.path.url";
/// Stable reason code: the reference is a UNC path (`\\host\share` or `//host/share`).
pub const UNC: &str = "xref.path.unc";
/// Stable reason code: the reference contains a `..` segment.
pub const TRAVERSAL: &str = "xref.path.traversal";
/// Stable reason code: the reference is a Windows drive path on a non-Windows host.
pub const WINDOWS_DRIVE: &str = "xref.path.windows_drive";
/// Stable reason code: an absolute path (or Windows drive path) is outside every allowed root.
pub const ABSOLUTE_OUTSIDE: &str = "xref.path.absolute_outside";
/// Stable reason code: a relative reference was given without a drawing directory.
pub const NO_DRAWING_DIR: &str = "xref.path.no_drawing_dir";

/// Outcome of [`check_xref_path`].
///
/// A refusal carries one of the stable `xref.path.*` codes above. Hosts record
/// the code verbatim in external-reference provenance; they must not translate
/// it into user-facing prose at this layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XrefPathVerdict {
    /// The reference is lexically allowed; `normalized` is the joined,
    /// separator-folded path (no `.`/`..` segments, no repeated separators).
    ///
    /// The host must still canonicalise before reading (see the module docs).
    Allowed { normalized: PathBuf },
    /// The reference is refused; `reason` is a stable `xref.path.*` code.
    Refused { reason: &'static str },
}

/// Check one external-reference path against the drawing directory and roots.
///
/// Order of checks, with the stable code each returns:
///
/// 1. Surrounding whitespace and matching single/double quotes are trimmed;
///    an empty result is [`EMPTY`]. The bare placeholder `unnamed` (the
///    resolver's sentinel) and a pathless `.` are also [`EMPTY`], because
///    neither names a file.
/// 2. NUL or any other control character is [`CONTROL`].
/// 3. A leading URI scheme, `^[A-Za-z][A-Za-z0-9+.-]*:`, is [`URL`]. A
///    single-letter scheme is treated as a Windows drive spec instead (next
///    steps) so `C:\...` is reported as [`WINDOWS_DRIVE`], not [`URL`]. The
///    spelling is **not** URL-decoded.
/// 4. `\\host\share` and `//host/share` are [`UNC`].
/// 5. `\` folds to `/`, repeated separators collapse, and `.` segments drop.
/// 6. Any `..` segment is [`TRAVERSAL`], even if it would stay inside a root.
/// 7. A Windows drive spec is [`WINDOWS_DRIVE`] on non-Windows hosts. On
///    Windows, `C:/...` is allowed only when the resulting path is under one of
///    `roots` (otherwise [`ABSOLUTE_OUTSIDE`]); a drive-relative `C:foo` cannot
///    be anchored lexically and is also [`ABSOLUTE_OUTSIDE`].
/// 8. An absolute Unix path is allowed only when it is lexically under
///    `drawing_dir` or one of `roots` (otherwise [`ABSOLUTE_OUTSIDE`]).
/// 9. A relative path is joined under `drawing_dir`; an empty `drawing_dir` is
///    [`NO_DRAWING_DIR`]. The joined path must be under `drawing_dir` or a
///    root (otherwise [`ABSOLUTE_OUTSIDE`]).
///
/// `roots` are expected to be absolute and free of `.`/`..` components; this
/// function does not resolve them either.
pub fn check_xref_path(raw: &str, drawing_dir: &Path, roots: &[PathBuf]) -> XrefPathVerdict {
    let trimmed = trim_quotes(raw);

    if trimmed.is_empty() {
        return refused(EMPTY);
    }
    if trimmed.chars().any(|c| c == '\0' || c.is_control()) {
        return refused(CONTROL);
    }
    if trimmed.eq_ignore_ascii_case("unnamed") || trimmed == "." {
        return refused(EMPTY);
    }

    // A leading URI scheme. A single ASCII letter followed by `:` is a Windows
    // drive spec, handled below, not a scheme.
    if let Some(colon) = trimmed.find(':') {
        let scheme = &trimmed[..colon];
        if scheme.len() > 1 && is_url_scheme(scheme) {
            return refused(URL);
        }
    }

    // UNC spellings are platform-independent hazards; refuse both before the
    // separator fold could turn one into the other.
    if trimmed.starts_with("\\\\") || trimmed.starts_with("//") {
        return refused(UNC);
    }

    let folded = trimmed.replace('\\', "/");
    let drive = is_windows_drive(&folded);

    let mut segments: Vec<&str> = Vec::new();
    for segment in folded.split('/') {
        match segment {
            "" | "." => continue,
            ".." => return refused(TRAVERSAL),
            other => segments.push(other),
        }
    }
    if segments.is_empty() {
        // `./`, `foo/..`-free but nothing left, or a root with no name.
        return refused(EMPTY);
    }
    if segments.len() == 1 && segments[0].eq_ignore_ascii_case("unnamed") {
        return refused(EMPTY);
    }

    let joined = segments.join("/");

    if drive {
        if !cfg!(windows) {
            return refused(WINDOWS_DRIVE);
        }
        // On Windows a drive path must be absolute (`C:/...`) and anchored under
        // an explicit root. `C:foo` is drive-relative: its base directory is
        // process state, so it cannot be checked lexically and is refused.
        if folded.as_bytes().get(2) != Some(&b'/') {
            return refused(ABSOLUTE_OUTSIDE);
        }
        let candidate = PathBuf::from(&joined);
        if roots.iter().any(|root| is_under(&candidate, root)) {
            return XrefPathVerdict::Allowed {
                normalized: candidate,
            };
        }
        return refused(ABSOLUTE_OUTSIDE);
    }

    let absolute = folded.starts_with('/');
    let candidate = if absolute {
        Path::new("/").join(&joined)
    } else {
        if drawing_dir.as_os_str().is_empty() {
            return refused(NO_DRAWING_DIR);
        }
        drawing_dir.join(&joined)
    };

    let inside = is_under(&candidate, drawing_dir) || roots.iter().any(|r| is_under(&candidate, r));
    if inside {
        XrefPathVerdict::Allowed {
            normalized: candidate,
        }
    } else {
        refused(ABSOLUTE_OUTSIDE)
    }
}

fn refused(reason: &'static str) -> XrefPathVerdict {
    XrefPathVerdict::Refused { reason }
}

/// Trim whitespace and any matching pair of surrounding quotes.
fn trim_quotes(raw: &str) -> &str {
    let mut trimmed = raw.trim();
    loop {
        let bytes = trimmed.as_bytes();
        if bytes.len() >= 2 {
            let quote = bytes[0];
            if (quote == b'"' || quote == b'\'') && bytes[bytes.len() - 1] == quote {
                // Both boundaries are ASCII quote bytes, so slicing is UTF-8 safe.
                trimmed = trimmed[1..trimmed.len() - 1].trim();
                continue;
            }
        }
        return trimmed;
    }
}

/// `^[A-Za-z][A-Za-z0-9+.-]*$`
fn is_url_scheme(scheme: &str) -> bool {
    let mut chars = scheme.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}

/// `^[A-Za-z]:` on the already-folded spelling.
fn is_windows_drive(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Lexical containment. An empty root cannot contain anything.
fn is_under(candidate: &Path, root: &Path) -> bool {
    !root.as_os_str().is_empty() && candidate.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawing() -> PathBuf {
        PathBuf::from("/work/drawings")
    }

    fn roots() -> Vec<PathBuf> {
        vec![PathBuf::from("/work/library")]
    }

    fn allowed(raw: &str, drawing_dir: &Path) -> PathBuf {
        match check_xref_path(raw, drawing_dir, &roots()) {
            XrefPathVerdict::Allowed { normalized } => normalized,
            XrefPathVerdict::Refused { reason } => {
                panic!("expected '{raw}' to be allowed, refused with {reason}")
            }
        }
    }

    fn refused_reason(raw: &str, drawing_dir: &Path) -> &'static str {
        match check_xref_path(raw, drawing_dir, &roots()) {
            XrefPathVerdict::Refused { reason } => reason,
            XrefPathVerdict::Allowed { normalized } => {
                panic!("expected '{raw}' to be refused, allowed as {normalized:?}")
            }
        }
    }

    #[test]
    fn mixed_separators_are_folded_under_the_drawing_directory() {
        assert_eq!(
            allowed(r"xref\base.dwg", &drawing()),
            PathBuf::from("/work/drawings/xref/base.dwg")
        );
        assert_eq!(
            allowed(r"xref/base\detail\a.dwg", &drawing()),
            PathBuf::from("/work/drawings/xref/base/detail/a.dwg")
        );
    }

    #[test]
    fn surrounding_quotes_and_whitespace_are_trimmed() {
        assert_eq!(
            allowed(r#"  "xref\base.dwg"  "#, &drawing()),
            PathBuf::from("/work/drawings/xref/base.dwg")
        );
        assert_eq!(
            allowed("  'xref/base.dwg'  ", &drawing()),
            PathBuf::from("/work/drawings/xref/base.dwg")
        );
        // Mismatched quotes are not a pair and stay part of the name.
        assert_eq!(
            allowed(r#""xref/base.dwg'"#, &drawing()),
            PathBuf::from(r#"/work/drawings/"xref/base.dwg'"#)
        );
    }

    #[test]
    fn empty_placeholder_and_pathless_references_are_refused() {
        for raw in ["", "   ", r#""""#, "''", "unnamed", "UNNAMED", ".", "./"] {
            assert_eq!(
                refused_reason(raw, &drawing()),
                EMPTY,
                "expected '{raw}' to be empty"
            );
        }
    }

    #[test]
    fn nul_and_control_characters_are_refused() {
        assert_eq!(refused_reason("a\0b.dwg", &drawing()), CONTROL);
        assert_eq!(refused_reason("a\u{1}b.dwg", &drawing()), CONTROL);
        assert_eq!(refused_reason("a\u{7f}b.dwg", &drawing()), CONTROL);
    }

    #[test]
    fn url_schemes_are_refused_before_normalization() {
        for raw in [
            "http://example.com/base.dwg",
            "https:base.dwg",
            "ftp://host/base.dwg",
            "file:///etc/passwd",
            "FILE:C:/secret.dwg",
        ] {
            assert_eq!(
                refused_reason(raw, &drawing()),
                URL,
                "expected '{raw}' to be a URL"
            );
        }
        // No URL decoding: `%2e%2e` is just a file name and stays relative.
        assert_eq!(
            allowed("a/%2e%2e/base.dwg", &drawing()),
            PathBuf::from("/work/drawings/a/%2e%2e/base.dwg")
        );
    }

    #[test]
    fn windows_drive_paths_are_refused_on_non_windows_hosts() {
        if cfg!(windows) {
            return;
        }
        for raw in [r"C:\project\base.dwg", "C:/project/base.dwg", r"C:foo.dwg"] {
            assert_eq!(
                refused_reason(raw, &drawing()),
                WINDOWS_DRIVE,
                "expected '{raw}' to be a Windows drive path"
            );
        }
    }

    #[test]
    fn unc_paths_are_refused_in_both_spellings() {
        for raw in [
            r"\\server\share\base.dwg",
            "//server/share/base.dwg",
            r"\\\\server\share\x.dwg",
        ] {
            assert_eq!(
                refused_reason(raw, &drawing()),
                UNC,
                "expected '{raw}' to be UNC"
            );
        }
    }

    #[test]
    fn ascent_is_refused_even_when_it_would_stay_inside_the_root() {
        for raw in ["../base.dwg", "a/../../base.dwg", r"xref\..\..\base.dwg"] {
            assert_eq!(
                refused_reason(raw, &drawing()),
                TRAVERSAL,
                "expected '{raw}' to be traversal"
            );
        }
        // Absolute ascent is refused for the same conservative reason.
        assert_eq!(
            refused_reason("/work/drawings/../secret.dwg", &drawing()),
            TRAVERSAL
        );
    }

    #[test]
    fn absolute_paths_must_stay_under_the_drawing_or_a_root() {
        assert_eq!(
            allowed("/work/drawings/xref/base.dwg", &drawing()),
            PathBuf::from("/work/drawings/xref/base.dwg")
        );
        assert_eq!(
            allowed("/work/library/xref/base.dwg", &drawing()),
            PathBuf::from("/work/library/xref/base.dwg")
        );
        assert_eq!(refused_reason("/etc/passwd", &drawing()), ABSOLUTE_OUTSIDE);
        // A prefix that is not a whole component is not containment.
        assert_eq!(
            refused_reason("/work/drawings-other/base.dwg", &drawing()),
            ABSOLUTE_OUTSIDE
        );
    }

    #[test]
    fn relative_paths_join_under_the_drawing_directory() {
        assert_eq!(
            allowed("xref/base.dwg", &drawing()),
            PathBuf::from("/work/drawings/xref/base.dwg")
        );
        assert_eq!(
            allowed("base.dwg", &drawing()),
            PathBuf::from("/work/drawings/base.dwg")
        );
    }

    #[test]
    fn relative_paths_require_a_drawing_directory() {
        assert_eq!(
            refused_reason("xref/base.dwg", Path::new("")),
            NO_DRAWING_DIR
        );
        // An absolute path does not need one when it is inside a root.
        assert_eq!(
            allowed("/work/library/xref/base.dwg", Path::new("")),
            PathBuf::from("/work/library/xref/base.dwg")
        );
    }

    #[test]
    fn utf8_file_names_survive_normalization() {
        assert_eq!(
            allowed("图纸/基础平面.dwg", &drawing()),
            PathBuf::from("/work/drawings/图纸/基础平面.dwg")
        );
    }

    #[test]
    fn repeated_separators_and_dot_segments_collapse() {
        assert_eq!(
            allowed("xref//base///detail\\a.dwg", &drawing()),
            PathBuf::from("/work/drawings/xref/base/detail/a.dwg")
        );
        assert_eq!(
            allowed("./xref/./base.dwg", &drawing()),
            PathBuf::from("/work/drawings/xref/base.dwg")
        );
        assert_eq!(
            allowed(r".\xref\.\base.dwg", &drawing()),
            PathBuf::from("/work/drawings/xref/base.dwg")
        );
    }
}
