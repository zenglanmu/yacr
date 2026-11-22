//! The [`cad_db::ByteSink`] adapter the zip writer streams into.
//!
//! The zip writer is generic over a `Write`; this adapter is that `Write`. It
//! forwards every chunk straight to the carrier's sink and enforces the
//! serialization budget per chunk, so the archive is never buffered whole.
//!
//! A failure is recorded in [`WriteMetrics`] (which outlives the borrow of the
//! sink) and the adapter is poisoned: later writes are accepted and discarded.
//! That lets the zip writer's `Drop` finalize without touching an aborted sink and
//! without printing to stderr, while the caller reads the real error back out.

use std::cell::RefCell;
use std::io::{self, Write};

use cad_db::{ByteSink, StoreError};
use cad_domain::ymodel::markers;

use crate::json::malformed;

/// Everything that must survive the borrow of the sink.
pub(crate) struct WriteMetrics {
    pub(crate) written: u64,
    pub(crate) failure: Option<StoreError>,
}

/// A zip sink that streams straight into a [`ByteSink`] and enforces the budget.
///
/// The budget check happens **before** the chunk reaches the sink, so the failure
/// is `PERSISTENCE_TOO_LARGE` rather than a partially written archive. After a
/// failure the writer goes quiet (every later `write` returns `Ok` and discards),
/// which lets the zip writer's `Drop` finalize without touching an aborted sink
/// and without printing to stderr.
pub(crate) struct SinkWriter<'a> {
    pub(crate) sink: &'a mut dyn ByteSink,
    pub(crate) metrics: &'a RefCell<WriteMetrics>,
    pub(crate) budget: Option<u64>,
    pub(crate) poisoned: bool,
}

impl SinkWriter<'_> {
    fn poison(&mut self, error: StoreError) {
        self.metrics.borrow_mut().failure = Some(error);
        self.poisoned = true;
    }
}

impl Write for SinkWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.poisoned {
            return Ok(buffer.len());
        }
        let written = self.metrics.borrow().written;
        if let Some(limit) = self.budget {
            if written.saturating_add(buffer.len() as u64) > limit {
                self.poison(StoreError::new(
                    markers::PERSISTENCE_TOO_LARGE,
                    format!("writing this document would exceed the {limit}-byte budget"),
                ));
                return Err(io::Error::other("ymodel serialization budget exceeded"));
            }
        }
        match self.sink.write_chunk(buffer) {
            Ok(()) => {
                self.metrics.borrow_mut().written = written + buffer.len() as u64;
                Ok(buffer.len())
            }
            Err(error) => {
                let detail = error.to_string();
                self.poison(error);
                Err(io::Error::other(detail))
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        // The sink has no flush: `commit` is the durability point (D16).
        Ok(())
    }
}

/// A zip failure: the carrier's own error if the sink refused a chunk, otherwise
/// a malformed-output diagnostic.
pub(crate) fn zip_error(
    metrics: &RefCell<WriteMetrics>,
    error: zip::result::ZipError,
    context: &str,
) -> StoreError {
    take_failure(metrics).unwrap_or_else(|| malformed(context, error))
}

pub(crate) fn io_error(
    metrics: &RefCell<WriteMetrics>,
    error: io::Error,
    context: &str,
) -> StoreError {
    take_failure(metrics).unwrap_or_else(|| malformed(context, error))
}

fn take_failure(metrics: &RefCell<WriteMetrics>) -> Option<StoreError> {
    metrics.borrow_mut().failure.take()
}
