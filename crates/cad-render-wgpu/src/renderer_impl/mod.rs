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
pub(crate) mod background_ink;
mod device;
mod encode;
mod fallback;
mod flags;
mod frame_pacing;
mod images;
mod init;
mod masks;
mod memory;
mod msaa;
mod msaa_geometry;
mod overlay;
mod overwrite;
pub(crate) mod pbr;
pub(crate) use pbr::PbrVertexLayout;
mod pipelines;
mod progressive;
mod pulled_plan;
mod pulled_rebuild;
mod render;
mod render_camera;
mod state;
mod texture_mip;
mod types;
mod upload;
mod upload_prepare;

#[cfg(test)]
mod cursor_transient_tests;
#[cfg(test)]
mod pick_table_incremental_tests;
#[cfg(test)]
mod pulled_budget_tests;
#[cfg(test)]
mod pulled_prefix_tests;

#[allow(unused_imports)]
pub(crate) use alloc::{
    allocation_guard, allocation_verdict, pop_captured_error, pulled_budget_for,
    pulled_capacity_fits, pulled_capacity_verdict, pulled_payload_bytes, pulled_skip_state,
    PulledAllocationError, PulledAllocationVerdict,
};
pub(crate) use background_ink::light_with_background_flag;
pub(crate) use encode::merged_texture_fallbacks;
// The CPU mip-chain builder (`texture_mip`) is shared with the GPU contract in
// `mesh_mip_gpu_tests`, so the test generates its levels with exactly the code
// the upload path runs.
pub(crate) use texture_mip::{
    generate_mip_levels, mip_level_count, mipped_rgba8_bytes, write_texture_level, MipRole,
};
pub(crate) use types::{prefix_outcome, CheckpointFacts, IncomingResources, PrefixOutcome};
