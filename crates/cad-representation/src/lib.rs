//! Disposable CPU display descriptions; no surfaces or GPU commands.
//!
//! Spec v2.0 §4.8: providers turn a read-only entity view plus resource/style
//! context into display primitives and a completeness report. They never touch
//! Slint, GPU objects or the event loop, and they never mutate the database.

use cad_db::{DbEntity, DrawingDatabase, EntityColor, EntityLineWeight, EntityTransparency};
use cad_domain::*;
use cad_geometry::{tessellate_geometry, TessellationParams};
use cad_kernel_adapter::{TessellationMesh, TessellationOutcome, TessellationResult};
use cad_resources::ResourceKey;
use std::sync::Arc;

/// Default display colour when a fragment carries no resolved source colour.
///
/// `[1, 1, 1]` is white, AutoCAD's nominal default entity colour (ACI 7), so the
/// fallback is a documented convention rather than an invented value.
pub const DEFAULT_RENDER_COLOR: [f32; 3] = [1.0, 1.0, 1.0];

/// Default lineweight in millimetres for an unresolved value or acadrust's
/// `LineWeight::Default`. AutoCAD's nominal default is 0.25 mm. The renderer
/// does **not** draw this width yet; it is carried so the gap is reported
/// (`docs/entity-style.md`).
pub const DEFAULT_LINEWEIGHT_MM: f32 = 0.25;

pub mod text;

pub mod shx;

pub mod layout;

pub use layout::{
    clip_polyline_to_rect, enumerate_layouts, paper_per_model_from_view, viewport_reason,
    viewport_transform, LayoutDescriptor, SpaceSelection, ViewportState, ViewportTransform,
    ViewportUnsupported,
};
pub use text::{
    parse_mtext, sanitize_text, text_issue, FontEngine, ParsedText, ShapedText, StackedFraction,
    TextColor, TextFormatIssue, TextLine, TextRun,
};
mod primitive;
mod provider;
mod representation;
mod transform;

pub use primitive::*;
pub use provider::*;
pub use representation::*;
pub use transform::*;

#[cfg(test)]
mod tests;
