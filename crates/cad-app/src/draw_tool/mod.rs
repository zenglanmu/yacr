//! Interactive drawing/editing tool: point capture, preview and commit intent.
//!
//! Spec v2.0 §4.3/§4.6/§4.7 and `docs/drawing-edit.md` §2/§4. This is the
//! drawing counterpart of [`crate::measure_tool`]:
//! a pure, host-agnostic state machine that captures exactly the points a given
//! edit needs, reports the remaining steps through a preview, and describes the
//! commit as a [`DrawIntent`].
//!
//! The module deliberately does **not** build a [`crate::Command`] itself. The
//! application commands (`CommandId::CreateLine`, `CreateCircle`,
//! `MoveEntities`, `TrimEntity`) are owned by the command layer (workstream B),
//! and `Move`/`Trim` additionally need the session's `SelectionRef`s, which a
//! pure point-capture tool must not invent. [`DrawIntent`] is the stable,
//! transaction-free description the UI/host turns into exactly one command; the
//! command layer maps it as documented in `docs/drawing-edit.md` §2.
//!
//! Cancelling never opens a transaction (the tool holds no database handle), so
//! a cancelled draw always leaves the drawing and history untouched. Confirming
//! yields exactly one intent, which the caller commits through one transaction.

use cad_domain::{CadError, CadResult, Point3, SelectionRef};

mod intent;
mod kind;
mod text;

pub use self::intent::DrawIntent;
pub use self::kind::{DrawToolKind, PointArity};

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
    /// The POLYGON side count for a live POLYGON capture, or `None` for every
    /// other kind.
    ///
    /// The count is the session sysvar (ADR 0015 decision C), injected by the
    /// session when it builds the preview so the outline tracks the *current*
    /// value even if it changes mid-capture. It is `None` whenever the active
    /// tool is not POLYGON, which is the same rule the frozen frontend
    /// `CadTool.params.sides` projection uses.
    pub polygon_sides: Option<u32>,
    /// Whether the capture is a text tool whose point complement is complete and
    /// whose typed text has not been submitted yet (ADR 0019 decision A.2).
    ///
    /// This is the **only** honest source for the frontend's text prompt phase: a
    /// UI must not infer it from `kind` + `points`, because `ready` alone cannot
    /// distinguish "missing points" from "missing text". While this is `true` no
    /// `Confirm` can complete the command — `SetToolText` is the terminal step.
    pub awaiting_text: bool,
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
        self.kind.arity()
    }

    /// Whether this capture is open-ended (accepts points past its minimum).
    pub fn is_open_ended(&self) -> bool {
        self.kind.arity().is_open_ended()
    }

    /// Honest projection of how many more points the capture requires.
    ///
    /// `Some(k)` means exactly `k` more points are needed to become confirmable
    /// (`Some(0)` = the exact complement is complete). `None` means the minimum
    /// is met on an **open-ended** tool: more points are still accepted up to
    /// the hard cap and the user finishes with Enter, so there is no fixed
    /// "remaining" count to report (never a fabricated `0`).
    pub fn remaining_points(&self) -> Option<usize> {
        let minimum = self.kind.required_points();
        if self.points.len() < minimum {
            Some(minimum - self.points.len())
        } else if self.kind.arity().is_open_ended() {
            None
        } else {
            Some(0)
        }
    }

    /// One-line, locale-facing capture step for the tool panel.
    ///
    /// Kept in `cad-app` so the UI layer never re-implements the state
    /// machine's counting rules. The entity-picking kinds count *resolved
    /// entities* ([`Self::picks`]) rather than raw points, so the step text
    /// cannot claim a step that an empty-space click never resolved. The
    /// open-ended kinds report the captured count and the minimum to confirm.
    ///
    /// NOTE (pre-existing i18n deviation): the branches below hardcode Chinese.
    /// The open-ended branch adds **locale-neutral ASCII** text instead of more
    /// hardcoded copy, so this change does not widen that deviation.
    pub fn status_line(&self) -> String {
        if self.kind.requires_text() {
            // TEXT (ADR 0019 decision A): locale-neutral ASCII only — the catalog
            // (`tool.prompt.text.*`, `status.tool_text.*`) is the chrome source,
            // and this frozen line must not add CJK literals on the Rust side.
            return if self.awaiting_text {
                format!(
                    "{}: insertion point captured ({}), type the text",
                    self.kind.key(),
                    self.points.len()
                )
            } else {
                format!("{}: specify the insertion point", self.kind.key())
            };
        }
        if self.kind.arity().is_open_ended() {
            let minimum = self.kind.required_points();
            if self.points.len() < minimum {
                return format!(
                    "{}: {} point(s) captured, at least {} needed",
                    self.kind.key(),
                    self.points.len(),
                    minimum
                );
            }
            return format!(
                "{}: {} point(s) captured, press Enter to finish",
                self.kind.key(),
                self.points.len()
            );
        }
        if self.kind.requires_selection() && self.selection_count == 0 {
            return format!("{}：请先选择对象", self.kind.label());
        }
        if self.kind == DrawToolKind::Trim {
            return match self.picks.len() {
                0 => "修剪：请选择要修剪的目标".to_string(),
                1 => "修剪：请选择边界".to_string(),
                _ => "修剪：已选目标与边界，可确认".to_string(),
            };
        }
        if self.kind == DrawToolKind::Extend {
            return match self.picks.len() {
                0 => "延伸：请选择要延伸的目标".to_string(),
                1 => "延伸：请选择边界".to_string(),
                _ => "延伸：已选目标与边界，可确认".to_string(),
            };
        }
        if self.remaining > 0 {
            format!(
                "{}：已选 {} 点，还需 {} 点",
                self.kind.label(),
                self.points.len(),
                self.remaining
            )
        } else {
            format!(
                "{}：已选 {} 点，可确认",
                self.kind.label(),
                self.points.len()
            )
        }
    }
}

