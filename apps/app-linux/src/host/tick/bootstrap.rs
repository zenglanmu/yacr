//! Per-tick host bootstrap: child-webview sizing, deferred bridge GPU init, the
//! React CAD layout -> native margins pass, and the CAD session create/bind.
//!
//! All of it runs on the main thread with the shared lock held only for short
//! bookkeeping reads/writes (never across GPU or GTK work).

use std::sync::{Arc, Mutex};

use cad_engine::render::{BackendPreference, RenderSessionHandle};
use cad_host::HostPlatform;
use cad_platform::HostCapabilities;
use cad_ui_core::layout::LayoutError;
use cad_ui_core::session::CadSession;
use cad_ui_core::shell::{bind_allowed, clear_host_error, note_host_error, HostErrorKind};
use cad_ui_core::unsupported_status;

use super::super::admission::{
    aligned_surface_metrics, bridge_buffer_scale, cad_surface_geometry, css_to_gtk, gtk_margins,
    gtk_rect, layout_needs_revalidate, BindStep,
};
use super::super::tick_evidence::log_layout_geometry;
use super::super::{apply_gpu_budget_percent, lock, Shared, BRIDGE, CAD, CAD_ENV};

/// Size the child webview and run the deferred bridge GPU init.
///
/// Returns `true` when the init produced a result (success or failure) and the
/// caller must stop for this tick, exactly like the former inline `return`.
pub(super) fn sync_and_init_gpu(
    shared: &Arc<Mutex<Shared>>,
    size: tauri::PhysicalSize<u32>,
) -> bool {
    // ---- child webview sizing (main thread, no shared lock held) ----
    // Unconditional on every frame: this host does not drain GTK's queued
    // layout by itself, so an event-driven contract (`size-allocate`, window
    // events) is not available here. The call is idempotent and cheap when the
    // allocation already matches, and every attempt logs a `skipped=` reason
    // (deduped per distinct state) so the next run says which branch ran. The
    // window's inner size is the pre-layout fallback; the container allocation
    // wins as soon as GTK reports one.
    crate::gtk_bridge::sync_webview_size((size.width, size.height), "tick");

    // ---- deferred bridge GPU init (main thread, no shared lock held) ----
    // `LayoutTracker::confirmed` is the single source of truth for the CAD-rect
    // gate; the bridge keeps no second copy.
    let layout_confirmed = lock(shared).layout.confirmed();
    let init_result = BRIDGE.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .and_then(|owner| owner.try_init_gpu(layout_confirmed))
    });
    if let Some(result) = init_result {
        let mut state = lock(shared);
        match result {
            Ok(()) => {
                let mut surface = None;
                BRIDGE.with(|cell| {
                    if let Some(owner) = cell.borrow().as_ref() {
                        if let Some(adapter) = owner.adapter() {
                            state.shell.status.backend = Some(adapter.backend.clone());
                            state.shell.status.adapter = Some(adapter.name.clone());
                        }
                        if let Some(mode) = owner.present_mode() {
                            state.shell.present_mode = Some(format!("{mode:?}"));
                        }
                        surface = owner.surface_size();
                    }
                });
                // A new GPU generation: keep the DOM input sequence monotonic and
                // re-validate the cached CAD rect against the current top geometry
                // (do not blindly re-apply it; there is no cross-generation reset).
                state.layout.note_generation();
                state.layout_revalidate = true;
                // Report the actual child-surface size, not the window size.
                if let Some((w, h)) = surface {
                    state.shell.status.width = w;
                    state.shell.status.height = h;
                }
            }
            Err(error) => {
                state.shell.status.error = Some(error);
            }
        }
        return true;
    }
    false
}

