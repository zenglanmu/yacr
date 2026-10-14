//! Browser request-queue wiring logic, free of UI/browser types so host tests
//! exercise the exact decisions.
//!
//! The P3 owner-thread queue lives in the engine/host; the browser wrapper only
//! contributes platform decisions that belong to no shared layer:
//!
//! * the wheel `Scroll` sign is the opposite of the desktop winit convention the
//!   host assumes, so it is normalized exactly once (on the synchronous path and,
//!   for consistency, before an async `submit`); and
//! * the browser-only `SwitchBackend` command is classified for the wrapper,
//!   which runs its store/reload as the action of the host's traced
//!   `dispatch_external_with_origin` bridge — never a nested guard, a probe or a
//!   faked id.
//!
//! The origin-tagged P4 path reuses [`origin_plan`] so a request gets the same
//! browser interception regardless of its trace origin (`Api` requests forward
//! verbatim; the browser backend switch routes to the host's traced bridge as
//! [`OriginPlan::Backend`], never a raw dispatch or a faked id).
//! Subscription delegation is verbatim ([`forwarded_event_limit`]); the coalesced
//! wake scheduler is modeled purely (host-testable failure path); and the pump
//! time source is wrapped so installation needs no fabricated performance sample.

use cad_app::interaction::InteractionRequest;
use cad_app::{BackendChoice, CommandId, CommandPayload};
use cad_domain::{CadError, CadResult};
use cad_engine::protocol::EngineRequest;
use std::rc::Rc;

/// What the browser wrapper does with one inbound request.
pub(crate) enum WebRequest {
    /// Not browser-owned: forward verbatim to the host.
    Forward(EngineRequest),
    /// A wheel `Scroll`, already sign-normalized for the browser.
    Scroll(EngineRequest),
    /// The browser-owned backend-switch command (admission still belongs to the
    /// host).
    Backend(EngineRequest),
}

/// Normalize the browser wheel sign exactly once (vertical axis only).
///
/// The browser reports the opposite vertical sign to the desktop winit backend
/// the shared scroll handler assumes. This is intentionally not idempotent:
/// applying it twice returns the original, which is precisely the double
/// normalization the wrapper must avoid by normalizing once.
pub(crate) fn normalized_scroll_delta(delta_logical: [f64; 2]) -> [f64; 2] {
    [delta_logical[0], -delta_logical[1]]
}

/// Apply the one-time browser scroll normalization, forwarding everything else
/// verbatim (queued-submit path).
pub(crate) fn normalize_once(request: EngineRequest) -> EngineRequest {
    match classify(request) {
        WebRequest::Forward(request)
        | WebRequest::Scroll(request)
        | WebRequest::Backend(request) => request,
    }
}

/// What the origin-tagged synchronous path does with one request.
pub(crate) enum OriginPlan {
    /// Forward the (already scroll-normalized) request to the host origin call.
    Forward(EngineRequest),
    /// The browser-owned backend switch goes to the host's traced external-closure
    /// bridge (`HostCore::dispatch_external_with_origin`), which owns the permit /
    /// real request id / `RequestStarted`+`RequestFinished` lifecycle. The command
    /// is carried through unchanged; the wrapper never fakes an id or treats a host
    /// refusal as authorization.
    Backend(EngineRequest),
}

/// Classify a request for the origin-tagged path.
///
/// `Api` requests forward verbatim (the host owns their authorization); a wheel
/// scroll normalizes exactly once, matching the synchronous path so the direction
/// cannot differ by origin; the browser backend switch is carried as
/// [`OriginPlan::Backend`] for the host's traced bridge.
pub(crate) fn origin_plan(request: EngineRequest) -> OriginPlan {
    match classify(request) {
        WebRequest::Forward(request) | WebRequest::Scroll(request) => OriginPlan::Forward(request),
        WebRequest::Backend(request) => OriginPlan::Backend(request),
    }
}

