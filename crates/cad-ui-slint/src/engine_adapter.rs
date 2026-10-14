//! Owner-thread adapter between the UI surface and an engine client.
//!
//! P2 requests are synchronous. P3 adds the bounded queue: `MoveCursor` is the
//! only request the UI admits asynchronously (`submit`), while every other
//! request runs the fixed barrier `poll → pump → poll → dispatch` and reports
//! `Busy` when executable work is still pending. A receipt never means success;
//! completions are drained in bounded batches and `Superseded` is consumed
//! quietly.

use crate::{CadView, UiAdapter, UiHandle};
use cad_app::interaction::{InteractionRequest, PointInput};
use cad_domain::{CadError, CadResult};
use cad_engine::protocol::{
    EngineClient, EngineRequest, HostUpdates, InteractionOutcome, PumpReport, RequestCompletion,
    RequestError, RequestId, RequestResult,
};
use std::cell::RefCell;
use std::rc::Rc;

/// Bounded completion drain per poll call. The queue capacity bounds the total,
/// so a handful of passes always empties the completion list.
const COMPLETION_POLL_LIMIT: usize = 64;

/// Which UI action submitted an async request, so a terminal error is reported
/// through that action's catalog copy. Bounded by [`OriginBookkeeping`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionOrigin {
    /// A `MoveCursor` submitted from the draw cursor path.
    Cursor,
}

/// Bounded request-id → origin bookkeeping.
///
/// Only ids the UI actually submitted are recorded, and the map can never grow
/// past the queue's outstanding limit: at most that many requests can be in
/// flight. A dropped oldest entry (only possible if the bound is ever exceeded)
/// degrades to the generic origin rather than misattributing another request.
struct OriginBookkeeping {
    entries: Vec<(RequestId, CompletionOrigin)>,
    capacity: usize,
}

impl OriginBookkeeping {
    fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            capacity,
        }
    }

    fn record(&mut self, id: RequestId, origin: CompletionOrigin) {
        if self.entries.len() >= self.capacity {
            self.entries.remove(0);
        }
        self.entries.push((id, origin));
    }

    fn take(&mut self, id: RequestId) -> Option<CompletionOrigin> {
        let index = self.entries.iter().position(|(entry, _)| *entry == id)?;
        Some(self.entries.remove(index).1)
    }
}

/// Shared callback router. Cloning the client before dispatch ensures no RefCell
/// borrow is held while business callbacks run or update properties are applied.
pub(crate) struct EngineDispatch {
    client: Rc<RefCell<Option<Rc<dyn EngineClient>>>>,
    handle: Rc<RefCell<Option<UiHandle>>>,
    view: Rc<RefCell<Option<CadView>>>,
    origins: Rc<RefCell<OriginBookkeeping>>,
}

impl EngineDispatch {
    pub(crate) fn new() -> Self {
        Self {
            client: Rc::new(RefCell::new(None)),
            handle: Rc::new(RefCell::new(None)),
            view: Rc::new(RefCell::new(None)),
            origins: Rc::new(RefCell::new(OriginBookkeeping::new(
                cad_engine::queue::QUEUE_CAPACITY,
            ))),
        }
    }

    pub(crate) fn client_slot(&self) -> Rc<RefCell<Option<Rc<dyn EngineClient>>>> {
        self.client.clone()
    }

    pub(crate) fn has_client(&self) -> bool {
        self.client.borrow().is_some()
    }

    pub(crate) fn set_handle(&self, handle: UiHandle) {
        *self.handle.borrow_mut() = Some(handle);
    }

    pub(crate) fn attach(&self, client: Rc<dyn EngineClient>, view: CadView) {
        *self.client.borrow_mut() = Some(client);
        *self.view.borrow_mut() = Some(view);
    }

