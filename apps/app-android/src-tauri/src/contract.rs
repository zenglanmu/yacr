//! Source-structure contracts of the Android-only host modules, run on the
//! **development host**.
//!
//! `crate::host`/`crate::surface` are `cfg(target_os = "android")` (they name
//! `tauri`, `jni`, `ndk` and GPU types), so a `#[cfg(test)]` module inside them is
//! never compiled — let alone run — by `cargo test -p app-android`, which builds
//! for the host. The assertions that are about the **source structure** of those
//! modules therefore live here and read the files with `include_str!`; the
//! behavioural tests stay next to the code they exercise (and are Android-target
//! tests when they need the Android-only types).
//!
//! Rules for what belongs here:
//!
//! * an assertion must match the **real** source (a gate that cannot fail is worse
//!   than no gate — the defect this module fixes was a needle that no longer existed
//!   in the file);
//! * a **negative** assertion must not match its own text: this module is not
//!   `include_str!`-ed by itself today, but needles are still assembled
//!   (`concat!`/runtime `format!`) so they cannot be found in this file if that ever
//!   changes — and so the assertion message can spell the forbidden name without
//!   making the check pass for the wrong reason;
//! * pin the **wiring**, not phrase-of-the-day formatting where a stable anchor
//!   exists (call sites, markers, error kinds, field order).
//!
//! The frozen JNI symbol list is already covered by [`crate::policy`]'s own tests
//! (all four names, and exactly three in `surface.rs` / one in `platform.rs`), so it
//! is not duplicated here.

use crate::policy;

/// The owner tick (`host` module, Android only).
const TICK: &str = include_str!("host/tick.rs");
/// The surface/presenter module (Android only).
const SURFACE: &str = include_str!("surface.rs");
/// The Android file contracts (compiled on the development host too).
const PLATFORM: &str = include_str!("platform.rs");
/// The SAF JNI bridge (Android only).
const SAF: &str = include_str!("saf.rs");
/// The Tauri entry point and the seams it registers (Android only).
const HOST: &str = include_str!("host.rs");
/// The BIM query seam (refusal half host-compiled, marshalling half Android only).
const HOST_READ: &str = include_str!("host_read.rs");
/// The popup seam (toolkit-neutral `PopupOps` + the Android marshalling half).
const POPUP: &str = include_str!("popup.rs");
/// The Android release-page opener (Android only).
const RELEASE: &str = include_str!("release.rs");
/// The committed Kotlin Activity the JNI bridges call into (Android only).
const MAIN_ACTIVITY: &str =
    include_str!("../gen/android/app/src/main/java/io/github/zenglanmu/yacr/MainActivity.kt");

/// The slice of `source` between the `start` anchor and the next `end` anchor.
///
/// Used to scope an assertion to one match arm: a bare `contains` over the whole
/// file would be satisfied by an unrelated arm (a false gate). A missing anchor
/// panics with its own name, so a renamed anchor fails the test instead of passing
/// vacuously.
fn slice_between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let from = source
        .find(start)
        .unwrap_or_else(|| panic!("missing anchor: {start}"));
    let to = source[from..]
        .find(end)
        .map(|offset| from + offset)
        .unwrap_or_else(|| panic!("missing end anchor {end} after {start}"));
    &source[from..to]
}

#[test]
fn the_tick_creates_the_session_only_after_the_accepted_layout_rect() {
    // The session is created only once (a) CAD is enabled and (b) the chrome has
    // reported its layout rect; the surface source must be able to report first, and
    // the tick can never invent a canvas, a document or a frame.
    assert!(
        TICK.contains(
            "if cad_mode && metrics.is_some() && CAD.with(|cell| cell.borrow().is_none())"
        ),
        "session creation must be gated on a valid accepted layout rect and a missing session"
    );
    // The surface metrics are installed both before `CadSession::new` and per tick.
    assert!(
        TICK.contains("env.surface.set_metrics(metrics?);"),
        "the tick must report the surface metrics before the session is created"
    );
}