/// Drawing/editing capture state machine.
#[derive(Debug, Clone)]
pub struct DrawTool {
    kind: DrawToolKind,
    points: Vec<Point3>,
    /// Entity refs resolved at capture time, one per captured step for the
    /// entity-picking kinds (TRIM/EXTEND); empty otherwise.
    picks: Vec<SelectionRef>,
    cursor: Option<Point3>,
    /// Entity under the current cursor for the pending pick step, or `None`.
    candidate: Option<SelectionRef>,
    selection_count: usize,
    /// The text submitted for a text tool (ADR 0019 decision A), or `None` while
    /// the tool still awaits it. Text is **terminal**: submitting it completes
    /// the command immediately, so this is only ever `Some` between the
    /// submission and the command's completion.
    text: Option<String>,
}

impl DrawTool {
    /// Start a tool for `kind` with the current selection count.
    ///
    /// A selection-requiring kind is still created; [`Self::is_ready`] refuses
    /// to confirm until `selection_count > 0`, so the refusal is visible in the
    /// step text rather than hidden.
    pub fn new(kind: DrawToolKind, selection_count: usize) -> Self {
        Self {
            kind,
            points: Vec::new(),
            picks: Vec::new(),
            cursor: None,
            candidate: None,
            selection_count,
            text: None,
        }
    }

    pub fn kind(&self) -> DrawToolKind {
        self.kind
    }

    pub fn points(&self) -> &[Point3] {
        &self.points
    }

    /// The entity refs resolved at capture time (empty for free-point kinds).
    pub fn picks(&self) -> &[SelectionRef] {
        &self.picks
    }

    pub fn cursor(&self) -> Option<Point3> {
        self.cursor
    }

    /// The entity under the current cursor for the pending pick step, if any.
    pub fn candidate(&self) -> Option<&SelectionRef> {
        self.candidate.as_ref()
    }

    pub fn selection_count(&self) -> usize {
        self.selection_count
    }

    /// Record the current selection count (MOVE needs a non-empty selection).
    pub fn set_selection_count(&mut self, count: usize) {
        self.selection_count = count;
    }

