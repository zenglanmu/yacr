//! Browser-scoped `EngineClient` wrapper over the UI-free `HostCore`.
//!
//! With P2-B the engine owns the synchronous command/interaction surface: the
//! Slint `EngineAdapter` binds every callback to the `Rc<dyn EngineClient>` it is
//! attached with. The host itself is a plain `EngineClient`, but the browser
//! build has one genuinely platform-specific responsibility that is not part of
//! the shared boundary: choosing a GPU/GL backend is a page-reload (the browser
//! has no in-place hot swap), discovered from `?backend=`/`localStorage`, so a
//! `SwitchBackend` command must be intercepted before the host's explicit
//! refusal. Opening also clicks the hidden file input through the platform’s
//! `begin_open`, which the host already drives; this wrapper only pre-grades the
//! "choose a file" status. The browser wheel delta sign is opposite to the
//! desktop winit convention the shared scroll handler assumes, so a `Scroll`
//! interaction is normalised here.
//!
//! Everything else delegates verbatim to the host. The wrapper stores no UI/CAD
//! view type; the facade and handle stay with the composition root.

use std::rc::Rc;
use std::sync::Arc;

use cad_app::CanvasMetrics;
use cad_domain::CadResult;
use cad_engine::events::{EventPoll, EventWatermark, SubscriptionId};
use cad_engine::protocol::{
    ConfigSnapshot, EngineClient, EngineRequest, HostUpdates, PumpReport, RequestCompletion,
    RequestContext, RequestError, RequestOrigin, RequestResult, SubmissionReceipt,
};
use cad_engine::render::RenderSessionHandle;
use cad_host::HostCore;

use crate::request_queue::{self, OriginPlan, WebRequest};

/// An owner-thread `EngineClient` that delegates to the host and intercepts the
/// browser-only `SwitchBackend`/`OpenDrawing` commands.
pub(super) struct WebEngineClient {
    core: HostCore,
}

impl WebEngineClient {
    pub(super) fn new(core: HostCore) -> Self {
        Self { core }
    }

    /// Route the browser backend switch through the host's traced external-closure
    /// bridge (`HostCore::dispatch_external_with_origin`), which owns the permit,
    /// the real request id and the `RequestStarted`/`RequestFinished` lifecycle.
    ///
    /// The action closure performs the payload validation, the preference store and
    /// the page reload, returning a real `Arc<CommandOutcome>` on success; a failure
    /// is a real completion from the host. There is no nested `with_sync_request`
    /// (the bridge owns its RAII), no probe and no faked id.
    fn dispatch_backend(
        &self,
        origin: RequestOrigin,
        request: EngineRequest,
    ) -> Result<RequestCompletion, RequestError> {
        // Read the Copy choice before the command moves into the bridge; an illegal
        // payload is refused here, before any id is allocated.
        let choice = request_queue::backend_choice(&request).map_err(RequestError::Execution)?;
        self.core
            .dispatch_external_with_origin(origin, request, move || {
                crate::backend_switch::switch_backend_for_choice(
                    choice,
                    super::persist_backend_preference,
                    super::reload_page,
                )
                .map(|()| Arc::new(cad_app::CommandOutcome::none()))
                .map_err(RequestError::Execution)
            })
    }
}

impl EngineClient for WebEngineClient {
    fn dispatch(&self, request: EngineRequest) -> Result<RequestResult, RequestError> {
        match request_queue::classify(request) {
            // A wheel scroll is already normalized once by `classify`; forward it
            // verbatim. `pump` executes queued requests through the host, never
            // through this wrapper, so nothing is normalized twice.
            WebRequest::Forward(request) | WebRequest::Scroll(request) => {
                if matches!(
                    &request,
                    EngineRequest::Command(command)
                        if command.id == cad_app::CommandId::OpenDrawing
                ) {
                    // The host's `request_open` clicks the hidden file input
                    // through `PlatformFiles::begin_open`; grade the status first
                    // so the choose-a-file hint is visible while the picker is
                    // open.
                    let _ = self.core.set_catalog_status("file.choose", &[]);
                }
                self.core.dispatch(request)
            }
            // The Ui shorthand takes the real completion result from the host's
            // traced backend bridge (no synthesized command outcome).
            WebRequest::Backend(request) => self
                .dispatch_backend(RequestOrigin::Ui, request)
                .and_then(|completion| completion.result),
        }
    }

    fn request_context(&self) -> RequestContext {
        self.core.request_context()
    }

    fn take_updates(&self) -> CadResult<HostUpdates> {
        self.core.take_updates()
    }

    fn set_canvas_metrics(&self, metrics: CanvasMetrics) -> CadResult<()> {
        self.core.set_canvas_metrics(metrics)
    }

