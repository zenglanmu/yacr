//! The capture grammar and point/pick entry points of [`DrawTool`].
//!
//! Split out of [`super`] to keep the state-machine module inside the code-size
//! limit (the DIMANGULAR mixed capture pushed it over). These are inherent
//! methods of the same type, so every public path is unchanged
//! (`cad_app::DrawTool::push_point`, `DrawTool::remaining`, …).
//!
//! [`CapturePlan`] is the single resolution of "what does the pending step accept,
//! and how many points/picks confirm it": every capture, readiness and intent
//! check reads it instead of re-deriving the two models, so they can never
//! disagree.

use super::*;

impl DrawTool {
    /// The capture arity actually in force (the parameter-selected target or the
    /// kind's static arity).
    pub(super) fn capture_arity(&self) -> PointArity {
        match self.point_target {
            Some(target) => PointArity::Exact(target),
            None => self.kind.arity(),
        }
    }

    /// The **live** capture plan: the pending step's accepted input and the
    /// complements the current form needs.
    ///
    /// Every step of every kind is described by [`PointArity`] plus
    /// [`DrawToolKind::picked_step_count`] except the one mixed capture
    /// ([`CaptureStep::PickOrPoint`], DIMANGULAR's "select a line **or** specify a
    /// vertex"): there the first capture picks the form, so the point and pick
    /// complements are state-dependent. This is the single place that resolves
    /// both, and every capture/readiness/intent check reads it, so the two models
    /// can never disagree.
    pub(super) fn capture_plan(&self) -> CapturePlan {
        if self.kind.has_mixed_first_step() && self.point_target.is_none() {
            return self.mixed_capture_plan();
        }
        let arity = self.capture_arity();
        CapturePlan {
            step: if self.points.len() < self.kind.picked_step_count() {
                CaptureStep::Pick
            } else {
                CaptureStep::Point
            },
            points: arity.minimum(),
            picks: self.kind.picked_step_count(),
            open_ended: arity.is_open_ended(),
        }
    }

    /// The DIMANGULAR plan: the first capture (a line pick or a vertex point)
    /// chooses one of its two forms.
    ///
    /// * **two-line** — two line picks, then the dimension-arc location (3
    ///   captures, 2 resolved refs). This is the pre-existing interactive form.
    /// * **three-point** — the vertex, the two side points, then the same location
    ///   (4 captures, no refs).
    ///
    /// Nothing is captured yet: the first step accepts either. A capture of the
    /// kind the chosen form does not take is refused **by name** in
    /// [`Self::push_point`] / [`Self::push_pick`]; the two forms are never
    /// reinterpreted into each other.
    fn mixed_capture_plan(&self) -> CapturePlan {
        let two_line = self.kind.arity().minimum();
        let three_point = self.kind.extended_point_count().unwrap_or(two_line);
        let (step, points, picks) = match self.angular_form() {
            AngularForm::Undecided => (CaptureStep::PickOrPoint, two_line, 0),
            // The two-line form: the second line while it is still missing, then
            // the free dimension-arc location.
            AngularForm::TwoLine if self.picks.len() < 2 => (CaptureStep::Pick, two_line, 2),
            AngularForm::TwoLine => (CaptureStep::Point, two_line, 2),
            AngularForm::ThreePoint => (CaptureStep::Point, three_point, 0),
        };
        CapturePlan {
            step,
            points,
            picks,
            open_ended: false,
        }
    }

    /// Which DIMANGULAR construction the captures so far select.
    ///
    /// The first capture decides: a resolved entity ref means a line was picked
    /// (the two-line form), and no ref means the click was a vertex (the
    /// three-point form). Both are counted in `points` — a pick stores its raw
    /// click there too — so `picks` is what distinguishes them.
    pub(super) fn angular_form(&self) -> AngularForm {
        match self.picks.len() {
            0 if self.points.is_empty() => AngularForm::Undecided,
            0 => AngularForm::ThreePoint,
            _ => AngularForm::TwoLine,
        }
    }