    /// Move the preview cursor without capturing a point.
    pub fn set_cursor(&mut self, cursor: Option<Point3>) {
        self.cursor = cursor;
    }

    /// Record the entity under the cursor for the pending pick step.
    ///
    /// Presentation feedback only: it never advances the capture and is cleared
    /// when the next step's pick is captured. Passing `None` is the honest "no
    /// entity under the cursor" state, never a fabricated ref.
    pub fn set_candidate(&mut self, candidate: Option<SelectionRef>) {
        self.candidate = candidate;
    }

    /// Whether the pending step is an entity pick (TRIM/EXTEND, CHAMFER/FILLET,
    /// or the leading arc/circle pick of DIMRADIUS/DIMDIAMETER/DIMCENTER).
    pub fn requires_entity_pick(&self) -> bool {
        self.points.len() < self.kind.picked_step_count()
    }

    /// Capture one free point, refusing non-finite input and overflow past the
    /// exact complement (or the open-ended hard cap) rather than silently
    /// resetting or dropping it.
    ///
    /// For a **pending entity-pick step** this refuses: a blind point that cannot
    /// carry an entity must never be captured as one ([`Self::push_pick`] is the
    /// only path for those steps). Once every entity-pick step is resolved the tool
    /// is ready and waiting for confirmation, so a further click is refused with
    /// **that** reason instead of the misleading "captures entity picks, not free
    /// points" (which describes a step that is still un-captured). A kind whose
    /// leading steps are picks but whose later steps are free points (the radial
    /// pair) accepts the free point once the pick is behind it. The refusal is a
    /// machine reason, never a silent drop.
    pub fn push_point(&mut self, point: Point3) -> CadResult<()> {
        if self.kind.picked_step_count() > 0 {
            if self.points.len() >= self.kind.required_points() {
                return Err(CadError::InvalidInput(format!(
                    "draw kind '{}' is ready; press Enter to confirm",
                    self.kind.key()
                )));
            }
            if self.points.len() < self.kind.picked_step_count() {
                return Err(CadError::InvalidInput(format!(
                    "draw kind '{}' captures entity picks, not free points",
                    self.kind.key()
                )));
            }
        }
        self.push_point_checked(point)
    }

    /// Capture one entity pick: the raw point and the ref resolved for it.
    ///
    /// This is the atomic capture for an entity-pick step — the point and the
    /// entity it was resolved against are stored together, so the two can never
    /// drift. A kind that does not take an entity pick for the **pending** step
    /// (every free-point kind, or a radial capture whose pick is already behind it)
    /// refuses it.
    pub fn push_pick(&mut self, point: Point3, reference: SelectionRef) -> CadResult<()> {
        if self.points.len() >= self.kind.picked_step_count() {
            return Err(CadError::InvalidInput(format!(
                "draw kind '{}' does not take an entity pick for this step",
                self.kind.key()
            )));
        }
        self.push_point_checked(point)?;
        self.picks.push(reference);
        // The next step starts with no candidate; it is re-resolved on the next
        // cursor move rather than carried over from the previous step.
        self.candidate = None;
        Ok(())
    }

