//! Step-and-time pump budget and monotonic pump clock for the owner thread.

use super::{PUMP_MILLIS, PUMP_STEPS};

/// Step-and-time pump budget. Time is supplied by the caller so this helper
/// stays usable on wasm32 (where `Instant` is unavailable) and deterministic in
/// tests (no sleeping).
#[derive(Debug, Clone, Copy)]
pub struct PumpBudget {
    max_steps: usize,
    max_millis: f64,
    steps: usize,
}

impl Default for PumpBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl PumpBudget {
    /// The contract default: 64 steps or 4 ms.
    pub fn new() -> Self {
        Self {
            max_steps: PUMP_STEPS,
            max_millis: PUMP_MILLIS,
            steps: 0,
        }
    }

    /// Explicit step/time budget.
    pub fn with_limits(max_steps: usize, max_millis: f64) -> Self {
        Self {
            max_steps,
            max_millis,
            steps: 0,
        }
    }

    /// Whether one more step may run, given `elapsed_millis` since pump start.
    pub fn allow(&self, elapsed_millis: f64) -> bool {
        self.steps < self.max_steps && elapsed_millis < self.max_millis
    }

    /// Record one executed step.
    pub fn record(&mut self) {
        self.steps = self.steps.saturating_add(1);
    }

    /// Steps recorded so far.
    pub fn steps(&self) -> usize {
        self.steps
    }

    /// Whether the budget is exhausted for the given elapsed time.
    pub fn is_exhausted(&self, elapsed_millis: f64) -> bool {
        !self.allow(elapsed_millis)
    }
}

/// Monotonic pump clock for native hosts. Not compiled on wasm32, where
/// `Instant::now` is unavailable; wasm hosts supply elapsed from their own
/// platform clock.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct PumpClock {
    start: std::time::Instant,
}

#[cfg(not(target_arch = "wasm32"))]
impl PumpClock {
    pub fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }

    /// Milliseconds since this clock was created.
    pub fn elapsed_millis(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for PumpClock {
    fn default() -> Self {
        Self::new()
    }
}
