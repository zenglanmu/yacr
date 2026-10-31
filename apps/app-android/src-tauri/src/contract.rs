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
/// The Tauri entry point and the seams it registers (Android only).
const HOST: &str = include_str!("host.rs");
/// The popup seam (toolkit-neutral `PopupOps` + the Android marshalling half).
const POPUP: &str = include_str!("popup.rs");

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
            "if cad_mode && dom_rect.is_some() && CAD.with(|cell| cell.borrow().is_none())"
        ),
        "session creation must be gated on the accepted layout rect and a missing session"
    );
    // The surface metrics are installed both before `CadSession::new` and per tick.
    assert!(
        TICK.contains("env.surface.set_metrics(metrics);"),
        "the tick must report the surface metrics before the session is created"
    );
}

#[test]
fn the_carrier_never_fabricates_a_document_or_an_open_funnel() {
    // Built at run time so this file's own assertion text cannot satisfy the check.
    let from_bytes = concat!("from", "_bytes");
    let open_funnel = concat!("open_configured", "_drawing");
    assert!(
        !TICK.contains(from_bytes),
        "Android v1 must not fabricate a document from bytes"
    );
    assert!(
        !TICK.contains(open_funnel),
        "Android v1 has no `--open` funnel to call"
    );
    // The environment's drawing is `None`: the session starts from the shared host
    // core's built-in document, never from invented file bytes.
    assert!(
        HOST.contains("drawing: None"),
        "the Android composition must not open a drawing it cannot read"
    );
}

#[test]
fn the_tick_moves_partition_local_input_into_canvas_space_before_the_session() {
    // The native surface is the full window while the chrome reports positions
    // relative to its `.native-input` rect, so the origin must be added *before*
    // `handle_input` — a pick/hover anchor is otherwise offset by the partition.
    let translate = TICK
        .find("crate::input::canvas_position(")
        .expect("the tick must translate partition-local input into canvas space");
    let handle = TICK
        .find("session.handle_input(")
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
