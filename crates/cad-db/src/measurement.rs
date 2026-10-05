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
