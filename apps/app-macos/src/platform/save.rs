//! Save-target chooser for the macOS host.
//!
//! This is the `NSSavePanel` half of the file dialogs the open path already uses
//! (`super::native_choose`); it lives in its own module so `platform.rs` stays
//! under the repository's 1000-line file ceiling.
//!
//! The chooser is dependency-injected exactly like the open chooser, so the whole
//! save funnel (admission → one terminal event) is unit-testable without a GUI;
//! [`native_save`] is the real AppKit implementation and is compiled only for the
//! macOS target. No save target is ever fabricated: a cancellation and a failure
//! stay distinct events, and a path only exists after the panel's own OK response.
//!
//! User-visible dialog strings come from the single UI i18n catalogue
//! (`crates/cad-ui-core/i18n/*.json`) through [`MessageSource`] — no host-side
//! hardcoded UI text. The catalogue has `macos.picker_*` (shared with open) but no
//! `macos.save_*` entries, so the purpose titles/filters reuse the existing
//! shipped wording (`plot.save_*` and the `linux.save_*` entries, which carry no
//! host-specific text). When `macos.save_*` keys land, only the six constants
//! below change.

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

/// `SAVE` offers the native archive only (ADR-0041 D1).
pub const YMODEL_EXTENSIONS: &[&str] = &["ymodel"];

/// The export filter: every format the connector registry can write.
///
/// One filter (not one per format) because the chosen target's extension decides
/// the writer in the shared host; listing them in one panel filter keeps the
/// dispatch rule in one place. ASCII STL has no extension of its own and is
/// reachable only through the registry API.
pub const EXPORT_EXTENSIONS: &[&str] = &[
    "dxf", "dwg", "stl", "obj", "ply", "off", "coff", "ifc", "skp", "glb", "gltf",
];

/// `PLOT` writes vector output only.
pub const PLOT_EXTENSIONS: &[&str] = &["svg", "pdf"];

/// The extensions `NSSavePanel` allows for a save purpose.
///
/// Pure and public so the filter contract is testable without AppKit: AppKit only
/// consumes this list.
pub fn save_allowed_extensions(purpose: SavePurpose) -> &'static [&'static str] {
    match purpose {
        SavePurpose::SaveYmodel => YMODEL_EXTENSIONS,
        SavePurpose::SaveExport => EXPORT_EXTENSIONS,
        SavePurpose::Plot => PLOT_EXTENSIONS,
    }
}

/// The localized labels the native save dialog uses.
///
/// Built from the embedded UI catalogue; [`Self::from_locale`] selects the
/// shipped locale for the request and falls back to English for anything else.
/// The empty/local/failed wording is shared with the open picker (one wording per
/// concept); title and filter are per [`SavePurpose`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacosSaveDialogLabels {
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

