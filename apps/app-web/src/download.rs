//! The browser [`PlatformFiles`]: **save/plot are real downloads**, open is not.
//!
//! A page has no filesystem, but it does have a download gesture. The shared
//! session's export funnel (`cad_host::save`) asks the platform for a save target
//! inside the user gesture and then writes the finished bytes to it. On the web
//! there is no dialog to wait for, so this adapter **synthesizes the target**
//! (`download://<name>`) from the name the session proposed, exactly like the
//! `SyncSaveFiles` reference fake in `cad_host`'s tests. The funnel drains that
//! event immediately and finishes the export inside the same gesture — which is
//! what lets [`DownloadSink::publish`] run `Blob` + `<a download>` synchronously,
//! the one shape a browser accepts as a real user-initiated download.
//!
//! | interaction | browser answer |
//! |---|---|
//! | `begin_save` | **Ok**: one [`FileEvent::SaveTarget`] with `display_name = default_name` and `opaque = download://<default_name>`; a blank name is refused |
//! | `write_atomic` | `sink.publish(display_name, bytes)` — a real download on wasm, an in-memory record on the host target |
//! | `begin_open` / `begin_open_target` | [`crate::platform::UNSUPPORTED_FILESYSTEM`]: the page's file picker owns selection and the bytes arrive through the session's `open_bytes` entry |
//! | `refuses_source_overwrite` | `false` (a browser has no device+inode identity), `set_source` still round-trips |
//! | `poll` | drains every queued event exactly once |
//!
//! Nothing here fabricates a save the sink did not perform: a sink failure is
//! returned as the real error and **no** download is recorded.
//!
//! # Two sinks, one contract
//!
//! * `DomDownload` (wasm32 only) publishes a real browser download.
//! * [`RecordingSink`] (every target) records name+bytes, so `cargo test -p
//!   app-web` asserts the whole contract on the host without a DOM. On a native
//!   build [`PlatformDownload`] is the recording sink (there is no page to
//!   download into); on wasm32 it is `DomDownload`.

use std::cell::RefCell;

use cad_domain::{CadError, CadResult};
use cad_platform::{FileEvent, FileTarget, PlatformFiles, SavePurpose};

use crate::platform::UNSUPPORTED_FILESYSTEM;

/// Stable prefix of a browser download that failed after the gesture started.
pub const DOWNLOAD_FAILED: &str = "download-failed";

/// Where a finished export goes.
///
/// The session calls this from inside the SAVE/PLOT gesture; an implementation
/// must act immediately (a browser download is only allowed in that gesture) and
/// must report a real error when it cannot publish.
pub trait DownloadSink {
    /// Deliver `bytes` under `name`.
    fn publish(&self, name: &str, bytes: &[u8]) -> CadResult<()>;
}

/// One completed [`PlatformFiles::write_atomic`], for diagnostics and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadRecord {
    /// The download's file name (the session's chosen `display_name`).
    pub name: String,
    /// How many bytes the sink accepted.
    pub len: usize,
}

/// The MIME type a download of `name` is published with.
///
/// The extension decides; an unknown/absent extension is the generic binary type,
/// never a guessed document type.
pub fn download_mime(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("dxf") => "application/dxf",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// A [`PlatformFiles`] that turns the save half into downloads.
///
/// `S` is the publishing mechanism, so the contract is testable on the host
/// ([`RecordingSink`]) and real in the browser (`DomDownload`).
#[derive(Debug)]
pub struct BrowserFiles<S: DownloadSink> {
    sink: S,
    events: RefCell<Vec<FileEvent>>,
    source: RefCell<Option<FileTarget>>,
    downloads: RefCell<Vec<DownloadRecord>>,
}

impl<S: DownloadSink> BrowserFiles<S> {
    pub fn new(sink: S) -> Self {
        Self {
            sink,
            events: RefCell::new(Vec::new()),
            source: RefCell::new(None),
            downloads: RefCell::new(Vec::new()),
        }
    }

    /// The sink this adapter publishes through (diagnostics/tests).
    pub fn sink(&self) -> &S {
        &self.sink
    }

    /// Every successful `write_atomic`, in call order.
    pub fn downloads(&self) -> Vec<DownloadRecord> {
        self.downloads.borrow().clone()
    }

    /// The target of the last successful open install, if any.
    ///
    /// Diagnostics only: it is a **label**, not a path, and no operation below
    /// resolves it.
    pub fn source(&self) -> Option<FileTarget> {
        self.source.borrow().clone()
    }

    /// The open refusal: the page owns selection, the session owns the bytes.
    fn refuse_open(operation: &str) -> CadError {
        CadError::Unsupported(format!(
            "{UNSUPPORTED_FILESYSTEM}: {operation}: the page's file picker owns selection; \
             the bytes arrive through open_bytes"
        ))
    }
}

impl<S: DownloadSink + Default> Default for BrowserFiles<S> {
    fn default() -> Self {
        Self::new(S::default())
    }
}

impl<S: DownloadSink> PlatformFiles for BrowserFiles<S> {
    fn begin_open(&self) -> CadResult<()> {
        Err(Self::refuse_open("begin_open"))
    }

    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Err(Self::refuse_open("begin_open_target"))
    }

    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        if default_name.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "begin_save: default_name is empty; a browser download needs a real file name"
                    .to_string(),
            ));
        }
        self.events.borrow_mut().push(FileEvent::SaveTarget {
            purpose,
            target: FileTarget {
                display_name: default_name.to_string(),
                opaque: format!("download://{default_name}"),
            },
        });
        Ok(())
    }

    fn write_atomic(&self, target: &FileTarget, bytes: &[u8]) -> CadResult<()> {
        // The sink's error is the truth; nothing is recorded for a failed publish,
        // so a later diagnostic cannot claim a download that never happened.
        self.sink.publish(&target.display_name, bytes)?;
        self.downloads.borrow_mut().push(DownloadRecord {
            name: target.display_name.clone(),
            len: bytes.len(),
        });
        Ok(())
    }

    fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
        // No device+inode identity exists in a browser, so the guard cannot be
        // armed. Answering `true` would refuse every save; `false` is the
        // documented web answer (see `cad_platform::PlatformFiles`).
        false
    }

    fn set_source(&self, target: Option<&FileTarget>) {
        *self.source.borrow_mut() = target.cloned();
    }

    fn poll(&self) -> Vec<FileEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