    /// Finite/overflow-checked point push shared by both capture paths.
    ///
    /// A fixed-arity kind refuses the `(n + 1)`-th point exactly as before; an
    /// open-ended kind accepts points up to [`MAX_OPEN_CAPTURE_POINTS`] and then
    /// refuses explicitly rather than growing without bound.
    fn push_point_checked(&mut self, point: Point3) -> CadResult<()> {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(CadError::InvalidInput("draw point must be finite".into()));
        }
        match self.kind.arity() {
            PointArity::Exact(n) => {
                if self.points.len() >= n {
                    return Err(CadError::InvalidInput(format!(
                        "draw kind '{}' already has its {} points",
                        self.kind.key(),
                        n
                    )));
                }
            }
            PointArity::AtLeast(_) => {
                if self.points.len() >= MAX_OPEN_CAPTURE_POINTS {
                    return Err(CadError::InvalidInput(format!(
                        "draw kind '{}' has reached its maximum of {} points",
                        self.kind.key(),
                        MAX_OPEN_CAPTURE_POINTS
                    )));
                }
            }
        }
        self.points.push(point);
        Ok(())
    }

    /// How many more points the capture still requires, honestly per arity.
    ///
    /// `Some(k)`: exactly `k` more points are needed before the tool is
    /// confirmable (identical to the old fixed-arity complement for every fixed
    /// kind). `None`: the minimum is met on an **open-ended** tool — more points
    /// are still accepted and the capture is finished with Enter, so there is no
    /// fixed count to report (never a fabricated `0`).
    pub fn remaining(&self) -> Option<usize> {
        let minimum = self.kind.required_points();
        if self.points.len() < minimum {
            Some(minimum - self.points.len())
        } else if self.kind.arity().is_open_ended() {
            None
        } else {
            Some(0)
        }
    }

    /// Whether the captured parameters define the operation.
    ///
    /// The entity-picking kinds additionally require a resolved ref for **every**
    /// entity-pick step, so a radial capture with its pick missed is never
    /// confirmable even once the location point is captured.
    pub fn is_ready(&self) -> bool {
        self.points.len() >= self.kind.required_points()
            && (!self.kind.requires_selection() || self.selection_count > 0)
            && self.picks.len() >= self.kind.picked_step_count()
    }

    pub fn preview(&self) -> DrawPreview {
        DrawPreview {
            kind: self.kind,
            points: self.points.clone(),
            picks: self.picks.clone(),
            cursor: self.cursor,
            candidate: self.candidate.clone(),
            // The legacy numeric field is the minimum complement; the honest
            // open-ended projection lives in `DrawPreview::remaining_points`.
            remaining: self.remaining().unwrap_or(0),
            selection_count: self.selection_count,
            ready: self.is_ready(),
            // The pure tool never invents a sysvar: the session injects the
            // POLYGON side count when it builds the preview/tool snapshot.
            polygon_sides: None,
            awaiting_text: self.awaiting_text(),
        }
    }

    /// The exact commit this tool describes, or an explicit error when a
    /// parameter is still missing. Never fabricates a point or a default delta.
    pub fn intent(&self) -> CadResult<DrawIntent> {
        if self.kind.requires_selection() && self.selection_count == 0 {
            return Err(CadError::InvalidInput(format!(
                "draw kind '{}' needs a non-empty selection",
                self.kind.key()
            )));
        }
        if self.points.len() < self.kind.required_points() {
            return Err(CadError::InvalidInput(format!(
                "draw kind '{}' needs at least {} point(s), has {}",
                self.kind.key(),
                self.kind.required_points(),
                self.points.len()
            )));
        }
        if self.picks.len() < self.kind.picked_step_count() {
            return Err(CadError::InvalidInput(format!(
                "draw kind '{}' needs {} resolved entity pick(s), has {}",
                self.kind.key(),
                self.kind.picked_step_count(),
                self.picks.len()
            )));
        }
        // A text tool cannot produce an intent without its typed text: `Confirm`
        // must never complete it through the back door (ADR 0019 decision A.1).
        // The host refuses `Confirm` earlier with the prompt-specific reason; this
        // is the same rule at the single intent-construction point.
        if self.kind.requires_text() && self.text.is_none() {
            return Err(CadError::InvalidInput(format!(
                "tool.text_required: draw kind '{}' needs its text before it can be committed",
                self.kind.key()
            )));
        }
        // ARRAYRECT captures no points: its grid is defined entirely by the typed
        // session parameters plus the selection snapshot, so it is ready as soon as
        // the selection is non-empty. It must return **before** the `points[0]`
        // index below, which the zero-point arity model has nothing to supply.
        if self.kind == DrawToolKind::ArrayRect {
            return Ok(DrawIntent::ArrayRect);
        }
        // HATCH has the same zero-point pre-selection shape: its boundary is the
        // selection snapshot and its parameters are the typed channel, so it is
        // ready as soon as the selection is non-empty and never reads a point.
        if self.kind == DrawToolKind::Hatch {
            return Ok(DrawIntent::Hatch);
        }
        let a = self.points[0];
        // One-point tools (OFFSET, ARRAYPOLAR) never index a second point;
        // `unwrap_or(a)` is a harmless placeholder only ever used when a kind's arm
        // does not read `b`.
        let b = self.points.get(1).copied().unwrap_or(a);
        Ok(match self.kind {
            DrawToolKind::Line => DrawIntent::Line { start: a, end: b },
            DrawToolKind::Circle => DrawIntent::Circle { center: a, edge: b },
            DrawToolKind::Arc => DrawIntent::Arc {
                points: [a, b, self.points[2]],
            },
            // PLINE: every captured vertex, in order. The open-ended capture is
            // refused above while below the minimum, so `points` has at least
            // the two vertices the command layer requires.
            DrawToolKind::Polyline => DrawIntent::Polyline {
                points: self.points.clone(),
            },
            // RECTANG: the two captured corners exactly as clicked. The
            // geometry-side refusal (non-finite, degenerate, non-coplanar) and
            // the canonical vertex ordering live in the command layer, so the
            // edit tool's capture never invents or reorders a corner.
            DrawToolKind::Rectang => DrawIntent::Rect {
                corner1: a,
                corner2: b,
            },
            // ELLIPSE: the three captured picks exactly as clicked, in order. The
            // geometry-side construction, axis swap and degeneracy refusals live
            // in the command layer, so the tool never invents a centre or ratio.
            DrawToolKind::Ellipse => DrawIntent::Ellipse {
                p1: a,
                p2: b,
                p3: self.points[2],
            },
            // POLYGON: the centre and radius point exactly as clicked; the side
            // count is the session sysvar read by the command layer, so the
            // capture never invents a vertex count.
            DrawToolKind::Polygon => DrawIntent::Polygon {
                center: a,
                radius_point: b,
            },
            // POINT: the single captured click. A single point has no coplanarity
            // constraint, so the intent carries no plane.
            DrawToolKind::Point => DrawIntent::Point { position: a },
            // TEXT: the single captured insertion point plus the submitted text.
            // The height is the session typed channel read by the command layer,
            // and the text presence was enforced above.
            DrawToolKind::Text => DrawIntent::Text {
                position: a,
                text: self.text.clone().ok_or_else(|| {
                    CadError::InvalidInput(
                        "tool.text_required: TEXT needs its text before commit".into(),
                    )
                })?,
            },
            // SPLINE: every captured fit point, in order. The open-ended capture is
            // refused above while below the four-point minimum, so the command
            // layer always receives at least four fit points.
            DrawToolKind::Spline => DrawIntent::Spline {
                fit_points: self.points.clone(),
            },
            DrawToolKind::Move => DrawIntent::Move {
                delta: Point3 {
                    x: b.x - a.x,
                    y: b.y - a.y,
                    z: b.z - a.z,
                },
            },
            DrawToolKind::Copy => DrawIntent::Copy {
                delta: Point3 {
                    x: b.x - a.x,
                    y: b.y - a.y,
                    z: b.z - a.z,
                },
            },
            DrawToolKind::Trim => DrawIntent::Trim {
                target: self.picks[0].clone(),
                boundary: self.picks[1].clone(),
                pick_point: a,
            },
            DrawToolKind::Extend => DrawIntent::Extend {
                target: self.picks[0].clone(),
                boundary: self.picks[1].clone(),
                pick_point: a,
            },
            // ROTATE: the absolute direction angle from `base` to the second
            // point, measured about +Z from the +X axis. A zero-length direction
            // (or one differing only in Z) has no defined angle, so it is refused
            // rather than silently becoming a 0° no-op.
            DrawToolKind::Rotate => {
                if (b.x - a.x).hypot(b.y - a.y) <= MIN_DIRECTION_EXTENT {
                    return Err(CadError::InvalidInput(
                        "rotate direction point must differ from the base".into(),
                    ));
                }
                let angle = (b.y - a.y).atan2(b.x - a.x);
                if !angle.is_finite() {
                    return Err(CadError::InvalidInput(
                        "rotate direction must define a finite angle".into(),
                    ));
                }
                DrawIntent::Rotate { base: a, angle }
            }
            // SCALE: factor = |target − base| / |reference − base|; a degenerate
            // reference (== base) is an explicit refusal, never an infinite scale.
            DrawToolKind::Scale => {
                let reference = distance(a, b);
                if !reference.is_finite() || reference <= 0.0 {
                    return Err(CadError::InvalidInput(
                        "scale reference point must differ from the base".into(),
                    ));
                }
                let target = distance(a, self.points[2]);
                let factor = target / reference;
                if !factor.is_finite() || factor <= 0.0 {
                    return Err(CadError::InvalidInput(
                        "scale factor must be positive and finite".into(),
                    ));
                }
                DrawIntent::Scale { base: a, factor }
            }
            // MIRROR: the axis is the line through the two points; an axis of zero
            // length is refused rather than reflecting across a made-up line.
            DrawToolKind::Mirror => {
                if (b.x - a.x).hypot(b.y - a.y) <= MIN_DIRECTION_EXTENT {
                    return Err(CadError::InvalidInput(
                        "mirror axis needs two distinct points".into(),
                    ));
                }
                DrawIntent::Mirror { p1: a, p2: b }
            }
            // OFFSET: the single captured point is the side/distance point; the
            // source is the session selection, resolved by the command layer.
            DrawToolKind::Offset => DrawIntent::Offset { side_point: a },
            // CHAMFER/FILLET: the two resolved refs and the two raw pick points,
            // in pick order. The distances/radius are the session typed channel,
            // read by the command layer, so the intent stays the pure description
            // of what the user clicked.
            DrawToolKind::Chamfer => DrawIntent::Chamfer {
                first: self.picks[0].clone(),
                second: self.picks[1].clone(),
                pick_first: a,
                pick_second: b,
            },
            DrawToolKind::Fillet => DrawIntent::Fillet {
                first: self.picks[0].clone(),
                second: self.picks[1].clone(),
                pick_first: a,
                pick_second: b,
            },
            // ARRAYRECT: handled by the early return above (zero captured points).
            // The arm is unreachable here but keeps the match exhaustive over kinds.
            DrawToolKind::ArrayRect => DrawIntent::ArrayRect,
            // ARRAYPOLAR: the single captured point is the array centre; the item
            // count, fill angle and rotate-items flag are the session typed channel,
            // read by the command layer.
            DrawToolKind::ArrayPolar => DrawIntent::ArrayPolar { center: a },
            // ALIGN: the four captured points in AutoCAD's order (source1, dest1,
            // source2, dest2). The `AlignScale` flag is the session typed channel
            // read by the command layer, so the intent stays the pure description
            // of what the user clicked; the degeneracy refusals live in the command
            // layer too (the transform is built there).
            DrawToolKind::Align => DrawIntent::Align {
                source1: self.points[0],
                dest1: self.points[1],
                source2: self.points[2],
                dest2: self.points[3],
            },
            // STRETCH: the two window corners, the base point and the displacement
            // point, exactly as captured (the enclosure classification and the
            // effective translation live in the command layer, so the intent stays
            // the pure description of what the user clicked).
            DrawToolKind::Stretch => DrawIntent::Stretch {
                rect: [self.points[0], self.points[1]],
                base: self.points[2],
                delta: self.points[3],
            },
            // DIMLINEAR/DIMALIGNED: the two origins and the dimension-line
            // location exactly as clicked, in order. The horizontal/vertical
            // decision (DIMLINEAR), the definition construction and every refusal
            // live in the command layer, so the capture never invents a direction
            // or a measurement.
            DrawToolKind::DimLinear => DrawIntent::DimLinear {
                first: a,
                second: b,
                location: self.points[2],
            },
            DrawToolKind::DimAligned => DrawIntent::DimAligned {
                first: a,
                second: b,
                location: self.points[2],
            },
            // DIMRADIUS/DIMDIAMETER: the ref resolved at capture time for the
            // arc/circle pick plus the free location point. The definition
            // construction (group 10 centre vs opposite chord point), the
            // measurement and every refusal live in the command layer, so the
            // capture never invents a radius or a direction.
            DrawToolKind::DimRadius => DrawIntent::DimRadius {
                target: self.picks[0].clone(),
                location: b,
            },
            DrawToolKind::DimDiameter => DrawIntent::DimDiameter {
                target: self.picks[0].clone(),
                location: b,
            },
            // DIMCENTER: the ref resolved at capture time; the command layer reads
            // the resolved `DIMCEN` and produces plain geometry (or nothing).
            DrawToolKind::DimCenter => DrawIntent::DimCenter {
                target: self.picks[0].clone(),
            },
            // DIMANGULAR: the two refs resolved at capture time plus the free
            // dimension-arc location. The intersection, the angle selection and
            // every refusal live in the command layer.
            DrawToolKind::DimAngular => DrawIntent::DimAngular {
                first: self.picks[0].clone(),
                second: self.picks[1].clone(),
                location: self.points[2],
            },
            // DIMORDINATE: the feature point and the leader endpoint exactly as
            // clicked; the axis is derived by the command layer.
            DrawToolKind::DimOrdinate => DrawIntent::DimOrdinate {
                feature: a,
                leader: b,
            },
            // HATCH: handled by the early return above (zero captured points). The
            // arm is unreachable here but keeps the match exhaustive over kinds.
            DrawToolKind::Hatch => DrawIntent::Hatch,
        })
    }
}