    fn render_session(&self) -> RenderSessionHandle {
        self.core.render_session()
    }

    fn config_snapshot(&self) -> ConfigSnapshot {
        self.core.config_snapshot()
    }

    fn set_config_json(&self, json: &str) -> CadResult<()> {
        self.core.set_config_json(json)
    }

    fn update_config_json(&self, json: &str) -> CadResult<()> {
        self.core.update_config_json(json)
    }

    fn apply_user_preference_json(&self, json: &str) -> CadResult<()> {
        self.core.apply_user_preference_json(json)
    }

    /// Drop the stored user preference through the host's real, single-authority
    /// store.
    ///
    /// The trait default refuses explicitly ("not implemented by this engine
    /// client"); the browser owns a real preference store, so it must override
    /// that default rather than inherit it, exactly like the host.
    fn clear_user_preference(&self) -> CadResult<()> {
        self.core.clear_user_preference()
    }

    /// Forward the async submit after the same one-time browser normalization as
    /// the synchronous path (a wheel `Scroll` normalizes once either way), so the
    /// queued and synchronous directions agree.
    ///
    /// Every other request — including the gesture commands the host rejects
    /// (`OpenDrawing`/`PlotDrawing`/`SaveDrawingAs`/`SwitchBackend`) — is
    /// forwarded verbatim: the host owns the submit admission rule and refusal
    /// semantics, and the wrapper neither duplicates nor widens it.
    fn submit(
        &self,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError> {
        self.core
            .submit(context, request_queue::normalize_once(request))
    }

    /// Advance the owner-thread queue by one bounded pass.
    ///
    /// Delegates straight to the host; it never re-runs the browser scroll
    /// normalization (queued requests were normalized once at `dispatch`).
    fn pump(&self) -> Result<PumpReport, RequestError> {
        self.core.pump()
    }

    fn poll_completions(&self, limit: usize) -> Vec<RequestCompletion> {
        self.core.poll_completions(limit)
    }

    fn set_request_waker(&self, waker: Option<Rc<dyn Fn()>>) {
        self.core.set_request_waker(waker)
    }

    // --- P4: origin-tagged requests and bounded event subscriptions ---
    //
    // The origin variants carry the *same* browser interception as the `Ui`
    // shorthand: a wheel scroll normalizes once (so the direction cannot differ
    // by origin) and `OpenDrawing` pre-grades the picker status. The origin is
    // trace metadata only, never a permission, so it does not change the guard or
    // the normalization.
    fn dispatch_with_origin(
        &self,
        origin: RequestOrigin,
        request: EngineRequest,
    ) -> Result<RequestCompletion, RequestError> {
        match request_queue::origin_plan(request) {
            OriginPlan::Forward(request) => {
                if matches!(
                    &request,
                    EngineRequest::Command(command)
                        if command.id == cad_app::CommandId::OpenDrawing
                ) {
                    let _ = self.core.set_catalog_status("file.choose", &[]);
                }
                self.core.dispatch_with_origin(origin, request)
            }
            // The browser backend switch goes to the host's traced bridge, which
            // owns the permit / real id / `RequestStarted`+`RequestFinished`
            // lifecycle; the wrapper never fakes an id or bypasses it.
            OriginPlan::Backend(request) => self.dispatch_backend(origin, request),
        }
    }

    /// Queue an origin-tagged request after the same one-time scroll normalization
    /// as the `Ui` shorthand; every other request (including gesture commands the
    /// host rejects) forwards verbatim to the host's admission rule.
    fn submit_with_origin(
        &self,
        origin: RequestOrigin,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError> {
        self.core
            .submit_with_origin(origin, context, request_queue::normalize_once(request))
    }

    /// Subscribe at the host event ring's current tail. A verbatim delegation: the
    /// wrapper never auto-subscribes and never filters the result.
    fn subscribe(&self) -> CadResult<SubscriptionId> {
        self.core.subscribe()
    }

    /// Poll a subscription's events verbatim. The wrapper never synthesizes an
    /// empty poll and never clamps the caller's limit.
    fn poll_events(&self, id: SubscriptionId, limit: usize) -> CadResult<EventPoll> {
        self.core
            .poll_events(id, request_queue::forwarded_event_limit(limit))
    }

    /// Drop a subscription verbatim.
    fn unsubscribe(&self, id: SubscriptionId) -> CadResult<()> {
        self.core.unsubscribe(id)
    }

    /// Current host event-ring watermark, forwarded verbatim for snapshots/resync.
    fn event_watermark(&self) -> EventWatermark {
        self.core.event_watermark()
    }
}
