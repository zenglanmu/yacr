//! Representation providers and the provider registry.

mod clip;
mod default;
mod expand;
mod image;
mod registry;

pub use clip::{block_clip_rect, BlockClipRejection, WorldRect};
pub use default::DefaultRepresentationProvider;
pub use registry::{ProviderRegistry, RepresentationProvider};

#[allow(unused_imports)]
pub(crate) use clip::{rejection_code, rejection_message};
#[allow(unused_imports)]
pub(crate) use default::chord_tessellation_params;
