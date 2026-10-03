//! Desktop portal selection, with cancellation distinct from service failure.
use super::*;
use ashpd::desktop::{
    file_chooser::{FileFilter, SelectedFiles},
    ResponseError,
};

pub(super) fn pick(locale: &str) -> CadResult<PathBuf> {
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
