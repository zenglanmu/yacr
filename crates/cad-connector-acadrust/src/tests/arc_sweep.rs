//! Entity-arc sweep conventions, and why the HATCH boundary-arc mirror rule
//! does **not** apply to them (ADR 0018 §B.7, U-14).
//!
//! A HATCH boundary arc stores absolute angles **plus** an `is_ccw` flag, and a
//! clockwise edge stores them as complementary angles. `ARC` and `ELLIPSE`
//! entities store absolute angles with **no** direction flag — an entity arc is
//! always read counter-clockwise from start to end — and a polyline carries a
//! *signed* bulge (`tan(sweep/4)`), which has no wrap ambiguity at all. Applying
//! the hatch rule to them would flip a legitimate arc, so these tests pin the
//! difference.

use super::*;
use acadrust::entities::{Arc, Ellipse, EntityType};

fn convert(geometry: EntityType) -> SemanticGeometry {
    let req = request(Vec::new());
    let acad = acadrust::CadDocument::new();
    let mut builder =
        ImporterBuilder::new(&req, &acad, ReadStats::default(), compute_identity(&[]));
    let common = geometry.common();
    builder.convert(&geometry, common).0
}

fn arc_sweep(start_degrees: f64, end_degrees: f64) -> f64 {
    let mut arc = Arc::new();
    arc.center = acadrust::types::Vector3::new(0.0, 0.0, 0.0);
    arc.radius = 10.0;
    arc.start_angle = start_degrees.to_radians();
    arc.end_angle = end_degrees.to_radians();
    arc.normal = acadrust::types::Vector3::new(0.0, 0.0, 1.0);
    match convert(EntityType::Arc(arc)) {
        SemanticGeometry::Arc { sweep, .. } => sweep,
        other => panic!("expected an arc, got {other:?}"),
    }
}

#[test]
fn an_arc_entity_reads_its_stored_sweep_counter_clockwise() {
    let tau = std::f64::consts::TAU;
    // Wrapping the short way round 0° is the short way, not the long way.
    assert!((arc_sweep(350.0, 10.0) - 20.0_f64.to_radians()).abs() < 1e-12);
    // The same two numbers the other way round really are a 340° arc: an ARC
    // entity has no direction flag, so "start > end" is not a mirrored edge.
    assert!((arc_sweep(10.0, 350.0) - 340.0_f64.to_radians()).abs() < 1e-12);
    assert!((arc_sweep(10.0, 350.0) - tau).abs() > 1e-3);
}

#[test]
fn an_arc_entity_never_takes_the_mirrored_hatch_reading() {
    // The case that separates the two conventions: stored `start > end`.
    //   * a HATCH boundary edge with `ccw = 0` stores complementary angles, so
    //     the edge is the 0.1 rad mirror of these numbers;
    //   * an ARC entity has no direction flag: it is counter-clockwise from
    //     start to end, i.e. almost a whole turn.
    let stored_start = 1.7_f64;
    let stored_end = 1.6_f64;
    let mut arc = Arc::new();
    arc.center = acadrust::types::Vector3::new(0.0, 0.0, 0.0);
    arc.radius = 10.0;
    arc.start_angle = stored_start;
    arc.end_angle = stored_end;
    arc.normal = acadrust::types::Vector3::new(0.0, 0.0, 1.0);
    let SemanticGeometry::Arc { sweep, .. } = convert(EntityType::Arc(arc)) else {
        panic!("expected an arc");
    };
    let expected = (stored_end - stored_start).rem_euclid(std::f64::consts::TAU);
    assert!((sweep - expected).abs() < 1e-12, "{sweep} vs {expected}");
    assert!(
        (sweep - std::f64::consts::TAU).abs() < 0.2,
        "an entity arc keeps its counter-clockwise sweep: {sweep}"
    );
    assert!(
        (sweep - 0.1).abs() > 1.0,
        "an entity arc must not be mirrored like a clockwise hatch edge: {sweep}"
    );
}

#[test]
fn an_ellipse_entity_reads_its_parameters_counter_clockwise() {
    let mut ellipse = Ellipse::new();
    ellipse.center = acadrust::types::Vector3::new(0.0, 0.0, 0.0);
    ellipse.major_axis = acadrust::types::Vector3::new(5.0, 0.0, 0.0);
    ellipse.minor_axis_ratio = 0.5;
    ellipse.normal = acadrust::types::Vector3::new(0.0, 0.0, 1.0);
    ellipse.start_parameter = 4.7;
    ellipse.end_parameter = 0.1;
    let SemanticGeometry::Ellipse { sweep, .. } = convert(EntityType::Ellipse(ellipse)) else {
        panic!("expected an ellipse");
    };
    let expected = (0.1_f64 - 4.7_f64).rem_euclid(std::f64::consts::TAU);
    assert!((sweep - expected).abs() < 1e-12, "{sweep} vs {expected}");
    assert!(sweep > 1.5 && sweep < 2.0, "{sweep}");
}
