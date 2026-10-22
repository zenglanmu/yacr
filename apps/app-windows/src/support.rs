//! Windows runtime support contracts: the WebView2 **missing runtime** refusal
//! and the pinned transparent-background values.
//!
//! Two invariants live here because they are the difference between an honest
//! refusal and a blank window:
//!
//! 1. **WebView2 must exist.** The React chrome *is* a WebView2 document, so a
//!    missing runtime means there is no UI at all. The host checks for it
//!    *before* creating a window, and on failure prints a stable machine marker
//!    (for evidence collection), the localized catalog text, and exits with
//!    [`UNSUPPORTED_WEBVIEW2_EXIT_CODE`] — never an empty window and never a
//!    silent fallback.
//! 2. **The background alpha is exactly zero.** wry silently clamps any
//!    non-zero alpha to 255 (`webview2/mod.rs`), which would make the "transparent"
//!    DOM opaque with no error. The constant here is unit-tested to be zero and
//!    the host reads the value back from
//!    `ICoreWebView2Controller2::DefaultBackgroundColor` at startup.
//!
//! Detection is deliberately registry-first (the documented Edge Update client
//! key) with a filesystem fallback, and honors the Fixed Version strategy
//! through `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER`. Detection never fabricates a
//! positive answer: an unreadable/absent runtime is a refusal.
//!
//! The pure contract half (marker, exit code, alpha, catalog keys) compiles on
//! every host so `cargo test -p app-windows --lib` can exercise it without
//! Windows; the Win32/registry probes are `cfg(target_os = "windows")` only.

use cad_ui_core::i18n::MessageSource;

/// Stable machine marker printed **first** when the WebView2 runtime is absent.
///
/// Evidence collection classifies on this line; the human-facing text below it
/// is locale-dependent and must never be parsed.
pub const UNSUPPORTED_WEBVIEW2_MARKER: &str = "yacr: unsupported-runtime: webview2";

/// Process exit status for the WebView2 refusal.
///
/// The registry is append-only: `0` success, `1` generic failure, `2` invalid
/// display policy, `3` reserved (legacy), `4` unsupported display backend — so
/// the next free code is `5`.
pub const UNSUPPORTED_WEBVIEW2_EXIT_CODE: i32 = 5;

/// Catalog key: one-line user-facing refusal title.
pub const WEBVIEW2_MISSING_KEY: &str = "startup.webview2_missing";
/// Catalog key: actionable refusal detail.
pub const WEBVIEW2_MISSING_DETAIL_KEY: &str = "startup.webview2_missing.detail";

/// The pinned webview background colour.
///
/// Alpha is **exactly** `0`; wry clamps any other alpha to `255`, so a
/// half-transparent value would silently become opaque. The RGB channels are
/// irrelevant at alpha 0 and are pinned for a deterministic contract.
pub const TRANSPARENT_BACKGROUND: (u8, u8, u8, u8) = (0, 0, 0, 0);

/// Whether `alpha` is the one value WebView2 supports for a transparent
/// background (`0`; `255` is the only other legal value and is opaque).
pub const fn background_alpha_is_transparent(alpha: u8) -> bool {
    alpha == 0
}

/// A runtime was detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeInfo {
    /// `"Evergreen"` or `"FixedVersion"`.
    pub kind: &'static str,
    /// The runtime version string when known.
    pub version: Option<String>,
    /// Where the answer came from (a stable diagnostic label, not user prose).
    pub source: &'static str,
}

impl RuntimeInfo {
    /// The one-line startup diagnostic (technical English, evidence only).
    pub fn diagnostic(&self) -> String {
        format!(
            "[probe] webview2 runtime kind={} version={} source={}",
            self.kind,
            self.version.as_deref().unwrap_or("-"),
            self.source
        )
    }
}

/// Why no usable WebView2 runtime was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeMissing {
    /// Technical detail for the evidence log (English, never UI prose).
    pub detail: String,
}

