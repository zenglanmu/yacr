//! The participant set shared by every numeric FILLET form.
//!
//! Declared from [`super`] with an explicit `#[path]`. Everything here is pure:
//! no database, no session, no history.
//!
//! A numeric FILLET form is always driven by a **profiled** curve — a SPLINE
//! ([`super::corner_spline`]) or an ELLIPSE ([`super::corner_ellipse`]) — and this
//! module owns everything it can be paired with, together with the per-kind rules
//! all three forms share:
//!
//! * [`Other::closest`] — the closest point on the participant's supporting curve
//!   and its own parameter (the tangency polish and the corner search both use it);
//! * [`Other::signed`] — a signed function of position whose zeros are the
//!   participant's carrier, i.e. the residual the corner root scan bisects
//!   (`None` for a SPLINE, which has no closed form — the caller minimises the
//!   point-to-curve distance instead);
//! * [`Other::accepts`] — whether a **tangency** (and the pick) lies on the picked
//!   finite object (an `R > 0` rule: a tangent on an arc's supporting circle
//!   outside its sweep is the wrong fillet);
//! * [`Other::on_carrier_span`] — whether a **corner** may land there (`R = 0`):
//!   a line may be extended to it, an arc/elliptical arc may not;
//! * [`Other::trim`] — the pick-side trim, per kind. A CIRCLE has no trim form: it
//!   is a closed curve this FILLET never cuts, so it is left whole at every radius
//!   (see [`super::corner_sharp`] for what that implies at `R = 0`).

use super::super::helpers::{distance, is_finite_point};
use super::super::*;
use super::corner_ellipse::{
    closest_on_ellipse, ellipse_accepts, trim_ellipse, ClosestPoint, EllipseCurve,
};
use super::corner_geometry::{
    arc_position, retarget_pick_side, trim_arc_to, Curve, GeometryUpdate,
};
use super::corner_spline::SplineCurve;
use cad_geometry::{add, dot, length, scale, sub};

/// One resolved participant that is **not** the driver of a numeric form.
///
/// The driver of a numeric FILLET is a profiled curve — a SPLINE
/// ([`super::corner_spline`]) or an ELLIPSE ([`super::corner_ellipse`]); this is
/// everything it can be paired with, plus the per-kind rules the two numeric forms
/// and the `R = 0` sharp-corner form ([`super::corner_sharp`]) all share:
/// closest-point projection, span membership, a signed carrier function (for root
/// finding), and the pick-side trim.
pub(super) enum Other {
    Line {
        start: Point3,
        end: Point3,
    },
    Round {
        center: Point3,
        radius: f64,
        arc: Option<(f64, f64)>,
    },
    Ellipse(EllipseCurve),
    Spline(SplineCurve),
}

