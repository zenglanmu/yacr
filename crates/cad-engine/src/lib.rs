//! UI-independent CAD engine orchestration.
pub mod events;
pub mod protocol;
pub mod queue;
pub mod render;

/// Controlled API request/outcome types owned by `cad-app`; re-exported so
/// adapters can name them without depending on `cad-app` directly. The engine
/// never defines or re-implements these.
pub use cad_app::api;
