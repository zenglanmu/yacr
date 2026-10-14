//! Android-scoped `EngineClient` wrapper over the UI-free `HostCore`.
//!
//! With P2-B the engine owns the synchronous command/interaction surface: the
//! Slint `EngineAdapter` binds every callback to the `Rc<dyn EngineClient>` it
//! is attached with. The host itself is a plain `EngineClient`, but the Android
//! build has one genuinely platform-specific responsibility that is not part of
//! the shared boundary: the asynchronous SAF picker / background import is
//! drained by a Slint poll timer (`poll::ensure_polling`), and that timer must be
//! (re-)armed whenever a dispatched request leaves asynchronous file work in
//! flight.
//!
//! This wrapper delegates the whole `EngineClient` surface — the P3 bounded
//! queue (`submit`/`pump`/`poll_completions`/`set_request_waker`) and the P4
//! origin-tagged requests / bounded event subscriptions
//! (`dispatch_with_origin`/`submit_with_origin`/`subscribe`/`poll_events`/
//! `unsubscribe`/`event_watermark`) — to the host. `dispatch`/`submit` stay the
//! `Ui` shorthand. It adds two Android facts the UI-free host cannot know: the
//! async file poll re-arm after a dispatch/pump/origin-dispatch, and the
//! asset-loader font/`images_modeled` resource facts injected into every full
//! update (see `state_push::apply_android_resources`). The `backend` lifecycle
//! delta is left untouched. It stores no UI/CAD view type; the facade and handle
//! stay with the composition root. `origin` is trace metadata only, never a
//! permission the wrapper enforces.

use std::rc::Rc;

use cad_app::CanvasMetrics;
use cad_domain::CadResult;
use cad_engine::events::{EventPoll, EventWatermark, SubscriptionId};
use cad_engine::protocol::{
    ConfigSnapshot, EngineClient, EngineRequest, HostUpdates, PumpReport, RequestCompletion,
    RequestContext, RequestError, RequestOrigin, RequestResult, SubmissionReceipt,
};
use cad_engine::render::RenderSessionHandle;
use cad_host::HostCore;

/// An owner-thread `EngineClient` that delegates to the host, keeps the Android
/// async file poll alive across a dispatch/pump, and injects the Android
/// resource facts into every full update.
pub(crate) struct AndroidEngineClient {
    core: HostCore,
}

impl AndroidEngineClient {
    pub(crate) fn new(core: HostCore) -> Self {
        Self { core }
    }

    /// Re-arm the Android async file poll when a request left a picker/read or a
    /// background import in flight. Inert when nothing file-async is pending.
    ///
    /// Shared by `dispatch`, `pump` and `dispatch_with_origin` so the origin
    /// variant keeps exactly the same SAF rearm as the legacy path.
    fn rearm_async_file_poll(&self) {
        if self.core.loading() || self.core.save_pending() {
            super::poll::ensure_polling();
        }
    }
}

impl EngineClient for AndroidEngineClient {
    fn dispatch(&self, request: EngineRequest) -> Result<RequestResult, RequestError> {
        let result = self.core.dispatch(request);
        self.rearm_async_file_poll();
        result
    }

    fn submit(
        &self,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError> {
        // Admission only: a receipt never means the work executed. The queue
        // (host-owned) wakes `set_request_waker` on the empty→nonempty edge; a
        // submit of a cursor does not itself launch file IO.
        self.core.submit(context, request)
    }

    fn pump(&self) -> Result<PumpReport, RequestError> {
        let result = self.core.pump();
        // `pump` can execute a queued command that starts the same asynchronous
        // SAF read/import `dispatch` would; re-arm the poll on the same thread so
        // the later file work is never left unpolled.
        self.rearm_async_file_poll();
        result
    }

    fn poll_completions(&self, limit: usize) -> Vec<RequestCompletion> {
        self.core.poll_completions(limit)
    }

    fn set_request_waker(&self, waker: Option<Rc<dyn Fn()>>) {
        self.core.set_request_waker(waker)
    }

    // --- P4: origin-tagged requests and bounded event subscriptions ---
    //
    // Origin is trace metadata only, never a permission: the Android wrapper
    // forwards it verbatim to the host's single authority. The wrapper does not
    // deep-clone a `ChangeSet`; every payload is shared behind the host's `Arc`.

    fn dispatch_with_origin(
        &self,
        origin: RequestOrigin,
        request: EngineRequest,
    ) -> Result<RequestCompletion, RequestError> {
        let result = self.core.dispatch_with_origin(origin, request);
        // Same async-file rearm as the legacy `dispatch` path.
        self.rearm_async_file_poll();
        result
    }

    fn submit_with_origin(
        &self,
        origin: RequestOrigin,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError> {
        // Admission only; the legacy `submit` is the `Ui` shorthand. A rejected
        // submission allocates no identity and produces no completion.
        self.core.submit_with_origin(origin, context, request)
    }

    fn subscribe(&self) -> CadResult<SubscriptionId> {
        self.core.subscribe()
    }

    fn poll_events(&self, id: SubscriptionId, limit: usize) -> CadResult<EventPoll> {
        // Unknown ids and lag are the host's explicit results; never translated
        // into an empty success here.
        self.core.poll_events(id, limit)
    }

    fn unsubscribe(&self, id: SubscriptionId) -> CadResult<()> {
        // An unknown subscription must stay an explicit error, not a silent no-op.
        self.core.unsubscribe(id)
    }

    fn event_watermark(&self) -> EventWatermark {
        self.core.event_watermark()
    }

    fn request_context(&self) -> RequestContext {
        self.core.request_context()
    }

    fn take_updates(&self) -> CadResult<HostUpdates> {
        let mut updates = self.core.take_updates()?;
        // The host cannot read the APK asset loader, so its own resource state
        // would drop the Android font summary and claim images are modeled on
        // every push. Inject the Android facts into any full update so the
        // generic `apply_host_state` mirrors them correctly.
        if let Some(full) = updates.full.as_mut() {
            super::state_push::apply_android_resources(&mut full.resources);
        }
        Ok(updates)
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

    fn clear_user_preference(&self) -> CadResult<()> {
        // Delegate the real clear to the host's single viewer-config store. The
        // trait default (Unsupported) is deliberately not inherited: clearing is
        // a genuine user action on Android, not a silent fake success.
        self.core.clear_user_preference()
    }
}