/// The host-target sink: records the download instead of performing one.
///
/// There is no page in a native build, so this is the honest stand-in the unit
/// tests and diagnostics use; it is a real recorded write (name+bytes), not a
/// silent no-op.
#[derive(Debug, Default, Clone)]
pub struct RecordingSink {
    published: RefCell<Vec<(String, Vec<u8>)>>,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every publication in call order.
    pub fn published(&self) -> Vec<(String, Vec<u8>)> {
        self.published.borrow().clone()
    }

    /// How many publications were recorded.
    pub fn len(&self) -> usize {
        self.published.borrow().len()
    }

    /// Whether nothing was published yet.
    pub fn is_empty(&self) -> bool {
        self.published.borrow().is_empty()
    }
}

impl DownloadSink for RecordingSink {
    fn publish(&self, name: &str, bytes: &[u8]) -> CadResult<()> {
        self.published
            .borrow_mut()
            .push((name.to_string(), bytes.to_vec()));
        Ok(())
    }
}

/// The sink a native build uses (no page to download into).
#[cfg(not(target_arch = "wasm32"))]
pub type PlatformDownload = RecordingSink;

/// The sink the browser uses: a real DOM download.
#[cfg(target_arch = "wasm32")]
pub type PlatformDownload = DomDownload;

/// A real browser download via `Blob` + a synthetic `<a download>` click.
///
/// Everything runs synchronously inside the call so the download stays in the
/// same user gesture as the SAVE/PLOT click. Every DOM step is checked: a missing
/// document/body, a rejected `Blob`/object URL, a non-anchor element, a failed
/// append or removal all return [`DOWNLOAD_FAILED`] — never a silent no-op.
#[cfg(target_arch = "wasm32")]
#[derive(Debug, Default, Clone, Copy)]
pub struct DomDownload;

#[cfg(target_arch = "wasm32")]
impl DomDownload {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(target_arch = "wasm32")]
impl DownloadSink for DomDownload {
    fn publish(&self, name: &str, bytes: &[u8]) -> CadResult<()> {
        publish_dom_download(name, bytes)
            .map_err(|reason| CadError::Unsupported(format!("{DOWNLOAD_FAILED}: {name}: {reason}")))
    }
}

/// Publish `bytes` as a browser download named `name`.
///
/// The DOM steps, in order: `Blob` (with the name's MIME), object URL, an
/// `<a download=name href=objectUrl>`, click, remove, revoke. The reversals after
/// the click run on the success path too; a failure at any step is returned.
#[cfg(target_arch = "wasm32")]
fn publish_dom_download(name: &str, bytes: &[u8]) -> Result<(), String> {
    use wasm_bindgen::{JsCast, JsValue};

    let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let document = window.document().ok_or_else(|| "no document".to_string())?;
    let body = document
        .body()
        .ok_or_else(|| "no document body".to_string())?;

    let parts = js_sys::Array::new();
    let chunk = js_sys::Uint8Array::from(bytes);
    parts.push(&JsValue::from(&chunk));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(download_mime(name));
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .map_err(|error| format!("Blob constructor failed: {}", js_error(&error)))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|error| format!("createObjectURL failed: {}", js_error(&error)))?;

    let element = document
        .create_element("a")
        .map_err(|error| format!("createElement(a) failed: {}", js_error(&error)))?;
    let anchor = element
        .dyn_into::<web_sys::HtmlAnchorElement>()
        .map_err(|_| "the created element is not an HTML anchor".to_string())?;
    anchor.set_href(&url);
    anchor.set_download(name);
    body.append_child(&anchor)
        .map_err(|error| format!("appendChild(a) failed: {}", js_error(&error)))?;
    anchor.click();
    body.remove_child(&anchor)
        .map_err(|error| format!("removeChild(a) failed: {}", js_error(&error)))?;
    web_sys::Url::revoke_object_url(&url)
        .map_err(|error| format!("revokeObjectURL failed: {}", js_error(&error)))?;
    Ok(())
}

/// A JS exception's message, or its debug form when it is not a string.
#[cfg(target_arch = "wasm32")]
fn js_error(value: &wasm_bindgen::JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}

#[cfg(test)]
mod tests;
