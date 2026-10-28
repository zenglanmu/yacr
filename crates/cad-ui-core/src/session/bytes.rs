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

    /// Replace the open document with new **bytes**, in place.
    ///
    /// This is the carrier's second open: [`CadSession::from_bytes`] builds the
    /// session, and this replaces the document inside it, through the **same**
    /// host funnel the desktop `open` uses
    /// ([`cad_host::HostCore::deliver_external_open`]). Nothing about the session
    /// is rebuilt: the render session, its attached device, the platform adapters,
    /// the command log and the import counter all stay, exactly as on the desktop
    /// hosts where opening a second file is a document replacement, not a new
    /// session.
    ///
    /// # Atomicity
    ///
    /// A refused delivery returns `Err` and leaves the previous document alone
    /// (the host's own contract: the parse failure is reported without publishing
    /// or advancing the document epoch). The caller's live session therefore keeps
    /// its pixels and stays navigable, which is what makes an over-budget or
    /// unparseable file a refusal instead of a blank canvas.
    ///
    /// # What it does not do
    ///
    /// It runs **no** memory budget: an ADR 0025 input-byte or wasm-heap budget
    /// belongs to the host that owns the linear memory (`apps/app-web` measures the
    /// module heap) and must be checked *before* calling this — a check after the
    /// delivery cannot undo it. It also adds no import pipeline of its own: the
    /// sync-vs-worker choice stays the host's advertised
    /// [`cad_platform::HostCapabilities::import_worker`].
    pub fn open_bytes(&self, label: &str, bytes: &[u8]) -> CadResult<()> {
        if label.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "bytes open: a non-empty document label is required".into(),
            ));
        }
        // The label is the handle for a file that has no path (see
        // [`CadSession::from_bytes`]); it is never resolved on a filesystem.
        let target = FileTarget {
            display_name: label.to_string(),
            opaque: label.to_string(),
        };
        self.core.deliver_external_open(target, Arc::from(bytes))
    }
}

#[cfg(test)]
mod tests {
    //! Contract tests for the bytes entries, on the host target (no GPU, no DOM).
    //!
    //! A `RenderSessionHandle` with no attached device is enough: the import, the
    //! document replacement and the status projection are the CAD half, and none of
    //! them needs a renderer.

    use std::cell::Cell;
    use std::rc::Rc;

    use cad_engine::render::{BackendPreference, RenderSessionHandle};
    use cad_host::{HostOptions, RequestClock};
    use cad_platform::HostCapabilities;

    use super::*;

    const SYNTHETIC: &[u8] = include_bytes!("../../../../fixtures/dwg/synthetic-four-lines.dwg");
    const SYNTHETIC_LABEL: &str = "synthetic-four-lines.dwg";

    fn mono_clock() -> RequestClock {
        let millis = Cell::new(0.0_f64);
        Rc::new(move || {
            let now = millis.get();
            millis.set(now + 0.05);
            now
        })
    }

    /// A session over the synthetic fixture with a reported surface (so a tick is
    /// not refused as an unmapped canvas).
    ///
    /// The platform adapters are the CLI-style no-ops: this test is about the
    /// document replacement, not about the browser carrier.
    fn session() -> CadSession {
        let platform = cad_host::HostPlatform {
            files: Rc::new(NoFiles),
            config_store: Rc::new(NoConfig),
            surface: Rc::new(FixedSurface),
            resources: Rc::new(NoResources),
            capabilities: HostCapabilities::default(),
        };
        CadSession::from_bytes(
            HostOptions {
                fonts: Vec::new(),
                fonts_dir: None,
                images_dir: None,
                headless: false,
                drawing: None,
                size: [800.0, 600.0],
            },
            platform,
            RenderSessionHandle::new(BackendPreference::WebGl2),
            mono_clock(),
            BytesDocument {
                label: SYNTHETIC_LABEL,
                bytes: SYNTHETIC,
            },
        )
        .expect("the synthetic fixture opens from bytes")
    }

