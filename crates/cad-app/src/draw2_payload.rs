//! Payloads for the 2026-11-07 2D draw/modify batch
//! (`DONUT`/`SOLID`/`HELIX`/`MLINE`/`MLSTYLE`/`TREX`/`EXTRIM`/`MPEDIT`/
//! `DRAWORDER`).
//!
//! Grouped behind one [`crate::CommandPayload::Draw2`] variant so the command
//! enum itself stays inside the file-size budget. Every payload here is plain
//! data: validation, refusals and the single transaction belong to the commands
//! in `app_drawing::draw2_batch` / `app_drawing::draw2_edit`.

use cad_domain::{Point3, SelectionRef};

// The real PEDIT types live in `command_payload`; imported here so the MPEDIT
// payload shares the exact option type PEDIT uses (including the `MPEDIT`
// `WIDTH`/`LTYPE GEN` forms that delegate to PEDIT).
use crate::PeditEditCommand;

/// One payload of the 2026-11-07 draw/modify batch.
#[derive(Debug, Clone, PartialEq)]
pub enum Draw2Payload {
    /// `DONUT`: one filled disk (`inner == 0`) or annulus of `inner` and `outer`
    /// radius, stored through the SOLID triangle-mesh path.
    Donut {
        center: Point3,
        inner: f64,
        outer: f64,
    },
    /// `SOLID`: 3 or 4 boundary points of one filled face (ring order).
    Solid { points: Vec<Point3> },
    /// `HELIX`: a flat logarithmic-ish spiral in the XY plane. `height == 0` is
    /// the only accepted height (a 3D spring is not implemented).
    Helix {
        base: Point3,
        radius: f64,
        turns: f64,
        height: f64,
    },
    /// `MLINE`: the baseline endpoints plus the style spacing / lateral offset.
    /// `spacing: None` means "use the current MLSTYLE spacing".
    Mline {
        spacing: Option<f64>,
        offset: f64,
        points: Vec<Point3>,
    },
    /// `MLSTYLE LIST` or `MLSTYLE SET <name> <spacing>`.
    MlStyle(MlStyleCommand),
    /// `TREX TRIM|EXTEND` over the current selection.
    Trex(TrexMode),
    /// `EXTRIM`: one boundary plus the crossing targets to trim.
    Extrim {
        boundary: SelectionRef,
        targets: Vec<SelectionRef>,
    },
    /// `MPEDIT`: one PEDIT option applied to every selected LWPOLYLINE.
    ///
    /// `CLOSE`/`OPEN`/`REVERSE` run the batch planner; every other PEDIT option
    /// (including `WIDTH <w>` and `LTYPE GEN`) goes through the same shared
    /// `PEDIT` executor and keeps its selection rules and named refusals.
    Mpedit(PeditEditCommand),
    /// `DRAWORDER`: the draw-order operation over the current selection.
    DrawOrder(DrawOrderMode),
}

/// The typed argument of `MLSTYLE`.
#[derive(Debug, Clone, PartialEq)]
pub enum MlStyleCommand {
    /// Report every stored style.
    List,
    /// Create, update and select one style. `spacing` is finite and `> 0`.
    Set { name: String, spacing: f64 },
}

/// The `TREX` operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrexMode {
    Trim,
    Extend,
}

/// The `DRAWORDER` operation.
///
/// `Above`/`Below` use the **first selected** object as the reference and move
/// the rest relative to it; the command refuses a selection without a reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawOrderMode {
    Front,
    Back,
    Above,
    Below,
}
