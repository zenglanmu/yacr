//! Queue admission, staleness checks and the bounded pump pass.
use super::*;
use cad_app::interaction::PointerPhase;
use cad_engine::events::{EventPoll, EventWatermark, SubscriptionId};

impl HostCore {
    /// Whether a queued request is stale against the current host context.
    ///
    /// Every request checks document / document epoch / viewport, and a command
    /// also its own target; tool requests additionally check the tool
    /// generation; canvas-local requests check the metrics revision; gesture
    /// requests check the config revision.
    fn request_is_stale(&self, context: &RequestContext, request: &EngineRequest) -> bool {
        let current = self.request_context();
        if context.document != current.document
            || context.document_generation != current.document_generation
            || context.viewport != current.viewport
        {
            return true;
        }
        if let EngineRequest::Command(command) = request {
            if command.document != current.document || command.viewport != current.viewport {
                return true;
            }
        }
        if tool_generation_checked(request) && context.tool_generation != current.tool_generation {
            return true;
        }
        if let Some(metrics_revision) = request_metrics_revision(request) {
            if Some(metrics_revision) != current.metrics_revision {
                return true;
            }
        }
        if config_checked(request) && context.config_revision != current.config_revision {
            return true;
        }
        false
    }

    /// Acquire the synchronous dispatch guard, or `Busy` while queued/executing
    /// work or another pump/clock/sync window is active.
    pub(crate) fn begin_sync_request(&self) -> Result<ActiveGuard<'_>, RequestError> {
        if self.request_engine_busy() || self.queue.pending() || self.queue.executing_id().is_some()
        {
            return Err(RequestError::Busy);
        }
        Ok(ActiveGuard::new(&self.sync_active))
    }

    /// Clear the pointer life for a stale `Up`/`Cancel` without navigating.
    fn clear_pointer_life(&self, id: PointerId) {
        let mut active = self.navigation.active.borrow_mut();
        active.retain(|active| active != &id);
        let empty = active.is_empty();
        drop(active);
        if empty {
            self.navigation.anchor.set(None);
            self.navigation.last.set(None);
            self.navigation.nav_last.set(None);
            self.navigation.dragged.set(false);
            self.navigation.navigating.set(false);
            self.navigation.button.set(None);
            self.navigation.press_modifiers.set(None);
            *self.navigation.box_gesture.borrow_mut() = None;
        }
    }

    /// Invoke the registered waker with every queue borrow released.
    pub(crate) fn invoke_request_waker(&self) {
        let waker = self.request_waker.borrow().clone();
        if let Some(waker) = waker {
            waker();
        }
    }

    fn on_stale_request(&self, request: &EngineRequest) {
        if let EngineRequest::Interaction(InteractionRequest::Pointer {
            id,
            phase: PointerPhase::Up | PointerPhase::Cancel,
            ..
        }) = request
        {
            self.clear_pointer_life(*id);
        }
    }

    /// One bounded pump pass over the queue.
    ///
    /// The time budget is a single per-pass 4 ms window (not per request): the
    /// clock is sampled before each `begin` and the pass stops once the budget is
    /// exhausted. Without an installed clock, pending work is refused with an
    /// explicit `Unsupported` and no request is taken; an empty queue needs no
    /// clock.
    fn pump_inner(&self) -> Result<PumpReport, RequestError> {
        // No executable work: nothing to do, and no clock is needed (or sampled).
        if !self.queue.pending() {
            return Ok(self.queue.report(0));
        }
        // Clone the clock out and release the borrow before running the callback.
        let clock = self.request_clock.borrow().clone();
        let Some(clock) = clock else {
            return Err(RequestError::Execution(CadError::Unsupported(
                "request clock is not configured for this host".into(),
            )));
        };

        let start = clock();
        if !start.is_finite() || start < 0.0 {
            return Err(RequestError::Execution(CadError::InvalidInput(
                "request clock must return a finite non-negative millisecond timestamp".into(),
            )));
        }
        let mut previous = start;
        let mut budget = cad_engine::queue::PumpBudget::new();
        let mut processed = 0usize;
        loop {
            // Sample and validate before each request so the 4 ms window is a
            // per-pass budget, never a per-request preemption. A non-finite or
            // backwards sample is an explicit failure, never a fabricated time.
            let now = clock();
            if !now.is_finite() || now < previous || now < start {
                return Err(RequestError::Execution(CadError::InvalidInput(
                    "request clock must return a finite non-decreasing timestamp".into(),
                )));
            }
            previous = now;
            if budget.is_exhausted(now - start) {
                break;
            }
            let Some(mut executing) = self.queue.begin() else {
                break;
            };
            budget.record();
            let origin = executing.origin();
            let context = *executing.context();
            let Some(request) = executing.take_request() else {
                let error = RequestError::Execution(CadError::Invariant(
                    "queued request had no payload".into(),
                ));
                if let Some(entry) = self.permits.borrow_mut().remove(&executing.id()) {
                    let mut permit = entry.permit;
                    self.publish_finished_only(
                        &mut permit,
                        entry.origin,
                        &entry.context,
                        executing.id(),
                        &Err(RequestError::Execution(CadError::Invariant(
                            "queued request had no payload".into(),
                        ))),
                    );
                }
                executing.complete(Err(error));
                processed += 1;
                continue;
            };
            if self.request_is_stale(&context, &request) {
                // A stale queued request publishes only its terminal Finished
                // (no Started), with its old context; no business runs.
                self.on_stale_request(&request);
                self.finish_queued_stale(executing, origin, context);
                processed += 1;
                continue;
            }
            self.run_queued(executing, origin, request);
            processed += 1;
        }
        // Re-arm only while executable work remains, then notify once. All queue
        // borrows are released here; the waker only schedules.
        if self.queue.rearm_wake() && self.queue.take_wake() {
            self.invoke_request_waker();
        }
        Ok(self.queue.report(processed))
    }
}