    pub(crate) fn dispatch(&self, request: EngineRequest) -> CadResult<RequestResult> {
        let client = self
            .client
            .borrow()
            .clone()
            .ok_or_else(|| CadError::Unsupported("engine client is not attached".into()))?;
        let dispatched = client.dispatch(request);
        // `dispatch` has completed and the client-slot borrow is dropped before
        // updates are pulled or Slint properties touched (no re-entrancy, no
        // strong cycle). A publish failure must never mask the authoritative
        // execution result, and a committed request is never retried: the update
        // error is reported as the UI failure only when the request succeeded.
        match client
            .take_updates()
            .and_then(|updates| self.apply_updates(updates))
        {
            Ok(()) => {}
            Err(update_error) => {
                if dispatched.is_ok() {
                    return Err(update_error);
                }
                log::warn!(
                    "engine update publish failed after a refused request; \
                     reporting the request error: {update_error}"
                );
            }
        }
        dispatched.map_err(request_error)
    }

    fn client(&self) -> CadResult<Rc<dyn EngineClient>> {
        self.client
            .borrow()
            .clone()
            .ok_or_else(|| CadError::Unsupported("engine client is not attached".into()))
    }

    /// Fixed synchronous barrier: `poll → pump → poll`, then report `Busy` when
    /// executable work is still queued. Never auto-queues a rejected request and
    /// never retries a committed one; the file/gesture callstack stays synchronous.
    ///
    /// The pump result is saved: even when the pump fails (for example its clock
    /// budget expired after executing something), the terminal completions it
    /// produced are drained so their quota is released and their errors surface. A
    /// real execution error wins over the pump/clock error and is never
    /// overwritten; the request is not dispatched in that case.
    fn sync_barrier(&self) -> CadResult<()> {
        let client = self.client()?;
        // Pre-poll: an old async error is surfaced but never blocks this request.
        let _ = self.drain_completions(&client);
        let pump_result = client.pump();
        // Post-pump drain always runs, even when the pump failed.
        let completion_error = self.drain_completions(&client);
        match pump_result {
            Ok(report) => {
                // A completed old async error was already shown; it is not this
                // sync request's result, so proceed by queued pending only. The
                // current dispatch's own real error/update error is reported later
                // by the caller and thus has the last word.
                if report.pending {
                    return Err(CadError::InvalidInput("engine is busy".into()));
                }
                Ok(())
            }
            Err(pump_error) => match completion_error {
                // A real execution error from the drained completions wins over
                // the clock/update error rather than being overwritten by it.
                Some(error) => Err(error),
                None => Err(request_error(pump_error)),
            },
        }
    }

