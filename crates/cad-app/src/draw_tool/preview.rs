//! The UI-facing snapshot of an in-progress draw capture.
//!
//! Split out of `draw_tool/mod.rs` to keep that module inside the code-size limit
//! (the new STRETCH crossing-window fields pushed it over); the type is re-exported
//! by the parent under its original name, so every existing path and every wire
//! projection is unchanged.

use cad_domain::{Point3, SelectionRef};

use super::{DrawToolKind, PointArity};

/// Snapshot of an in-progress draw for the UI preview layer.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawPreview {
    pub kind: DrawToolKind,
    /// Captured points in order.
    pub points: Vec<Point3>,
    /// Entities resolved at capture time, in step order.
    ///
    /// Empty for the free-point kinds. For the entity-picking kinds
    /// ([`DrawToolKind::needs_picked_entities`]) this is one parallel entry per
    /// captured step, so readiness is honest: a TRIM/EXTEND with two raw points
    /// but fewer than two resolved entities is **not** confirmable.
    pub picks: Vec<SelectionRef>,
    /// Live cursor position, if any; never committed until confirmed.
    pub cursor: Option<Point3>,
    /// The entity under the **current cursor** for the pending capture step, or
    /// `None`.
    ///
    /// Resolved through the exact pick path a click uses, only while a step is
    /// pending, and never fabricated: it is presentation feedback for the
    /// follow-up highlight wiring, not a commit source.
    pub candidate: Option<SelectionRef>,
    /// Captured points still needed before the operation is defined.
    ///
    /// This is the *minimum* complement ([`PointArity::minimum`] minus the
    /// captured count), so it is `0` once the operation can be confirmed. For an
    /// **open-ended** tool a further point is still accepted while this is `0`;
    /// the honest projection is [`Self::remaining_points`] (which returns `None`
    /// there) — this raw count is retained as the legacy numeric field the
    /// `cad-host`/`cad-engine` wire shape reads.
    pub remaining: usize,
    /// Number of selected entities the tool was started with (MOVE needs > 0).
    pub selection_count: usize,
    /// Whether the operation can be committed right now.
    pub ready: bool,
    /// The capture's point target when it is **not** the kind's static arity, or
    /// `None`.
    ///
    /// The interactive shape of ALIGN/BREAK/LENGTHEN depends on a session tool
    /// parameter (`alignPairs`, `breakFirstPoint`, `lengthenMode = 3`), so the
    /// honest complement is the live plan rather than [`DrawToolKind::arity`].
    /// `None` means the kind's own arity applies.
    pub point_target: Option<usize>,
    /// The POLYGON side count for a live POLYGON capture, or `None` for every
    /// other kind.
    ///
    /// The count is the session sysvar (ADR 0015 decision C), injected by the
    /// session when it builds the preview so the outline tracks the *current*
    /// value even if it changes mid-capture. It is `None` whenever the active
    /// tool is not POLYGON, which is the same rule the frozen frontend
    /// `CadTool.params.sides` projection uses.
    pub polygon_sides: Option<u32>,
    /// Whether the capture is still collecting its **free-form crossing window
    /// ring** (STRETCH's polygon/lasso mode) rather than its fixed point run.
    ///
    /// While this is `true` every click is another boundary vertex and the ring
    /// is closed by `CONFIRM` (or by a click on its first vertex), so the capture
    /// is open-ended in the honest sense: more points are accepted and there is no
    /// fixed "remaining" count. `false` for the rectangle window and for every
    /// other tool.
    pub window_open: bool,
    /// Whether the capture is a text tool whose point complement is complete and
    /// whose typed text has not been submitted yet (ADR 0019 decision A.2).
    ///
    /// This is the **only** honest source for the frontend's text prompt phase: a
    /// UI must not infer it from `kind` + `points`, because `ready` alone cannot
    /// distinguish "missing points" from "missing text". While this is `true` no
    /// `Confirm` can complete the command — `SetToolText` is the terminal step.
    pub awaiting_text: bool,
    /// The **round-trippable current string** of the entity a text tool will
    /// edit, or `None` (ADR 0020 amendment A).
    ///
    /// Only `TEXTEDIT` fills this, and only once its entity pick resolved to a
    /// single `Text`: it is that entity's stored string, sent so the UI can show
    /// it for editing instead of making the user retype it. A fresh `TEXT` never
    /// has a prefill (there is nothing to edit yet), and no non-text tool
    /// projects one. It is **not** a display row: it is the exact value
    /// `TEXTEDIT` will replace.
    pub text_prefill: Option<String>,
}

impl DrawPreview {
    /// Whether the captured parameters can be committed right now.
    ///
    /// A text tool whose points are complete but whose text has not been
    /// submitted is **not** confirmable: `SetToolText` is the terminal step
    /// (ADR 0019 decision A.1), so reporting `true` here would claim a `Confirm`
    /// that the host refuses.
    pub fn can_confirm(&self) -> bool {
        self.ready && !self.awaiting_text
    }

    /// The capture arity of the tool this preview describes.
    pub fn arity(&self) -> PointArity {
        match self.point_target {
            Some(target) => PointArity::Exact(target),
            None => self.kind.arity(),
        }
    }

    /// Whether this capture is open-ended (accepts points past its minimum).
    ///
    /// An open **STRETCH polygon/lasso ring** is open-ended in exactly this
    /// sense — every click is another boundary vertex and Enter closes the ring —
    /// even though the displacement run that follows it is a fixed two points.
    pub fn is_open_ended(&self) -> bool {
        self.window_open || self.arity().is_open_ended()
    }

    /// Honest projection of how many more points the capture requires.
    ///
    /// `Some(k)` means exactly `k` more points are needed to become confirmable
    /// (`Some(0)` = the exact complement is complete). `None` means the minimum
    /// is met on an **open-ended** tool: more points are still accepted up to
    /// the hard cap and the user finishes with Enter, so there is no fixed
    /// "remaining" count to report (never a fabricated `0`).
    ///
    /// While a STRETCH polygon/lasso **ring** is open this is `None` for the same
    /// reason (the vertex run has no fixed complement); once the ring is closed
    /// the two displacement points are the fixed run this reports.
    pub fn remaining_points(&self) -> Option<usize> {
        if self.window_open {
            return None;
        }
        let minimum = self.arity().minimum();
        if self.points.len() < minimum {
            Some(minimum - self.points.len())
        } else if self.arity().is_open_ended() {
            None
        } else {
            Some(0)
        }
    }
}
