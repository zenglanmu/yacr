//! The OS "open with"/double-click half of the macOS file associations.
//!
//! `CFBundleDocumentTypes` (`scripts/package-macos-release.sh`) only makes Yacr a
//! **candidate** for those extensions. macOS delivers the chosen document to a
//! running or freshly launched app as an AppKit open event, which Tauri surfaces
//! as `RunEvent::Opened { urls }` — never as a process argument, so the startup
//! `--open`/positional parser in `cad_ui_core::launch` cannot see it. The event
//! handler in [`crate::entry`] therefore needs a URL → path step.
//!
//! This module is deliberately **target-independent and pure**: it is compiled
//! and unit-tested on the development host (Linux), where the macOS Tauri host
//! itself cannot run (`AGENTS.md`: a contract that only compiles on the
//! unverified carrier is not evidence). It does not depend on the `url` crate so
//! it adds no dependency to the macOS table.
//!
//! Accepted: local `file://` URLs with an empty authority or `localhost`, with
//! percent escapes decoded. Refused (`None`, never a guessed path): other
//! schemes (`http:`, `x-apple.systempreferences:` …), a remote authority
//! (`file://server/share/x.rvt` is not locally readable), malformed escapes and
//! non-UTF-8 bytes (a documented bound of this carrier).

use std::path::PathBuf;

/// Convert one parsed file URL (`url::Url::as_str()`) into a local path.
///
/// The caller owns the refusal: `None` means "this is not a local file URL" and
/// the host logs a stable `[probe] open event refused` line instead of opening
/// something the user did not choose.
pub fn file_url_to_path(url: &str) -> Option<PathBuf> {
    // `Url::as_str()` normalizes the scheme to lowercase, so the parsed form of
    // every local URL starts with exactly this prefix.
    let rest = url.strip_prefix("file://")?;
    // `file:///Users/...` has an empty authority and keeps its leading `/`;
    // `file://localhost/Users/...` names the local host explicitly. Anything
    // else is a remote file URL this process cannot read.
    let encoded = if let Some(after_authority) = rest.strip_prefix("localhost") {
        after_authority
    } else if rest.starts_with('/') {
        rest
    } else {
        return None;
    };
    // A query/fragment is not part of a path. A real `?`/`#` in a file name is
    // percent-encoded by the producer, so splitting here is unambiguous.
    let encoded = encoded.split(['?', '#']).next().unwrap_or_default();
    if !encoded.starts_with('/') {
        return None;
    }
    let bytes = percent_decode(encoded.as_bytes())?;
    // macOS file names are UTF-8 on the wire (`NSURL` percent-encodes non-ASCII
    // as UTF-8). Invalid UTF-8 is refused rather than lossily converted into a
    // path that points somewhere else.
    String::from_utf8(bytes).ok().map(PathBuf::from)
}

/// Decode `%XX` escapes; `+` is a literal plus (this is not form encoding).
///
/// A truncated or non-hex escape is a malformed URL, not a path with a percent
/// sign: refuse it (`None`) instead of copying the raw text through.
fn percent_decode(encoded: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = Vec::with_capacity(encoded.len());
    let mut index = 0;
    while index < encoded.len() {
        if encoded[index] == b'%' {
            let high = hex_value(*encoded.get(index + 1)?)?;
            let low = hex_value(*encoded.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(encoded[index]);
            index += 1;
        }
    }
    Some(decoded)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Append `path` to a bounded FIFO, returning the path displaced by overflow.
///
/// The macOS host queues an OS open that cannot start yet (no session, or a load
/// in flight). The policy lives here, not in the host, so the bound and its
/// "drop the oldest, never the newest" rule are unit-tested on the development
/// host: the carrier itself never runs there.
pub fn enqueue_bounded(
    pending: &mut std::collections::VecDeque<PathBuf>,
    path: PathBuf,
    cap: usize,
) -> Option<PathBuf> {
    let dropped = if pending.len() >= cap {
        pending.pop_front()
    } else {
        None
    };
    pending.push_back(path);
    dropped
}

#[cfg(test)]
mod tests {
    use super::file_url_to_path;
    use std::path::Path;

    #[test]
    fn a_plain_file_url_maps_to_its_path() {
        assert_eq!(
            file_url_to_path("file:///Users/me/plan.rvt"),
            Some(Path::new("/Users/me/plan.rvt").to_path_buf())
        );
    }

    #[test]
    fn percent_escapes_are_decoded_as_utf8() {
        assert_eq!(
            file_url_to_path("file:///Users/me/My%20Drawings/%E5%9B%BE.dwg"),
            Some(Path::new("/Users/me/My Drawings/图.dwg").to_path_buf())
        );
    }

    #[test]
    fn plus_is_a_literal_plus_not_a_space() {
        assert_eq!(
            file_url_to_path("file:///tmp/a+b.dxf"),
            Some(Path::new("/tmp/a+b.dxf").to_path_buf())
        );
    }

    #[test]
    fn localhost_authority_is_a_local_path() {
        assert_eq!(
            file_url_to_path("file://localhost/tmp/a.dgn"),
            Some(Path::new("/tmp/a.dgn").to_path_buf())
        );
    }

    #[test]
    fn a_query_or_fragment_is_not_part_of_the_path() {
        assert_eq!(
            file_url_to_path("file:///tmp/a.dwg?x=1#frag"),
            Some(Path::new("/tmp/a.dwg").to_path_buf())
        );
    }

    #[test]
    fn non_file_schemes_are_refused() {
        assert_eq!(file_url_to_path("https://example.com/a.dwg"), None);
        assert_eq!(
            file_url_to_path("x-apple.systempreferences:com.apple"),
            None
        );
        assert_eq!(file_url_to_path("/tmp/plain-path.dwg"), None);
    }

    #[test]
    fn a_remote_authority_is_refused() {
        assert_eq!(file_url_to_path("file://server/share/a.rvt"), None);
        // `localhost` must be the whole authority, not a prefix of a host name.
        assert_eq!(file_url_to_path("file://localhost.example/a.rvt"), None);
    }

    #[test]
    fn malformed_or_non_utf8_escapes_are_refused() {
        assert_eq!(file_url_to_path("file:///tmp/%zz.dwg"), None);
        assert_eq!(file_url_to_path("file:///tmp/%4.dwg"), None);
        // A lone 0xFF byte is not valid UTF-8.
        assert_eq!(file_url_to_path("file:///tmp/%FF.dwg"), None);
        assert_eq!(file_url_to_path("file://"), None);
        assert_eq!(file_url_to_path("file://localhost"), None);
    }

    #[test]
    fn the_pending_queue_drops_the_oldest_path_on_overflow() {
        use super::enqueue_bounded;
        use std::collections::VecDeque;
        use std::path::PathBuf;

        let mut pending = VecDeque::new();
        assert_eq!(
            enqueue_bounded(&mut pending, PathBuf::from("/tmp/a.dwg"), 2),
            None
        );
        assert_eq!(
            enqueue_bounded(&mut pending, PathBuf::from("/tmp/b.dwg"), 2),
            None
        );
        // Full: the oldest is reported as dropped and the newest is kept.
        assert_eq!(
            enqueue_bounded(&mut pending, PathBuf::from("/tmp/c.dwg"), 2),
            Some(PathBuf::from("/tmp/a.dwg"))
        );
        assert_eq!(
            pending.into_iter().collect::<Vec<_>>(),
            vec![PathBuf::from("/tmp/b.dwg"), PathBuf::from("/tmp/c.dwg")]
        );
    }
}
