//! The **bytes-based** session entry: a drawing that arrives as raw bytes
//! instead of a filesystem path.
//!
//! A browser tab (and an Android WebView) has no filesystem and no `PathBuf` to
//! open: the page reads a `File` through the DOM, and the bytes are the only
//! thing that exists. [`CadSession::from_bytes`] exists so that carrier can build
//! the **same** owner-thread session the native Wayland host drives, instead of
//! growing a second CAD session implementation (ADR 0024 D2 / rule 8: the wasm
//! session is the single `cad-db` authority, the native shell never mirrors it).
//!
//! It adds no import pipeline of its own: the document travels through
//! [`cad_host::HostCore::deliver_external_open`], the host's existing delivery
//! funnel, so the sync-vs-worker choice stays exactly what the host advertises
//! (`HostCapabilities::import_worker`) — the browser reports `false` and gets the
//! synchronous install path that already exists for headless/CLI hosts.

use std::sync::Arc;

use cad_domain::{CadError, CadResult};
use cad_engine::render::RenderSessionHandle;
use cad_host::{HostCore, HostOptions, HostPlatform, RequestClock};
use cad_platform::FileTarget;

use super::CadSession;

/// A drawing supplied as bytes, plus the label the status line shows.
///
/// `label` is the source file name (`report.dwg`, `unsaved-1.dwg`): it becomes
/// both the status `documentLabel` and the `FileTarget` display name, and it is
/// **never** treated as a path — a browser has no directory to resolve a sibling
/// `fonts/`/`images/` package from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BytesDocument<'a> {
    /// Source label shown by the status line (must not be blank).
    pub label: &'a str,
    /// The raw drawing bytes.
    pub bytes: &'a [u8],
}

impl CadSession {
    /// Build the session over a drawing that arrives as **bytes**.
    ///
    /// Same composition as [`CadSession::new`] — host core, startup config, then
    /// the resource installs and the document — with two differences that the
    /// carrier forces:
    ///
    /// 1. **The clock is injected.** `HostOptions`/`HostCore` install a default
    ///    monotonic clock only off wasm32; on a wasm target `HostCore::new`
    ///    deliberately installs *none*, so any queued command would be refused
    ///    with "request clock is not configured for this host". The caller passes
    ///    its platform clock (`performance.now()` in the browser, any monotonic
    ///    millisecond source on a native host) and it is validated and installed
    ///    before a single request can be queued.
    /// 2. **The document arrives as bytes**, delivered through the host's own
    ///    funnel ([`cad_host::HostCore::deliver_external_open`]): a real parse
    ///    failure returns `Err` and leaves the previous document alone; nothing
    ///    is turned into an empty success.
    ///
    /// `options.drawing` must be `None`: naming a path here would be a second,
    /// contradictory document source, so it is refused explicitly instead of one
    /// of the two silently winning.
    ///
    /// No surface is required at construction (the browser reports its canvas
    /// metrics after the first layout). Until it does,
    /// [`CadSession::tick`] refuses with the host's explicit `Cancelled` — a CAD
    /// frame is never rendered against a fabricated size.
    pub fn from_bytes(
        options: HostOptions,
        platform: HostPlatform,
        render: RenderSessionHandle,
        clock: RequestClock,
        document: BytesDocument<'_>,
    ) -> CadResult<Self> {
        if options.drawing.is_some() {
            return Err(CadError::InvalidInput(
                "bytes session: HostOptions.drawing must be None (the document is the given bytes)"
                    .into(),
            ));
        }
        if document.label.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "bytes session: a non-empty document label is required".into(),
            ));
        }
        let core = HostCore::new(options, platform, render)?;
        // Before any queued request: without an installed clock the pump refuses
        // pending work rather than inventing an elapsed time.
        core.set_request_clock(clock)?;
        // Startup config diagnostics are retained on the core, not fatal.
        let _startup_diagnostics = core.load_startup();
        // A strategy that cannot resolve a resource refuses explicitly, so a real
        // failure is propagated instead of being hidden behind a demo document.
        core.install_fonts()?;
        core.install_images()?;
        // The label is *the* handle for a file that has no path: it is what the
        // status line shows and what a save would suggest. It is never resolved
        // against the filesystem (the platform refuses every file operation).
        let target = FileTarget {
            display_name: document.label.to_string(),
            opaque: document.label.to_string(),
        };
        core.deliver_external_open(target, Arc::from(document.bytes))?;
        Ok(Self::wrap(core))
    }
}