impl RuntimeMissing {
    // Only the Windows runtime probes construct this; on other targets the pure
    // contract half compiles the type without a constructor call site.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

/// The localized refusal text `(title, detail)` for `locale`.
///
/// The keys are validated against both shipped catalogs by [`MessageSource`]'s
/// own fallback rules and by the unit test below.
pub fn missing_messages(locale: &str) -> (String, String) {
    let source = MessageSource::from_request(locale);
    (
        source.text(WEBVIEW2_MISSING_KEY, &[]),
        source.text(WEBVIEW2_MISSING_DETAIL_KEY, &[]),
    )
}

#[cfg(target_os = "windows")]
mod imp {
    use std::path::{Path, PathBuf};

    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_ROUTINE_FLAGS,
        RRF_RT_REG_SZ, RRF_SUBKEY_WOW6432KEY,
    };

    use super::{RuntimeInfo, RuntimeMissing};

    /// The Edge Update client id for the Evergreen WebView2 runtime.
    const EDGE_UPDATE_KEY: &str =
        r"SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Read the `pv` version value under `root`, or `None` on any failure.
    fn registry_version(root: HKEY, flags: REG_ROUTINE_FLAGS) -> Option<String> {
        let subkey = wide(EDGE_UPDATE_KEY);
        let value = wide("pv");
        let mut buffer = [0u16; 64];
        let mut bytes = (buffer.len() * std::mem::size_of::<u16>()) as u32;
        let status = unsafe {
            RegGetValueW(
                root,
                PCWSTR::from_raw(subkey.as_ptr()),
                PCWSTR::from_raw(value.as_ptr()),
                RRF_RT_REG_SZ | flags,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let len = (bytes as usize / std::mem::size_of::<u16>()).min(buffer.len());
        let text = String::from_utf16_lossy(&buffer[..len]);
        let text = text.trim_end_matches('\0').trim().to_string();
        // `0.0.0.0` is the documented "not installed" sentinel.
        (!text.is_empty() && text != "0.0.0.0").then_some(text)
    }

    /// The highest installed version under an `Application` directory.
    fn installed_version(application: &Path) -> Option<String> {
        let mut versions: Vec<String> = std::fs::read_dir(application)
            .ok()?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join("msedgewebview2.exe").is_file())
            .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
            .collect();
        versions.sort_by_key(|version| version_sort_key(version));
        versions.pop()
    }

    fn version_sort_key(version: &str) -> Vec<u32> {
        version
            .split('.')
            .map(|part| part.parse::<u32>().unwrap_or(0))
            .collect()
    }

    fn application_version(root: &Path) -> Option<String> {
        installed_version(
            &root
                .join("Microsoft")
                .join("EdgeWebView")
                .join("Application"),
        )
    }

    /// Detect a usable WebView2 runtime without creating a window.
    pub fn detect_runtime() -> Result<RuntimeInfo, RuntimeMissing> {
        // 1. Fixed Version strategy: the loader honors this variable, so the
        //    host only has to validate it and record the directory as evidence.
        if let Some(folder) = std::env::var_os("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER") {
            let folder = PathBuf::from(folder);
            let exe = folder.join("msedgewebview2.exe");
            if exe.is_file() {
                return Ok(RuntimeInfo {
                    kind: "FixedVersion",
                    version: None,
                    source: "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER",
                });
            }
            return Err(RuntimeMissing::new(format!(
                "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER is set to {} but msedgewebview2.exe is not there",
                folder.display()
            )));
        }

        // 2. Documented Evergreen detection: the Edge Update client's `pv`.
        for (root, flags, source) in [
            (HKEY_CURRENT_USER, REG_ROUTINE_FLAGS(0), "registry-hkcu"),
            (
                HKEY_LOCAL_MACHINE,
                RRF_SUBKEY_WOW6432KEY,
                "registry-hklm-wow6432",
            ),
        ] {
            if let Some(version) = registry_version(root, flags) {
                return Ok(RuntimeInfo {
                    kind: "Evergreen",
                    version: Some(version),
                    source,
                });
            }
        }

        // 3. Filesystem fallback for installs the registry key missed
        //    (per-user and per-machine application directories).
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            if let Some(version) = application_version(&PathBuf::from(local)) {
                return Ok(RuntimeInfo {
                    kind: "Evergreen",
                    version: Some(version),
                    source: "localappdata",
                });
            }
        }
        for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(root) = std::env::var_os(variable) {
                if let Some(version) = application_version(&PathBuf::from(root)) {
                    return Ok(RuntimeInfo {
                        kind: "Evergreen",
                        version: Some(version),
                        source: "programfiles",
                    });
                }
            }
        }

