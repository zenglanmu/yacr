//! Implementation of [`Renderer`]: GPU upload, batching and frame submission.

use super::*;
use std::sync::atomic::Ordering;

// `std::time::Instant::now()` panics on wasm32-unknown-unknown ("time not
// implemented on this platform"); the browser gets `Performance.now()` through
// `web-time`. Native keeps the std type.
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

mod alloc;
mod device;
mod encode;
mod fallback;
mod flags;
mod frame_pacing;
mod images;
mod init;
mod masks;
mod pipelines;
mod pulled_plan;
mod pulled_rebuild;
mod render;
mod state;
mod types;
mod upload;

#[cfg(test)]
mod pick_table_incremental_tests;
#[cfg(test)]
mod pulled_budget_tests;
#[cfg(test)]
mod pulled_prefix_tests;

#[allow(unused_imports)]
pub(crate) use alloc::{
    allocation_guard, allocation_verdict, pop_captured_error, pulled_budget_for,
    pulled_capacity_fits, pulled_payload_bytes, pulled_skip_state, PulledAllocationError,
    PulledAllocationVerdict,
};
pub(crate) use encode::EncodeOutcome;
pub(crate) use types::{prefix_outcome, CheckpointFacts, IncomingResources, PrefixOutcome};