impl MacosSaveDialogLabels {
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

/// A dependency-injected save chooser: purpose, labels and the suggested file
/// name in, one outcome out. `Send + Sync` so it can run on the worker thread.
pub type SaveChooser =
    Arc<dyn Fn(SavePurpose, &MacosSaveDialogLabels, &str) -> FileChooserOutcome + Send + Sync>;

/// The real native save chooser (in-process AppKit `NSSavePanel`).
///
/// Runs on the single-flight worker; the panel itself is created inside a
/// main-thread hop (`AppHandle::run_on_main_thread`) and the worker blocks on a
/// one-shot channel for the outcome, exactly like the open chooser. `runModal`
/// pumps AppKit's modal loop on the main thread, so the host stays responsive.
#[cfg(target_os = "macos")]
pub fn native_save(
    app: &tauri::AppHandle,
    purpose: SavePurpose,
    labels: &MacosSaveDialogLabels,
    default_name: &str,
) -> FileChooserOutcome {
    let (outcome_tx, outcome_rx) = std::sync::mpsc::sync_channel::<FileChooserOutcome>(1);
    let main_labels = labels.clone();
    let default_name = default_name.to_string();
    let dispatched = app.run_on_main_thread(move || {
        // Capacity 1 means this send never blocks, even when the worker has
        // already gone away; a dropped receiver is not an error worth reporting.
        let _ = outcome_tx.send(show_save_panel(purpose, &main_labels, &default_name));
    });
    if let Err(error) = dispatched {
        return FileChooserOutcome::Failed(labels.picker_failed(&error.to_string()));
    }
    match outcome_rx.recv() {
        Ok(outcome) => outcome,
        Err(error) => FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    }
}

/// Build, configure and run `NSSavePanel` on the main thread.
///
/// The suggested name is prefilled verbatim (`SAVE` already supplies
/// `<stem>.ymodel`; the panel appends the first allowed extension to a bare
/// name). `setAllowsOtherFileTypes(false)` makes the per-purpose extension set a
/// real filter, not a hint. The panel's OK response with a resolved local path is
/// the only way a path is produced; any non-OK response is a cancellation.
#[cfg(target_os = "macos")]
#[allow(deprecated)] // `setAllowedFileTypes:` is deprecated but still the only
                     // extension filter available without UTType bindings.
fn show_save_panel(
    purpose: SavePurpose,
    labels: &MacosSaveDialogLabels,
    default_name: &str,
) -> FileChooserOutcome {
    use std::path::PathBuf;

    use objc2_app_kit::{NSModalResponseOK, NSSavePanel};
    use objc2_foundation::{NSArray, NSString};

    let mtm = match objc2::MainThreadMarker::new() {
        Some(mtm) => mtm,
        None => {
            return FileChooserOutcome::Failed(
                labels.picker_failed("the system save panel requires the main thread"),
            );
        }
    };

    let panel = NSSavePanel::savePanel(mtm);
    panel.setCanCreateDirectories(true);
    panel.setExtensionHidden(false);
    // The extension set *is* the per-purpose filter; an unlisted suffix would be
    // written by no writer in the shared host.
    panel.setAllowsOtherFileTypes(false);

    let title = NSString::from_str(labels.title(purpose));
    panel.setTitle(Some(&title));
    let filter = NSString::from_str(labels.filter(purpose));
    panel.setMessage(Some(&filter));
    let name = NSString::from_str(default_name);
    panel.setNameFieldStringValue(&name);

    let types = NSArray::from_retained_slice(
        &save_allowed_extensions(purpose)
            .iter()
            .map(|extension| NSString::from_str(extension))
            .collect::<Vec<_>>(),
    );
    panel.setAllowedFileTypes(Some(&types));

    let accepted = panel.runModal() == NSModalResponseOK;
    if !accepted {
        return FileChooserOutcome::Cancelled;
    }
    match panel.URL().and_then(|url| url.path()) {
        Some(path) => FileChooserOutcome::Path(PathBuf::from(path.to_string())),
        None => FileChooserOutcome::Failed(labels.picker_local.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{FileTarget, ReactFiles};
    use cad_platform::PlatformFiles;
    use std::path::PathBuf;
    use std::time::Duration;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "yacr-macos-save-test-{}-{name}",
            std::process::id()
        ))
    }