        Err(RuntimeMissing::new(
            "no Evergreen WebView2 runtime found in the Edge Update registry key or the \
             application directories, and WEBVIEW2_BROWSER_EXECUTABLE_FOLDER is unset",
        ))
    }

    /// Convert a `PWSTR` owned by COM into a Rust string, then free it.
    ///
    /// Used by the native open dialog; kept here because both the runtime and
    /// the platform layer already depend on `windows`.
    pub fn take_pwstr(value: PWSTR) -> Option<String> {
        if value.is_null() {
            return None;
        }
        let text = unsafe { value.to_string().ok() };
        unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(value.as_ptr().cast())) };
        text
    }
}

#[cfg(target_os = "windows")]
pub use imp::{detect_runtime, take_pwstr};

/// Detect a usable WebView2 runtime, or report exactly why none was found.
#[cfg(target_os = "windows")]
pub fn ensure_runtime() -> Result<RuntimeInfo, RuntimeMissing> {
    detect_runtime()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The marker is a stable, greppable line that names the missing runtime.
    #[test]
    fn the_marker_is_stable_and_names_webview2() {
        assert_eq!(
            UNSUPPORTED_WEBVIEW2_MARKER,
            "yacr: unsupported-runtime: webview2"
        );
        assert!(UNSUPPORTED_WEBVIEW2_MARKER.starts_with("yacr: "));
    }

    /// `5` is the next free code in the append-only registry (0–4 are taken).
    #[test]
    fn the_exit_code_is_the_next_free_registry_slot() {
        assert_eq!(UNSUPPORTED_WEBVIEW2_EXIT_CODE, 5);
        assert_ne!(UNSUPPORTED_WEBVIEW2_EXIT_CODE, 0);
        assert_ne!(UNSUPPORTED_WEBVIEW2_EXIT_CODE, 1);
        assert_ne!(UNSUPPORTED_WEBVIEW2_EXIT_CODE, 2);
        assert_ne!(UNSUPPORTED_WEBVIEW2_EXIT_CODE, 4);
    }

    /// wry clamps any non-zero alpha to 255: only exactly `0` stays transparent.
    #[test]
    fn the_background_alpha_is_exactly_zero() {
        assert!(background_alpha_is_transparent(TRANSPARENT_BACKGROUND.3));
        assert_eq!(TRANSPARENT_BACKGROUND.3, 0);
        // The clamp boundary is documented by this contract, not guessed.
        assert!(!background_alpha_is_transparent(1));
        assert!(!background_alpha_is_transparent(128));
        assert!(!background_alpha_is_transparent(255));
    }

    /// Both catalogs carry the refusal, so the host cannot print a missing-key
    /// placeholder in either shipped language.
    #[test]
    fn both_catalogs_carry_the_missing_runtime_copy() {
        for locale in ["en", "zh-CN"] {
            let (title, detail) = missing_messages(locale);
            for (key, text) in [
                (WEBVIEW2_MISSING_KEY, &title),
                (WEBVIEW2_MISSING_DETAIL_KEY, &detail),
            ] {
                assert!(!text.is_empty(), "{locale}/{key} is empty");
                assert!(
                    !text.contains('\u{27e6}'),
                    "{locale}/{key} fell back to a missing-key placeholder: {text}"
                );
            }
            assert!(
                title.to_ascii_lowercase().contains("webview2"),
                "{locale} title must name WebView2: {title}"
            );
        }
    }

    /// The diagnostics line is technical, stable and safe to parse for evidence.
    #[test]
    fn the_runtime_diagnostic_is_stable() {
        let info = RuntimeInfo {
            kind: "Evergreen",
            version: Some("154.0.4258.62".into()),
            source: "registry-hkcu",
        };
        assert_eq!(
            info.diagnostic(),
            "[probe] webview2 runtime kind=Evergreen version=154.0.4258.62 source=registry-hkcu"
        );
        let fixed = RuntimeInfo {
            kind: "FixedVersion",
            version: None,
            source: "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER",
        };
        assert!(fixed.diagnostic().contains("version=-"));
    }
}