    /// The accepted input of the **pending** capture step.
    ///
    /// This is the one authority a host/UI needs to route a click: for DIMANGULAR
    /// the first step is [`CaptureStep::PickOrPoint`] ("select a line **or**
    /// specify a vertex"), and the capture's form is fixed by whichever the first
    /// capture was. Every other kind's pending step is [`CaptureStep::Pick`] while
    /// an entity-pick step is outstanding and [`CaptureStep::Point`] afterwards.
    pub fn pending_step(&self) -> CaptureStep {
        self.capture_plan().step
    }

    /// Whether the pending step accepts an **entity pick** (pick-only kinds, and
    /// the mixed DIMANGULAR step while nothing is captured yet).
    ///
    /// A host routes a click that resolves an entity through [`Self::push_pick`]
    /// when this is `true`; for the mixed step a click that resolves nothing is a
    /// free point instead ([`Self::requires_entity_pick`] is `false` there, so the
    /// point path is the fallback).
    pub fn accepts_entity_pick(&self) -> bool {
        self.pending_step().accepts_pick()
    }

    /// Whether the pending step accepts a **free point**.
    pub fn accepts_free_point(&self) -> bool {
        self.pending_step().accepts_point()
    }

    /// Whether the pending step is an entity pick **only** (TRIM/EXTEND,
    /// CHAMFER/FILLET, the leading arc/circle pick of
    /// DIMRADIUS/DIMDIAMETER/DIMCENTER, DIMBASELINE/DIMCONTINUE's base pick,
    /// BREAK/BREAKATPOINT's object pick, or TEXTEDIT's leading text pick).
    ///
    /// `false` for DIMANGULAR's mixed first step, which also accepts a free point.
    pub fn requires_entity_pick(&self) -> bool {
        self.pending_step().is_only_pick()
    }

    /// The named refusal for a capture the pending step does not take.
    ///
    /// Every kind keeps its own message; DIMANGULAR's two forms share one
    /// `dimangular.capture.mixed` reason, because that is exactly the mistake —
    /// mixing a line pick into the three-point form (or a point into the two-line
    /// form) is never reinterpreted as the other form.
    fn step_kind_refusal(&self, got: CaptureStep) -> String {
        if self.kind.has_mixed_first_step() {
            return format!(
                "dimangular.capture.mixed: DIMANGULAR takes either two line picks (then the \
                 dimension-arc location) or three points (vertex, two side points, then the \
                 location); a {} is not accepted here and is not reinterpreted as the other form",
                match got {
                    CaptureStep::Pick => "line pick",
                    _ => "free point",
                }
            );
        }
        match got {
            CaptureStep::Pick => format!(
                "draw kind '{}' does not take an entity pick for this step",
                self.kind.key()
            ),
            _ => format!(
                "draw kind '{}' captures entity picks, not free points",
                self.kind.key()
            ),
        }
    }

    /// Capture one free point, refusing non-finite input and overflow past the
    /// exact complement (or the open-ended hard cap) rather than silently
    /// resetting or dropping it.
    ///
    /// For a **pick-only** step this refuses: a blind point that cannot carry an
    /// entity must never be captured as one ([`Self::push_pick`] is the only path
    /// for those steps). Once every entity-pick step is resolved the tool is ready
    /// and waiting for confirmation, so a further click is refused with **that**
    /// reason instead of the misleading "captures entity picks, not free points"
    /// (which describes a step that is still un-captured). A kind whose leading
    /// steps are picks but whose later steps are free points (the radial pair)
    /// accepts the free point once the pick is behind it, and DIMANGULAR's mixed
    /// first step accepts one immediately — that is what selects its three-point
    /// form. The refusal is a machine reason, never a silent drop.
    pub fn push_point(&mut self, point: Point3) -> CadResult<()> {
        // A STRETCH polygon/lasso capture collects its crossing-window ring
        // **before** its fixed base/displacement run: every click there is another
        // boundary vertex (or, for a lasso, refused — a lasso is a drag), so the
        // ring never consumes a displacement slot.
        if self.window_open() {
            return self.push_window_vertex(point);
        }
        let plan = self.capture_plan();
        let captures_picks = self.kind.picked_step_count() > 0 || self.kind.has_mixed_first_step();
        // Only a kind that captures entity picks reports the "ready" reason: for a
        // free-point-only kind the honest refusal for a click past its complement
        // is the exact-arity one from [`Self::push_point_checked`] ("already has
        // its N points"), unchanged. Only a **fixed-arity** plan can be past its
        // complement: an open-ended kind with a leading pick
        // (DIMBASELINE/DIMCONTINUE) keeps accepting measured points after the
        // pick, up to the shared cap.
        if captures_picks && !plan.open_ended && self.points.len() >= plan.points {
            return Err(CadError::InvalidInput(format!(
                "draw kind '{}' is ready; press Enter to confirm",
                self.kind.key()
            )));
        }
        if !plan.step.accepts_point() {
            return Err(CadError::InvalidInput(
                self.step_kind_refusal(CaptureStep::Point),
            ));
        }
        self.push_point_checked(point, plan)
    }