    /// Drain terminal completions in bounded batches, regardless of `pending`,
    /// returning the first real terminal error (already reported) so callers can
    /// give it precedence over a later clock/update failure.
    fn drain_completions(&self, client: &Rc<dyn EngineClient>) -> Option<CadError> {
        let mut first_error: Option<CadError> = None;
        let mut drained = 0usize;
        loop {
            let batch = client.poll_completions(COMPLETION_POLL_LIMIT);
            if batch.is_empty() {
                break;
            }
            drained += batch.len();
            if let Some(error) = self.process_completions(batch) {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
            if drained >= cad_engine::queue::QUEUE_CAPACITY {
                break;
            }
        }
        first_error
    }

    /// Consume completions: a receipt never means success, `Superseded` is quiet,
    /// and a real terminal error is reported through its recorded origin. Returns
    /// the first real error so a caller can preserve its precedence.
    fn process_completions(&self, completions: Vec<RequestCompletion>) -> Option<CadError> {
        let mut first_error: Option<CadError> = None;
        for completion in completions {
            // Consume the recorded origin so the bounded map never leaks an id.
            // The cursor origin currently maps to the single `command.failed`
            // catalog copy; a future async origin would branch on it here.
            let _ = self.origins.borrow_mut().take(completion.id);
            match completion.result {
                Ok(_) => {}
                Err(RequestError::Superseded { .. }) => {}
                Err(error) => {
                    let cad = request_error(error);
                    if let Some(handle) = self.handle.borrow().clone() {
                        let _ = handle.report_action_error(&cad);
                    }
                    if first_error.is_none() {
                        first_error = Some(cad);
                    }
                }
            }
        }
        first_error
    }

    pub(crate) fn interaction(&self, request: InteractionRequest) -> CadResult<InteractionOutcome> {
        self.sync_barrier()?;
        match self.dispatch(EngineRequest::Interaction(request))? {
            RequestResult::Interaction(outcome) => Ok(outcome),
            // An API/completion result routed to an interaction verb is a type
            // mismatch: refuse explicitly rather than fake a handler success.
            RequestResult::Command(_) => Err(CadError::Invariant(
                "interaction dispatch returned a command result".into(),
            )),
            RequestResult::Api(_) => Err(CadError::Invariant(
                "interaction dispatch returned an api result".into(),
            )),
        }
    }

    pub(crate) fn flush_updates(&self) -> CadResult<()> {
        // Deliberately no pump/poll: apps run `pump_requests` before this so the
        // IO-first order is not hidden inside a state flush.
        let client = self.client()?;
        let updates = client.take_updates()?;
        self.apply_updates(updates)
    }

    /// Advance the queue by one bounded pass and drain its completions. Apps call
    /// this before [`Self::flush_updates`]; it never touches render state.
    pub(crate) fn pump_requests(&self) -> CadResult<PumpReport> {
        let client = self.client()?;
        let pump_result = client.pump();
        let execution_error = self.drain_completions(&client);
        match pump_result {
            // A successful pump still returns its report; any real execution error
            // from the drained completions is already shown through the UI.
            Ok(report) => Ok(report),
            // A real execution error wins over the pump/clock error instead of
            // being overwritten by it.
            Err(pump_error) => match execution_error {
                Some(error) => Err(error),
                None => Err(request_error(pump_error)),
            },
        }
    }

    /// Forward a queue wake registration to the engine client. The waker must
    /// only schedule (never synchronously pump) and must not hold a strong
    /// reference back to the adapter.
    pub(crate) fn set_request_waker(&self, waker: Option<Rc<dyn Fn()>>) -> CadResult<()> {
        let client = self.client()?;
        client.set_request_waker(waker);
        Ok(())
    }

    pub(crate) fn command(&self, command: cad_app::Command) -> CadResult<()> {
        self.sync_barrier()?;
        match self.dispatch(EngineRequest::Command(command))? {
            // The command verdict is all this verb needs; the payload is already
            // behind an `Arc`, so it is never deep-cloned.
            RequestResult::Command(_) => Ok(()),
            RequestResult::Interaction(_) => Err(CadError::Invariant(
                "command dispatch returned an interaction result".into(),
            )),
            RequestResult::Api(_) => Err(CadError::Invariant(
                "command dispatch returned an api result".into(),
            )),
        }
    }

    pub(crate) fn point_input(&self, position: [f64; 2]) -> CadResult<PointInput> {
        let client = self.client()?;
        point_input_for(&client, position)
    }

    /// Submit one `MoveCursor` asynchronously. The newest cursor sample is
    /// admitted after freeing completion quota (so coalescing never backpressures
    /// on unclaimed completions); the receipt is not an execution result.
    pub(crate) fn submit_cursor(&self, position: [f64; 2]) -> CadResult<()> {
        let client = self.client()?;
        let _ = self.drain_completions(&client);
        // Fresh context: a post-`BeginDraw` cursor must not reuse an older tool
        // generation.
        let context = client.request_context();
        let point = point_input_for(&client, position)?;
        let receipt = client
            .submit(
                context,
                EngineRequest::Interaction(InteractionRequest::MoveCursor(point)),
            )
            .map_err(request_error)?;
        self.origins
            .borrow_mut()
            .record(receipt.id, CompletionOrigin::Cursor);
        Ok(())
    }

    fn apply_updates(&self, updates: HostUpdates) -> CadResult<()> {
        // Nothing published: do not require a bound handle. This keeps a
        // dispatch that produced no state routable before the UI surface exists.
        if updates.full.is_none()
            && updates.navigation.is_none()
            && updates.config.is_none()
            && updates.status.is_none()
            && updates.backend.is_none()
        {
            return Ok(());
        }
        let handle = self.handle.borrow().clone().ok_or(CadError::Cancelled)?;
        // Backend is an independent lifecycle delta: apply it even when nothing
        // else is published, and never let a full push clear the cached label.
        if let Some(backend) = updates.backend {
            handle.apply_backend_state(backend)?;
        }
        if let Some(config) = updates.config {
            handle.apply_config_snapshot(config)?;
        }
        if let Some(full) = updates.full {
            handle.apply_host_state(full)?;
        }
        if let Some(navigation) = updates.navigation {
            // The host has already synced the shared session's authoritative
            // camera, space, mode and canvas metrics (safe insets / DPR). The UI
            // must NOT rebuild that snapshot from its own uncropped surface size:
            // doing so desynced the CPU pick metrics from the renderer. The UI
            // only mirrors the raw navigation state into its presentation model
            // (view state) and the snap-hint overlay, so there is exactly one
            // writer of the render snapshot (the host) and the presenter wake
            // stays driven by the session's own revision.
            let view = self.view.borrow().clone().ok_or(CadError::Cancelled)?;
            let hints = navigation
                .snap_hints
                .into_iter()
                .map(|hint| {
                    let kind = match hint.kind.as_str() {
                        "endpoint" => cad_measure::SnapKind::Endpoint,
                        "midpoint" => cad_measure::SnapKind::Midpoint,
                        "center" => cad_measure::SnapKind::Center,
                        "quadrant" => cad_measure::SnapKind::Quadrant,
                        "perpendicular" => cad_measure::SnapKind::Perpendicular,
                        "intersection" => cad_measure::SnapKind::LocalIntersection,
                        unknown => {
                            return Err(CadError::InvalidInput(format!(
                                "unknown snap hint kind: {unknown}"
                            )))
                        }
                    };
                    Ok(cad_app::render_scene::SnapHint {
                        kind,
                        point: hint.point,
                        space: hint.space,
                        source: hint.source,
                        secondary: hint.secondary,
                        precision: hint.precision,
                        logical_pixel_distance: hint.logical_pixel_distance,
                    })
                })
                .collect::<CadResult<Vec<_>>>()?;
            view.set_snap_hints(hints);
            // Mirror the raw navigation state into the UI model. `is_3d` comes
            // from the authoritative `mode`, never guessed from `perspective`, so
            // an orthographic 3D view stays 3D.
            handle.set_view_state(crate::ViewStateUi {
                is_3d: navigation.mode == cad_app::ProjectionKind::ThreeD,
                perspective: !navigation.camera.projection.is_orthographic(),
                standard_view: crate::standard_view_for_camera(&navigation.camera),
            })?;
        }
        if let Some(status) = updates.status {
            handle.set_status_message(status)?;
        }
        Ok(())
    }
}

fn request_error(error: RequestError) -> CadError {
    match error {
        RequestError::Execution(error) => error,
        RequestError::Cancelled => CadError::Cancelled,
        RequestError::StaleContext => CadError::StaleResult,
        RequestError::Busy => CadError::InvalidInput("engine is busy".into()),
        RequestError::QueueFull => CadError::InvalidInput("engine request queue is full".into()),
        // A coalesced-away request did no work and is never an error to show;
        // callers that care match it before converting.
        RequestError::Superseded { .. } => CadError::Cancelled,
    }
}

/// Build a canvas-local point from the client's current metrics revision.
fn point_input_for(client: &Rc<dyn EngineClient>, position: [f64; 2]) -> CadResult<PointInput> {
    let revision = client
        .request_context()
        .metrics_revision
        .ok_or_else(|| CadError::InvalidInput("canvas metrics revision is unavailable".into()))?;
    Ok(PointInput::CanvasLocal {
        logical_position: position,
        metrics_revision: revision,
    })
}

/// Reject an attach whose client and UI facade do not share one owner-thread
/// render session. Factored out so the identity rule is unit-testable without a
/// Slint window.
pub(crate) fn require_shared_session(
    client: &cad_engine::render::RenderSessionHandle,
    facade: &cad_engine::render::RenderSessionHandle,
) -> CadResult<()> {
    if client.same_session(facade) {
        Ok(())
    } else {
        Err(CadError::InvalidInput(
            "UI facade and engine client must share the same render session".into(),
        ))
    }
}

/// Attached UI adapter for one engine client and its shared render session.
pub struct EngineAdapter {
    client: Rc<dyn EngineClient>,
    handle: UiHandle,
    dispatch: Rc<EngineDispatch>,
}

impl EngineAdapter {
    pub fn attach(
        adapter: &mut UiAdapter,
        client: Rc<dyn EngineClient>,
        view: CadView,
    ) -> CadResult<Self> {
        require_shared_session(&client.render_session(), &view.render_session())?;
        let handle = adapter.handle();
        handle.apply_config_snapshot(client.config_snapshot())?;
        adapter.engine_dispatch.set_handle(handle.clone());
        adapter.engine_dispatch.attach(client.clone(), view.clone());
        *adapter.engine_client.borrow_mut() = Some(client.clone());
        Ok(Self {
            client,
            handle,
            dispatch: adapter.engine_dispatch.clone(),
        })
    }

