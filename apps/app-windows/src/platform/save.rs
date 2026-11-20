//! Save-target chooser contract for the Windows host.
//!
//! This is the `IFileSaveDialog` half of the shell file dialogs the open path
//! already uses (`platform/native.rs`); it lives in its own module so
//! `platform.rs` stays under the repository's 1000-line file ceiling.
//!
//! The chooser is dependency-injected exactly like the open chooser, so the whole
//! save funnel (admission → one terminal event) is unit-testable without a GUI;
//! [`super::native::native_save`] is the real COM implementation. No save target
//! is ever fabricated: a cancellation and a failure stay distinct events.
//!
//! User-visible dialog strings come from the single UI i18n catalogue
//! (`crates/cad-ui-core/i18n/*.json`) through [`MessageSource`] — no host-side
//! hardcoded UI text. The catalogue has `windows.picker_*` (shared with open)
//! but no `windows.save_*` entries yet, so the purpose titles/filters reuse the
//! existing shipped wording (`plot.save_*` and the `linux.save_*` entries, which
//! carry no host-specific text). When `windows.save_*` keys land, only the five
//! constants below change.

use std::sync::Arc;

use cad_domain::{CadError, CadResult};
use cad_platform::SavePurpose;
use cad_ui_core::i18n::MessageSource;

use super::{FileChooserOutcome, KEY_PICKER_EMPTY, KEY_PICKER_FAILED, KEY_PICKER_LOCAL};

const KEY_SAVE_TITLE_YMODEL: &str = "linux.save_title_ymodel";
const KEY_SAVE_TITLE_EXPORT: &str = "linux.save_title_export";
const KEY_SAVE_TITLE_PLOT: &str = "plot.save_title";
const KEY_SAVE_FILTER_YMODEL: &str = "linux.save_filter_ymodel";
const KEY_SAVE_FILTER_EXPORT: &str = "linux.save_filter_dxf";
const KEY_SAVE_FILTER_PLOT: &str = "plot.save_filter";

/// The COM `COMDLG_FILTERSPEC` pattern for `SAVE` (ADR-0041 D1).
pub const YMODEL_FILTER_SPEC: &str = "*.ymodel";

/// The export filter: every format the connector registry can write.
///
/// One filter (not one per format) because the chosen target's extension decides
/// the writer in the shared host; listing them in one dialog filter keeps the
/// dispatch rule in one place. ASCII STL has no extension of its own and is
/// reachable only through the registry API.
pub const EXPORT_FILTER_SPEC: &str =
    "*.dxf;*.dwg;*.stl;*.obj;*.ply;*.off;*.coff;*.ifc;*.skp;*.glb;*.gltf";

/// `PLOT` writes vector output only.
pub const PLOT_FILTER_SPEC: &str = "*.svg;*.pdf";

/// The localized labels the native save dialog uses.
///
/// Built from the embedded UI catalogue; [`Self::from_locale`] selects the
/// shipped locale for the request and falls back to English for anything else.
/// The empty/local/failed wording is shared with the open picker (one wording per
/// concept); title and filter are per [`SavePurpose`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsSaveDialogLabels {
    pub title_ymodel: String,
    pub title_export: String,
    pub title_plot: String,
    pub filter_ymodel: String,
    pub filter_export: String,
    pub filter_plot: String,
    pub picker_empty: String,
    pub picker_local: String,
    /// Template containing a `{error}` placeholder.
    pub picker_failed: String,
}

impl WindowsSaveDialogLabels {
    /// Labels for `locale`, resolved through the shared catalog. An unknown
    /// locale falls back to English (with a recorded reason) rather than showing
    /// a wrong language.
    pub fn from_locale(locale: &str) -> CadResult<Self> {
        let source = MessageSource::from_request(locale);
        let get = |key: &str| {
            source
                .catalog()
                .get(key)
                .map(str::to_string)
                .ok_or_else(|| CadError::InvalidInput(format!("i18n key missing: {key}")))
        };
        Ok(Self {
            title_ymodel: get(KEY_SAVE_TITLE_YMODEL)?,
            title_export: get(KEY_SAVE_TITLE_EXPORT)?,
            title_plot: get(KEY_SAVE_TITLE_PLOT)?,
            filter_ymodel: get(KEY_SAVE_FILTER_YMODEL)?,
            filter_export: get(KEY_SAVE_FILTER_EXPORT)?,
            filter_plot: get(KEY_SAVE_FILTER_PLOT)?,
            picker_empty: get(KEY_PICKER_EMPTY)?,
            picker_local: get(KEY_PICKER_LOCAL)?,
            picker_failed: get(KEY_PICKER_FAILED)?,
        })
    }

    /// Fill the `{error}` placeholder in the picker-failure template.
    pub fn picker_failed(&self, error: &str) -> String {
        self.picker_failed.replace("{error}", error)
    }

    /// Dialog title for a save purpose.
    pub fn title(&self, purpose: SavePurpose) -> &str {
        match purpose {
            SavePurpose::SaveYmodel => &self.title_ymodel,
            SavePurpose::SaveExport => &self.title_export,
            SavePurpose::Plot => &self.title_plot,
        }
    }

    /// The one filter label for a save purpose.
    pub fn filter(&self, purpose: SavePurpose) -> &str {
        match purpose {
            SavePurpose::SaveYmodel => &self.filter_ymodel,
            SavePurpose::SaveExport => &self.filter_export,
            SavePurpose::Plot => &self.filter_plot,
        }
    }
}

/// The COM `COMDLG_FILTERSPEC` pattern for a save purpose.
pub fn save_filter_spec(purpose: SavePurpose) -> &'static str {
    match purpose {
        SavePurpose::SaveYmodel => YMODEL_FILTER_SPEC,
        SavePurpose::SaveExport => EXPORT_FILTER_SPEC,
        SavePurpose::Plot => PLOT_FILTER_SPEC,
    }
}

/// The extension the shell appends when the user types a bare name.
///
/// `SAVE` is always the native archive, and `PLOT` defaults to SVG; an export has
/// no single default because its extension *is* the writer choice, so the dialog
/// must not guess one.
pub fn save_default_extension(purpose: SavePurpose) -> Option<&'static str> {
    match purpose {
        SavePurpose::SaveYmodel => Some("ymodel"),
        SavePurpose::SaveExport => None,
        SavePurpose::Plot => Some("svg"),
    }
}

/// A dependency-injected save chooser: purpose, labels and the suggested file
/// name in, one outcome out. `Send + Sync` so it can run on the worker thread.
pub type SaveChooser =
    Arc<dyn Fn(SavePurpose, &WindowsSaveDialogLabels, &str) -> FileChooserOutcome + Send + Sync>;
