//! Redraw pump: wakes the single Tauri event loop so the host renders
//! continuously.
//!
//! Tauri does not expose `request_redraw`, and `RunEvent::MainEventsCleared`
//! only fires when the loop has work; with no input the surface would render only
//! a handful of frames. This pump enqueues a no-op task on the main thread at a
//! bounded rate, which wakes the loop and makes `MainEventsCleared` fire again.
//!
//! It does **not** create a second event loop and performs no rendering itself:
//! all GPU work still happens in the single Tauri loop, on the main thread, in
//! [`crate::host::tick`]. (This is the same pump as the Linux host: the wake
//! protocol is Tauri-level, not toolkit-level.)
//!
//! **Single-pending**: the pump only sends a wake when no wake is already
//! outstanding (the tick clears the flag), so a slow main thread cannot
//! accumulate an unbounded backlog of wake messages.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How often the pump considers waking the main loop (125 Hz; the tick still
/// throttles to ~60 fps, so this only prevents the loop from going idle).
const PUMP_INTERVAL: Duration = Duration::from_millis(8);

/// Owns the pump thread's stop flag; dropping it stops the pump.
pub struct RedrawPump {
    stop: Arc<AtomicBool>,
}

impl RedrawPump {
    /// Start the pump for `app`, sharing the single-pending flag with the tick.
    pub fn spawn(app: &tauri::AppHandle, wake_pending: Arc<AtomicBool>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let handle = app.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("probe-redraw-pump".into())
            .spawn(move || {
                while !thread_stop.load(Ordering::SeqCst) {
                    // Only one outstanding wake at a time.
                    if !wake_pending.swap(true, Ordering::SeqCst)
                        && handle.run_on_main_thread(|| {}).is_err()
                    {
                        break;
                    }
                    std::thread::sleep(PUMP_INTERVAL);
                }
            })
        {
            eprintln!("[probe] redraw pump failed to spawn: {error}");
        }
        Self { stop }
    }
}

impl Drop for RedrawPump {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}