impl Other {
    pub(super) fn from_curve(curve: Curve) -> Self {
        match curve {
            Curve::Line { start, end } => Self::Line { start, end },
            Curve::Circle { center, radius } => Self::Round {
                center,
                radius,
                arc: None,
            },
            Curve::Arc {
                center,
                radius,
                start,
                sweep,
            } => Self::Round {
                center,
                radius,
                arc: Some((start, sweep)),
            },
            Curve::Ellipse(ellipse) => Self::Ellipse(ellipse),
            Curve::Spline(spline) => Self::Spline(spline),
        }
    }

    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Line { .. } => "LINE",
            Self::Round { arc: None, .. } => "CIRCLE",
            Self::Round { arc: Some(_), .. } => "ARC",
            Self::Ellipse(_) => "ELLIPSE",
            Self::Spline(_) => "SPLINE",
        }
    }

    pub(super) fn validate(&self, tol: f64) -> Result<(), String> {
        match self {
            Self::Line { start, end } => {
                if !is_finite_point(*start) || !is_finite_point(*end) {
                    return Err("FILLET.non_finite: the picked object must be finite".into());
                }
                if distance(*start, *end) <= tol {
                    return Err("FILLET.degenerate_line: the picked object has no direction".into());
                }
                Ok(())
            }
            Self::Round { radius, .. } => {
                if !radius.is_finite() || *radius <= tol {
                    return Err("FILLET.degenerate_circle: the picked curve has no radius".into());
                }
                Ok(())
            }
            Self::Ellipse(ellipse) => {
                if !ellipse.a.is_finite()
                    || !ellipse.b.is_finite()
                    || ellipse.a <= tol
                    || ellipse.b <= tol
                {
                    return Err(
                        "FILLET.degenerate_ellipse: the picked ellipse has no usable axes".into(),
                    );
                }
                Ok(())
            }
            Self::Spline(_) => Ok(()),
        }
    }

    pub(super) fn z_extent(&self) -> (f64, f64) {
        match self {
            Self::Line { start, end } => (start.z.min(end.z), start.z.max(end.z)),
            Self::Round { center, .. } => (center.z, center.z),
            Self::Ellipse(EllipseCurve { center, .. }) => (center.z, center.z),
            Self::Spline(spline) => (spline.z(), spline.z()),
        }
    }

    /// The closest point of the participant's supporting curve.
    pub(super) fn closest(&self, from: Point3) -> Option<ClosestPoint> {
        match self {
            Self::Line { start, end } => {
                let direction = sub(*end, *start);
                let len = length(direction);
                if !len.is_finite() || len <= 1e-12 {
                    return None;
                }
                let unit = scale(direction, 1.0 / len);
                let along = dot(sub(from, *start), unit);
                let point = add(*start, scale(unit, along));
                Some(ClosestPoint {
                    distance: distance(point, from),
                    point,
                    parameter: along,
                })
            }
            Self::Round {
                center,
                radius,
                arc: _,
            } => {
                let offset = sub(from, *center);
                let len = length(offset);
                if !len.is_finite() || len <= 1e-12 {
                    return None;
                }
                let point = add(*center, scale(offset, radius / len));
                Some(ClosestPoint {
                    distance: distance(point, from),
                    point,
                    parameter: offset.y.atan2(offset.x),
                })
            }
            Self::Ellipse(ellipse) => closest_on_ellipse(*ellipse, from),
            Self::Spline(spline) => spline.nearest(from).map(|(parameter, point)| ClosestPoint {
                distance: distance(point, from),
                point,
                parameter,
            }),
        }
    }

    /// Whether a tangent point (and the pick) lies on the finite participant.
    pub(super) fn accepts(&self, tangent: Point3, pick: Point3, tol: f64) -> bool {
        match self {
            Self::Line { start, end } => {
                let direction = sub(*end, *start);
                let len = length(direction);
                if !len.is_finite() || len <= tol {
                    return false;
                }
                let unit = scale(direction, 1.0 / len);
                let tangent_along = dot(sub(tangent, *start), unit);
                let pick_along = dot(sub(pick, *start), unit);
                tangent_along > tol
                    && tangent_along <= len + 1e-9
                    && pick_along > tol
                    && pick_along <= len + 1e-9
            }
            Self::Round {
                center,
                radius,
                arc,
            } => {
                let Some((start, sweep)) = arc else {
                    return true;
                };
                let tangent_position = arc_position(*center, *start, *sweep, tangent);
                let pick_position = arc_position(*center, *start, *sweep, pick);
                tangent_position > tol
                    && tangent_position <= sweep.abs() + 1e-9
                    && pick_position <= sweep.abs() + 1e-9
                    && (radius - distance(tangent, *center)).abs() <= 1e-7 * (1.0 + radius)
            }
            Self::Ellipse(ellipse) => ellipse_accepts(*ellipse, tangent, pick, tol),
            Self::Spline(spline) => spline.accepts(tangent, pick, tol),
        }
    }

    /// A signed function of position whose zeros are this participant's carrier
    /// (`None` for a SPLINE, which has no such closed form — the corner search then
    /// minimises the point-to-curve distance instead).
    pub(super) fn signed(&self, point: Point3) -> Option<f64> {
        match self {
            Self::Line { start, end } => {
                let direction = sub(*end, *start);
                let len = length(direction);
                if !len.is_finite() || len <= 1e-12 {
                    return None;
                }
                let unit = scale(direction, 1.0 / len);
                let offset = sub(point, *start);
                Some(offset.x * unit.y - offset.y * unit.x)
            }
            Self::Round { center, .. } => {
                let offset = sub(point, *center);
                let gap = length(offset);
                if !gap.is_finite() {
                    return None;
                }
                let radius = match self {
                    Self::Round { radius, .. } => *radius,
                    _ => unreachable!(),
                };
                Some(gap - radius)
            }
            Self::Ellipse(ellipse) => {
                let offset = sub(point, ellipse.center);
                let x = dot(offset, ellipse.major) / ellipse.a;
                let y = dot(offset, ellipse.minor) / ellipse.b;
                Some((x * x + y * y - 1.0) * ellipse.a.min(ellipse.b))
            }
            Self::Spline(_) => None,
        }
    }

    /// Whether a **corner** (`R = 0`) may land on this participant.
    ///
    /// The two numeric forms accept a tangency anywhere on the carrier (see
    /// [`Self::accepts`]); the sharp-corner form is stricter about what it may cut:
    ///
    /// * a LINE is trimmed **or extended** to the corner, so any crossing of its own
    ///   infinite line is valid (the same rule the line/line form documents);
    /// * a CIRCLE and a SPLINE are their whole carrier, so any point on them is a
    ///   corner of the picked object;
    /// * an ARC must be met inside its stored sweep, and an ELLIPSE inside its stored
    ///   parametric span: FILLET does not extend those, and cutting on the
    ///   supporting circle outside the picked arc would trim a piece the user did
    ///   not pick.
    pub(super) fn on_carrier_span(&self, point: Point3, _tol: f64) -> bool {
        match self {
            Self::Line { .. } | Self::Round { arc: None, .. } | Self::Spline(_) => true,
            Self::Round {
                center,
                radius,
                arc: Some((start, sweep)),
            } => {
                let position = arc_position(*center, *start, *sweep, point);
                position.is_finite()
                    && position <= sweep.abs() + 1e-9
                    && (radius - distance(point, *center)).abs() <= 1e-7 * (1.0 + radius)
            }
            Self::Ellipse(ellipse) => {
                let Some(theta) = ellipse.theta_of(point) else {
                    return false;
                };
                ellipse.position(theta) <= ellipse.span() + 1e-9
            }
        }
    }

    /// Trim the participant at `tangent` on the pick side. `parameter` is the
    /// participant's own parameter at `tangent` (only a SPLINE trim needs it).
    pub(super) fn trim(
        &self,
        tangent: Point3,
        parameter: f64,
        pick: Point3,
        tol: f64,
    ) -> Result<Option<GeometryUpdate>, String> {
        match self {
            Self::Line { start, end } => {
                retarget_pick_side((*start, *end), pick, tangent, tol, "FILLET").map(Some)
            }
            Self::Round {
                center,
                arc: Some((start, sweep)),
                ..
            } => trim_arc_to(*center, *start, *sweep, tangent, pick, tol).map(Some),
            Self::Round { arc: None, .. } => Ok(None),
            Self::Ellipse(ellipse) => trim_ellipse(*ellipse, tangent, pick, tol).map(Some),
            Self::Spline(spline) => spline.trim_keep_pick(parameter, pick, tol),
        }
    }
}
