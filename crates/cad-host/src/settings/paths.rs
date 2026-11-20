//! The persisted font-directory preference (`ui.paths.fontsDir`).
//!
//! A child of [`crate::settings`] so both files stay inside the file-size gate;
//! the closed key table, its validation and the dispatch stay in the parent.
//! What lives here is the apply half, the startup diagnostic and the one
//! resolution rule the snapshot and the font funnel share.
//!
//! # Precedence
//!
//! 1. `--fonts-dir` (CLI / carrier-supplied [`HostOptions::fonts_dir`]): wins, and
//!    a missing directory stays a **hard open error** (unchanged behaviour).
//! 2. The persisted `ui.paths.fontsDir` preference, when it still names an
//!    absolute existing directory.
//! 3. Discovery via [`cad_platform::fonts::local::font_dir_candidates`].
//!
//! A persisted directory that disappeared between runs is **not** a startup
//! failure: the preference is kept (so the Options row can show what the user
//! chose), the resolution falls through to discovery, and the startup diagnostic
//! records the fact. `--fonts-dir` deliberately keeps its hard-error semantics;
//! only the preference is tolerant.

use std::path::PathBuf;

use super::*;

/// Whether `path` may be persisted as the font directory.
///
/// The one rule the key table and the load-time resolution share: an absolute
/// path to an existing directory. Nothing is trimmed or canonicalized, so the
/// value written is exactly the value resolved later.
pub(crate) fn fonts_dir_is_usable(path: &str) -> bool {
    let path = std::path::Path::new(path);
    path.is_absolute() && path.is_dir()
}

impl HostCore {
    /// Persist `ui.paths.fontsDir` (`""` clears it).
    ///
    /// The value was already validated by the closed key table, so only the
    /// shape is re-read here (a future caller cannot skip the table). The write
    /// goes through [`HostCore::apply_user_preference_json`], which persists the
    /// allowed projection and restores the store's previous document when the
    /// store refuses — never a success that would disappear on the next start.
    /// Clearing writes `""` rather than deleting the leaf, mirroring how
    /// `ui.recovery.intervalSeconds` expresses "off".
    ///
    /// The directory takes effect on the **next** font load: the installed
    /// shaping engine is host-wide and is reused while it covers the drawing, so
    /// a write does not tear down a working chain mid-edit.
    pub(crate) fn apply_fonts_dir(&self, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Text(path) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        if !path.is_empty() && !fonts_dir_is_usable(path) {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        }
        let patch = serde_json::json!({"ui": {"paths": {"fontsDir": path}}}).to_string();
        self.apply_user_preference_json(&patch)
    }

    /// The persisted font-directory choice, `None` when never chosen or cleared.
    ///
    /// This is the **editable** value the Options dialog shows, not necessarily
    /// the directory in use (a missing one falls back, see
    /// [`Self::effective_fonts_dir`]).
    pub(crate) fn fonts_dir_override(&self) -> Option<String> {
        let choice = self
            .viewer_config
            .borrow()
            .effective()
            .ui
            .paths
            .fonts_dir
            .clone()?;
        (!choice.is_empty()).then_some(choice)
    }

    /// The persisted choice when it is still usable (absolute and a directory).
    fn usable_persisted_fonts_dir(&self) -> Option<PathBuf> {
        self.fonts_dir_override()
            .filter(|choice| fonts_dir_is_usable(choice))
            .map(PathBuf::from)
    }

    /// Whether the persisted choice names something that is no longer a usable
    /// directory (deleted, replaced by a file, or a non-absolute path).
    ///
    /// This is the Options row's own state, independent of precedence: a stale
    /// choice is still what the user picked ([`Self::fonts_dir_override`]), while
    /// the directory in use is [`Self::effective_fonts_dir`] — `--fonts-dir` or
    /// the discovered default. `false` when the user never chose one.
    pub(crate) fn fonts_dir_is_missing(&self) -> bool {
        self.fonts_dir_override()
            .is_some_and(|choice| !fonts_dir_is_usable(&choice))
    }

    /// The directory actually used for font loading, in precedence order.
    ///
    /// `--fonts-dir` is reported verbatim even when it does not exist: the font
    /// funnel then refuses the open with that path, and the snapshot must not
    /// silently present a different directory as the configured one.
    pub(crate) fn effective_fonts_dir(&self) -> Option<PathBuf> {
        if let Some(explicit) = self.options.fonts_dir.clone() {
            return Some(explicit);
        }
        if let Some(persisted) = self.usable_persisted_fonts_dir() {
            return Some(persisted);
        }
        // The browser carrier has no filesystem to discover a package in (faces
        // arrive from the page's same-origin `fonts/`), so there is no third step
        // there — and no `cad-platform` local-font module to call.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let exe = std::env::current_exe().unwrap_or_default();
            cad_platform::fonts::local::font_dir_candidates(&exe)
                .into_iter()
                .find(|dir| dir.is_dir())
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    /// Record one startup diagnostic when the persisted font directory is stale.
    ///
    /// Startup must not fail over a preference whose directory was removed
    /// between runs: [`Self::effective_fonts_dir`] already falls through to
    /// discovery, and this only publishes the fact. `--fonts-dir` is deliberately
    /// **not** diagnosed here — it keeps its hard-error semantics at the font
    /// load, where the path is reported to the caller.
    pub(crate) fn diagnose_fonts_dir(&self, diagnostics: &mut Vec<String>) {
        if let Some(choice) = self.fonts_dir_override() {
            if !fonts_dir_is_usable(&choice) {
                diagnostics.push(format!(
                    "preference font directory {choice} is not an existing directory; \
                     it is ignored for font loading"
                ));
            }
        }
    }

    /// The explicit directory handed to `resolve_font_dir` when a font load runs.
    ///
    /// `--fonts-dir` keeps its hard-error semantics (it is returned as-is, so a
    /// missing directory is refused by the resolver). A persisted directory is
    /// only returned while it is still usable; a stale one is dropped here so
    /// the resolver falls through to discovery instead of failing the open.
    pub(crate) fn explicit_font_dir(&self) -> Option<PathBuf> {
        self.options
            .fonts_dir
            .clone()
            .or_else(|| self.usable_persisted_fonts_dir())
    }
}
