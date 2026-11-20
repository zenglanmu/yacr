//! The Options > Files **directory picker** shell capability.
//!
//! The fonts-directory row asks the host for one local absolute directory. The
//! chooser is platform I/O (Linux: the XDG desktop portal), so it is injected as
//! a plain closure by each concrete host; this module owns only the parts that
//! must not be re-spelled per carrier:
//!
//! * **single-flight admission** — one pick at a time, exactly like the file
//!   open/save admission in `apps/app-linux::platform::ReactFiles`;
//! * the **pending slot** — a worker thread runs the blocking portal call and
//!   publishes one terminal [`DirectoryOutcome`] into a capacity-1 slot; the
//!   owner tick drains it ([`DirectoryPicker::drain`]) and hands the answer to the
//!   waiting IPC call. The frame loop is never blocked by an open dialog: the
//!   async command only starts the worker and waits off-thread;
//! * the **answer vocabulary** — a cancelled dialog is `null`, a failed chooser is
//!   an explicit `pick_directory.failed: …` error (never a silent `null`), and a
//!   carrier that installs no picker refuses with `unsupported-runtime:
//!   pick_directory` before any thread starts.
//!
//! Which constant belongs to which case lives in
//! `cad_ui_core::session::directory_pick`, so the TS transport, the web refusal
//! and this host cannot drift.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use cad_ui_core::session::{
    pick_directory_failed, PICK_DIRECTORY_BUSY, PICK_DIRECTORY_LOST, UNSUPPORTED_PICK_DIRECTORY,
};

/// One finished folder pick, before it is mapped to the command's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectoryOutcome {
    /// The user chose this absolute directory.
    Path(String),
    /// The user closed the dialog: a real answer (`null`), not a failure.
    Cancelled,
    /// The chooser ran and failed; the reason is the host's technical text.
    Failed(String),
}

/// What the waiting command receives: `Ok(Some(path))`, `Ok(None)` (cancelled) or
/// an explicit `pick_directory.failed: …` error.
pub type DirectoryPickAnswer = Result<Option<String>, String>;

/// The receiving end of one admitted pick, resolved by the owner tick.
pub type DirectoryPickReceiver = Receiver<DirectoryPickAnswer>;

/// The blocking chooser a concrete host injects (`Send + Sync` so the worker
/// thread can own a clone).
pub type DirectoryChooser = Arc<dyn Fn() -> DirectoryOutcome + Send + Sync>;

/// Single-flight + pending-slot state, guarded by one mutex so admission and the
/// terminal hand-off cannot interleave.
///
/// `busy` covers admission until the owner tick takes the terminal; a worker
/// publishes the outcome but never touches `busy`, so a stale worker cannot
/// unlock a newer request.
#[derive(Default)]
struct PickState {
    busy: bool,
    /// The one accepted terminal awaiting the tick (capacity 1).
    terminal: Option<DirectoryOutcome>,
    /// The admitted call's channel; `None` when no request is in flight.
    waiter: Option<Sender<DirectoryPickAnswer>>,
}

/// A native folder picker over one injected chooser.
pub struct DirectoryPicker {
    chooser: DirectoryChooser,
    state: Arc<Mutex<PickState>>,
}

impl DirectoryPicker {
    /// A picker whose worker runs `chooser` on its own thread.
    pub fn new(chooser: DirectoryChooser) -> Arc<Self> {
        Arc::new(Self {
            chooser,
            state: Arc::new(Mutex::new(PickState::default())),
        })
    }

    /// Admit one pick and start its worker; the returned receiver resolves when
    /// the owner tick drains the terminal.
    ///
    /// A refusal (another pick in flight, or a thread that could not start) is
    /// explicit and leaves no half-admitted state behind.
    pub fn begin(&self) -> Result<DirectoryPickReceiver, String> {
        let (sender, receiver) = channel();
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| pick_directory_failed("the picker lock is poisoned"))?;
            if state.busy {
                return Err(PICK_DIRECTORY_BUSY.to_string());
            }
            state.busy = true;
            state.waiter = Some(sender);
        }
        let chooser = self.chooser.clone();
        let state = self.state.clone();
        let spawned = std::thread::Builder::new()
            .name("react-directory-chooser".into())
            .spawn(move || {
                let outcome = chooser();
                // Publish only; `busy` stays set until the tick drains it.
                if let Ok(mut state) = state.lock() {
                    state.terminal = Some(outcome);
                }
            });
        if let Err(error) = spawned {
            // The worker never started, so no terminal can arrive: release the
            // admission this call just took.
            if let Ok(mut state) = self.state.lock() {
                state.busy = false;
                state.waiter = None;
            }
            return Err(pick_directory_failed(&format!(
                "the chooser thread could not start: {error}"
            )));
        }
        Ok(receiver)
    }

    /// Drain a finished pick (owner tick). Returns whether an answer was handed
    /// over; a second call for the same pick returns `false` (capacity 1).
    pub fn drain(&self) -> bool {
        let (answer, waiter) = {
            let Ok(mut state) = self.state.lock() else {
                return false;
            };
            let Some(outcome) = state.terminal.take() else {
                return false;
            };
            state.busy = false;
            (answer_for(outcome), state.waiter.take())
        };
        match waiter {
            Some(waiter) => {
                // The admitted call may already be gone (the frontend closed):
                // a dropped receiver is not an error here.
                let _ = waiter.send(answer);
                true
            }
            None => false,
        }
    }

    /// The refusal a carrier without a picker answers with. Kept next to the
    /// picker so a host that installs none has one spelling for it.
    pub fn unsupported() -> String {
        UNSUPPORTED_PICK_DIRECTORY.to_string()
    }

    /// The answer when the admitted call's channel is gone before the tick ran.
    pub fn lost() -> String {
        PICK_DIRECTORY_LOST.to_string()
    }
}

