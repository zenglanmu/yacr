//! The Linux folder chooser behind `cad_pick_directory` (Options > Files).
//!
//! One call to [`portal_pick_directory`] runs a **single-directory** XDG desktop
//! portal chooser (`ashpd` `SelectedFiles::open_file().directory(true)`) and
//! returns the picked absolute path, a cancellation, or an explicit failure.
//!
//! It is a blocking function on purpose: the shared picker
//! ([`cad_desktop_shell::directory::DirectoryPicker`]) runs it on its own worker
//! thread, so the GTK main thread never blocks on the dialog. The blocking portal
//! is the same one the open/save choosers in [`super::save`] use.
//!
//! The dialog title is the empty string: the options row already labels the field,
//! and inventing host-side copy would need a new i18n key for a string the portal
//! only shows as a window caption.

use cad_desktop_shell::directory::DirectoryOutcome;

/// Run the portal folder chooser and classify the outcome.
///
/// A closed dialog (`ResponseError::Cancelled`) is [`DirectoryOutcome::Cancelled`],
/// never a failure. A portal error, a non-local URI and a relative path are
/// [`DirectoryOutcome::Failed`] with the reason relayed verbatim, so the command
/// can report `pick_directory.failed: …` instead of pretending the user cancelled.
pub fn portal_pick_directory() -> DirectoryOutcome {
    use ashpd::desktop::file_chooser::SelectedFiles;
    use ashpd::desktop::ResponseError;

    let result = pollster::block_on(async {
        SelectedFiles::open_file()
            .title("")
            .modal(true)
            .directory(true)
            // Single pick: the settings row holds exactly one directory.
            .multiple(false)
            .send()
            .await?
            .response()
    });
    match result {
        Ok(files) => match files.uris().first() {
            Some(uri) => match uri.to_file_path() {
                Ok(path) if path.is_absolute() => {
                    DirectoryOutcome::Path(path.to_string_lossy().into_owned())
                }
                Ok(path) => DirectoryOutcome::Failed(format!(
                    "the portal returned a relative path: {}",
                    path.display()
                )),
                Err(_) => {
                    DirectoryOutcome::Failed("the portal returned a non-local location".to_string())
                }
            },
            // A directory-mode pick with no URI is not a cancel: nothing was
            // chosen and nothing said so.
            None => DirectoryOutcome::Failed("the portal returned no directory".to_string()),
        },
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => DirectoryOutcome::Cancelled,
        Err(error) => DirectoryOutcome::Failed(error.to_string()),
    }
}
