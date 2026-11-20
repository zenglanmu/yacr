//! The real COM file dialogs of the Windows host.
//!
//! `IFileOpenDialog` and `IFileSaveDialog` are shell STA objects; each chooser
//! runs on its own worker thread and owns its COM apartment for the whole call.
//! The dialogs have no owner HWND: the host never passes a window handle across
//! threads, so they are not modal to the Tauri window (they still appear in the
//! foreground because the calling process owns them).
//!
//! Both functions are called through the dependency-injected seams in
//! `platform.rs` (`FileChooser`/`SaveChooser`), so this is the only part of the
//! file layer that needs the real Windows target to type-check. Failures are
//! explicit and localized; a user cancellation is
//! [`FileChooserOutcome::Cancelled`], never a failure and never a fabricated path.

use std::path::{Path, PathBuf};

use cad_platform::SavePurpose;
use windows::core::{Interface, HRESULT, PCWSTR};
use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileOpenDialog, FileSaveDialog, IFileOpenDialog, IFileSaveDialog, IShellItem,
    SHCreateItemFromParsingName, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT,
    FOS_PATHMUSTEXIST, SIGDN_FILESYSPATH,
};

use super::save::{save_default_extension, save_filter_spec, WindowsSaveDialogLabels};
use super::{FileChooserOutcome, WindowsOpenDialogLabels};

/// A COM apartment for the worker thread that shows a native chooser.
///
/// Both dialogs are STA objects; the worker owns the apartment for its whole
/// lifetime. `CoUninitialize` is called only when initialization succeeded
/// (including `S_FALSE`, "already initialized on this thread").
struct Apartment {
    initialized: bool,
}

impl Apartment {
    fn new() -> Result<Self, String> {
        let result = unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            )
        };
        if result.is_err() {
            return Err(format!("CoInitializeEx failed: {result:?}"));
        }
        Ok(Self { initialized: true })
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

/// The real native open chooser (COM `IFileOpenDialog`).
///
/// Every extension the shared importer can read is offered in one filter; a
/// scheme handle or a failed shell call is an explicit localized failure.
pub fn native_choose(
    labels: &WindowsOpenDialogLabels,
    initial: Option<&Path>,
) -> FileChooserOutcome {
    let _apartment = match Apartment::new() {
        Ok(apartment) => apartment,
        Err(reason) => return FileChooserOutcome::Failed(labels.picker_failed(&reason)),
    };

    let dialog: IFileOpenDialog = match unsafe {
        CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
    } {
        Ok(dialog) => dialog,
        Err(error) => return FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    };

    let title = wide(&labels.picker_title);
    let filter_name = wide(&labels.picker_filter);
    let filter_spec = wide(&super::open_filter_spec());
    let filters = [COMDLG_FILTERSPEC {
        pszName: PCWSTR::from_raw(filter_name.as_ptr()),
        pszSpec: PCWSTR::from_raw(filter_spec.as_ptr()),
    }];

    let configure = unsafe {
        dialog
            .SetTitle(PCWSTR::from_raw(title.as_ptr()))
            .and_then(|()| dialog.SetFileTypes(&filters))
            .and_then(|()| dialog.SetOptions(FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM))
    };
    if let Err(error) = configure {
        return FileChooserOutcome::Failed(labels.picker_failed(&error.to_string()));
    }

    // Start in the remembered directory when it still exists. A hint that cannot
    // be resolved is a convenience loss, never a reason to fail the open.
    if let Ok(dialog_folder) = dialog.cast::<windows::Win32::UI::Shell::IFileDialog>() {
        start_in(&dialog_folder, initial);
    }

    match unsafe { dialog.Show(None) } {
        Ok(()) => {}
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => {
            return FileChooserOutcome::Cancelled;
        }
        Err(error) => return FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    }

    let item: IShellItem = match unsafe { dialog.GetResult() } {
        Ok(item) => item,
        Err(error) => return FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    };
    match unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) } {
        Ok(raw) => match crate::support::take_pwstr(raw) {
            Some(path) => FileChooserOutcome::Path(PathBuf::from(path)),
            None => FileChooserOutcome::Failed(labels.picker_local.clone()),
        },
        Err(error) => FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    }
}