/// Extract the (Copy) backend choice the browser switch needs, without moving the
/// command into the action closure.
///
/// A non-`Backend` payload is an explicit error; it never reaches the bridge, so an
/// illegal payload allocates no id and has no side effect.
pub(crate) fn backend_choice(request: &EngineRequest) -> CadResult<BackendChoice> {
    match request {
        EngineRequest::Command(command) => match &command.payload {
            CommandPayload::Backend(choice) => Ok(*choice),
            _ => Err(CadError::InvalidInput(
                "SwitchBackend requires a backend payload".into(),
            )),
        },
        _ => Err(CadError::Invariant(
            "backend classification lost its command".into(),
        )),
    }
}

/// Classify one request for the browser wrapper, normalizing a wheel scroll once.
pub(crate) fn classify(request: EngineRequest) -> WebRequest {
    match request {
        EngineRequest::Command(command) => {
            if command.id == CommandId::SwitchBackend {
                WebRequest::Backend(EngineRequest::Command(command))
            } else {
                WebRequest::Forward(EngineRequest::Command(command))
            }
        }
        EngineRequest::Interaction(InteractionRequest::Scroll {
            delta_logical,
            position,
            modifiers,
        }) => WebRequest::Scroll(EngineRequest::Interaction(InteractionRequest::Scroll {
            delta_logical: normalized_scroll_delta(delta_logical),
            position,
            modifiers,
        })),
        other => WebRequest::Forward(other),
    }
}

/// Coalesced wake scheduler state — **logical** only.
///
/// Pure so host tests can verify the coalesce/rearm decisions without a browser.
/// It models whether a callback is currently scheduled; it does **not** model or
/// prove allocation release. The real `Closure` is reclaimed by an explicit
/// `Drop` of the `LIVE_RAF_CALLBACK` slot at the next schedule (or on a schedule
/// failure) — that ownership is proven by the source, not by this model.
///
/// There is deliberately no "synchronous pump" action: a failed schedule only
/// frees the slot so a later wake may retry, and the app heartbeat is the
/// backstop.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WakeSchedule {
    /// Whether a coalesced pump callback is currently scheduled (logical).
    callback_alive: bool,
}

impl WakeSchedule {
    /// Claim the single slot. `true` means the caller should create and schedule
    /// a callback now; `false` means one is already scheduled (coalesced).
    pub(crate) fn claim(&mut self) -> bool {
        if self.callback_alive {
            return false;
        }
        self.callback_alive = true;
        true
    }

    /// The scheduled callback ran: clear the scheduled flag so a pending rearm
    /// during the pump may schedule a new callback.
    ///
    /// This is a logical transition only; the previously scheduled handle is
    /// released by the explicit `Drop` performed at the next schedule.
    pub(crate) fn on_executed(&mut self) {
        self.callback_alive = false;
    }

    /// Scheduling failed: the callback handle is dropped by the caller and the
    /// slot is freed so a later wake may retry. Never pumps synchronously.
    pub(crate) fn on_schedule_failed(&mut self) {
        self.callback_alive = false;
    }

    #[cfg(test)]
    pub(crate) fn callback_alive(&self) -> bool {
        self.callback_alive
    }
}

/// Subscription delegation is verbatim: the browser wrapper never auto-subscribes,
/// never clamps the caller's `limit` and never synthesizes an empty poll. The
/// engine event ring owns the real poll bound and lag semantics.
pub(crate) fn forwarded_event_limit(limit: usize) -> usize {
    limit
}

