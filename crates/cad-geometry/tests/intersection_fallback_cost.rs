//! Synthetic, local cost guard for the *approximate* intersection fallback.
//!
//! This is **not** a performance conclusion: it is a single-process wall-clock
//! observation on whatever machine runs `cargo test`, dominated by debug-build
//! codegen. It exists only to catch a regression back to an unbounded sample
//! density (the old fallback used a fixed 1e-9 chord and hit the 4096-segment
//! cap, measured at ~263 ms per pair in release). The deterministic budget
//! assertions live in the crate's unit tests; this file only guards the
//! order-of-magnitude regression.

use cad_domain::*;
use cad_geometry::*;
use std::time::Instant;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn pz(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// Circle of radius 3 at the origin crossed by a two-point polyline whose
/// bulge -1 arc is the upper semicircle of radius 4 centred at (0,-2). A bulge
/// polyline is not a conic primitive, so this pair takes the discretisation
/// fallback. The exact crossings are `(±3√15/4, 3/4)`.
fn probe(scale: f64) -> (SemanticGeometry, SemanticGeometry) {
    let circle = SemanticGeometry::Circle {
        center: pz(0.0, 0.0, 0.0),
        normal: pz(0.0, 0.0, 1.0),
        radius: 3.0 * scale,
    };
    let polyline = SemanticGeometry::Polyline {
        points: vec![p(-4.0 * scale, -2.0 * scale), p(4.0 * scale, -2.0 * scale)],
        bulges: vec![-1.0],
        closed: false,
    };
    (circle, polyline)
}

#[test]
fn fallback_probe_pair_stays_far_below_the_old_unbounded_cost() {
    let (circle, polyline) = probe(1.0);
    let policy = TolerancePolicy::default();

    // Warm up (allocator/ICache), then report the best of a few runs so the
    // number is not dominated by scheduler noise.
    let _ = DefaultGeometryEngine
        .intersect_local(&circle, &polyline, &policy)
        .unwrap();
    let runs = 5;
    let mut best = f64::INFINITY;
    for _ in 0..runs {
        let start = Instant::now();
        let hits = DefaultGeometryEngine
            .intersect_local(&circle, &polyline, &policy)
            .unwrap();
        let ms = start.elapsed().as_secs_f64() * 1e3;
        best = best.min(ms);
        assert_eq!(hits.len(), 2, "probe must keep both genuine crossings");
    }
    eprintln!("fallback probe pair: best of {runs} runs = {best:.3} ms (synthetic, local)");
    // Generous: the old release-measured cost was ~263 ms; even a 20x debug
    // slowdown of the bounded path stays well under this guard.
    assert!(
        best < 100.0,
        "fallback probe pair regressed to {best:.3} ms (old unbounded cost ~263 ms release)"
    );
}

#[test]
fn fallback_cost_does_not_grow_with_entity_scale() {
    // The chord is derived from the geometry's own extent, so the same probe
    // scaled by 1e3 must not cost meaningfully more. This catches an absolute
    // chord slipping back in, which would always saturate the segment cap.
    let policy = TolerancePolicy::default();
    let time = |scale: f64| {
        let (a, b) = probe(scale);
        let _ = DefaultGeometryEngine
            .intersect_local(&a, &b, &policy)
            .unwrap();
        let start = Instant::now();
        let hits = DefaultGeometryEngine
            .intersect_local(&a, &b, &policy)
            .unwrap();
        let ms = start.elapsed().as_secs_f64() * 1e3;
        assert_eq!(hits.len(), 2, "scale {scale}: both crossings kept");
        ms
    };
    let small = time(1.0);
    let large = time(1e3);
    eprintln!("fallback probe: scale=1 {small:.3} ms, scale=1e3 {large:.3} ms (synthetic, local)");
    assert!(
        large < (small.max(1.0) * 5.0) + 1.0,
        "cost must be scale-independent: {small:.3} ms (scale 1) vs {large:.3} ms (scale 1e3)"
    );
}