/// The real native save chooser (COM `IFileSaveDialog`).
///
/// `SAVE` offers `*.ymodel` only (ADR-0041 D1); `EXPORT` offers every writable
/// connector format in one filter and the chosen extension decides the writer;
/// `PLOT` offers SVG/PDF. `default_name` is prefilled with `SetFileName` so the
/// user's current document name is the starting point, never a renamed copy.
/// The shell's own overwrite prompt is kept on purpose: overwriting is allowed,
/// and the atomic write keeps the previous bytes in `<name>.<ext>.bak`.
pub fn native_save(
    purpose: SavePurpose,
    labels: &WindowsSaveDialogLabels,
    default_name: &str,
) -> FileChooserOutcome {
    let _apartment = match Apartment::new() {
        Ok(apartment) => apartment,
        Err(reason) => return FileChooserOutcome::Failed(labels.picker_failed(&reason)),
    };

    let dialog: IFileSaveDialog = match unsafe {
        CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)
    } {
        Ok(dialog) => dialog,
        Err(error) => return FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    };

    // Every string buffer is kept alive for the whole COM call; `COMDLG_FILTERSPEC`
    // holds raw pointers into them.
    let title = wide(labels.title(purpose));
    let filter_name = wide(labels.filter(purpose));
    let filter_spec = wide(save_filter_spec(purpose));
    let file_name = wide(default_name);
    let default_extension = save_default_extension(purpose).map(wide);
    let filters = [COMDLG_FILTERSPEC {
        pszName: PCWSTR::from_raw(filter_name.as_ptr()),
        pszSpec: PCWSTR::from_raw(filter_spec.as_ptr()),
    }];

    let configure = unsafe {
        dialog
            .SetTitle(PCWSTR::from_raw(title.as_ptr()))
            .and_then(|()| dialog.SetFileTypes(&filters))
            .and_then(|()| dialog.SetFileName(PCWSTR::from_raw(file_name.as_ptr())))
            .and_then(|()| {
                match &default_extension {
                    Some(extension) => {
                        dialog.SetDefaultExtension(PCWSTR::from_raw(extension.as_ptr()))
                    }
                    // An export's extension *is* the writer choice: defaulting it
                    // would silently pick a format the user did not choose.
                    None => Ok(()),
                }
            })
            .and_then(|()| {
                // Overwriting is allowed (ADR-0041 D4); the shell prompt is the
                // documented confirmation, and `write_atomic` keeps the `.bak`.
                dialog.SetOptions(FOS_OVERWRITEPROMPT | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
            })
    };
    if let Err(error) = configure {
        return FileChooserOutcome::Failed(labels.picker_failed(&error.to_string()));
    }

    match unsafe { dialog.Show(None) } {
        Ok(()) => {}
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => {
            return FileChooserOutcome::Cancelled;
        }
        Err(error) => return FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    }

    let item: IShellItem = match unsafe { dialog.GetResult() } {
        Ok(item) => item,
        Err(error) => return FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    };
    match unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) } {
        Ok(raw) => match crate::support::take_pwstr(raw) {
            Some(path) => FileChooserOutcome::Path(PathBuf::from(path)),
            None => FileChooserOutcome::Failed(labels.picker_local.clone()),
        },
        Err(error) => FileChooserOutcome::Failed(labels.picker_failed(&error.to_string())),
    }
}

/// Navigate a dialog to `initial` when it is a real directory.
///
/// `SetFolder` is honoured by the shell; `SetDefaultFolder` is the fallback some
/// versions use. A hint that cannot be resolved is ignored.
fn start_in(dialog: &windows::Win32::UI::Shell::IFileDialog, initial: Option<&Path>) {
    let Some(directory) = initial.filter(|directory| directory.is_dir()) else {
        return;
    };
    let path = wide(&directory.to_string_lossy());
    let bind: Option<&windows::Win32::System::Com::IBindCtx> = None;
    if let Ok(folder) = unsafe {
        SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR::from_raw(path.as_ptr()), bind)
    } {
        let _ = unsafe { dialog.SetFolder(&folder) };
        let _ = unsafe { dialog.SetDefaultFolder(&folder) };
    }
}

/// A NUL-terminated UTF-16 copy of `value` (kept alive across the COM call).
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
