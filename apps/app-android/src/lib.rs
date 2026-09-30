//! Android composition root: `android_main` → Slint shell → command routing.
//!
//! Spec v2.0 §12.3 asks for the first cross-platform GPU/UI composition
//! evidence. On Android the shared shell (`cad-ui-slint`) owns the window and
//! the renderer is Skia-on-wgpu (`unstable-wgpu-30`); this crate supplies the
//! `android-activity` entry point, starts the platform, and routes the shell's
//! callbacks into `cad-app` so the application, not the UI, executes commands.
//!
//! What is deliberately not claimed here (kept searchable, never a silent
//! success):
//! * SAF/content-URI import — `open_document` returns `NotImplemented`.
//! * recovery persistence — `save_recovery` returns `NotImplemented`.
//! * the shared wgpu texture bridge between the CAD renderer and Slint — the
//!   shell exposes `UiHandle::set_cad_frame`, but no producer is wired yet.
//!
//! The per-area status is recorded in [`CAPABILITIES`].

use cad_app::{AppMode, Application, Command, CommandOutcome, SessionState};
use cad_domain::{pending, CadResult, DocumentId, SupportStatus, ViewportId};

/// Immutable description of one Android host instance.
#[derive(Debug, Clone)]
pub struct AndroidHostConfiguration {
    pub application_title: String,
    pub locale: String,
    /// `true` runs the shell in Work mode; `false` is read-only Viewer mode.
    pub work_mode: bool,
    pub document: DocumentId,
    pub viewport: ViewportId,
    /// Whether a recovery cache should be offered on the next launch.
    pub recovery_enabled: bool,
}

impl Default for AndroidHostConfiguration {
    fn default() -> Self {
        AndroidHostConfiguration {
            application_title: "yacr CAD".to_string(),
            locale: "zh-CN".to_string(),
            work_mode: false,
            document: DocumentId(1),
            viewport: ViewportId(1),
            recovery_enabled: true,
        }
    }
}

/// One host area and its honest support status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostCapability {
    pub area: &'static str,
    pub status: SupportStatus,
    pub evidence: &'static str,
}

/// The Android host capability table. `Verified` is reserved for a real device
/// run; a successful APK build is not a device run.
pub const CAPABILITIES: &[HostCapability] = &[
    HostCapability {
        area: "host.android.activity_entry",
        status: SupportStatus::Partial,
        evidence: "android_main + Slint AndroidPlatform; APK packages libyacr.so (docs/adr/0002)",
    },
    HostCapability {
        area: "host.android.ui_composition",
        status: SupportStatus::Partial,
        evidence: "shared Slint shell runs the android-activity event loop; no device run yet",
    },
    HostCapability {
        area: "host.android.device_run",
        status: SupportStatus::Unverified,
        evidence: "no emulator/device attached to this environment",
    },
    HostCapability {
        area: "host.android.gpu_texture_bridge",
        status: SupportStatus::Unsupported,
        evidence: "UiHandle::set_cad_frame exists; no shared wgpu texture producer yet",
    },
    HostCapability {
        area: "host.android.saf_content_uri",
        status: SupportStatus::Unsupported,
        evidence: "open_document returns NotImplemented",
    },
    HostCapability {
        area: "host.android.recovery_store",
        status: SupportStatus::Unsupported,
        evidence: "save_recovery returns NotImplemented",
    },
];

/// Owns application + session state and turns shell commands into application
/// calls. Free of Slint types so it can be exercised on the host.
pub struct CommandRouter {
    application: Application,
    session: SessionState,
}

impl CommandRouter {
    pub fn new(configuration: &AndroidHostConfiguration) -> Self {
        let mode = if configuration.work_mode { AppMode::Work } else { AppMode::Viewer };
        CommandRouter {
            application: Application::new(),
            session: SessionState::new(configuration.document, mode),
        }
    }

    pub fn session(&self) -> &SessionState {
        &self.session
    }

    /// Execute one shell command. `Ok` carries the status line for the shell;
    /// the application remains the single authority for what actually ran.
    pub fn route(&mut self, command: Command) -> CadResult<String> {
        let outcome = self.application.execute(&mut self.session, command)?;
        Ok(status_for(&outcome))
    }
}

fn status_for(outcome: &CommandOutcome) -> String {
    if let Some(diagnostic) = outcome.diagnostics.first() {
        return diagnostic.message.clone();
    }
    if outcome.changes.is_some() {
        return "已提交事务".to_string();
    }
    "完成".to_string()
}

/// Import a drawing from an Android SAF content URI.
///
/// Not implemented: the Android file grant, persistence and decode path are
/// still missing, so this must not pretend to have opened a document.
pub fn open_document(uri: &str) -> CadResult<()> {
    let _ = uri;
    pending("host.android.saf_content_uri")
}

/// Persist the recovery cache for a document.
pub fn save_recovery(document: DocumentId, bytes: &[u8]) -> CadResult<()> {
    let _ = (document, bytes);
    pending("host.android.recovery_store")
}

