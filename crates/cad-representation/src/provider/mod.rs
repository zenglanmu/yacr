//! Representation providers and the provider registry.

mod clip;
mod default;
mod display_modes;
mod expand;
mod image;
mod registry;
mod stroke;

pub use clip::{block_clip_rect, BlockClipRejection, WorldRect};
pub use default::DefaultRepresentationProvider;
pub use display_modes::{viewres_segment_ceiling, VIEWRES_MAX, VIEWRES_MIN, VIEWRES_MIN_SEGMENTS};
pub use registry::{ProviderRegistry, RepresentationProvider};

#[allow(unused_imports)]
pub(crate) use clip::{rejection_code, rejection_message};
#[allow(unused_imports)]
pub(crate) use default::chord_tessellation_params;
#[allow(unused_imports)]
pub(crate) use display_modes::{apply_qtext, hatch_outline_geometry, solid_outline_primitive};
