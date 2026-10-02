//! Measurement spaces, provenance-carrying picked points and measurement
//! requests.
//!
//! This module owns the space model ([`MeasurementSpace`]) and the policy that
//! decides which coordinates belong to it: paper picks inside a viewport are
//! mapped to model coordinates through the verified inverse transform, paper
//! space measures the sheet directly, and a measurement may not mix inputs
//! gathered from different spaces.

use cad_db::MeasurementAlgorithm;
use cad_domain::*;

use crate::algorithms::sub;

/// The space a measurement is evaluated in.
///
/// The variant chosen must agree with the algorithm's dimensionality, and every
/// point supplied for the measurement must originate from that slot (see
/// [`MeasurementPoint`]); a mismatch is an explicit error.
#[derive(Debug, Clone, PartialEq)]
pub enum MeasurementSpace {
    /// Explicit 2D work plane (u/v/out-of-plane in world units).
    Plane(WorkPlane),
    /// 3D model space; only [`MeasurementAlgorithm::Distance3d`] and
    /// [`MeasurementAlgorithm::Angle3Points`] are defined here.
    World3d,
    /// Paper space of a layout. Without a verified inverse transform the engine
    /// has no model-space scale and refuses the measurement.
    Paper(LayoutId),
    /// A paper viewport showing model space through `inverse`.
    ViewportModel {
        layout: LayoutId,
        inverse: Transform3,
    },
}

impl MeasurementSpace {
    /// Whether planar (2D) algorithms are defined in this space.
    ///
    /// Paper space is a 2D sheet and a viewport maps paper onto a model plane,
    /// so both have a well-defined measurement plane. `World3d` alone does not.
    pub fn is_planar(&self) -> bool {
        matches!(
            self,
            MeasurementSpace::Plane(_)
                | MeasurementSpace::Paper(_)
                | MeasurementSpace::ViewportModel { .. }
        )
    }

    /// The layout id for paper-space-located geometry, if any.
    ///
    /// A model-space measurement returns `None`; geometry tagged with an
    /// incompatible space is a mixed input.
    pub fn paper_layout(&self) -> Option<LayoutId> {
        match self {
            MeasurementSpace::Plane(_) | MeasurementSpace::World3d => None,
            MeasurementSpace::Paper(layout) | MeasurementSpace::ViewportModel { layout, .. } => {
                Some(*layout)
            }
        }
    }
}

/// A picked point together with the space it was captured from.
///
/// The space tag is what lets the engine refuse to mix paper-space and
/// model-space geometry inside a single measurement, and what makes "all points
/// must fit the measurement's plane" a check instead of an assumption.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasurementPoint {
    pub point: Point3,
    pub space: SpaceId,
}

impl MeasurementPoint {
    /// A point captured in model space.
    pub fn model(point: Point3) -> Self {
        MeasurementPoint {
            point,
            space: SpaceId::Model,
        }
    }

    /// A point captured on `layout` paper.
    pub fn paper(point: Point3, layout: LayoutId) -> Self {
        MeasurementPoint {
            point,
            space: SpaceId::Paper(layout),
        }
    }

    /// The raw coordinate, dropping the provenance.
    pub fn as_point3(&self) -> Point3 {
        self.point
    }
}

pub struct MeasurementRequest {
    pub algorithm: MeasurementAlgorithm,
    /// Raw coordinates. Kept for callers that already enforce provenance; the
    /// `space` field still constrains which values are acceptable.
    pub points: Vec<Point3>,
    /// Preferred provenance-carrying form. When non-empty it takes precedence
    /// over `points` and enables mixed-space rejection.
    pub tapped: Vec<MeasurementPoint>,
    pub space: MeasurementSpace,
    pub units: UnitContext,
    pub source: GeometrySource,
    pub precision: Precision,
}

impl MeasurementRequest {
    /// Build a request from provenance-carrying points, filling `points` from
    /// the tapped coordinates.
    pub fn from_tapped(
        algorithm: MeasurementAlgorithm,
        tapped: Vec<MeasurementPoint>,
        space: MeasurementSpace,
        units: UnitContext,
        source: GeometrySource,
        precision: Precision,
    ) -> Self {
        MeasurementRequest {
            algorithm,
            points: tapped.iter().map(|t| t.point).collect(),
            tapped,
            space,
            units,
            source,
            precision,
        }
    }

    pub(crate) fn coordinates(&self) -> Vec<Point3> {
        if self.tapped.is_empty() {
            self.points.clone()
        } else {
            self.tapped.iter().map(|t| t.point).collect()
        }
    }
}

pub(crate) fn require_planar_space(space: &MeasurementSpace) -> CadResult<()> {
    if !space.is_planar() {
        return Err(CadError::InvalidInput(
            "planar measurement needs an explicit work plane or paper space, not 3D space"
                .to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn require_plane(plane: Option<WorkPlane>) -> CadResult<WorkPlane> {
    plane.ok_or_else(|| {
        CadError::InvalidInput("planar measurement needs a defined measurement plane".to_string())
    })
}

fn transform_finite(t: &Transform3) -> bool {
    t.matrix.iter().flatten().all(|c| c.is_finite())
}

/// Whether a viewport inverse (paper → model) is usable for measurement.
///
/// It must be finite and invertible: a singular map cannot recover a model
/// point from a paper pick, so model measurement is disabled rather than
/// guessed (spec §3.3, F04).
pub(crate) fn valid_inverse(t: &Transform3) -> bool {
    if !transform_finite(t) {
        return false;
    }
    let s = t.max_scale();
    if !s.is_finite() || s <= 0.0 {
        return false;
    }
    let det = t.determinant().abs();
    det.is_finite() && det > 1e-12 * s * s * s
}

/// The paper sheet as a 2D work plane (x/y in paper units).
pub(crate) fn paper_plane() -> WorkPlane {
    WorkPlane {
        origin: Point3::default(),
        u: Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        v: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
    }
}

/// The model work plane that a viewport's paper plane maps onto.
///
/// A `ViewportModel.inverse` maps paper → model, so the paper x/y axes become
/// the plane's u/v basis and the model origin is the image of the paper origin.
pub(crate) fn model_plane_from_inverse(inverse: &Transform3) -> WorkPlane {
    let origin = inverse.apply_point(Point3::default());
    let u = sub(
        inverse.apply_point(Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }),
        origin,
    );
    let v = sub(
        inverse.apply_point(Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        }),
        origin,
    );
    WorkPlane { origin, u, v }
}

/// The effective measurement plane of a space, when it has one.
pub(crate) fn effective_plane(space: &MeasurementSpace) -> Option<WorkPlane> {
    match space {
        MeasurementSpace::Plane(plane) => Some(*plane),
        MeasurementSpace::Paper(_) => Some(paper_plane()),
        MeasurementSpace::ViewportModel { inverse, .. } => Some(model_plane_from_inverse(inverse)),
        MeasurementSpace::World3d => None,
    }
}

/// Bring picked points into the space the measurement is evaluated in.
///
/// For `ViewportModel` the picks are paper coordinates, so the verified
/// paper→model inverse maps them into model space before any distance/area is
/// computed. All other spaces already carry their own coordinates.
pub(crate) fn measure_space_points(space: &MeasurementSpace, points: &[Point3]) -> Vec<Point3> {
    match space {
        MeasurementSpace::ViewportModel { inverse, .. } => {
            points.iter().map(|p| inverse.apply_point(*p)).collect()
        }
        MeasurementSpace::Plane(_) | MeasurementSpace::World3d | MeasurementSpace::Paper(_) => {
            points.to_vec()
        }
    }
}