#[test]
fn the_visible_cad_rect_is_reported_with_its_origin() {
    // The shared layout helper validates the rect against the window and keeps the
    // rect origin; the tick must not go back to "the canvas is the whole surface".
    assert!(
        TICK.contains("cad_ui_core::layout::window_surface_metrics(&rect, window_logical, scale)"),
        "the tick must derive SurfaceMetrics from the accepted rect, not the full window"
    );
    assert!(
        TICK.contains("env.surface.set_metrics(metrics)"),
        "the accepted rect metrics must reach the platform surface source"
    );
    assert!(
        !TICK.contains("logical: [f64::from(width) / scale, f64::from(height) / scale]"),
        "the tick must not report the full surface as the CAD content size"
    );
}

#[test]
fn the_render_target_stays_the_full_surface() {
    // The swapchain extent is the native window; the camera placement happens in
    // the engine. `surface.rs` must keep sizing the target from the presenter.
    let render_call = SURFACE
        .find("session.render(logical, scale)")
        .expect("the presenter must call session.render(logical, scale)");
    let logical = &SURFACE[..render_call];
    let assignment = logical
        .rfind("let logical = [f64::from(width) / scale, f64::from(height) / scale];")
        .expect("surface.rs must derive the render target logical size from the surface");
    assert!(
        render_call > assignment,
        "the full-surface size must feed the render call"
    );
}

#[test]
fn the_native_session_installs_the_ymodel_archive_and_recovery_store() {
    // SAVE writes only `.ymodel` (ADR-0041 D1), so a carrier without this adapter
    // cannot save anything at all. The archive is the carrier's composition:
    // `cad-store-zip` may only be linked here, and the tick must install it on the
    // one native session. `set_persistence` is the recovery snapshot store, which
    // must be installed exactly once the `nativeAssetsReady` handshake names
    // `filesDir` (never invented).
    assert!(
        TICK.contains("session.set_document_archive(Some(crate::ymodel_archive::ZipArchive::new()"),
        "the tick must install the carrier's .ymodel archive"
    );
    assert!(
        TICK.contains("install_recovery_store(&session);"),
        "the session must be given the recovery store at creation"
    );
    assert!(
        TICK.contains("install_recovery_store(session);"),
        "a late nativeAssetsReady handshake must still install the recovery store"
    );
    assert!(
        TICK.contains(
            "session.set_persistence(Some(crate::recovery_store::FsRecoveryStore::new(dir)"
        ),
        "the recovery store must be the filesDir/recovery FsRecoveryStore"
    );
    assert!(
        !TICK.contains("NoRecovery"),
        "the host must not install the explicit 'no recovery' stub"
    );
}

#[test]
fn the_native_session_installs_the_mesh_kernel() {
    // ADR-0061 P-MESH-1a/1b: the mesh modeler is the carrier's composition, like
    // the `.ymodel` archive. The shared layers never link the kernel, so a
    // carrier that forgets this line silently loses mesh booleans/repairs (it
    // still refuses by name, but the capability is gone). Pin the injection.
    assert!(
        TICK.contains("session.set_mesh_modeler(Some(std::sync::Arc::new("),
        "the tick must install the carrier's mesh modeler"
    );
    assert!(
        TICK.contains("cad_kernel_manifold::ManifoldMeshModeler::new()"),
        "the injected mesh modeler must be the real manifold-rust implementation"
    );
}

