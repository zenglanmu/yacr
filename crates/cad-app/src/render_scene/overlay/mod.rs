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
//! | world axes/grid ([`AXES_GRID_DRAW_ORDER`]) | `800_000` |
//! | POINT markers ([`POINT_MARKER_DRAW_ORDER`]) | `850_000` |
//! | selection highlight (`HIGHLIGHT_DRAW_ORDER`) | `900_000` |
//! | box-selection rubber band ([`BOX_SELECT_DRAW_ORDER`]) | `1_750_000` |
//! | tool preview ([`PREVIEW_DRAW_ORDER`]) | `2_000_000` |
//! | pointer cursor crosshair + pickbox ([`CURSOR_DRAW_ORDER`]) | `2_500_000` |
//! | object-snap hints ([`SNAP_HINT_DRAW_ORDER`]) | `3_000_000` |
//!
//! A selection is a transient decoration; a tool preview is the live feedback
//! the user is acting on, so it draws above everything it does not anchor.
//! Visible POINT markers are real drawing content, so they draw above the
//! reference axes/grid but below all transient chrome. The box-selection
//! rubber band sits below the live tool preview. The pointer cursor crosshair
//! and its pickbox are the pointer itself, so they draw above the preview — the
//! rubber band the user is dragging must never hide the pointer it is anchored
//! to. The object-snap hints draw **above the crosshair**: the snap marker is
//! what the user reads at the snap point, and when the pointer sits on the snap
//! target the crosshair's pickbox would otherwise be painted over it, hiding the
//! snap type exactly when it matters.
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
mod cursor;
mod fingerprint;
mod point_markers;
mod preview;
mod selection;
mod types;

#[cfg(test)]
mod tests;

pub use axes_grid::*;
pub use cursor::*;
pub use fingerprint::*;
pub use point_markers::*;
pub use preview::*;
pub use selection::*;
pub use types::*;

use preview::line_batch;
use types::SNAP_HINT_CIRCLE_SEGMENTS;
