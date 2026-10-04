//! Bounded native background work: one running input, one latest pending input.
//! No live business state or platform/GPU handles cross this boundary.

use std::sync::{Arc, Condvar, Mutex};

struct Mailbox<I, O> {
    generation: u64,
    pending: Option<(u64, I)>,
    completed: Option<(u64, O)>,
    failed: Option<(u64, String)>,
    stopped: bool,
}

pub struct LatestTask<I, O> {
    shared: Arc<(Mutex<Mailbox<I, O>>, Condvar)>,
}

impl<I: Send + 'static, O: Send + 'static> LatestTask<I, O> {
    pub fn new(name: &str, mut work: impl FnMut(I) -> O + Send + 'static) -> std::io::Result<Self> {
        let shared = Arc::new((
            Mutex::new(Mailbox {
                generation: 0,
                pending: None,
                completed: None,
                failed: None,
                stopped: false,
            }),
            Condvar::new(),
        ));
        let worker = shared.clone();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || loop {
                let (generation, input) = {
                    let (lock, wake) = &*worker;
                    let mut state = lock.lock().unwrap();
                    while !state.stopped && state.pending.is_none() {
                        state = wake.wait(state).unwrap();
                    }
                    if state.stopped {
                        break;
                    }
                    state.pending.take().unwrap()
                };
                let output = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(input)));
                let (lock, _) = &*worker;
                let mut state = lock.lock().unwrap();
                if !state.stopped && state.generation == generation {
                    match output {
                        Ok(output) => state.completed = Some((generation, output)),
                        Err(_) => {
                            state.failed = Some((generation, "background task panicked".into()))
                        }
                    }
                }
                drop(state);
                // Stale heavy output is freed here, never on the UI thread.
            })?;
        Ok(Self { shared })
    }

    pub fn submit(&self, input: I) -> u64 {
        let (lock, wake) = &*self.shared;
        let mut state = lock.lock().unwrap();
        state.generation += 1;
        let generation = state.generation;
        state.pending = Some((generation, input));
        state.completed = None;
        state.failed = None;
        wake.notify_one();
        generation
    }

    pub fn take_completed(&self) -> Option<(u64, O)> {
        self.shared.0.lock().unwrap().completed.take()
    }

    pub fn take_failure(&self) -> Option<(u64, String)> {
        self.shared.0.lock().unwrap().failed.take()
    }

    pub fn cancel(&self) {
        let mut state = self.shared.0.lock().unwrap();
        state.generation += 1;
        state.pending = None;
        state.completed = None;
        state.failed = None;
    }
}

impl<I, O> Drop for LatestTask<I, O> {
    fn drop(&mut self) {
        let (lock, wake) = &*self.shared;
        let mut state = lock.lock().unwrap();
        state.stopped = true;
        state.pending = None;
        wake.notify_one();
        // Do not join a live parser/scene builder on the event loop. The worker
        // owns its snapshot and exits after its current task, discarding output.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn superseded_pending_inputs_are_bounded_and_only_latest_error_is_visible() {
        let (started_tx, started_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let task = LatestTask::new("latest-contract", move |input: u32| {
            started_tx.send(input).unwrap();
            if input == 1 {
                resume_rx.recv().unwrap();
            }
            Err::<(), _>(input)
        })
        .unwrap();
        task.submit(1);
        assert_eq!(started_rx.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
        task.submit(2);
        let latest = task.submit(3);
        resume_tx.send(()).unwrap();
        assert_eq!(started_rx.recv_timeout(Duration::from_secs(2)).unwrap(), 3);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = task.take_completed() {
                assert_eq!(result, (latest, Err(3)));
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }

    #[test]
    fn drop_does_not_join_running_work() {
        let (started_tx, started_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let task = LatestTask::new("drop-contract", move |_: ()| {
            started_tx.send(()).unwrap();
            resume_rx.recv().unwrap();
        })
        .unwrap();
        task.submit(());
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(task); // Would deadlock if Drop joined before the release below.
        resume_tx.send(()).unwrap();
    }

    #[test]
    fn worker_panic_is_reported_and_a_new_request_can_recover() {
        let task = LatestTask::new("panic-contract", |input: u32| {
            assert!(input != 1, "synthetic worker failure");
            input
        })
        .unwrap();
        let generation = task.submit(1);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some((failed, error)) = task.take_failure() {
                assert_eq!(failed, generation);
                assert!(error.contains("panicked"));
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        task.submit(2);
        loop {
            if let Some((_, output)) = task.take_completed() {
                assert_eq!(output, 2);
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
}