    fn wait_event(files: &ReactFiles) -> Vec<cad_platform::FileEvent> {
        for _ in 0..300 {
            let events = files.poll();
            if !events.is_empty() {
                return events;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no file event within the bounded wait");
    }

    fn files_with_save(save: SaveChooser) -> ReactFiles {
        ReactFiles::with_choosers(
            "zh-CN",
            Arc::new(|_, _| FileChooserOutcome::Cancelled),
            Some(save),
        )
        .expect("labels load")
    }

    /// (a) `SAVE` prefills the host-supplied `<stem>.ymodel` and offers exactly
    /// the native archive filter; the other purposes get their own sets and never
    /// include `.ymodel`.
    #[test]
    fn save_ymodel_prefills_the_name_and_filters_to_the_native_archive() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = seen.clone();
        let files = files_with_save(Arc::new(move |purpose, _, default_name| {
            recorder
                .lock()
                .unwrap()
                .push((purpose, default_name.to_string()));
            FileChooserOutcome::Cancelled
        }));
        files
            .begin_save(SavePurpose::SaveYmodel, "plan.ymodel")
            .expect("admitted");
        assert!(matches!(
            wait_event(&files).as_slice(),
            [cad_platform::FileEvent::Cancelled { .. }]
        ));
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            [(SavePurpose::SaveYmodel, "plan.ymodel".to_string())],
            "the chooser receives the purpose and the host's exact suggested name"
        );

        assert_eq!(
            save_allowed_extensions(SavePurpose::SaveYmodel),
            &["ymodel"][..]
        );
        for extension in [
            "dxf", "dwg", "stl", "obj", "ply", "off", "coff", "ifc", "skp", "glb", "gltf",
        ] {
            assert!(
                save_allowed_extensions(SavePurpose::SaveExport).contains(&extension),
                "EXPORT must offer .{extension}"
            );
        }
        assert!(!save_allowed_extensions(SavePurpose::SaveExport).contains(&"ymodel"));
        assert_eq!(
            save_allowed_extensions(SavePurpose::Plot),
            &["svg", "pdf"][..]
        );
        assert!(!save_allowed_extensions(SavePurpose::Plot).contains(&"ymodel"));
    }

    /// (b) A cancelled panel is `Cancelled` and writes nothing.
    #[test]
    fn cancelled_save_is_a_cancel_and_writes_nothing() {
        let directory = temp_path("cancel");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let intended = directory.join("plan.ymodel");
        let files = files_with_save(Arc::new(|_, _, _| FileChooserOutcome::Cancelled));
        files
            .begin_save(SavePurpose::SaveYmodel, "plan.ymodel")
            .unwrap();
        match wait_event(&files).as_slice() {
            [cad_platform::FileEvent::Cancelled { purpose }] => assert_eq!(
                *purpose,
                cad_platform::FilePurpose::Save(SavePurpose::SaveYmodel)
            ),
            other => panic!("expected Cancelled, got {other:?}"),
        }
        assert!(
            !intended.exists(),
            "a cancelled chooser must not write a file"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// (c) A failed panel is `Failed` with its reason and writes nothing.
    #[test]
    fn failed_save_reports_the_reason_and_writes_nothing() {
        let files = files_with_save(Arc::new(|_, labels, _| {
            FileChooserOutcome::Failed(labels.picker_failed("panel down"))
        }));
        files
            .begin_save(SavePurpose::SaveExport, "plan.dxf")
            .unwrap();
        match wait_event(&files).as_slice() {
            [cad_platform::FileEvent::Failed { purpose, reason }] => {
                assert_eq!(
                    *purpose,
                    cad_platform::FilePurpose::Save(SavePurpose::SaveExport)
                );
                assert!(reason.contains("panel down"), "{reason}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// (d) The carrier's atomic write keeps the previous bytes as `.bak`, and
    /// (e) a failed write leaves the target untouched (see `platform/fs.rs` for
    /// the failure path). Here the frontend-facing target round-trips.
    #[test]
    fn write_atomic_replaces_the_target_and_keeps_a_bak() {
        use cad_platform::PlatformFiles;

        let directory = temp_path("write");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let target = directory.join("plan.ymodel");
        std::fs::write(&target, b"previous document").unwrap();

        let files = files_with_save(Arc::new(|_, _, _| FileChooserOutcome::Cancelled));
        let file_target = FileTarget::from_path(&target.to_string_lossy());
        files
            .write_atomic(&file_target, b"next document")
            .expect("written");

        assert_eq!(std::fs::read(&target).unwrap(), b"next document");
        assert_eq!(
            std::fs::read(directory.join("plan.ymodel.bak")).unwrap(),
            b"previous document"
        );
        assert_eq!(
            files.read_target(&file_target).expect("re-readable"),
            b"next document".to_vec(),
            "the in-place conflict check can re-read the target"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }
}