/// Feed the accepted DOM rect to the layout tracker and apply the resulting GTK
/// margins + CAD surface metrics.
pub(super) fn apply_react_layout(shared: &Arc<Mutex<Shared>>) {
    // ---- React CAD layout -> native margins (main thread, no shared lock) ----
    // The reference bounds for the CAD viewport insets are the **logical size of
    // the GTK container that receives the margins** (the overlay the React WebView
    // fills), in the frontend's DOM/CSS coordinate space. The Tauri window
    // `inner_size()` is WRONG here: it includes CSD/decorations and is larger than
    // the DOM viewport, so using it inflated every inset (native capture A:
    // 1012x652 vs the real 960x553 viewport → margins end=86/bottom=183 instead of
    // the correct end=34/bottom=84).
    let container_bounds = BRIDGE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|owner| owner.logical_container_bounds())
    });
    // Defer explicitly while the container allocation is not settled: never
    // accept a fake empty layout (zero bounds) and never disturb a confirmed one.
    let last_rect = lock(shared).shell.input.last_rect();
    if let (Some(bounds), Some(rect)) = (container_bounds, last_rect) {
        // The container is GTK logical (the geometry the margins are applied
        // in); the frontend rect is CSS. WebKitGTK folds GTK text scaling into
        // `devicePixelRatio`, so the CSS viewport is `container / ratio` and the
        // layout tracker must be fed CSS bounds — otherwise a text-scale change
        // looks like a CSS viewport change (or hides one).
        let parent_scale = bridge_buffer_scale();
        let ratio = css_to_gtk(f64::from(rect.dpr), parent_scale);
        let gtk = gtk_rect(&rect, ratio);
        let css_bounds = (
            (f64::from(bounds.0) / ratio) as f32,
            (f64::from(bounds.1) / ratio) as f32,
        );
        let (revalidate, decision) = {
            let mut state = lock(shared);
            // Re-validate the cached rect when a GPU generation changed **or** the
            // margin-receiving container bounds changed — the latter even without a
            // new DOM sequence (the container resized under an unchanged rect).
            let revalidate =
                layout_needs_revalidate(state.layout_revalidate, state.layout_bounds, css_bounds);
            state.layout_revalidate = false;
            let decision = if revalidate {
                state.layout.revalidate(&rect, css_bounds.0, css_bounds.1)
            } else {
                state.layout.accept(&rect, css_bounds.0, css_bounds.1)
            };
            (revalidate, decision)
        };
        match decision {
            Ok(_css_margins) => {
                lock(shared).layout_bounds = Some(css_bounds);
                // Keep the CAD surface source in sync with the accepted DOM
                // rect. The host `tick` re-reads this surface every cycle
                // (`HostCore::tick` -> `metrics`), so `viewport.logical_size`
                // always matches the rect the renderer draws into. Leaving it
                // at the creation-time size made `Camera::zoom_at` anchor off
                // the cursor after any DOM-rect change (window resize, panel
                // toggle, DPR change).
                // One rounding shared with the render target (at the WebView's
                // dpr, i.e. including page zoom), then grown to a legal Wayland
                // buffer size (`wl_surface` error 2 otherwise).
                let (logical, physical) = cad_surface_geometry(&rect, parent_scale);
                CAD_ENV.with(|cell| {
                    if let Some(env) = cell.borrow().as_ref() {
                        env.surface.set_metrics(aligned_surface_metrics(
                            rect,
                            logical,
                            f64::from(rect.dpr),
                        ));
                    }
                });
                // GTK margins are GTK logical px, derived from the same converted
                // rect the frame path's drawing-area guard compares against.
                let margins = gtk_margins(bounds, gtk);
                BRIDGE.with(|cell| {
                    if let Some(owner) = cell.borrow_mut().as_mut() {
                        owner.apply_layout(margins, physical);
                    }
                });
                let geometry =
                    BRIDGE.with(|cell| cell.borrow().as_ref().map(|owner| owner.layout_geometry()));
                let surface =
                    BRIDGE.with(|cell| cell.borrow().as_ref().and_then(|o| o.surface_size()));
                let top = lock(shared)
                    .window
                    .as_ref()
                    .and_then(|window| window.inner_size().ok());
                // Evidence only; emitted on a layout change, never per frame.
                log_layout_geometry(
                    top, bounds, geometry, rect, ratio, physical, surface, margins, revalidate,
                );
            }
            Err(LayoutError::StaleSequence) => {}
            Err(error) => {
                if revalidate {
                    // The cached rect no longer fits the current container: wait
                    // for fresh metrics instead of applying stale margins.
                    eprintln!(
                        "[probe] layout revalidation deferred (waiting for fresh metrics): {error:?}"
                    );
                } else {
                    lock(shared).shell.status.error = Some(format!("layout rejected: {error:?}"));
                }
            }
        }
    }
}

