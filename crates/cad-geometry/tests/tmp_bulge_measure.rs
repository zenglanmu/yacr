//! TEMPORARY measurement harness (to be deleted).

use cad_domain::{Point3, SemanticGeometry};
use cad_geometry::{tessellate_geometry, TessellationParams};

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

/// Four quadrant vertices, CCW starting at angle 0 (the created hatch loop).
fn loop_pts(radius: f64, ccw: bool) -> Vec<Point3> {
    let mut pts: Vec<Point3> = (0..4)
        .map(|i| {
            let a = i as f64 * std::f64::consts::FRAC_PI_2;
            p(radius * a.cos(), radius * a.sin())
        })
        .collect();
    if !ccw {
        pts.reverse();
    }
    pts
}

fn measure(label: &str, pts: Vec<Point3>, bulges: Vec<f64>, radius: f64) {
    let g = SemanticGeometry::Polyline {
        points: pts.clone(),
        bulges: bulges.clone(),
        closed: true,
    };
    let out = tessellate_geometry(&g, TessellationParams::default());
    let max_err = out
        .iter()
        .map(|q| (q.x.hypot(q.y) - radius).abs())
        .fold(0.0f64, f64::max);
    // chord-plane overshoot: a point is "outside" the inscribed polygon region if
    // its radius exceeds the analytic radius.
    let max_radius = out.iter().map(|q| q.x.hypot(q.y)).fold(0.0f64, f64::max);
    println!(
        "{label}: n={} max radial dev={:.6} max radius={:.6} (analytic {radius})",
        out.len(),
        max_err,
        max_radius
    );
    if max_err > 1e-6 {
        println!("   first 12 samples: {:?}", &out[..out.len().min(12)]);
    }
}

#[test]
fn tmp_measure() {
    let radius = 10.0;
    let b = std::f64::consts::FRAC_PI_8.tan();
    println!("bulge = {b}");
    measure("CCW +b", loop_pts(radius, true), vec![b; 4], radius);
    measure("CCW -b", loop_pts(radius, true), vec![-b; 4], radius);
    measure("CW  +b", loop_pts(radius, false), vec![b; 4], radius);
    measure("CW  -b", loop_pts(radius, false), vec![-b; 4], radius);
    measure(
        "CCW mixed (chord-order signs)",
        loop_pts(radius, true),
        vec![b, b, b, b],
        radius,
    );
}