/// Whether a queued request must also match the tool generation.
fn tool_generation_checked(request: &EngineRequest) -> bool {
    matches!(
        request,
        EngineRequest::Interaction(
            InteractionRequest::MoveCursor(_)
                | InteractionRequest::CapturePoint(_)
                | InteractionRequest::Confirm
                | InteractionRequest::ClearCursor
        )
    )
}

/// Whether a request carries a coordinate/gesture intent and so must also match
/// the current config revision (World or canvas-local alike).
///
/// `BeginDraw`/`BeginMeasurement`/`SetMeasurementKind`/`BeginPan`/`ClearCursor`/
/// `Confirm`/`Cancel` carry no coordinate and keep their original rules; metrics
/// are checked separately and only for canvas-local coordinates.
fn config_checked(request: &EngineRequest) -> bool {
    matches!(
        request,
        EngineRequest::Interaction(
            InteractionRequest::SelectAt { .. }
                | InteractionRequest::CapturePoint(_)
                | InteractionRequest::MoveCursor(_)
                | InteractionRequest::Pointer { .. }
                | InteractionRequest::Scroll { .. }
        )
    )
}

/// The metrics revision a request's canvas-local coordinate was tagged with.
fn request_metrics_revision(request: &EngineRequest) -> Option<u64> {
    let point = match request {
        EngineRequest::Interaction(InteractionRequest::CapturePoint(point))
        | EngineRequest::Interaction(InteractionRequest::MoveCursor(point)) => *point,
        EngineRequest::Interaction(InteractionRequest::SelectAt { point, .. }) => *point,
        EngineRequest::Interaction(InteractionRequest::Pointer { position, .. }) => *position,
        EngineRequest::Interaction(InteractionRequest::Scroll { position, .. }) => *position,
        _ => return None,
    };
    match point {
        PointInput::CanvasLocal {
            metrics_revision, ..
        } => Some(metrics_revision),
        PointInput::World(_) => None,
    }
}

/// Whether a request may be admitted through the asynchronous `submit` path.
///
/// Commands that need a user gesture (file pickers, backend switch) must run
/// synchronously through `dispatch`.
pub(crate) fn admissible_for_submit(request: &EngineRequest) -> bool {
    match request {
        EngineRequest::Command(command) => !matches!(
            command.id,
            CommandId::OpenDrawing
                | CommandId::PlotDrawing
                | CommandId::SaveDrawingAs
                | CommandId::SwitchBackend
        ),
        EngineRequest::Interaction(_) => true,
        // Controlled API requests are queue-admissible; only gesture commands
        // are refused (P3 rule unchanged).
        EngineRequest::Api { .. } => true,
    }
}

impl EngineClient for HostCore {
    fn dispatch(&self, request: EngineRequest) -> Result<RequestResult, RequestError> {
        // The legacy `dispatch` is the `Ui` shorthand for an origin-tagged,
        // traced synchronous request.
        self.dispatch_with_origin_impl(RequestOrigin::Ui, request)
            .and_then(|completion| completion.result)
    }