#[test]
fn the_saf_save_path_never_claims_an_atomic_replace_or_a_backup_it_did_not_keep() {
    // SAF has no atomic replace: a write truncates, and the sibling `.bak` create
    // is best effort. The carrier therefore keeps the previous-bytes read, the
    // staged write and the `save.no_backup` notice in one module, and the tick
    // publishes that key only through the real status snapshot.
    const SAF_WRITE: &str = include_str!("platform/saf_write.rs");
    assert!(
        SAF_WRITE.contains("pub const SAVE_NO_BACKUP_STATUS: &str = \"save.no_backup\";"),
        "the notice key must be the catalog's own save.no_backup"
    );
    assert!(
        SAF_WRITE.contains("fn overwrite_safety(backup_written: bool) -> OverwriteSafety"),
        "the capability decision must live in one function with its doc comment"
    );
    assert!(
        SAF_WRITE.contains("let previous = io.read(uri)?;"),
        "an overwrite must read the current bytes first"
    );
    // The Kotlin lane stages the bytes in a cache file before opening the output.
    const PICKER: &str =
        include_str!("../gen/android/app/src/main/java/io/github/zenglanmu/yacr/DocumentPicker.kt");
    assert!(
        PICKER.contains("File.createTempFile(\"yacr-save-\", \".tmp\", activity.cacheDir)"),
        "the write must be staged in an app-cache file"
    );
    assert!(
        PICKER.contains("output.fd.sync()"),
        "the staged file must be synced before the provider is touched"
    );
    assert!(
        PICKER.contains("fun documentWriteSiblingBackup("),
        "the sibling .bak create must exist on the Kotlin lane"
    );
    // And the notice reaches the chrome: the tick folds it into the mirrored
    // status when the session itself has no error.
    assert!(
        TICK.contains("snapshot.error = Some(crate::platform::SAVE_NO_BACKUP_STATUS.to_string());"),
        "the tick must publish the latched notice in the status snapshot"
    );
}

#[test]
fn an_opened_ymodel_is_decoded_by_the_archive_and_never_offered_to_a_connector() {
    // (b) The SAF delivery is only `FileEvent::Opened`; the format decision is the
    // shared host's, and it must route `.ymodel` to the carrier archive **before**
    // any importer. The Android platform layer therefore names no importer at all:
    // there is no second open funnel here that could hand these bytes to a
    // connector probe.
    assert!(
        !PLATFORM.contains("cad_import"),
        "the Android file contracts must not call an importer"
    );
    assert!(
        !PLATFORM.contains("open_bytes"),
        "the Android file contracts must not open bytes; the shared host owns that"
    );
    const SHARED_OPEN: &str = include_str!("../../../../crates/cad-host/src/open.rs");
    let ymodel_route = SHARED_OPEN
        .find("if policy::is_ymodel_target(&target) {")
        .expect("the shared open funnel must recognize a .ymodel target");
    let importer_probe = SHARED_OPEN
        .find("open_bytes_with_xref(")
        .expect("the shared open funnel must reach the importer for other formats");
    assert!(
        ymodel_route < importer_probe,
        "a .ymodel must be decoded by the archive before any importer probe"
    );
    // The archive open is the same call the Linux carrier uses
    // (`archive.read`), not an importer: `deliver_ymodel_open` must be present.
    assert!(
        SHARED_OPEN.contains("fn deliver_ymodel_open(&self, target: FileTarget, bytes: Arc<[u8]>)"),
        "the shared funnel must keep the archive open path"
    );
}

#[test]
fn the_carrier_never_fabricates_a_document_or_an_open_funnel() {
    // Built at run time so this file's own assertion text cannot satisfy the check.
    let from_bytes = concat!("from", "_bytes");
    let open_funnel = concat!("open_configured", "_drawing");
    assert!(
        !TICK.contains(from_bytes),
        "the tick must not fabricate a document from bytes"
    );
    assert!(
        !TICK.contains(open_funnel),
        "the tick must not call a configured-drawing open funnel"
    );
    // The environment's drawing is `None`: the session starts from the shared host
    // core's built-in document, never from invented file bytes. A real open arrives
    // as a platform `FileEvent`, not as a startup drawing.
    assert!(
        HOST.contains("drawing: None"),
        "the Android composition must not open a drawing it cannot read"
    );
}

#[test]
fn the_open_funnel_is_the_platform_and_never_the_tick() {
    // The only open funnel is `AndroidFiles` -> the SAF bridge. The tick cannot
    // open bytes itself; it drains the platform queue.
    assert!(
        PLATFORM.contains("saf::launch_open"),
        "begin_open must launch the SAF picker through the platform"
    );
    assert!(
        SAF.contains("documentLaunchOpen"),
        "the platform funnel must reach the Kotlin documentLaunchOpen method"
    );
    assert!(
        SAF.contains("documentReadUri"),
        "the opened bytes must come from the Kotlin documentReadUri method"
    );
}

