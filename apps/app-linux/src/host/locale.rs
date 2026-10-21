//! The host-resolved UI locale: one value for the native labels and the chrome.
//!
//! The native open-dialog labels (`platform::UseOpenDialogLabels`) and the React
//! chrome must speak the same language. Resolving the locale twice — once here
//! for the native labels and once in the WebView from `navigator.languages` —
//! let them disagree: `YACR_LOCALE` is invisible to the WebView, and WebKitGTK
//! may read `LANG`/the GTK locale rather than `LC_ALL`.
//!
//! [`host_locale_tag`] is now the single source of the system default. Native
//! labels are built from that tag, and [`host_locale_script`] injects exactly the
//! same tag as `window.__YACR_HOST_LOCALE__` before the document is parsed (an
//! `initialization_script` runs before any document script, in the main frame).

/// The locale tag the host resolved for this process, in canonical catalog form
/// (`zh-CN` or `en`).
///
/// `YACR_LOCALE` wins when set (and is itself resolved through the shared table);
/// otherwise the POSIX precedence (`LC_ALL`, `LC_MESSAGES`, `LANGUAGE`, `LANG`)
/// selects the system language. The result is always **resolved**: an
/// unsupported request (`fr`, `zh-TW`) is English, never a raw environment string
/// handed to the WebView.
pub(super) fn host_locale_tag() -> &'static str {
    match std::env::var("YACR_LOCALE") {
        Ok(requested) => resolved_tag(&requested),
        Err(_) => cad_ui_core::i18n::locale_from_env(|key| std::env::var(key).ok()).tag(),
    }
}

/// Resolve a requested tag to the canonical tag this build ships.
///
/// This is the one implementation shared by the native labels and the injected
/// chrome; the mapping table itself lives in `cad_ui_core::i18n`.
pub(super) fn resolved_tag(requested: &str) -> &'static str {
    cad_ui_core::i18n::resolve_locale(requested).locale.tag()
}

/// Initialization script: hand the host-resolved tag to the document before load.
pub(super) fn host_locale_script() -> String {
    host_locale_script_for(host_locale_tag())
}

/// The literal injected for `tag`: `window.__YACR_HOST_LOCALE__ = "<tag>";`.
///
/// `tag` is a canonical catalog tag (ASCII), and `serde_json` quotes/escapes it,
/// so the script is always a valid assignment regardless of caller input.
pub(super) fn host_locale_script_for(tag: &str) -> String {
    format!("window.__YACR_HOST_LOCALE__ = {};", serde_json::json!(tag))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::UseOpenDialogLabels;

    #[test]
    fn the_injected_tag_is_the_one_native_labels_use() {
        for request in [
            "zh-CN",
            "en",
            "zh_CN.UTF-8",
            "zh-Hans-CN",
            "en-US",
            "fr",
            "zh-TW",
            "C",
            "",
        ] {
            let tag = resolved_tag(request);
            // Always one of the two shipped catalog tags, never a raw request.
            assert!(matches!(tag, "zh-CN" | "en"), "{request:?} -> {tag}");
            // The literal the WebView receives carries that exact tag.
            assert_eq!(
                host_locale_script_for(tag),
                format!("window.__YACR_HOST_LOCALE__ = {tag:?};"),
                "{request:?}"
            );
            // The tag and the request select the same native-label catalog, so
            // the chrome and the native dialogs cannot diverge.
            assert_eq!(
                UseOpenDialogLabels::from_locale(tag).expect("labels"),
                UseOpenDialogLabels::from_locale(request).expect("labels"),
                "{request:?}"
            );
        }
    }

    #[test]
    fn the_injection_is_a_literal_assignment_for_both_tags() {
        assert_eq!(
            host_locale_script_for("zh-CN"),
            "window.__YACR_HOST_LOCALE__ = \"zh-CN\";"
        );
        assert_eq!(
            host_locale_script_for("en"),
            "window.__YACR_HOST_LOCALE__ = \"en\";"
        );
    }
}