/// Minimum XY extent for a gesture that defines a direction (rotate) or an axis
/// (mirror): a shorter span has no well-defined angle or line and is refused.
const MIN_DIRECTION_EXTENT: f64 = 1e-12;

/// Hard cap on an open-ended capture (PLINE), in points.
///
/// An interactive polyline is digitised a handful of vertices at a time; 1024 is
/// far beyond any hand-placed screen/plan run (and ≈32 KiB of `Point3`, a small
/// bounded footprint), while the bound stops a stuck pointer or a synthetic
/// click loop from growing the capture without limit. Reaching it is an explicit
/// refusal, never a silent drop of the extra click.
pub const MAX_OPEN_CAPTURE_POINTS: usize = 1024;

/// Hard cap on one submitted tool text, in **characters** (ADR 0019 decision
/// A.1: "长度上限 1024 字符（有限且声明）").
///
/// The unit is characters, not bytes, so the bound is language-independent: a
/// CJK string of 1024 characters is accepted at the same bound as an ASCII one.
/// Exceeding it is an explicit `tool.text_too_long` refusal, never a silent
/// truncation.
pub const MAX_TOOL_TEXT_CHARS: usize = 1024;

/// Euclidean distance between two points.
fn distance(a: Point3, b: Point3) -> f64 {
    (a.x - b.x).hypot(a.y - b.y).hypot(a.z - b.z)
}
mod geometry;
mod parameter;
// The general typed tool-parameter model (ADR 0016 decision A); re-exported so
// `crate::draw_tool::<name>` and the public `cad_app::<name>` paths resolve.
pub use parameter::{
    ToolParameter, ToolParameterStore, ToolValue, ToolValueKind, HATCH_PATTERN_TYPE_SOLID,
    HATCH_PATTERN_TYPE_USER_DEFINED, HATCH_STYLE_IGNORE, HATCH_STYLE_NORMAL, HATCH_STYLE_OUTER,
    TEXT_DEFAULT_HEIGHT,
};
// Re-exported so the historical `crate::draw_tool::<name>` paths and the public
// `cad_app::circle_radius` (via `lib.rs`) stay valid for the callers that used the
// pre-split module layout.
pub use geometry::{
    arc_parameters, circle_radius, ellipse_parameters, polygon_vertices, spline_control_points,
    ArcParameters, EllipseParameters, DEFAULT_POLYGON_SIDES, MAX_POLYGON_SIDES, MIN_POLYGON_SIDES,
};
pub(crate) use geometry::{mirror_transform, rotation_transform, scale_transform};

#[cfg(test)]
mod tests;
