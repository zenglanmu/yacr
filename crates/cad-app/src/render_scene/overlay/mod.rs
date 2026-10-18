//! Transient visual overlays: selection highlight and tool previews.
//!
//! This module produces the **derived** overlay batches a host draws on top of
//! the authoritative scene:
//!
//! * a **selection highlight** that resolves the exact `(entity, instance,
//!   sub-element)` refs in a [`SelectionSet`] against the database, expanding
//!   `INSERT`s exactly like [`crate::picking::drawing_pick_items`], so two
//!   placements of the same block highlight independently;
//! * a **tool preview** that turns an in-progress [`MeasurementPreview`] into
//!   line geometry (a rubber-band chain and point crosses).
//!
//! Nothing here mutates the database, the scene cache or history. Previews are
//! pure snapshots: a cancelled preview simply stops being passed in, and no
//! transaction is ever opened (see `cad-app::measure_tool`).
//!
//! ## Reuse, not a second renderer
//!
//! Highlight batches are built by [`cad_scene::highlight_batches`], the one
//! existing highlight→[`RenderBatch`] converter, from `DisplayRepresentation`s
//! the default provider tessellates (the same discretisation the base scene
//! uses). There is no second tessellator and no new renderer.
//!
//! ## Paint order
//!
//! | overlay | `draw_order` |
//! |---|---|
//! | base drawing (`SceneCache`) | `0` |
//! | selection highlight (`HIGHLIGHT_DRAW_ORDER`) | `900_000` |
//! | object-snap hints ([`SNAP_HINT_DRAW_ORDER`]) | `1_500_000` |
//! | tool preview ([`PREVIEW_DRAW_ORDER`]) | `2_000_000` |
//!
//! A selection is a transient decoration; a tool preview is the live feedback
//! the user is acting on, so it draws above everything. Object-snap hints sit
//! below the live preview rubber band.
//!
//! The implementation is split across the submodules of this directory; every
//! moved item is re-exported here under its original name, so the module path
//! `crate::render_scene::overlay::…` is unchanged.

use std::collections::HashSet;
use std::sync::Arc;

use cad_db::DrawingDatabase;
use cad_domain::{
    Completeness, Diagnostic, DocumentId, EntityId, Point3, SelectionRef, TaskStamp, Transform3,
};
use cad_measure::{SnapCandidate, SnapKind};
use cad_representation::{
    DisplayRepresentation, FontEngine, ProviderRegistry, RepresentationContext,
};
use cad_scene::{HighlightOptions, HighlightScene, RenderBatch, RenderTopology};

use crate::draw_tool::{DrawPreview, DrawToolKind};
use crate::measure_tool::MeasurementPreview;
use crate::selection::SelectionSet;
use crate::viewer_config::OverlayVisibility;

mod axes_grid;
mod fingerprint;
mod preview;
mod selection;
mod types;

#[cfg(test)]
mod tests;

pub use axes_grid::*;
pub use fingerprint::*;
pub use preview::*;
pub use selection::*;
pub use types::*;

use preview::line_batch;
use types::SNAP_HINT_CIRCLE_SEGMENTS;