#[cfg(target_os = "android")]
mod android {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    use cad_domain::CadError;
    use cad_ui_slint::{
        select_wgpu_backend, UiAdapter, UiCommandSink, UiConfiguration, UiHandle,
    };

    /// Bridges shell callbacks into the router and reflects outcomes back into
    /// the status bar. The handle is installed after `UiAdapter::new`, which is
    /// why it lives behind a shared slot instead of being a plain field.
    struct HostCommandSink {
        router: Rc<RefCell<CommandRouter>>,
        ui: Rc<RefCell<Option<UiHandle>>>,
    }

    impl UiCommandSink for HostCommandSink {
        fn send(&mut self, command: Command) -> CadResult<()> {
            let result = self.router.borrow_mut().route(command);
            if let Some(ui) = self.ui.borrow().clone() {
                let status = match &result {
                    Ok(status) => status.clone(),
                    Err(error) => format!("未完成：{error}"),
                };
                let _ = ui.set_status(status);
            }
            result.map(|_| ())
        }
    }

    /// Select wgpu, install the Android platform, then run the shared shell.
    pub fn run(app: slint::android::AndroidApp, configuration: AndroidHostConfiguration) -> CadResult<()> {
        // Records the wgpu request with the Android backend before the platform
        // is installed. On Android the chosen renderer is Skia-on-wgpu.
        select_wgpu_backend()?;
        slint::android::init(app).map_err(|error| {
            CadError::Unsupported(format!("slint android init: {error}"))
        })?;
        run_shell(configuration)
    }

    /// Build and run the shared shell (no Android-only types in the signature).
    pub fn run_shell(configuration: AndroidHostConfiguration) -> CadResult<()> {
        let router = Rc::new(RefCell::new(CommandRouter::new(&configuration)));
        let ui_slot: Rc<RefCell<Option<UiHandle>>> = Rc::new(RefCell::new(None));
        let sink = HostCommandSink { router: router.clone(), ui: ui_slot.clone() };
        let ui_configuration = UiConfiguration {
            compact: true,
            locale: configuration.locale.clone(),
            safe_insets: [0.0; 4],
            application_title: configuration.application_title.clone(),
            document: configuration.document,
            viewport: configuration.viewport,
        };
        let adapter = UiAdapter::new(ui_configuration, sink, configuration.work_mode)?;
        let handle = adapter.handle();
        *ui_slot.borrow_mut() = Some(handle.clone());
        handle.set_status("框架：未接入图纸导入")?;
        adapter.run()
    }
}

/// NativeActivity entry point, looked up by `android-activity` at load time.
#[cfg(target_os = "android")]
#[no_mangle]
pub fn android_main(app: slint::android::AndroidApp) {
    if let Err(error) = android::run(app, AndroidHostConfiguration::default()) {
        eprintln!("yacr android host failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_app::{CommandId, CommandPayload};
    use cad_domain::{CadError, LayerId};

    fn command(id: CommandId, payload: CommandPayload) -> Command {
        Command { schema_version: 1, id, document: DocumentId(1), viewport: ViewportId(1), payload }
    }

    #[test]
    fn viewer_mode_routes_a_work_command_to_permission_denied() {
        let mut router = CommandRouter::new(&AndroidHostConfiguration::default());
        let error = router
            .route(command(CommandId::CreateAnnotation, CommandPayload::None))
            .unwrap_err();
        assert_eq!(error, CadError::PermissionDenied);
    }

    #[test]
    fn open_drawing_is_reported_as_a_host_responsibility() {
        let mut router = CommandRouter::new(&AndroidHostConfiguration::default());
        match router.route(command(CommandId::OpenDrawing, CommandPayload::None)) {
            Err(CadError::Unsupported(message)) => assert!(message.contains("host")),
            other => panic!("expected a host responsibility, got {other:?}"),
        }
    }

    #[test]
    fn layer_override_is_accepted_without_a_document() {
        let mut router = CommandRouter::new(&AndroidHostConfiguration::default());
        let status = router
            .route(command(CommandId::ToggleLayer, CommandPayload::Layer(LayerId(3), false)))
            .unwrap();
        assert_eq!(status, "完成");
        assert_eq!(router.session().layer_overrides.get(&LayerId(3)), Some(&false));
    }

    #[test]
    fn unimplemented_host_areas_stay_explicit() {
        assert!(open_document("content://example/1").is_err());
        assert!(save_recovery(DocumentId(1), &[0u8; 4]).is_err());
    }

    #[test]
    fn capability_table_does_not_claim_an_unverified_device_run() {
        assert!(CAPABILITIES.iter().all(|entry| !entry.evidence.is_empty()));
        assert!(CAPABILITIES
            .iter()
            .any(|entry| entry.area == "host.android.saf_content_uri"
                && entry.status == SupportStatus::Unsupported));
        assert!(CAPABILITIES
            .iter()
            .all(|entry| entry.status != SupportStatus::Verified));
    }
}
