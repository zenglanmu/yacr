//! Redraw pump: keeps the single Tauri loop awake so the native surface renders
//! continuously.
//!
//! Tauri does not expose `request_redraw`, and [`tauri::RunEvent::MainEventsCleared`]
//! only fires while the loop has work. On Android the loop blocks in the main
//! looper (`ControlFlow::Wait` → `ALooper_pollAll`), so with no input the CAD
//! surface would render only a handful of frames. This pump enqueues a no-op task
//! on the loop thread at a bounded rate; Tauri's `run_on_main_thread` sends through
//! the tao event-loop proxy, whose Android implementation wakes the looper
//! (`ForeignLooper::wake`), so the next iteration fires `MainEventsCleared` and the
//! tick runs again.
//!
//! It does **not** create a second event loop and performs no rendering itself: all
//! GPU and `CadSession` work still happens in the single Tauri loop, on the main
//! thread, in [`super::tick`]. (Same wake protocol as the Linux/macOS hosts: the
//! wake is Tauri-level, not toolkit-level.)
//!
//! **Single-pending**: the pump only sends a wake when no wake is already
//! outstanding (the tick clears the flag), so a slow loop thread cannot accumulate
//! an unbounded backlog of wake messages.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How often the pump considers waking the main loop.
///
/// 125 Hz; the tick still throttles to ~60 fps (the shared
/// `cad_ui_core::shell::FRAME_INTERVAL`), so this only prevents the loop from
/// going idle.
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
            .name("yacr-android-pump".into())
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
            log::error!("app-android: redraw pump failed to spawn: {error}");
        }
        Self { stop }
    }
}

impl Drop for RedrawPump {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}