/// Return the millisecond clock to install on the host pump.
///
/// Pure and dependency-free: it forwards the caller's monotonic-millisecond
/// source (`performance.now()` in the browser). It never fabricates a sample, so
/// a missing/invalid performance reading surfaces as the source's own value and
/// is rejected by the host installer rather than replaced by a fake zero.
pub(crate) fn request_clock_from(now_ms: Rc<dyn Fn() -> f64>) -> Rc<dyn Fn() -> f64> {
    now_ms
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_app::api::{ApiRequest, ApiTarget, SelectionMode};
    use cad_app::interaction::{Modifiers, PointInput};
    use cad_app::{BackendChoice, Command, CommandPayload};
    use cad_domain::{DatabaseId, DocumentId, Point3, ViewportId};

    fn command(id: CommandId, payload: CommandPayload) -> EngineRequest {
        EngineRequest::Command(Command {
            schema_version: 1,
            id,
            document: DocumentId(1),
            viewport: ViewportId(2),
            payload,
        })
    }

    fn scroll(delta_logical: [f64; 2]) -> EngineRequest {
        EngineRequest::Interaction(InteractionRequest::Scroll {
            delta_logical,
            position: PointInput::World(Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }),
            modifiers: Modifiers::default(),
        })
    }

    #[test]
    fn backend_switch_is_classified_for_the_browser() {
        let request = command(
            CommandId::SwitchBackend,
            CommandPayload::Backend(BackendChoice::WebGl2),
        );
        match classify(request) {
            WebRequest::Backend(EngineRequest::Command(command)) => {
                assert_eq!(command.id, CommandId::SwitchBackend);
                assert!(matches!(
                    command.payload,
                    CommandPayload::Backend(BackendChoice::WebGl2)
                ));
            }
            _ => panic!("SwitchBackend must be classified as browser-owned"),
        }
    }

    #[test]
    fn scroll_is_normalized_exactly_once() {
        match classify(scroll([3.0, 4.0])) {
            WebRequest::Scroll(EngineRequest::Interaction(InteractionRequest::Scroll {
                delta_logical,
                ..
            })) => assert_eq!(delta_logical, [3.0, -4.0]),
            _ => panic!("a wheel scroll must be classified (and normalized) as Scroll"),
        }
    }

    #[test]
    fn non_browser_requests_forward_verbatim() {
        // A non-browser command forwards with its identity and payload intact
        // (destructured, not matched with `_`, so the forwarded request is really
        // read).
        match classify(command(CommandId::NewDrawing, CommandPayload::None)) {
            WebRequest::Forward(EngineRequest::Command(command)) => {
                assert_eq!(command.id, CommandId::NewDrawing);
                assert!(matches!(command.payload, CommandPayload::None));
            }
            _ => panic!("a non-browser command must forward verbatim"),
        }
        // A non-scroll interaction forwards verbatim too.
        match classify(EngineRequest::Interaction(InteractionRequest::Confirm)) {
            WebRequest::Forward(EngineRequest::Interaction(InteractionRequest::Confirm)) => {}
            _ => panic!("a non-scroll interaction must forward verbatim"),
        }
    }

    fn api_request() -> EngineRequest {
        EngineRequest::Api {
            target: ApiTarget {
                document: DocumentId(1),
                database: DatabaseId(2),
                viewport: ViewportId(3),
            },
            request: ApiRequest::SelectByObjectIds {
                ids: Vec::new(),
                mode: SelectionMode::Replace,
            },
        }
    }

    #[test]
    fn origin_plan_normalizes_a_scroll_once() {
        match origin_plan(scroll([5.0, 6.0])) {
            OriginPlan::Forward(EngineRequest::Interaction(InteractionRequest::Scroll {
                delta_logical,
                ..
            })) => assert_eq!(delta_logical, [5.0, -6.0]),
            _ => panic!("the origin path must normalize a wheel scroll once"),
        }
    }

    #[test]
    fn origin_plan_forwards_api_requests_verbatim() {
        // The origin path must not drop or reinterpret an `Api` request; the host
        // owns its authorization and forwarding is exhaustive.
        match origin_plan(api_request()) {
            OriginPlan::Forward(EngineRequest::Api { target, request }) => {
                assert_eq!(target.database, DatabaseId(2));
                assert!(matches!(request, ApiRequest::SelectByObjectIds { .. }));
            }
            _ => panic!("an Api request must forward verbatim on the origin path"),
        }
    }

    #[test]
    fn origin_plan_routes_the_backend_switch_to_the_traced_bridge() {
        // The origin path must route the browser backend switch to the host's
        // traced external-closure bridge with the command intact (never a faked
        // id, never a raw host dispatch).
        match origin_plan(command(
            CommandId::SwitchBackend,
            CommandPayload::Backend(BackendChoice::Auto),
        )) {
            OriginPlan::Backend(EngineRequest::Command(command)) => {
                assert_eq!(command.id, CommandId::SwitchBackend);
                assert!(matches!(
                    command.payload,
                    CommandPayload::Backend(BackendChoice::Auto)
                ));
            }
            _ => panic!("SwitchBackend must route to the traced backend bridge"),
        }
    }

    #[test]
    fn backend_choice_reads_the_copy_choice_and_rejects_other_payloads() {
        assert_eq!(
            backend_choice(&command(
                CommandId::SwitchBackend,
                CommandPayload::Backend(BackendChoice::WebGpu)
            ))
            .unwrap(),
            BackendChoice::WebGpu
        );
        // A foreign payload is refused before the bridge (no id allocated).
        assert!(backend_choice(&command(CommandId::SwitchBackend, CommandPayload::None)).is_err());
    }

    #[test]
    fn normalize_once_only_touches_a_scroll() {
        match normalize_once(scroll([1.0, 2.0])) {
            EngineRequest::Interaction(InteractionRequest::Scroll { delta_logical, .. }) => {
                assert_eq!(delta_logical, [1.0, -2.0])
            }
            _ => panic!("a scroll must stay a scroll"),
        }
        match normalize_once(command(CommandId::NewDrawing, CommandPayload::None)) {
            EngineRequest::Command(command) => assert_eq!(command.id, CommandId::NewDrawing),
            _ => panic!("a command must forward unchanged"),
        }
    }

    #[test]
    fn wake_claim_coalesces_a_second_wake_while_the_callback_is_alive() {
        let mut schedule = WakeSchedule::default();
        assert!(schedule.claim(), "the first wake schedules a callback");
        assert!(
            schedule.callback_alive(),
            "the scheduled callback is live until it runs"
        );
        assert!(!schedule.claim(), "a second wake while live is coalesced");
    }

    #[test]
    fn wake_callback_alive_returns_to_zero_after_execution_and_rearms() {
        let mut schedule = WakeSchedule::default();
        assert!(schedule.claim());
        // The scheduled callback ran (logical transition; allocation release is
        // a separate explicit Drop, not asserted here).
        schedule.on_executed();
        assert!(!schedule.callback_alive(), "the slot is logically free");
        // Pending rearm: a later wake may schedule again.
        assert!(schedule.claim(), "a later wake may schedule again");
    }

    #[test]
    fn wake_schedule_failure_drops_the_callback_and_allows_retry() {
        let mut schedule = WakeSchedule::default();
        assert!(schedule.claim());
        // Scheduling (RAF) failed: the callback is explicitly dropped/reclaimed
        // and the slot freed; the waker never pumps synchronously.
        schedule.on_schedule_failed();
        assert!(!schedule.callback_alive());
        assert!(schedule.claim(), "a later wake may retry scheduling");
    }

    #[test]
    fn multiple_wake_rounds_return_to_the_idle_scheduled_state() {
        let mut schedule = WakeSchedule::default();
        for _ in 0..4 {
            assert!(schedule.claim());
            assert!(schedule.callback_alive());
            schedule.on_executed();
        }
        assert!(
            !schedule.callback_alive(),
            "each round returns the logical schedule state to idle (the handle \
             itself is released by the explicit Drop at the next schedule)"
        );
    }

    #[test]
    fn scroll_normalization_is_a_single_application_not_idempotent() {
        let once = normalized_scroll_delta([1.0, 2.0]);
        assert_eq!(once, [1.0, -2.0]);
        // A second pass would revert the sign: exactly the double-normalization
        // bug the wrapper avoids by normalizing once on the synchronous path.
        assert_eq!(normalized_scroll_delta(once), [1.0, 2.0]);
    }

    #[test]
    fn subscription_limit_is_forwarded_verbatim_without_an_empty_fake() {
        // The wrapper must not clamp, zero or otherwise rewrite the caller's poll
        // limit; the engine ring owns the real bound.
        assert_eq!(forwarded_event_limit(0), 0);
        assert_eq!(forwarded_event_limit(300), 300);
    }

    #[test]
    fn request_clock_delegates_without_fabricating_a_sample() {
        let clock = request_clock_from(Rc::new(|| 42.5));
        assert_eq!(clock(), 42.5);
    }

    #[test]
    fn request_clock_preserves_a_missing_sample_as_invalid() {
        // A missing performance source must surface as an invalid sample (the
        // host rejects it), never as a fabricated zero.
        let clock = request_clock_from(Rc::new(|| f64::NAN));
        assert!(clock().is_nan());
    }
}
