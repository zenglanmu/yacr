//! Native file selection.
//!
//! Linux uses the XDG desktop portal (ashpd); Windows and macOS use the common
//! native dialog through `rfd` (Win32 common dialog / `NSOpenPanel`). In every
//! case cancellation is distinct from a service failure so the UI can say which
//! happened.
use super::*;

#[cfg(target_os = "linux")]
pub(super) fn pick(locale: &str) -> CadResult<PathBuf> {
    use ashpd::desktop::{
        file_chooser::{FileFilter, SelectedFiles},
        ResponseError,
    };
    let messages = cad_ui_slint::MessageSource::from_request(locale);
    let title = messages.text("linux.picker_title", &[]);
    let result = pollster::block_on(async {
        SelectedFiles::open_file()
            .title(title.as_str())
            .modal(true)
            .multiple(false)
            .filter(
                FileFilter::new(&messages.text("linux.picker_filter", &[]))
                    .glob("*.[dD][wW][gG]")
                    .glob("*.[dD][xX][fF]"),
            )
            .send()
            .await?
            .response()
    });
    match result {
        Ok(files) => {
            let uri = files
                .uris()
                .first()
                .ok_or_else(|| CadError::InvalidInput(messages.text("linux.picker_empty", &[])))?;
            uri.to_file_path()
                .map_err(|_| CadError::InvalidInput(messages.text("linux.picker_local", &[])))
        }
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => Err(CadError::Cancelled),
        Err(error) => Err(CadError::Unsupported(
            messages.text("linux.picker_failed", &[("error", &error.to_string())]),
        )),
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(super) fn pick(locale: &str) -> CadResult<PathBuf> {
    let messages = cad_ui_slint::MessageSource::from_request(locale);
    let selection = rfd::FileDialog::new()
        .set_title(messages.text("linux.picker_title", &[]))
        .add_filter(messages.text("linux.picker_filter", &[]), &["dwg", "dxf"])
        .pick_file();
    // `rfd` returns `None` both for cancel and for a dialog that could not open;
    // it exposes no distinct error, so a closed dialog is reported as a cancel
    // rather than inventing a success or a specific failure.
    selection.ok_or(CadError::Cancelled)
}

/// Native SAVE dialog for the vector plot export (SVG/PDF).
///
/// Linux uses the same XDG desktop portal (ashpd `SaveFile`) as the open picker;
/// Windows/macOS use `rfd`'s native save dialog. A closed dialog is a cancel; a
/// service failure is explicit, never a fabricated path.
#[cfg(target_os = "linux")]
pub(super) fn pick_save(locale: &str, default_name: &str) -> CadResult<PathBuf> {
    use ashpd::desktop::{
        file_chooser::{FileFilter, SelectedFiles},
        ResponseError,
    };
    let messages = cad_ui_slint::MessageSource::from_request(locale);
    let title = messages.text("plot.save_title", &[]);
    let result = pollster::block_on(async {
        SelectedFiles::save_file()
            .title(title.as_str())
            .modal(true)
            .current_name(default_name)
            .filter(
                FileFilter::new(&messages.text("plot.save_filter", &[]))
                    .glob("*.[sS][vV][gG]")
                    .glob("*.[pP][dD][fF]"),
            )
            .send()
            .await?
            .response()
    });
    match result {
        Ok(files) => {
            let uri = files
                .uris()
                .first()
                .ok_or_else(|| CadError::InvalidInput(messages.text("linux.picker_empty", &[])))?;
            uri.to_file_path()
                .map_err(|_| CadError::InvalidInput(messages.text("linux.picker_local", &[])))
        }
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => Err(CadError::Cancelled),
        Err(error) => Err(CadError::Unsupported(
            messages.text("linux.picker_failed", &[("error", &error.to_string())]),
        )),
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(super) fn pick_save(locale: &str, default_name: &str) -> CadResult<PathBuf> {
    let messages = cad_ui_slint::MessageSource::from_request(locale);
    let selection = rfd::FileDialog::new()
        .set_title(messages.text("plot.save_title", &[]))
        .set_file_name(default_name)
        .add_filter(messages.text("plot.save_filter", &[]), &["svg", "pdf"])
        .save_file();
    selection.ok_or(CadError::Cancelled)
}
