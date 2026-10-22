//! The host-resolved UI locale: one value for the native labels and the chrome.
//!
//! The resolution itself lives in [`cad_ui_core::i18n`] (`host_locale_tag` /
//! `host_locale_script`) because it is UI-framework-free and shared with every
//! other host; this module only exposes it to the Linux host.
//!
//! The native open-dialog labels (`platform::UseOpenDialogLabels`) and the React
//! chrome must speak the same language. Resolving the locale twice — once for the
//! native labels and once in the WebView from `navigator.languages` — let them
//! disagree: `YACR_LOCALE` is invisible to the WebView, and WebKitGTK may read
//! `LANG`/the GTK locale rather than `LC_ALL`. Both sides are therefore built
//! from the one resolved tag, and the tag is injected as
//! `window.__YACR_HOST_LOCALE__` before the document is parsed (an
//! `initialization_script` runs before any document script, in the main frame).

pub(super) use cad_ui_core::i18n::{host_locale_script, host_locale_tag};

#[cfg(test)]
mod tests {
    use cad_ui_core::i18n::{host_locale_script_for, resolve_locale};

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
            let tag = resolve_locale(request).locale.tag();
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
}