    /// Drain host state updates into the UI. Never pumps or polls the queue, so
    /// apps can keep the fixed order `IO → refresh_metrics → pump_requests →
    /// flush_updates` without hidden work.
    pub fn flush_updates(&self) -> CadResult<()> {
        self.dispatch.flush_updates()
    }

    /// Advance the owner-thread queue one bounded pass and drain its completions.
    ///
    /// Apps call this after `refresh_metrics` and before `flush_updates`. The
    /// returned [`PumpReport::pending`] is queued-executable work, not unclaimed
    /// completions (which are always drained here).
    pub fn pump_requests(&self) -> CadResult<PumpReport> {
        self.dispatch.pump_requests()
    }

    /// Register (or clear) the engine queue wake callback.
    ///
    /// The waker must only schedule a platform wake; it must never synchronously
    /// pump and must not capture a strong reference back to this adapter (use a
    /// `Weak` to avoid a reference cycle).
    pub fn set_request_waker(&self, waker: Option<Rc<dyn Fn()>>) -> CadResult<()> {
        self.dispatch.set_request_waker(waker)
    }

    pub fn refresh_metrics(&self) -> CadResult<()> {
        let (size, dpi_scale) = self.handle.cad_surface_size().ok_or(CadError::Cancelled)?;
        let metrics = cad_app::CanvasMetrics::new([0.0, 0.0], size, dpi_scale);
        if !metrics.is_valid() {
            return Err(CadError::InvalidInput(
                "CAD canvas metrics are degenerate".into(),
            ));
        }
        self.client.set_canvas_metrics(metrics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_engine::render::RenderSessionHandle;
    use cad_render_wgpu::BackendPreference;

    #[test]
    fn attach_rejects_facade_and_client_with_distinct_sessions() {
        let shared = RenderSessionHandle::new(BackendPreference::Auto);
        let foreign = RenderSessionHandle::new(BackendPreference::Auto);
        // A clone shares the same owner-thread state and is accepted.
        assert!(require_shared_session(&shared, &shared.clone()).is_ok());
        // Two independently created sessions are rejected explicitly.
        let error = require_shared_session(&shared, &foreign).unwrap_err();
        assert!(matches!(error, CadError::InvalidInput(_)));
    }

    #[test]
    fn superseded_maps_to_a_non_error_verdict() {
        // A coalesced-away request did no work: it must never surface as an error.
        let mapped = request_error(RequestError::Superseded { by: RequestId(9) });
        assert!(matches!(mapped, CadError::Cancelled));
    }

    #[test]
    fn command_results_are_arc_shared_not_deep_cloned() {
        let result = RequestResult::Command(std::sync::Arc::new(cad_app::CommandOutcome::none()));
        let clone = result.clone();
        match (&result, &clone) {
            (RequestResult::Command(a), RequestResult::Command(b)) => {
                assert!(
                    std::sync::Arc::ptr_eq(a, b),
                    "a completion clone shares the outcome Arc instead of deep-copying"
                );
            }
            _ => panic!("expected command results"),
        }
    }

    #[test]
    fn origin_bookkeeping_is_bounded_and_consumes_each_id_once() {
        let mut origins = OriginBookkeeping::new(2);
        origins.record(RequestId(1), CompletionOrigin::Cursor);
        origins.record(RequestId(2), CompletionOrigin::Cursor);
        // A third id evicts the oldest, so the map never exceeds its bound.
        origins.record(RequestId(3), CompletionOrigin::Cursor);
        assert_eq!(origins.entries.len(), 2);
        assert_eq!(origins.take(RequestId(1)), None);
        assert_eq!(origins.take(RequestId(3)), Some(CompletionOrigin::Cursor));
        assert_eq!(origins.take(RequestId(3)), None);
        assert_eq!(origins.entries.len(), 1);
    }
}
