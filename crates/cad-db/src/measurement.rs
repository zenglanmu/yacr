//! Measurement payloads produced by the measurement engine.
//!
//! These types originally lived beside the annotation payloads; annotations
//! have been removed, but the measurement subsystem is independent and keeps
//! its result record here.

use cad_domain::*;

#[derive(Debug, Clone, PartialEq)]
pub enum MeasurementAlgorithm {
    Distance2d,
    Distance3d,
    PolylineLength,
    Angle3Points,
    PlanarPolygonArea,
    /// The exact area of **one closed entity**, computed from that entity's own
    /// stored geometry (`AREA OBJECT`).
    ///
    /// Deliberately its own variant: the value did **not** come from a picked
    /// polygon ring, and the point engine (`cad-measure`) has no input points to
    /// feed it — that engine refuses this variant by name rather than pretending
    /// a point list exists. The record's `inputs`/`plane` conventions are
    /// documented on `cad_app::CommandId::AreaObject`.
    ObjectArea,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeasurementRecord {
    pub algorithm: MeasurementAlgorithm,
    pub inputs: Vec<Point3>,
    pub plane: Option<WorkPlane>,
    pub value: f64,
    pub units: UnitContext,
    pub source: GeometrySource,
    pub precision: Precision,
}
