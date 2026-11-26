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
use cad_host::{HostCore, HostOptions, HostPlatform, RequestClock, StartDocument};
use cad_platform::{DocumentArchive, FileTarget};

use super::CadSession;

/// A drawing supplied as bytes, plus the label the status line shows.
///
/// `label` is the source file name (`report.dwg`, `unsaved-1.dwg`): it becomes
/// both the status `documentLabel` and the `FileTarget` display name, and it is
/// **never** treated as a path — a browser has no directory to resolve a sibling
/// `fonts/`/`images/` package from.
///
/// `archive` is the carrier's `.ymodel` container adapter (ADR-0037 D16). It is
/// installed on the host core **before** the bytes are delivered, so a carrier
/// whose first document is itself an archive (a page upload) decodes it instead
/// of refusing the `.ymodel` target for want of an adapter. `None` keeps the
/// host's explicit `unsupported-runtime: ymodel` refusal.
#[derive(Clone)]
pub struct BytesDocument<'a> {
    /// Source label shown by the status line (must not be blank).
    pub label: &'a str,
    /// The raw drawing bytes.
    pub bytes: &'a [u8],
    /// The carrier's native document archive, installed before the delivery.
    pub archive: Option<Arc<dyn DocumentArchive>>,
}

impl<'a> BytesDocument<'a> {
    /// A document with no archive adapter (the host refuses a `.ymodel` target).
    pub fn new(label: &'a str, bytes: &'a [u8]) -> Self {
        Self {
            label,
            bytes,
            archive: None,
        }
    }

    /// The same document with the carrier's `.ymodel` adapter installed first.
    pub fn with_archive(mut self, archive: Arc<dyn DocumentArchive>) -> Self {
        self.archive = Some(archive);
        self
    }
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
    /// The carrier archive in [`BytesDocument::archive`] is installed **before**
    /// that delivery: the same bytes may be the native `.ymodel` container, and
    /// the host must be able to decode the first document instead of refusing it
    /// for want of an adapter.
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
        // The archive seam is installed before the bytes are delivered, so a
        // carrier whose first document is the native container (a page upload)
        // can decode it. No archive keeps the host's named refusal.
        core.set_document_archive(document.archive.clone());
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

    /// Build the session with **no document** (the start page).
    ///
    /// This is the production carrier's first state: the page shows its start
    /// panel and the session holds no `cad-db` document until the user opens one
    /// through [`Self::open_bytes`]. The composition is the same as
    /// [`Self::from_bytes`] minus the delivery step: the injected clock, startup
    /// config, fonts/images and the carrier archive are installed exactly once, so
    /// a later `open_bytes` is an ordinary replacement (and the archive can decode
    /// a first `.ymodel` upload).
    ///
    /// `options.drawing` must be `None` and `options.document_at_start` must be
    /// [`StartDocument::Empty`]; anything else would contradict "no document" and
    /// is refused instead of silently winning.
    pub fn from_empty(
        options: HostOptions,
        platform: HostPlatform,
        render: RenderSessionHandle,
        clock: RequestClock,
        archive: Option<Arc<dyn DocumentArchive>>,
    ) -> CadResult<Self> {
        if options.drawing.is_some() {
            return Err(CadError::InvalidInput(
                "empty session: HostOptions.drawing must be None (there is no document)".into(),
            ));
        }
        if options.document_at_start != StartDocument::Empty {
            return Err(CadError::InvalidInput(
                "empty session: HostOptions.document_at_start must be StartDocument::Empty".into(),
            ));
        }
        let core = HostCore::new(options, platform, render)?;
        core.set_request_clock(clock)?;
        core.set_document_archive(archive);
        let _startup_diagnostics = core.load_startup();
        core.install_fonts()?;
        core.install_images()?;
        // A no-op for `StartDocument::Empty` (and no configured drawing), kept for
        // the same startup sequence as every other session entry.
        core.open_configured_drawing()?;
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
    use cad_host::{HostOptions, RequestClock, StartDocument};
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
                // historical demo-document fixture (not a production host)
                document_at_start: StartDocument::Demo,
                size: [800.0, 600.0],
            },
            platform,
            RenderSessionHandle::new(BackendPreference::WebGl2),
            mono_clock(),
            BytesDocument::new(SYNTHETIC_LABEL, SYNTHETIC),
        )
        .expect("the synthetic fixture opens from bytes")
    }

    /// The same platform adapters, but with no document at all.
    fn empty_session() -> CadSession {
        let platform = cad_host::HostPlatform {
            files: Rc::new(NoFiles),
            config_store: Rc::new(NoConfig),
            surface: Rc::new(FixedSurface),
            resources: Rc::new(NoResources),
            capabilities: HostCapabilities::default(),
        };
        CadSession::from_empty(
            HostOptions {
                fonts: Vec::new(),
                fonts_dir: None,
                images_dir: None,
                headless: false,
                drawing: None,
                document_at_start: StartDocument::Empty,
                size: [800.0, 600.0],
            },
            platform,
            RenderSessionHandle::new(BackendPreference::WebGl2),
            mono_clock(),
            None,
        )
        .expect("an empty session builds without a document")
    }

    #[test]
    fn an_empty_session_starts_with_no_document_and_can_still_open_bytes() {
        let session = empty_session();
        let status = session.snapshot(None, None, 0);
        assert_eq!(status.document_label, None, "no document was opened");
        assert_eq!(status.entity_count, None, "nothing was imported");
        assert!(!status.can_fit, "an empty session has nothing to fit");

        // The first open is the ordinary bytes replacement, not a second session.
        session
            .open_bytes(SYNTHETIC_LABEL, SYNTHETIC)
            .expect("the first bytes open installs the document");
        let opened = session.snapshot(None, None, 0);
        assert_eq!(opened.document_label.as_deref(), Some(SYNTHETIC_LABEL));
        assert_eq!(opened.entity_count, Some(4));
    }

    #[test]
    fn an_empty_session_refuses_a_contradictory_start_state() {
        let platform = || cad_host::HostPlatform {
            files: Rc::new(NoFiles),
            config_store: Rc::new(NoConfig),
            surface: Rc::new(FixedSurface),
            resources: Rc::new(NoResources),
            capabilities: HostCapabilities::default(),
        };
        let options = |document_at_start| HostOptions {
            fonts: Vec::new(),
            fonts_dir: None,
            images_dir: None,
            headless: false,
            drawing: None,
            document_at_start,
            size: [800.0, 600.0],
        };
        for document_at_start in [StartDocument::Blank, StartDocument::Demo] {
            let error = match CadSession::from_empty(
                options(document_at_start),
                platform(),
                RenderSessionHandle::new(BackendPreference::WebGl2),
                mono_clock(),
                None,
            ) {
                Ok(_) => panic!("a non-empty start state must be refused"),
                Err(error) => error,
            };
            assert!(matches!(error, CadError::InvalidInput(_)), "{error}");
        }
        let error = match CadSession::from_empty(
            HostOptions {
                drawing: Some("drawing.dwg".into()),
                ..options(StartDocument::Empty)
            },
            platform(),
            RenderSessionHandle::new(BackendPreference::WebGl2),
            mono_clock(),
            None,
        ) {
            Ok(_) => panic!("a configured drawing must be refused"),
            Err(error) => error,
        };
        assert!(matches!(error, CadError::InvalidInput(_)), "{error}");
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
                origin: [0.0, 0.0],
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
