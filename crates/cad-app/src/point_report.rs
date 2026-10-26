//! The `ID` point-coordinate report (measure/query area).
//!
//! `ID` answers one question: *what is the world coordinate of this pick, and
//! where did it come from?* The answer is a [`PointReport`], not a
//! `cad_db::MeasurementRecord`: a measurement record's `value` is a single `f64`
//! and its algorithm set has no point variant, so expressing an identifier
//! through the measurement engine would force a fabricated scalar and a fake
//! algorithm name. `ID` therefore never enters [`crate::MeasurementEngine`]; the
//! command branches before evaluation (`app_measure::confirm_measurement`).
//!
//! ## Our declared rule: the snapped point is the reported point
//!
//! The capture path resolves object snap **first** and the session then applies
//! grid snap and `ORTHO` (`SessionState::constrained_point`). For every other
//! tool that order is the documented one; for `ID` it would be wrong: reporting a
//! coordinate means reporting the coordinate the user aimed at. So for `ID`:
//!
//! * a resolved object-snap candidate's point is reported **verbatim**, and the
//!   session's grid-snap/`ORTHO` constraint is **skipped for this kind** — the
//!   snapped point is never re-quantised onto the grid;
//! * with no candidate the raw picked point is reported and `snap` is `null`,
//!   never a fabricated provenance;
//! * the report's `precision` is the **snapped geometry's** precision when a snap
//!   exists (a proxy/mesh snap is approximate), and `Precision::Analytic`
//!   otherwise — a point the user picked has no approximated source.
//!
//! This is a project decision, not a claim about AutoCAD's internal order.

use cad_domain::{Point3, Precision, SpaceId, Unit, UnitContext};
use cad_measure::{SnapCandidate, SnapKind};

/// The snap provenance stored with one captured point entry.
///
/// A deliberately small projection of [`SnapCandidate`]: the report needs the
/// kind, the precision and the screen-space distance, not the source refs (which
/// are presentation/selection concerns).
#[derive(Debug, Clone, PartialEq)]
pub struct InputSnap {
    /// Which object snap produced the point.
    pub kind: SnapKind,
    /// The precision of the geometry the snap was derived from.
    pub precision: Precision,
    /// Perpendicular distance from the raw pick, in logical pixels.
    pub logical_pixel_distance: f64,
}

impl InputSnap {
    /// Project a resolved candidate onto the stored provenance.
    pub fn from_candidate(candidate: &SnapCandidate) -> Self {
        InputSnap {
            kind: candidate.kind,
            precision: candidate.precision.clone(),
            logical_pixel_distance: candidate.logical_pixel_distance,
        }
    }
}

/// The space an [`PointReport`] was taken in.
///
/// Only the two observation spaces exist: model space and one named paper layout.
/// `SpaceId::Block` is block-definition space, reached only through an `INSERT`
/// and never selectable, so it is refused before a report exists (see
/// `app_measure::point_report_outcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdSpace {
    Model,
    Paper,
}

impl IdSpace {
    /// The stable machine token (`model`/`paper`) the wire/log layer uses.
    pub fn as_str(self) -> &'static str {
        match self {
            IdSpace::Model => "model",
            IdSpace::Paper => "paper",
        }
    }

    /// The report space of a validated session space, or `None` for
    /// `SpaceId::Block` (which has no representation here by construction).
    pub fn from_space_id(space: &SpaceId) -> Option<Self> {
        match space {
            SpaceId::Model => Some(IdSpace::Model),
            SpaceId::Paper(_) => Some(IdSpace::Paper),
            SpaceId::Block(_) => None,
        }
    }
}

/// The structured result of one `ID` pick.
///
/// `position` is the **snapped** point when a snap resolved (see the module
/// docs); `snap`/`precision`/`logical_pixel_distance` describe that pick and are
/// `None`/`Analytic`/`None` for a bare pick. `unit` is a known display unit
/// symbol (`mm`/`m`/`in`/`ft`) or `None` for drawing units — an unknown unit is
/// never fabricated.
#[derive(Debug, Clone, PartialEq)]
pub struct PointReport {
    pub position: Point3,
    pub space: IdSpace,
    pub snap: Option<SnapKind>,
    pub precision: Precision,
    pub unit: Option<String>,
    pub logical_pixel_distance: Option<f64>,
}

/// A known display unit symbol, or `None` for drawing units.
///
/// The single authority for the unit→symbol projection: `cad-app` uses it for the
/// point report and `cad-host`'s command echo uses it for a measurement record, so
/// the two wire projections cannot drift apart. `Unit::DrawingUnits` is unknown by
/// definition, so it maps to `None` instead of inventing millimetres.
pub fn unit_symbol(units: &UnitContext) -> Option<&'static str> {
    match units.display {
        Unit::DrawingUnits => None,
        Unit::Millimeter => Some("mm"),
        Unit::Meter => Some("m"),
        Unit::Inch => Some("in"),
        Unit::Foot => Some("ft"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::UnitContext;

    #[test]
    fn unit_symbol_never_invents_drawing_units() {
        assert_eq!(unit_symbol(&UnitContext::drawing_units()), None);
        assert_eq!(unit_symbol(&UnitContext::from_meter()), Some("m"));
    }

    #[test]
    fn id_space_maps_the_two_observation_spaces_and_refuses_block_space() {
        assert_eq!(
            IdSpace::from_space_id(&SpaceId::Model),
            Some(IdSpace::Model)
        );
        assert_eq!(
            IdSpace::from_space_id(&SpaceId::Paper(cad_domain::LayoutId(3))),
            Some(IdSpace::Paper)
        );
        assert_eq!(
            IdSpace::from_space_id(&SpaceId::Block(cad_domain::BlockId(0))),
            None,
            "block space is never a selectable observation space"
        );
        assert_eq!(IdSpace::Model.as_str(), "model");
        assert_eq!(IdSpace::Paper.as_str(), "paper");
    }
}
