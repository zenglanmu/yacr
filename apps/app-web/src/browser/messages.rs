//! Active message catalog for the web host.
//!
//! The Slint handle owns its own catalog, but it does not expose a
//! `MessageSource`; the host needs one to derive the catalog-driven empty/mixed
//! labels the panel-state push requires (`layers.empty`, `properties.mixed`, …).
//!
//! To keep the two in step, this module mirrors the locale the handle was last
//! asked to apply: [`set_locale`] is called at startup with the stored locale and
//! again after every successful runtime switch. It is deliberately a small,
//! single-slot mirror, not a second source of truth — the catalogs themselves
//! are still the shared `crates/cad-ui-slint/i18n` JSON.

use std::cell::RefCell;

use cad_ui_slint::{LocaleResolution, MessageSource};

thread_local! {
    /// The catalog the shell is currently displaying. Defaults to `zh-CN`, the
    /// same default [`MessageSource`] uses before any locale is applied.
    static MESSAGES: RefCell<MessageSource> = RefCell::new(MessageSource::default());
}

/// Resolve and store `tag` as the host's active catalog.
///
/// Returns the resolution actually applied so a caller can report a fallback;
/// the resolution is never silent, matching `UiHandle::set_locale`.
pub(super) fn set_locale(tag: &str) -> LocaleResolution {
    MESSAGES.with(|slot| slot.borrow_mut().set_locale(tag))
}

/// The active catalog, cloned so callers can derive labels while a handle is
/// borrowed.
pub(super) fn current_messages() -> MessageSource {
    MESSAGES.with(|slot| slot.borrow().clone())
}