/// Lazily create the native CAD session and bind its render session.
pub(super) fn create_and_bind_session(shared: &Arc<Mutex<Shared>>, cad_mode: bool) {
    // ---- CAD session: lazy creation once the DOM rect is confirmed ----
    if cad_mode && CAD.with(|cell| cell.borrow().is_none()) {
        let rect = lock(shared).shell.input.last_rect();
        if let Some(rect) = rect {
            // Same aligned geometry as the accepted-layout path (WebView dpr,
            // legal Wayland buffer): a Wayland buffer must be a multiple of the
            // integer buffer scale.
            let (logical, _physical) = cad_surface_geometry(&rect, bridge_buffer_scale());
            let created = CAD_ENV.with(|cell| {
                let env = cell.borrow();
                let env = env.as_ref()?;
                // The surface must report before `CadSession::new` (the open
                // funnel validates canvas metrics against it).
                env.surface.set_metrics(aligned_surface_metrics(
                    rect,
                    logical,
                    f64::from(rect.dpr),
                ));
                let platform = HostPlatform {
                    files: env.files.clone(),
                    config_store: env.config.clone(),
                    surface: std::rc::Rc::new(env.surface.clone()),
                    resources: std::rc::Rc::new(cad_host::DesktopResources),
                    capabilities: HostCapabilities {
                        open: true,
                        new: true,
                        save: true,
                        plot: true,
                        import_worker: true,
                    },
                };
                Some(CadSession::new(
                    env.options.clone(),
                    platform,
                    RenderSessionHandle::new(BackendPreference::WebGpu),
                ))
            });
            match created {
                Some(Ok(session)) => {
                    // ADR-0036 phase C1: this carrier is the **only** one that
                    // injects a solid modeler (Linux native session). Every other
                    // carrier passes `None`, so the seven primitive commands
                    // refuse with `unsupported-runtime: solid-modeler` instead of
                    // pretending to create a body.
                    session.set_solid_modeler(Some(std::sync::Arc::new(
                        cad_kernel_solidcore::SolidCoreModeler::new(),
                    )));
                    // P2.1c part 1: the display mesher rides on the same injection.
                    // No import path in this build can produce a
                    // `BrepSurface::Nurbs` face (`cad-import-acadrust`'s ACIS reader
                    // refuses spline surfaces by name, so the importer never lifts
                    // one), which means nothing *reachable* exercises this today.
                    // It is wired anyway because the seam is the carrier's
                    // composition: a body with a NURBS face (a future importer, or a
                    // kernel product) must draw here, not fall back to a missing
                    // face, and `cad-import-acadrust` stays free of kernel types.
                    session.set_solid_tessellator(Some(std::sync::Arc::new(
                        cad_kernel_solidcore::SolidCoreTessellator::new(),
                    )));
                    // The native document format. `cad-store-zip` may only be
                    // linked by a carrier app (`scripts/check-architecture.py`),
                    // so the shared layers see it exclusively through
                    // `cad_platform::DocumentArchive`. Without this line every
                    // `.ymodel` open/save refuses with
                    // `unsupported-runtime: ymodel` — the explicit refusal, not a
                    // silent DXF export under a `.ymodel` name.
                    session.set_document_archive(Some(crate::ymodel_archive::ZipArchive::new()
                        as std::sync::Arc<dyn cad_platform::DocumentArchive>));
                    // The Options dialog's GPU budget is a renderer policy: apply
                    // the configured percentage before the first device attach
                    // (the engine re-applies it on every re-attach).
                    apply_gpu_budget_percent(
                        &session.render_session(),
                        session.settings().gpu_budget_percent,
                    );
                    CAD.with(|cell| *cell.borrow_mut() = Some(session));
                    eprintln!("[probe] cad session created");
                }
                Some(Err(error)) => {
                    let text = format!("{error:?}");
                    eprintln!("[probe] cad session creation FAILED: {text}");
                    // A CadSession-level (core/open) failure lives in the
                    // snapshot, never in the host transient error.
                    lock(shared).shell.cad_status = Some(unsupported_status(Some(text)));
                }
                None => {}
            }
        }
    }

    // ---- CAD session bind (main thread): only a successful bind marks ready ----
    //
    // The device is attached through the presenter's `device_queue`. With no
    // presenter yet this stores the session and returns `Ok` (device deferred,
    // not a failure). A bind is attempted **at most once per presenter
    // generation**: a failure is recorded against the generation, kept visible,
    // and the session stays not-ready (waiting) until the generation changes (a
    // new device) — no per-tick initialization loop. A bridge that is not
    // installed yet, or is closing/failed, defers without a failure.
    if cad_mode && !lock(shared).shell.cad_ready {
        let render = CAD.with(|cell| cell.borrow().as_ref().map(|s| s.render_session()));
        if let Some(render) = render {
            let failed_generation = lock(shared).shell.cad_bind_failed_generation;
            let step = BRIDGE.with(|cell| {
                let mut slot = cell.borrow_mut();
                let Some(owner) = slot.as_mut() else {
                    return BindStep::NoBridge;
                };
                if owner.failure().is_some()
                    || matches!(owner.state(), crate::gtk_bridge::BridgeState::Closing)
                {
                    return BindStep::Skipped;
                }
                let generation = owner.generation();
                if !bind_allowed(false, failed_generation, generation) {
                    return BindStep::Skipped;
                }
                BindStep::Attempted(generation, owner.set_render_session(Some(render)))
            });
            match step {
                BindStep::Attempted(generation, Ok(())) => {
                    let mut state = lock(shared);
                    state.shell.cad_ready = true;
                    state.shell.cad_bind_failed_generation = None;
                    clear_host_error(&mut state.shell, HostErrorKind::Bind);
                    eprintln!("[probe] cad render session bound (generation {generation})");
                }
                BindStep::Attempted(generation, Err(error)) => {
                    let mut state = lock(shared);
                    state.shell.cad_bind_failed_generation = Some(generation);
                    note_host_error(
                        &mut state.shell,
                        HostErrorKind::Bind,
                        format!("cad bind failed: {error}"),
                    );
                    eprintln!(
                        "[probe] cad bind FAILED (generation {generation}); fail-stop until a new generation: {error}"
                    );
                }
                BindStep::NoBridge | BindStep::Skipped => {}
            }
        }
    }
}