#[test]
fn the_file_poll_only_drains_the_platform_queue() {
    // `poll` must translate a real platform result and return the queue; it must
    // never construct an empty `Vec` (a fake "nothing happened" success is not an
    // event, and a fabricated event is forbidden).
    let body = slice_between(PLATFORM, "fn poll(&self) -> Vec<FileEvent> {", "\n    }\n}");
    assert!(
        body.contains("saf::poll"),
        "the Android poll must read the SAF bridge result"
    );
    assert!(
        body.contains("std::mem::take(&mut *self.events.borrow_mut())"),
        "poll must return the platform-owned event queue"
    );
    assert!(
        !body.contains("Vec::new()"),
        "poll must not fabricate an empty success"
    );
}

#[test]
fn the_tick_moves_partition_local_input_into_canvas_space_before_the_session() {
    // The native surface is the full window while the chrome reports positions
    // relative to its `.native-input` rect, so the origin must be added *before*
    // `handle_inputs` — a pick/hover anchor is otherwise offset by the partition.
    let translate = TICK
        .find("crate::input::canvas_position(")
        .expect("the tick must translate partition-local input into canvas space");
    let handle = TICK
        .find("session.handle_inputs(")
        .expect("the tick must feed the session");
    assert!(
        translate < handle,
        "the partition origin must be applied before the session sees the input"
    );
    // Keyed on the protocol's own wire spelling (not a parallel enum), so the
    // partition-local kinds are classified in exactly one place.
    let drain = &TICK[translate..handle];
    assert!(
        drain.contains("input.kind.as_str(),"),
        "the translation must classify the real input kind"
    );
    assert!(
        drain.contains("partition_origin"),
        "the translation must use the accepted layout rect's origin"
    );
    // Input positions are CSS px measured with the WebView's dpr; the camera
    // viewport must use that same ratio. Falling back to `tao`'s separately
    // reported density scales the whole input space and moves the pinch anchor.
    assert!(
        TICK.contains("crate::input::canvas_scale("),
        "the tick must derive the canvas scale from the WebView dpr (canvas_scale)"
    );
}

