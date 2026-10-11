//! Browser implementations of the `cad-platform` host contracts.
//!
//! `cad_host::HostCore` drives the open/save/plot funnels through these traits;
//! the browser supplies the same event-based shape the desktop does, but every
//! interaction is synchronous (there is no worker thread and no filesystem):
//!
//! * [`WebFiles`] clicks the hidden `<input type=file>` for an open and queues
//!   the bytes the JS File API host hands back as a [`FileEvent::Opened`];
//!   a save synthesizes a download target in the gesture (no native dialog).
//! * [`WebConfigStore`] reads/writes the `localStorage` config preference.
//! * [`WebSurface`] reports the CSS-measured surface and safe-area insets.
use std::cell::RefCell;
use std::sync::Arc;

use cad_domain::{CadError, CadResult};
use cad_platform::{
    ConfigLoad, ConfigStore, FileEvent, FileTarget, PlatformFiles, SavePurpose, SurfaceMetrics,
    SurfaceSource,
};

/// The `localStorage` display path shown in startup diagnostics.
const CONFIG_PREFERENCE_PATH: &str = "localStorage:yacr.cad.config";

/// Browser file selection, read and (P3) write.
///
/// The queue is drained by [`PlatformFiles::poll`]. On the open path the host
/// drains it inside `open_document_bytes` (the browser read is synchronous, so
/// the event is available immediately); a save target queued by `begin_save` is
/// drained by the core's own `drain_files` after the gesture.
pub struct WebFiles {
    events: RefCell<Vec<FileEvent>>,
    /// The successfully-opened source target (armed by the core only on success).
    source: RefCell<Option<FileTarget>>,
}

impl WebFiles {
    pub fn new() -> Self {
        Self {
            events: RefCell::new(Vec::new()),
            source: RefCell::new(None),
        }
    }

    /// Queue a drawing the JS File API host read, as a platform file event.
    ///
    /// The browser has no filesystem identity, so `opaque` mirrors the display
    /// name; the core uses it only as a human-facing handle, never as a path.
    pub fn push_opened(&self, display_name: String, bytes: Vec<u8>) {
        let target = FileTarget {
            opaque: display_name.clone(),
            display_name,
        };
        self.events.borrow_mut().push(FileEvent::Opened {
            target,
            bytes: Arc::from(bytes.into_boxed_slice()),
        });
    }

    /// Put a drained event back so a later consumer (the core's save funnel) can
    /// still observe it.
    pub fn requeue(&self, event: FileEvent) {
        self.events.borrow_mut().push(event);
    }
}

impl Default for WebFiles {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformFiles for WebFiles {
    fn begin_open(&self) -> CadResult<()> {
        super::persistence::open_dialog("file-input");
        Ok(())
    }

    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        // The browser has no portable filesystem path to read.
        Err(CadError::Unsupported(
            "the web host has no filesystem path to open".into(),
        ))
    }

    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        // The browser has no native save dialog; the download target is
        // synthesized synchronously so a save can complete in the same gesture.
        // Save/plot are gated off this phase (P3 wires the download), but the
        // contract is implemented honestly rather than left as a no-op.
        let target = FileTarget {
            display_name: default_name.to_string(),
            opaque: format!("download://{default_name}"),
        };
        self.events
            .borrow_mut()
            .push(FileEvent::SaveTarget { purpose, target });
        Ok(())
    }

    fn write_atomic(&self, _target: &FileTarget, _bytes: &[u8]) -> CadResult<()> {
        // No browser download is wired yet: an explicit refusal, never a silent
        // success. P3 replaces this with a Blob download.
        Err(CadError::Unsupported(
            "browser download is not wired for this host yet".into(),
        ))
    }

    fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
        // No filesystem identity in the browser; a save cannot target the opened
        // source file.
        false
    }

    fn set_source(&self, target: Option<&FileTarget>) {
        *self.source.borrow_mut() = target.cloned();
    }

    fn poll(&self) -> Vec<FileEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

/// Browser config/preference persistence over `localStorage`.
///
/// The effective host config is built in memory (the host injects its
/// allowed-paths defaults); only the projected user preference is durable, so
/// `load_config` is always absent.
pub struct WebConfigStore;

impl ConfigStore for WebConfigStore {
    fn load_config(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }

    fn load_preference(&self) -> ConfigLoad {
        match cad_ui_slint::web::stored_config_preference() {
            Some(text) => ConfigLoad::Text {
                path: CONFIG_PREFERENCE_PATH.to_string(),
                text,
            },
            None => ConfigLoad::Absent,
        }
    }

    fn persist_preference(&self, text: &str) -> CadResult<()> {
        cad_ui_slint::web::store_config_preference(text)
    }
}

/// Browser surface metrics: the CSS-measured CAD surface plus the safe-area
/// insets the JS host reported. `None` until a real surface size is known.
pub struct WebSurface;

impl SurfaceSource for WebSurface {
    fn surface(&self) -> Option<SurfaceMetrics> {
        let (logical, scale) = super::surface_snapshot();
        if logical[0] > 0.0 && logical[1] > 0.0 {
            Some(SurfaceMetrics {
                logical,
                scale,
                safe_insets: super::safe_insets(),
            })
        } else {
            None
        }
    }
}
