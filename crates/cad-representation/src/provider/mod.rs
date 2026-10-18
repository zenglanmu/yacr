//! Representation providers and the provider registry.

mod default;
mod expand;
mod image;
mod registry;

pub use default::DefaultRepresentationProvider;
pub use registry::{ProviderRegistry, RepresentationProvider};

#[allow(unused_imports)]
pub(crate) use default::chord_tessellation_params;