    /// Capture one entity pick: the raw point and the ref resolved for it.
    ///
    /// This is the atomic capture for an entity-pick step — the point and the
    /// entity it was resolved against are stored together, so the two can never
    /// drift. A step that does not take an entity pick (every free-point kind, a
    /// radial capture whose pick is already behind it, or a DIMANGULAR capture that
    /// already committed to the three-point form) refuses it by name.
    pub fn push_pick(&mut self, point: Point3, reference: SelectionRef) -> CadResult<()> {
        let plan = self.capture_plan();
        if !plan.step.accepts_pick() {
            return Err(CadError::InvalidInput(
                self.step_kind_refusal(CaptureStep::Pick),
            ));
        }
        self.push_point_checked(point, plan)?;
        self.picks.push(reference);
        // The next step starts with no candidate; it is re-resolved on the next
        // cursor move rather than carried over from the previous step.
        self.candidate = None;
        Ok(())
    }

    /// Finite/overflow-checked point push shared by both capture paths.
    ///
    /// The **live plan** decides the bound, so DIMANGULAR's three-point form gets
    /// its fourth slot while the two-line form still refuses a fourth point. A
    /// fixed-arity plan refuses the `(n + 1)`-th point exactly as before; an
    /// open-ended kind accepts points up to [`MAX_OPEN_CAPTURE_POINTS`] and then
    /// refuses explicitly rather than growing without bound.
    fn push_point_checked(&mut self, point: Point3, plan: CapturePlan) -> CadResult<()> {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(CadError::InvalidInput("draw point must be finite".into()));
        }
        if plan.open_ended {
            if self.points.len() >= MAX_OPEN_CAPTURE_POINTS {
                return Err(CadError::InvalidInput(format!(
                    "draw kind '{}' has reached its maximum of {} points",
                    self.kind.key(),
                    MAX_OPEN_CAPTURE_POINTS
                )));
            }
        } else if self.points.len() >= plan.points {
            return Err(CadError::InvalidInput(format!(
                "draw kind '{}' already has its {} points",
                self.kind.key(),
                plan.points
            )));
        }
        self.points.push(point);
        Ok(())
    }
}

/// The **live** capture plan for one capture step.
///
/// Resolved once per check by [`DrawTool::capture_plan`] so the pending step's
/// accepted input and the complements stay a single truth. Every kind but
/// DIMANGULAR derives it from [`PointArity`] + [`DrawToolKind::picked_step_count`];
/// DIMANGULAR's mixed first step makes it state-dependent.
pub(super) struct CapturePlan {
    /// The input the pending step accepts.
    pub(super) step: CaptureStep,
    /// Points required to confirm the capture in its current form.
    pub(super) points: usize,
    /// Resolved entity refs required to confirm the capture in its current form.
    pub(super) picks: usize,
    /// Whether more points are accepted past `points` (the open-ended kinds).
    pub(super) open_ended: bool,
}

/// Which DIMANGULAR construction the captures so far select
/// ([`DrawToolKind::has_mixed_first_step`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AngularForm {
    /// Nothing captured: the first step accepts a line pick or a vertex point.
    Undecided,
    /// Two line picks, then the dimension-arc location.
    TwoLine,
    /// The vertex, the two side points, then the dimension-arc location.
    ThreePoint,
}
