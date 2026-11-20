//! The two host file seams `.ymodel` support adds to the Android carrier.
//!
//! * [`AndroidRecoveryHost`] serves `cad_recovery_files` / `cad_open_recovery`
//!   (the start panel's restore list). It reads the same `filesDir/recovery`
//!   store the session writes ([`crate::recovery_store`]); restore is the
//!   session's own `open_recovery`, never the SAF picker (a `content://` picker
//!   cannot browse app-private `filesDir`).
//! * [`AndroidYmodelHost`] serves `cad_ymodel_resource_preview` and gates
//!   `cad_set_embed_resources` (the save dialog's self-containment choice). Both
//!   are reads/writes of the **open document**, so they need the `!Send`
//!   `CadSession` and are marshalled onto the Tauri loop thread.
//!
//! Both are installed only because this carrier now has a `.ymodel` archive
//! ([`crate::ymodel_archive`]): a carrier without one refuses by name instead of
//! answering an empty preview or an unopenable row.
//!
//! # Threading
//!
//! Tauri IPC handlers on this carrier usually run on the loop thread, where the
//! session lives; the marshal is the same bounded hop the popup seam uses
//! (`cad_desktop_shell::threading::run_on_main_thread_timeout`). A command must
//! never be able to wedge the render loop, so the wait is bounded (5 s) and the
//! task still runs on the loop thread. No `Shared` guard crosses the hop.

#[cfg(target_os = "android")]
pub use android_hosts::{AndroidRecoveryHost, AndroidYmodelHost};

#[cfg(target_os = "android")]
mod android_hosts {
    use std::time::Duration;

    use cad_desktop_shell::state::{ShellRecoveryHost, ShellYmodelHost};
    use cad_desktop_shell::threading::{self, MainThread};
    use cad_ui_core::session::{
        CadRecoveryFiles, CadYmodelResources, OPEN_RECOVERY_COMMAND, RECOVERY_FILES_COMMAND,
    };

    use crate::host::CAD;
    use crate::platform;

    /// How long a marshalled host-file command waits for the loop thread.
    ///
    /// Same bound as the popup seam: the loop is also driving rendering, and an
    /// IPC reply must not be able to wedge the caller forever. The task still
    /// runs on the loop thread; only the caller stops waiting.
    pub const MAIN_THREAD_WAIT: Duration = Duration::from_secs(5);

    /// Run `task` on the Tauri loop thread and wait at most [`MAIN_THREAD_WAIT`].
    fn on_loop_thread<T, F>(app: &tauri::AppHandle, task: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        threading::run_on_main_thread_timeout(&AndroidMainThread(app), task, MAIN_THREAD_WAIT)
    }

    /// The Android/Tauri adapter for the shared [`MainThread`] seam.
    struct AndroidMainThread<'a>(&'a tauri::AppHandle);

    impl MainThread for AndroidMainThread<'_> {
        fn is_main_thread(&self) -> bool {
            // The recorded `ThreadId` of the thread that ran `run()`, i.e. the
            // tao/Tauri loop that owns the `!Send` CAD session. Never a guess.
            crate::host::on_loop_thread()
        }

        fn dispatch(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
            self.0
                .run_on_main_thread(task)
                .map_err(|error| error.to_string())
        }
    }

    /// The recovery list/open seam over the session's own store.
    pub struct AndroidRecoveryHost {
        app: tauri::AppHandle,
    }

    impl AndroidRecoveryHost {
        /// Bind the seam to the Tauri app handle.
        pub fn new(app: tauri::AppHandle) -> Self {
            Self { app }
        }
    }

    impl ShellRecoveryHost for AndroidRecoveryHost {
        fn list(&self) -> Result<CadRecoveryFiles, String> {
            on_loop_thread(&self.app, list_on_loop_thread)
        }

        fn open_recovery(&self, key: &str) -> Result<(), String> {
            let key = key.to_string();
            on_loop_thread(&self.app, move || open_on_loop_thread(&key))
        }
    }

    /// The real list, read on the CAD session's own thread.
    fn list_on_loop_thread() -> Result<CadRecoveryFiles, String> {
        if let Some(list) = CAD.with(|cad| {
            cad.borrow()
                .as_ref()
                .map(|session| session.recovery_files())
        }) {
            return list;
        }
        // No session yet (the chrome's start panel can ask before the first
        // tick): read the same durable store directly, exactly like the Linux
        // carrier, so a snapshot written in a previous process is still offered.
        let Some(dir) = platform::recovery_dir() else {
            return Ok(CadRecoveryFiles::unsupported(RECOVERY_FILES_COMMAND));
        };
        let store = crate::recovery_store::FsRecoveryStore::new(dir);
        use cad_platform::Persistence as _;
        let records = store.list_recovery().map_err(|error| error.to_string())?;
        Ok(CadRecoveryFiles::supported(
            records
                .into_iter()
                .map(cad_ui_core::session::RecoveryEntry::from)
                .collect(),
        ))
    }

    /// Start one snapshot open on the CAD session's own thread.
    ///
    /// Without a session there is no open funnel to run, so the answer is the
    /// named refusal; the session validates the key and resolves the location
    /// from its own store (the UI never sends a path).
    fn open_on_loop_thread(key: &str) -> Result<(), String> {
        CAD.with(|cad| {
            cad.borrow()
                .as_ref()
                .map(|session| session.open_recovery(key))
        })
        .unwrap_or_else(|| Err(format!("unsupported-runtime: {OPEN_RECOVERY_COMMAND}")))
    }

    /// The `.ymodel` resource preview seam (save dialog's embed report).
    pub struct AndroidYmodelHost {
        app: tauri::AppHandle,
    }

    impl AndroidYmodelHost {
        /// Bind the seam to the Tauri app handle.
        pub fn new(app: tauri::AppHandle) -> Self {
            Self { app }
        }
    }

    impl ShellYmodelHost for AndroidYmodelHost {
        fn preview(&self) -> Result<CadYmodelResources, String> {
            on_loop_thread(&self.app, preview_on_loop_thread)
        }
    }

    /// The real preview, read on the CAD session's own thread.
    fn preview_on_loop_thread() -> Result<CadYmodelResources, String> {
        match CAD.with(|cad| {
            cad.borrow()
                .as_ref()
                .map(|session| session.ymodel_resource_preview())
        }) {
            Some(answer) => answer,
            // Before the session exists there is no document to describe, so the
            // honest answer is the shared marker, never an empty preview.
            None => Err("unsupported-runtime: ymodel".to_string()),
        }
    }
}
