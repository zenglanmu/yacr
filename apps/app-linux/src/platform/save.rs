//! Save-target chooser for the React Linux host.
//!
//! This is the `save_file` half of the XDG desktop portal file chooser the open
//! path already uses (`ashpd`); it lives in its own module so `platform.rs` stays
//! under the repository's 1000-line file ceiling.
//!
//! The chooser is dependency-injected exactly like the open chooser, so the whole
//! save funnel (admission → one terminal event) is unit-testable without a GUI;
//! [`portal_save`] is the real portal implementation. No save target is ever
//! fabricated: a cancellation and a failure stay distinct events.
//!
//! User-visible dialog strings come from the single UI i18n catalogue
//! (`crates/cad-ui-core/i18n/*.json`) through [`MessageSource`] — no
//! host-side hardcoded UI text.

use std::sync::Arc;

use cad_domain::{CadError, CadResult};
use cad_platform::SavePurpose;
use cad_ui_core::i18n::MessageSource;

use super::{FileChooserOutcome, KEY_PICKER_EMPTY, KEY_PICKER_FAILED, KEY_PICKER_LOCAL};

const KEY_SAVE_TITLE_YMODEL: &str = "linux.save_title_ymodel";
const KEY_SAVE_TITLE_DXF: &str = "linux.save_title_dxf";
const KEY_SAVE_TITLE_DWG: &str = "linux.save_title_dwg";
const KEY_SAVE_TITLE_PLOT: &str = "linux.save_title_plot";
const KEY_SAVE_FILTER_YMODEL: &str = "linux.save_filter_ymodel";
const KEY_SAVE_FILTER_DXF: &str = "linux.save_filter_dxf";
const KEY_SAVE_FILTER_DWG: &str = "linux.save_filter_dwg";
const KEY_SAVE_FILTER_PLOT: &str = "linux.save_filter_plot";

/// The localized labels the native save dialog uses.
///
/// Built from the embedded UI catalogue; [`Self::from_locale`] selects the
/// shipped locale for the request and falls back to English for anything else.
/// The empty/local/failed wording is shared with the open picker (one wording per
/// concept), the title and filter are per [`SavePurpose`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseSaveDialogLabels {
    /// Native `.ymodel` Save As.
    pub title_ymodel: String,
    pub title_dxf: String,
    pub title_dwg: String,
    pub title_plot: String,
    pub filter_ymodel: String,
    pub filter_dxf: String,
    pub filter_dwg: String,
    pub filter_plot: String,
    pub picker_empty: String,
    pub picker_local: String,
    /// Template containing a `{error}` placeholder.
    pub picker_failed: String,
}

impl UseSaveDialogLabels {
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
            title_dxf: get(KEY_SAVE_TITLE_DXF)?,
            title_dwg: get(KEY_SAVE_TITLE_DWG)?,
            title_plot: get(KEY_SAVE_TITLE_PLOT)?,
            filter_ymodel: get(KEY_SAVE_FILTER_YMODEL)?,
            filter_dxf: get(KEY_SAVE_FILTER_DXF)?,
            filter_dwg: get(KEY_SAVE_FILTER_DWG)?,
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
            SavePurpose::SaveDxf => &self.title_dxf,
            SavePurpose::SaveDwg => &self.title_dwg,
            SavePurpose::Plot => &self.title_plot,
        }
    }

    /// The one filter label for a save purpose.
    pub fn filter(&self, purpose: SavePurpose) -> &str {
        match purpose {
            SavePurpose::SaveYmodel => &self.filter_ymodel,
            SavePurpose::SaveDxf => &self.filter_dxf,
            SavePurpose::SaveDwg => &self.filter_dwg,
            SavePurpose::Plot => &self.filter_plot,
        }
    }
}

/// A dependency-injected save chooser: purpose, labels and the suggested file
/// name in, one outcome out. `Send + Sync` so it can run on the worker thread.
pub type SaveChooser =
    Arc<dyn Fn(SavePurpose, &UseSaveDialogLabels, &str) -> FileChooserOutcome + Send + Sync>;

/// The real native save chooser (XDG desktop portal via `ashpd` `save_file`).
///
/// The drawing Save As is lossy: DXF plus the connector exchange formats
/// (binary STL, OBJ, PLY, OFF, IFC, SKP) share one filter because the target
/// extension decides what the shared host writes. DWG is the **experimental**
/// binary export (always a new file), and plot is SVG/PDF. Each purpose has its
/// own localized title/filter and its own extension filter.
pub fn portal_save(
    purpose: SavePurpose,
    labels: &UseSaveDialogLabels,
    default_name: &str,
) -> FileChooserOutcome {
    use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};
    use ashpd::desktop::ResponseError;

    let filter = match purpose {
        SavePurpose::SaveYmodel => {
            FileFilter::new(labels.filter(purpose)).glob("*.[yY][mM][oO][dD][eE][lL]")
        }
        // The drawing Save As covers the connector writers too: `.stl` (binary),
        // `.obj`, `.ply`, `.off`/`.coff`, `.ifc`, `.skp`, `.glb`/`.gltf`, plus
        // DXF. ASCII STL (`stl-ascii`) has no extension of its own.
        SavePurpose::SaveDxf => FileFilter::new(labels.filter(purpose))
            .glob("*.[dD][xX][fF]")
            .glob("*.[sS][tT][lL]")
            .glob("*.[oO][bB][jJ]")
            .glob("*.[pP][lL][yY]")
            .glob("*.[oO][fF][fF]")
            .glob("*.[cC][oO][fF][fF]")
            .glob("*.[iI][fF][cC]")
            .glob("*.[sS][kK][pP]")
            .glob("*.[gG][lL][bB]")
            .glob("*.[gG][lL][tT][fF]"),
        SavePurpose::SaveDwg => FileFilter::new(labels.filter(purpose)).glob("*.[dD][wW][gG]"),
        SavePurpose::Plot => FileFilter::new(labels.filter(purpose))
            .glob("*.[sS][vV][gG]")
            .glob("*.[pP][dD][fF]"),
    };
    let result = pollster::block_on(async {
        SelectedFiles::save_file()
            .title(labels.title(purpose))
            .modal(true)
            .current_name(default_name)
            .filter(filter)
            .send()
            .await?
            .response()
    });
    match result {
        Ok(files) => match files.uris().first() {
            Some(uri) => match uri.to_file_path() {
                Ok(path) => FileChooserOutcome::Path(path),
                Err(_) => FileChooserOutcome::Failed(labels.picker_local.clone()),
            },
            None => FileChooserOutcome::Failed(labels.picker_empty.clone()),
        },
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => FileChooserOutcome::Cancelled,
        Err(error) => FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    }
}