    struct NoFiles;
    impl cad_platform::PlatformFiles for NoFiles {
        fn begin_open(&self) -> CadResult<()> {
            Err(CadError::Unsupported("no filesystem".into()))
        }
        fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
            Err(CadError::Unsupported("no filesystem".into()))
        }
        fn begin_save(
            &self,
            _purpose: cad_platform::SavePurpose,
            _default_name: &str,
        ) -> CadResult<()> {
            Err(CadError::Unsupported("no filesystem".into()))
        }
        fn write_atomic(&self, _target: &FileTarget, _bytes: &[u8]) -> CadResult<()> {
            Err(CadError::Unsupported("no filesystem".into()))
        }
        fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
            false
        }
        fn set_source(&self, _target: Option<&FileTarget>) {}
        fn poll(&self) -> Vec<cad_platform::FileEvent> {
            Vec::new()
        }
    }

    struct NoConfig;
    impl cad_platform::ConfigStore for NoConfig {
        fn load_config(&self) -> cad_platform::ConfigLoad {
            cad_platform::ConfigLoad::Absent
        }
        fn load_preference(&self) -> cad_platform::ConfigLoad {
            cad_platform::ConfigLoad::Absent
        }
        fn persist_preference(&self, _text: &str) -> CadResult<()> {
            Ok(())
        }
    }

    struct FixedSurface;
    impl cad_platform::SurfaceSource for FixedSurface {
        fn surface(&self) -> Option<cad_platform::SurfaceMetrics> {
            Some(cad_platform::SurfaceMetrics {
                logical: [800.0, 600.0],
                scale: 1.0,
                safe_insets: [0.0; 4],
            })
        }
    }

    /// A resources strategy that installs nothing and claims nothing.
    ///
    /// The fixture references no font or image, so this is the honest "nothing was
    /// requested, nothing is installed" no-op the browser carrier also reports (a
    /// drawing that *did* request a face would be refused by that strategy, which
    /// is a carrier property this test does not need).
    struct NoResources;
    impl cad_host::HostResources for NoResources {
        fn install_fonts(
            &self,
            _core: &cad_host::HostCore,
            render: &RenderSessionHandle,
        ) -> CadResult<()> {
            render.set_fonts(None);
            Ok(())
        }
        fn install_images(
            &self,
            _core: &cad_host::HostCore,
            render: &RenderSessionHandle,
        ) -> CadResult<()> {
            render.set_images(None);
            Ok(())
        }
    }

    #[test]
    fn open_bytes_replaces_the_document_in_place_and_counts_the_import() {
        let session = session();
        session
            .set_canvas_metrics([800.0, 600.0], 1.0)
            .expect("valid metrics");
        session.tick().expect("a mapped surface ticks");
        let before = session.snapshot(None, None, 0);
        assert_eq!(before.document_label.as_deref(), Some(SYNTHETIC_LABEL));
        assert_eq!(before.import_count, 1);
        let entities = before.entity_count.expect("the fixture has entities");

        // The same bytes under a different label: the document is replaced, the
        // session is not.
        session
            .open_bytes("second-name.dwg", SYNTHETIC)
            .expect("a valid drawing replaces the open document");
        session.tick().expect("the replaced document ticks");
        let after = session.snapshot(None, None, 0);
        assert_eq!(after.document_label.as_deref(), Some("second-name.dwg"));
        assert_eq!(
            after.import_count, 2,
            "a replacement is a second successful import, not a re-count of the first"
        );
        assert_eq!(
            after.entity_count,
            Some(entities),
            "the same bytes produce the same document"
        );
        assert_eq!(after.error, None);
    }

    #[test]
    fn a_refused_replacement_leaves_the_open_document_alone() {
        let session = session();
        session.set_canvas_metrics([800.0, 600.0], 1.0).unwrap();
        session.tick().unwrap();

        let blank = session.open_bytes("  ", SYNTHETIC).unwrap_err();
        assert!(
            matches!(blank, CadError::InvalidInput(_)),
            "a blank label is refused before anything is delivered"
        );
        let garbage = session
            .open_bytes("garbage.dwg", b"this is not a DWG file")
            .unwrap_err();
        assert!(!garbage.to_string().is_empty(), "the importer's own reason");

        let after = session.snapshot(None, None, 0);
        assert_eq!(
            after.document_label.as_deref(),
            Some(SYNTHETIC_LABEL),
            "a refused replacement publishes no document"
        );
        assert_eq!(after.import_count, 1, "a refusal is not an import");
        assert!(
            after.entity_count.unwrap_or(0) > 0,
            "the previous document is still the authoritative one"
        );
    }
}
