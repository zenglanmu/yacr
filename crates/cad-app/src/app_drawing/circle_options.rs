//! `CIRCLE 2P` / `CIRCLE 3P` / `CIRCLE D`: the three CIRCLE construction options.
//!
//! Each command builds exactly one `SemanticGeometry::Circle` on the work-plane
//! `+Z` normal and commits it through the shared
//! [`Application::commit_new_entity`] funnel, so every option is one entity, one
//! transaction and one undo step on the session's active layer in model space.
//! The payload is always the **construction input** — the clicked points — never a
//! pre-derived centre/radius, so the geometry maths lives in exactly one place
//! here and the capture layer cannot invent a different answer.
//!
//! * `CIRCLE 2P` — two diameter endpoints: centre = midpoint, radius = half the
//!   distance between them.
//! * `CIRCLE 3P` — three points on the circle: the circumcircle comes from
//!   [`crate::draw_tool::arc_parameters`], the **same** helper `ARC` uses, so the
//!   two commands cannot disagree about the centre or radius for one triple. The
//!   arc's start angle and sweep are ignored.
//! * `CIRCLE D` — a centre plus a diameter point: radius = **half** the
//!   centre→pick distance (AutoCAD's Diameter option). This is the one
//!   difference from `CIRCLE`, whose radius is the full centre→edge distance.
//!
//! Every refusal is named, happens before any transaction is opened, and changes
//! nothing: `circle2p.degenerate` / `circle3p.degenerate` /
//! `circlediameter.degenerate` for a non-finite, coincident or collinear input.
//! The missing active layer is refused by the shared funnel, as for every other
//! create command.

use super::*;