#[test]
fn a_refused_surface_reaches_cad_status_as_a_present_error() {
    // A refusal must be visible in `cad_status.error`, not only in the transient
    // `ProbeStatus`: the chrome can then say why the CAD area is dark.
    let arm = slice_between(TICK, "SurfaceStep::Refused { reason } =>", "flush(shared);");
    assert!(
        arm.contains(r#"let text = format!("native surface refused: {reason}");"#),
        "the refusal text must be built once and reused verbatim"
    );
    assert!(
        arm.contains("state.shell.status.error = Some(text.clone());"),
        "the probe status must carry the refusal"
    );
    assert!(
        arm.contains("note_host_error(&mut state.shell, HostErrorKind::Present, text);"),
        "the refusal must also be a `Present` host error (the projected cad_status.error)"
    );
    // A refusal returns before the session exists, so without a published status the
    // shared projection would have nothing to attach the host error to and the chrome
    // would read `cad_status.error = null`.
    assert!(
        arm.contains("state.shell.cad_status = Some(unsupported_status(None));"),
        "a refusal before any session must still publish a status the projection can fill"
    );
}

#[test]
fn a_missing_vulkan_presenter_emits_the_stable_gpu_marker() {
    // A free-form `[probe]` line is not machine-readable: `logcat` evidence
    // classifies GPU refusals on the stable `yacr: unsupported-gpu:` prefix, exactly
    // like the software-adapter refusal.
    let arm = slice_between(
        SURFACE,
        "no Vulkan presenter could be created",
        "remember_refusal(",
    );
    assert!(
        arm.contains(r#"log::error!("{}", policy::unsupported_gpu_marker("no-vulkan-adapter"));"#),
        "the no-adapter refusal must emit the stable unsupported-gpu marker"
    );
    // And the marker is the shared policy line, not a re-spelled literal.
    assert_eq!(
        policy::unsupported_gpu_marker("no-vulkan-adapter"),
        format!("{}no-vulkan-adapter", policy::UNSUPPORTED_GPU_PREFIX)
    );
}

#[test]
fn the_presenter_is_dropped_before_its_native_window_and_detaches_on_drop() {
    // Field order is the drop order: after `Drop` runs, the swapchain goes before
    // the window it targets.
    let start = SURFACE
        .find("struct LiveSurface {")
        .expect("LiveSurface must be declared in surface.rs");
    let body = &SURFACE[start..];
    let presenter = body
        .find("presenter: SurfaceProbePresenter")
        .expect("the presenter field must be declared");
    let window = body
        .find("window: ndk::native_window::NativeWindow")
        .expect("the window field must be declared");
    assert!(
        presenter < window,
        "the presenter field must be declared before the window field"
    );

    // The borrowed device is detached first, on **every** drop path — including the
    // tick thread's exit, where the thread-local is dropped by its TLS destructor and
    // no explicit `release()` runs.
    let drop_body = slice_between(SURFACE, "impl Drop for LiveSurface {", "\n}");
    assert!(
        drop_body.contains("self.session.take()"),
        "the drop path must take the attached session"
    );
    assert!(
        drop_body.contains("detach_device()"),
        "the drop path must detach the borrowed device before the presenter is dropped"
    );
    // `release()` must go through the same drop (no second, divergent path).
    let release_fn = slice_between(SURFACE, "pub fn release() {", "\n}");
    assert!(
        release_fn.contains("drop(live)"),
        "the explicit release must use the same drop path"
    );
}

#[test]
fn the_popup_seam_is_registered_and_routes_through_the_shared_lifecycle() {
    // Registration: the seam is installed in `setup` with the app handle. The
    // negative needle is assembled so this file's own text cannot satisfy it.
    assert!(
        HOST.contains("state.popup_host = Some(Arc::new(crate::popup::AndroidPopupHost::new("),
        "the host must register the Android popup seam"
    );
    let unset = concat!("state.popup_host", " = None");
    assert!(
        !HOST.contains(unset),
        "the popup seam must stay registered (the v1 gap is closed)"
    );
    assert!(
        HOST.contains("register_seams(app.handle(), &setup_shared, &setup_telemetry);"),
        "the seam must be registered during setup with a real app handle"
    );

    // Routing: both commands call the **shared** lifecycle, not a local copy of
    // the gate, and the ops carried into `open` are the Android payload read.
    assert!(
        POPUP.contains("impl ShellPopupHost for AndroidPopupHost"),
        "the seam must implement the shared trait"
    );
    let open = slice_between(POPUP, "fn open(&self, shared", "fn close(");
    assert!(
        open.contains("cad_desktop_shell::popup::open("),
        "open must route through cad_desktop_shell::popup::open"
    );
    assert!(
        open.contains("&AndroidPopupOps"),
        "open must derive the payload through the Android ops"
    );
    assert!(
        POPUP.contains("cad_desktop_shell::popup::close(&shared)"),
        "close must route through cad_desktop_shell::popup::close"
    );

    // Threading: the hop is bounded and probes the recorded loop thread; the
    // `Shared` guard is never moved with it (only an `Arc` clone).
    assert!(
        POPUP.contains("run_on_main_thread_timeout("),
        "the marshalling must use the bounded shared protocol"
    );
    assert!(
        POPUP.contains("crate::host::on_loop_thread()"),
        "the probe must ask the host which thread owns the session"
    );
    let marshalled = slice_between(POPUP, "fn open(&self, shared", "fn close(");
    assert!(
        !marshalled.contains(".lock("),
        "the popup hop must not carry the shell mutex"
    );

    // The payload is the live session's, read on the owner thread; an off-thread
    // call is an explicit refusal, never a fabricated payload.
    assert!(
        POPUP.contains("crate::host::CAD.with("),
        "the textEdit payload must come from the live CAD session"
    );
    assert!(
        POPUP.contains("return Err(OFF_OWNER_THREAD.to_string())"),
        "an off-loop-thread read must refuse explicitly"
    );
}

#[test]
fn the_release_page_opener_is_wired_through_the_activity_without_a_new_native_symbol() {
    // The shared command's opener is installed during startup, like every other
    // seam; a carrier that skipped it would keep refusing with
    // `unsupported-runtime: cad_open_release_page`.
    assert!(
        HOST.contains("configure_release_opener"),
        "the Android host must install the shared release opener"
    );
    assert!(
        HOST.contains("AndroidReleaseOpener"),
        "the installed opener must be crate::release::AndroidReleaseOpener"
    );

    // The seam implements the shared trait and reaches the Activity method by name.
    assert!(
        RELEASE.contains("impl ShellReleaseOpener for AndroidReleaseOpener"),
        "the Android opener must implement the shared platform trait"
    );
    assert!(
        RELEASE.contains("openExternalUrl"),
        "the opener must call MainActivity.openExternalUrl"
    );
    // The URL is the shared compile-time constant, never a caller-supplied value.
    assert!(
        RELEASE.contains("crate::saf::with_env"),
        "the opener must use the captured JavaVM/Activity bridge"
    );

    // Kotlin: the method exists and hands the URL to the system browser.
    assert!(
        MAIN_ACTIVITY.contains("fun openExternalUrl("),
        "MainActivity must expose openExternalUrl"
    );
    assert!(
        MAIN_ACTIVITY.contains("Intent.ACTION_VIEW"),
        "MainActivity must hand the URL to ACTION_VIEW"
    );

    // The frozen Kotlin -> Rust contract stays four symbols: this method is
    // Rust -> Kotlin only.
    let frozen_native = MAIN_ACTIVITY.matches("private external fun native").count();
    assert_eq!(
        frozen_native, 4,
        "the frozen Kotlin->Rust contract must stay exactly four native symbols"
    );
}

#[test]
fn the_bim_read_seam_is_registered_and_reads_the_open_document() {
    // Registration (ADR-0042 D11): the shared `cad_bim_query` command reaches the
    // host only through this seam; a carrier that never installs it keeps refusing
    // with `unsupported-runtime: bimQuery` even though the native session owns the
    // document. Installed once during `setup`, with the app handle, after the shell
    // lock is released (the configure helper takes the lock itself).
    assert!(
        HOST.contains("cad_desktop_shell::state::configure_host_read_host("),
        "the Android host must install the shared request-carrying read seam"
    );
    assert!(
        HOST.contains("crate::host_read::host_read_host(app.clone())"),
        "the installed seam must be crate::host_read::host_read_host"
    );
    assert!(
        HOST.contains("register_seams(app.handle(), &setup_shared, &setup_telemetry);"),
        "the seam must be registered during setup with a real app handle"
    );

    // Routing: the command is the **live session's** own read (the backend owns
    // the filter semantics, ordering, paging and counters) — never a locally built
    // answer, and never a second summary call. The carrier only injects its hop;
    // the trait impl itself is the shared `HostReadFn`.
    assert!(
        HOST_READ.contains("HostReadFn::new("),
        "the seam must be the shared closure adapter"
    );
    assert!(
        HOST_READ.contains("Some(session) => session.host_read(&request),"),
        "a request must be answered by the session's own host_read"
    );
    assert!(
        !HOST_READ.contains("bim_summary"),
        "the summary must ride the one query answer, not a second command"
    );

    // Threading: the hop is bounded (the loop also renders) and probes the recorded
    // loop thread; the `Shared` guard never crosses it.
    assert!(
        HOST_READ.contains("run_on_main_thread_timeout("),
        "the marshalling must use the bounded shared protocol"
    );
    assert!(
        HOST_READ.contains("crate::host::on_loop_thread()"),
        "the probe must ask the host which thread owns the session"
    );
    assert!(
        !HOST_READ.contains(".lock("),
        "the seam must not take the shell mutex (no guard crosses the hop)"
    );

    // No session (before the first tick, or after a failed creation) is the named
    // refusal — never an empty page that would read as "no data".
    assert!(
        HOST_READ.contains("None => Err(super::no_document_refusal(&request)),"),
        "a missing session must answer the named refusal"
    );
    let refusal_fn = slice_between(
        HOST_READ,
        "pub fn no_document_refusal(request: &HostReadRequest) -> String {",
        "\n}",
    );
    assert!(
        refusal_fn.contains("cad_ui_core::host_read::no_seam_refusal(request)"),
        "the refusal must come from the shared per-kind seam"
    );
    let respelled = concat!("unsupported-runtime: ", "bimQuery");
    assert!(
        !refusal_fn.contains(respelled),
        "the marker must come from the shared constant, not a re-spelled literal"
    );
}