    fn request_context(&self) -> RequestContext {
        let c = self.controller.borrow();
        RequestContext {
            document: c.document_id,
            // The independent host document epoch, not the session generation:
            // edits/tools/camera never bump it.
            document_generation: self.document_epoch.get(),
            viewport: c.viewport_id,
            tool_generation: Some(c.session.tool_generation),
            metrics_revision: Some(self.metrics_revision.get()),
            config_revision: self.viewer_config.borrow().revision,
        }
    }

    fn take_updates(&self) -> CadResult<HostUpdates> {
        self.sync_render()?;
        let full_dirty = self.full_dirty.get();
        let navigation_dirty = self.navigation_dirty.get();
        let config_dirty = self.config_dirty.get();
        let status_dirty = self.status_dirty.get();
        let full = if full_dirty {
            Some(self.full_state()?)
        } else {
            None
        };
        let navigation = if navigation_dirty {
            Some(self.navigation_state()?)
        } else {
            None
        };
        let config = if config_dirty {
            Some(self.config_snapshot())
        } else {
            None
        };
        let status = if status_dirty {
            self.last_status.borrow().clone()
        } else {
            None
        };
        // The backend delta is derived unconditionally from the renderer's real
        // lifecycle/actual state, so a backend change is published even when
        // every other dirty flag is clear, and never forces a full query.
        let backend_candidate = self.backend_state();
        let backend = if self.last_backend.borrow().as_ref() != Some(&backend_candidate) {
            Some(backend_candidate.clone())
        } else {
            None
        };
        // Commit only after every fallible derivation above succeeded: on `Err`
        // the dirty markers (and the last published backend) are untouched.
        self.full_dirty.set(false);
        self.navigation_dirty.set(false);
        self.config_dirty.set(false);
        self.status_dirty.set(false);
        *self.last_backend.borrow_mut() = Some(backend_candidate);
        Ok(HostUpdates {
            full,
            navigation,
            config,
            status,
            backend,
        })
    }

    fn set_canvas_metrics(&self, metrics: CanvasMetrics) -> CadResult<()> {
        HostCore::set_canvas_metrics(self, metrics)
    }
    fn render_session(&self) -> RenderSessionHandle {
        self.render.clone()
    }
    fn config_snapshot(&self) -> ConfigSnapshot {
        HostCore::config_snapshot(self)
    }
    fn set_config_json(&self, json: &str) -> CadResult<()> {
        HostCore::set_config_json(self, json)
    }
    fn update_config_json(&self, json: &str) -> CadResult<()> {
        HostCore::update_config_json(self, json)
    }
    fn apply_user_preference_json(&self, json: &str) -> CadResult<()> {
        HostCore::apply_user_preference_json(self, json)
    }
    fn clear_user_preference(&self) -> CadResult<()> {
        // A real clear of the single store, never the trait's refusing default.
        HostCore::clear_user_preference(self)
    }

    fn submit(
        &self,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError> {
        // The legacy `submit` is the `Ui` shorthand for an origin-tagged submit.
        self.submit_with_origin_impl(RequestOrigin::Ui, context, request)
    }

    fn pump(&self) -> Result<PumpReport, RequestError> {
        if self.request_engine_busy() {
            return Err(RequestError::Busy);
        }
        // RAII so the re-entrancy flag is cleared even on panic/unwind.
        let _guard = ActiveGuard::new(&self.pump_active);
        self.pump_inner()
    }

    fn poll_completions(&self, limit: usize) -> Vec<RequestCompletion> {
        self.poll_completions_impl(limit)
    }

    fn set_request_waker(&self, waker: Option<Rc<dyn Fn()>>) {
        *self.request_waker.borrow_mut() = waker;
    }

    fn dispatch_with_origin(
        &self,
        origin: RequestOrigin,
        request: EngineRequest,
    ) -> Result<RequestCompletion, RequestError> {
        self.dispatch_with_origin_impl(origin, request)
    }

    fn submit_with_origin(
        &self,
        origin: RequestOrigin,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError> {
        self.submit_with_origin_impl(origin, context, request)
    }

    fn subscribe(&self) -> CadResult<SubscriptionId> {
        self.subscribe_impl()
    }

    fn poll_events(&self, id: SubscriptionId, limit: usize) -> CadResult<EventPoll> {
        self.poll_events_impl(id, limit)
    }

    fn unsubscribe(&self, id: SubscriptionId) -> CadResult<()> {
        self.unsubscribe_impl(id)
    }

    fn event_watermark(&self) -> EventWatermark {
        self.event_watermark_impl()
    }
}