impl Application {
    /// `CIRCLE 2P`: a circle whose **diameter endpoints** are the two picks.
    ///
    /// Payload is `Points([first, second])`. The stored centre is the midpoint and
    /// the radius is half the pick distance; a non-finite or coincident pair is an
    /// explicit `circle2p.degenerate` refusal that opens no transaction.
    pub(crate) fn create_circle_2p(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "CreateCircle2P needs a Points([first, second]) payload".into(),
            ));
        };
        let first = *points.first().ok_or_else(|| {
            CadError::InvalidInput("CreateCircle2P needs a first diameter endpoint".into())
        })?;
        let second = *points.get(1).ok_or_else(|| {
            CadError::InvalidInput("CreateCircle2P needs a second diameter endpoint".into())
        })?;
        if !is_finite_point(first) || !is_finite_point(second) {
            return Err(CadError::InvalidInput(
                "circle2p.degenerate: both diameter endpoints must be finite".into(),
            ));
        }
        let diameter = distance(first, second);
        if !diameter.is_finite() || diameter <= 0.0 {
            return Err(CadError::InvalidInput(
                "circle2p.degenerate: the two diameter endpoints must be distinct".into(),
            ));
        }
        let center = Point3 {
            x: (first.x + second.x) / 2.0,
            y: (first.y + second.y) / 2.0,
            z: (first.z + second.z) / 2.0,
        };
        let radius = diameter / 2.0;
        let geometry = circle_geometry(center, radius);
        self.commit_circle(
            session,
            command,
            geometry,
            "create circle 2p",
            "drawing.circle2p",
            "CIRCLE 2P",
            center,
            radius,
        )
    }

    /// `CIRCLE 3P`: the circumcircle through three picked points.
    ///
    /// Payload is `Points([first, second, third])`. The centre and radius are the
    /// ones [`crate::draw_tool::arc_parameters`] already computes for `ARC` (the
    /// single circumcircle source), so a `CIRCLE 3P` and an `ARC` through the same
    /// three points share a centre and radius. A non-finite, coincident or
    /// collinear triple is `circle3p.degenerate` and changes nothing.
    pub(crate) fn create_circle_3p(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "CreateCircle3P needs a Points([first, second, third]) payload".into(),
            ));
        };
        let first = *points
            .first()
            .ok_or_else(|| CadError::InvalidInput("CreateCircle3P needs a first point".into()))?;
        let second = *points
            .get(1)
            .ok_or_else(|| CadError::InvalidInput("CreateCircle3P needs a second point".into()))?;
        let third = *points
            .get(2)
            .ok_or_else(|| CadError::InvalidInput("CreateCircle3P needs a third point".into()))?;
        if !is_finite_point(first) || !is_finite_point(second) || !is_finite_point(third) {
            return Err(CadError::InvalidInput(
                "circle3p.degenerate: all three points must be finite".into(),
            ));
        }
        // One circumcircle implementation for ARC and CIRCLE 3P; the shared
        // collinear/coincident/non-coplanar refusals are re-labelled with this
        // command's stable token while keeping the helper's own sentence.
        let parameters = crate::draw_tool::arc_parameters(first, second, third)
            .map_err(|error| CadError::InvalidInput(format!("circle3p.degenerate: {error}")))?;
        let center = parameters.center;
        let radius = parameters.radius;
        let geometry = circle_geometry(center, radius);
        self.commit_circle(
            session,
            command,
            geometry,
            "create circle 3p",
            "drawing.circle3p",
            "CIRCLE 3P",
            center,
            radius,
        )
    }

    /// `CIRCLE D`: a circle from a centre and a **diameter** point.
    ///
    /// Payload is `Points([center, diameter_point])`. The first pick is the centre;
    /// the second lies on the far side of the circle, so the stored radius is
    /// **half** the picked distance (AutoCAD's Diameter option). A non-finite or
    /// coincident pair is `circlediameter.degenerate` and changes nothing.
    pub(crate) fn create_circle_diameter(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "CreateCircleDiameter needs a Points([center, diameter]) payload".into(),
            ));
        };
        let center = *points.first().ok_or_else(|| {
            CadError::InvalidInput("CreateCircleDiameter needs a center point".into())
        })?;
        let diameter_point = *points.get(1).ok_or_else(|| {
            CadError::InvalidInput("CreateCircleDiameter needs a diameter point".into())
        })?;
        if !is_finite_point(center) || !is_finite_point(diameter_point) {
            return Err(CadError::InvalidInput(
                "circlediameter.degenerate: the center and the diameter point must be finite"
                    .into(),
            ));
        }
        let diameter = distance(center, diameter_point);
        if !diameter.is_finite() || diameter <= 0.0 {
            return Err(CadError::InvalidInput(
                "circlediameter.degenerate: the center and the diameter point must be distinct"
                    .into(),
            ));
        }
        let radius = diameter / 2.0;
        let geometry = circle_geometry(center, radius);
        self.commit_circle(
            session,
            command,
            geometry,
            "create circle diameter",
            "drawing.circlediameter",
            "CIRCLE D",
            center,
            radius,
        )
    }

    /// Commit one construction-option circle and append its descriptive
    /// diagnostic naming the resulting centre and radius.
    ///
    /// The shared [`Application::commit_new_entity`] funnel owns the active-layer
    /// check, the single transaction, the single undo step and the all-or-nothing
    /// rollback, so this wrapper adds no second write path.
    #[allow(clippy::too_many_arguments)]
    fn commit_circle(
        &mut self,
        session: &SessionState,
        command: &Command,
        geometry: SemanticGeometry,
        label: &str,
        code: &str,
        selector: &str,
        center: Point3,
        radius: f64,
    ) -> CadResult<CommandOutcome> {
        let mut outcome = self.commit_new_entity(session, command, geometry, label)?;
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: code.into(),
            message: format!(
                "{selector}: center ({}, {}, {}), radius {}",
                center.x, center.y, center.z, radius
            ),
        });
        Ok(outcome)
    }
}

/// The work-plane `+Z` circle used by all three construction options.
///
/// A flat circle is a `+Z` construction; the normal is a documented plane hint,
/// never a fabricated tilt. `radius` is already validated positive and finite by
/// every caller.
fn circle_geometry(center: Point3, radius: f64) -> SemanticGeometry {
    SemanticGeometry::Circle {
        center,
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        radius,
    }
}