/// Map one terminal outcome onto the command's answer.
///
/// The mapping is the whole "cancel is not a failure" contract, so it is one
/// function (and is unit-tested directly).
pub fn answer_for(outcome: DirectoryOutcome) -> DirectoryPickAnswer {
    match outcome {
        DirectoryOutcome::Path(path) => Ok(Some(path)),
        DirectoryOutcome::Cancelled => Ok(None),
        DirectoryOutcome::Failed(reason) => Err(pick_directory_failed(&reason)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Run `picker.drain()` until the worker's terminal lands (bounded), so a
    /// fast unit test cannot race the worker thread.
    fn drain_until_ready(picker: &DirectoryPicker) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if picker.drain() {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("the chooser worker never published a terminal");
    }

    fn picker(outcome: DirectoryOutcome) -> Arc<DirectoryPicker> {
        DirectoryPicker::new(Arc::new(move || outcome.clone()))
    }

    #[test]
    fn a_cancelled_dialog_answers_null_and_not_an_error() {
        let picker = picker(DirectoryOutcome::Cancelled);
        let receiver = picker.begin().expect("admitted");
        drain_until_ready(&picker);
        let answer = receiver.recv().expect("the tick delivered the answer");
        assert_eq!(answer, Ok(None), "cancel is a real `null`, not a failure");
    }

    #[test]
    fn a_chosen_directory_answers_its_absolute_path() {
        let picker = picker(DirectoryOutcome::Path("/opt/yacr/fonts".to_string()));
        let receiver = picker.begin().expect("admitted");
        drain_until_ready(&picker);
        assert_eq!(
            receiver.recv().expect("delivered"),
            Ok(Some("/opt/yacr/fonts".to_string()))
        );
    }

    #[test]
    fn a_portal_failure_is_an_explicit_error_never_a_null() {
        let picker = picker(DirectoryOutcome::Failed("portal unavailable".to_string()));
        let receiver = picker.begin().expect("admitted");
        drain_until_ready(&picker);
        let answer = receiver.recv().expect("delivered");
        let error = answer.expect_err("a failure is not `Ok`, so it cannot read as cancel");
        assert_eq!(error, "pick_directory.failed: portal unavailable");
        assert_ne!(error, UNSUPPORTED_PICK_DIRECTORY);
    }

    #[test]
    fn one_pick_at_a_time_and_the_slot_drains_once() {
        let picker = picker(DirectoryOutcome::Path("/tmp/x".to_string()));
        let receiver = picker.begin().expect("first admitted");
        let busy = picker.begin().expect_err("a second pick must be refused");
        assert_eq!(busy, PICK_DIRECTORY_BUSY);

        drain_until_ready(&picker);
        assert!(!picker.drain(), "the slot drains exactly once");
        assert_eq!(
            receiver.recv().expect("delivered"),
            Ok(Some("/tmp/x".into()))
        );

        // The admission is free again after the tick took the terminal.
        let next = picker.begin().expect("a new pick is admitted");
        drain_until_ready(&picker);
        assert_eq!(next.recv().expect("delivered"), Ok(Some("/tmp/x".into())));
    }

    #[test]
    fn a_worker_that_never_published_is_not_reported_as_cancel() {
        // The command maps a dropped channel to the lost marker, never `Ok(None)`.
        assert_eq!(
            DirectoryPicker::lost(),
            "pick_directory.lost: the chooser stopped without an answer"
        );
        assert!(!DirectoryPicker::lost().contains("cancelled"));
    }

    #[test]
    fn a_carrier_without_a_picker_refuses_by_marker() {
        assert_eq!(
            DirectoryPicker::unsupported(),
            "unsupported-runtime: pick_directory"
        );
    }
}
