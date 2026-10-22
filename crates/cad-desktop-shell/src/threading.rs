//! Main-thread marshalling shared by the desktop hosts.
//!
//! A UI toolkit owns exactly one main thread, and some platform state can only
//! be read there: the Linux CAD session is `!Send` and stored in a GTK
//! thread-local. The protocol for "run this task on the main thread and wait for
//! the value" is the same for every desktop host, so it lives here instead of in
//! each `apps/app-*`. Only the platform's answer to *where* that thread is stays
//! in the host.
//!
//! [`MainThread`] makes that answer explicit. A host supplies a real probe
//! (GTK: `gtk::is_initialized_main_thread`) and a dispatcher (Tauri:
//! `AppHandle::run_on_main_thread`). A mistaken `true` would run a `!Send` read
//! on a worker thread, so the probe must never be a guess, and a refused or
//! dropped dispatch is reported as an error rather than turned into an empty
//! success.

use std::sync::mpsc;

/// The platform's explicit main-thread seam.
///
/// Implemented by each concrete desktop host (on Linux, over GTK + Tauri); the
/// shared [`run_on_main_thread`] owns the channel protocol.
pub trait MainThread {
    /// Whether the calling thread is the UI toolkit's main thread.
    ///
    /// Must be a real platform probe, not an assumption: the answer decides
    /// whether a `!Send` task runs inline.
    fn is_main_thread(&self) -> bool;

    /// Schedule `task` on the UI toolkit's main thread.
    ///
    /// Called only when [`MainThread::is_main_thread`] returned `false`, so the
    /// implementation may assume it is being called from another thread.
    fn dispatch(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String>;
}

/// Run `task` on the host's main thread and wait for its result.
///
/// Runs inline when the probe says we are already there (the normal Tauri
/// command case), otherwise dispatches and blocks the caller until the main
/// thread has produced the value. Errors are propagated verbatim; a dispatch
/// failure and a dropped request are named `host.main_thread_failed: …` so the
/// caller can tell them from the task's own refusal.
pub fn run_on_main_thread<T, F>(thread: &dyn MainThread, task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    if thread.is_main_thread() {
        return task();
    }
    let (sender, receiver) = mpsc::channel();
    thread
        .dispatch(Box::new(move || {
            let _ = sender.send(task());
        }))
        .map_err(|error| format!("host.main_thread_failed: {error}"))?;
    receiver
        .recv()
        .map_err(|_| "host.main_thread_failed: the main thread dropped the request".to_string())?
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// A probe/dispatcher that runs dispatched tasks immediately on the calling
    /// thread, standing in for the toolkit main thread in a test.
    struct FakeMain {
        on_main: bool,
        dispatched: AtomicUsize,
    }

    impl FakeMain {
        fn new(on_main: bool) -> Self {
            Self {
                on_main,
                dispatched: AtomicUsize::new(0),
            }
        }
    }

    impl MainThread for FakeMain {
        fn is_main_thread(&self) -> bool {
            self.on_main
        }

        fn dispatch(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
            self.dispatched.fetch_add(1, Ordering::SeqCst);
            task();
            Ok(())
        }
    }

    #[test]
    fn an_on_main_call_runs_inline_without_dispatching() {
        let thread = FakeMain::new(true);
        let value = run_on_main_thread(&thread, || Ok(7)).expect("inline result");
        assert_eq!(value, 7);
        assert_eq!(
            thread.dispatched.load(Ordering::SeqCst),
            0,
            "an on-main call must not be re-dispatched"
        );
    }

    #[test]
    fn an_off_main_call_is_dispatched_and_returns_the_value() {
        let thread = FakeMain::new(false);
        let value = run_on_main_thread(&thread, || Ok(7)).expect("dispatched result");
        assert_eq!(value, 7);
        assert_eq!(thread.dispatched.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_task_error_is_propagated_verbatim() {
        let thread = FakeMain::new(false);
        let error = run_on_main_thread(&thread, || {
            Err::<(), String>("popup.no_text_selection: none".into())
        })
        .expect_err("the task failed");
        assert_eq!(error, "popup.no_text_selection: none");
    }

    #[test]
    fn a_refused_dispatch_is_an_error_not_an_empty_success() {
        struct Refusing;

        impl MainThread for Refusing {
            fn is_main_thread(&self) -> bool {
                false
            }

            fn dispatch(&self, _task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
                Err("no event loop".into())
            }
        }

        let error = run_on_main_thread(&Refusing, || Ok(1)).expect_err("dispatch failed");
        assert!(error.starts_with("host.main_thread_failed:"), "{error}");
        assert!(error.contains("no event loop"), "{error}");
    }

    #[test]
    fn a_dropped_request_is_an_error() {
        struct Dropping;

        impl MainThread for Dropping {
            fn is_main_thread(&self) -> bool {
                false
            }

            fn dispatch(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
                drop(task);
                Ok(())
            }
        }

        let error = run_on_main_thread(&Dropping, || Ok(1)).expect_err("dropped request");
        assert!(error.contains("dropped the request"), "{error}");
    }
}
