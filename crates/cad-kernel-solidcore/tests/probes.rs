//! Probe tests for the `solidcore` modeler: the created geometry's parameters,
//! watertightness through this repository's own tessellator, real in-flight
//! cancellation, and a scale/offset sweep for mass properties.
//!
//! Same evidence boundary as `chain.rs`: Linux x86-64, real pinned kernel, no
//! mock, no GPU/pixel and no other platform (ADR-0036).

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cad_domain::{BrepSurface, CadError};
use cad_kernel_adapter::{
    execute_solid, tessellate_exchange, BooleanOp, PrimitiveSpec, SolidExchange, SolidModeler,
    SolidOperation, SolidRequest, TessellationBudget, TessellationOutcome, TessellationTolerance,
};
use cad_kernel_solidcore::SolidCoreModeler;

use common::*;

#[test]
fn probe_curved_primitive_parameters_match_the_request() {
    let modeler = SolidCoreModeler::new();

    // Cylinder: radius 1.5 about +Z from z = 0 to z = 2.
    let brep = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: PrimitiveSpec::Cylinder {
                origin: p(0.0, 0.0, 0.0),
                radius: 1.5,
                height: 2.0,
            },
        },
    ));
    let mut lateral_radius = None;
    for face in faces(&brep) {
        if let BrepSurface::Cylinder {
            axis,
            ref_dir,
            radius,
            ..
        } = &face.surface
        {
            lateral_radius = Some(*radius);
            assert!(
                near(axis.x, 0.0) && near(axis.y, 0.0) && near(axis.z, 1.0),
                "axis {axis:?}"
            );
            assert!(near(dot(*axis, *axis), 1.0));
            assert!(
                dot(*ref_dir, *axis).abs() < 1e-9,
                "ref_dir {ref_dir:?} not in the cap plane"
            );
        }
    }
    assert!(near(lateral_radius.expect("a cylindrical face"), 1.5));
    let mut circles = planar_circles(&brep);
    circles.sort_by(|a, b| a.0.z.partial_cmp(&b.0.z).expect("finite z"));
    assert_eq!(circles.len(), 2, "two cap circles: {circles:?}");
    for (center, radius) in &circles {
        assert!(
            near(center.x, 0.0) && near(center.y, 0.0),
            "cap centre {center:?}"
        );
        assert!(near(*radius, 1.5));
    }
    assert!(near(circles[0].0.z, 0.0) && near(circles[1].0.z, 2.0));

    // Sphere: radius 1.25 at the origin.
    let brep = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: PrimitiveSpec::Sphere {
                origin: p(0.0, 0.0, 0.0),
                radius: 1.25,
            },
        },
    ));
    let sphere = faces(&brep)
        .find_map(|face| match &face.surface {
            BrepSurface::Sphere {
                center,
                radius,
                u_dir,
                pole,
            } => Some((*center, *radius, *u_dir, *pole)),
            _ => None,
        })
        .expect("a spherical face");
    assert!(near(sphere.1, 1.25), "radius {}", sphere.1);
    assert!(near(sphere.0.x, 0.0) && near(sphere.0.y, 0.0) && near(sphere.0.z, 0.0));
    assert!(
        near(dot(sphere.2, sphere.3), 0.0),
        "u_dir and pole must be perpendicular"
    );

    // Cone: base radius 2 at z = 0, top radius 0.5 at z = 3, so tan(half angle)
    // is (0.5 - 2) / 3 = -0.5 and the reference radius is 2.
    let brep = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: PrimitiveSpec::Cone {
                origin: p(0.0, 0.0, 0.0),
                radius_bottom: 2.0,
                radius_top: 0.5,
                height: 3.0,
            },
        },
    ));
    let cone = faces(&brep)
        .find_map(|face| match &face.surface {
            BrepSurface::Cone {
                origin,
                axis,
                radius,
                sin_half_angle,
                cos_half_angle,
                ..
            } => Some((*origin, *axis, *radius, *sin_half_angle, *cos_half_angle)),
            _ => None,
        })
        .expect("a conical face");
    assert!(near(cone.2, 2.0), "reference radius {}", cone.2);
    assert!(cone.4 > 0.0, "cos(half angle) must be positive: {}", cone.4);
    let slope = cone.3 / cone.4;
    assert!(
        (slope + 0.5).abs() < 1e-9,
        "slope {slope} should be (0.5 - 2) / 3"
    );
    assert!(near(dot(cone.1, cone.1), 1.0));
    let mut circles = planar_circles(&brep);
    circles.sort_by(|a, b| a.0.z.partial_cmp(&b.0.z).expect("finite z"));
    assert_eq!(circles.len(), 2, "two cap circles: {circles:?}");
    assert!(near(circles[0].1, 2.0), "base radius {}", circles[0].1);
    assert!(near(circles[1].1, 0.5), "top radius {}", circles[1].1);
    assert!(near(circles[0].0.z, 0.0) && near(circles[1].0.z, 3.0));

    // Torus: major 2.5 about +Z, tube 0.4.
    let brep = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: PrimitiveSpec::Torus {
                origin: p(0.0, 0.0, 0.0),
                major_radius: 2.5,
                minor_radius: 0.4,
            },
        },
    ));
    let torus = faces(&brep)
        .find_map(|face| match &face.surface {
            BrepSurface::Torus {
                center,
                normal,
                major_radius,
                minor_radius,
                u_dir,
            } => Some((*center, *normal, *major_radius, *minor_radius, *u_dir)),
            _ => None,
        })
        .expect("a toroidal face");
    assert!(near(torus.2, 2.5), "major radius {}", torus.2);
    assert!(near(torus.3, 0.4), "minor radius {}", torus.3);
    assert!(
        near(torus.1.z, 1.0) && near(torus.1.x, 0.0),
        "normal {:?}",
        torus.1
    );
    assert!(dot(torus.4, torus.1).abs() < 1e-9);
    assert!(near(torus.0.x, 0.0) && near(torus.0.z, 0.0));
}

#[test]
fn probe_every_created_body_tessellates_watertight() {
    let modeler = SolidCoreModeler::new();
    let specs = [
        ("box", box_spec(p(0.0, 0.0, 0.0), 1.0, 2.0, 3.0)),
        (
            "wedge",
            PrimitiveSpec::Wedge {
                origin: p(0.0, 0.0, 0.0),
                length: 2.0,
                width: 3.0,
                height: 4.0,
            },
        ),
        (
            "pyramid",
            PrimitiveSpec::Pyramid {
                base_center: p(0.0, 0.0, 0.0),
                length: 2.0,
                height: 3.0,
            },
        ),
        (
            "cylinder",
            PrimitiveSpec::Cylinder {
                origin: p(0.0, 0.0, 0.0),
                radius: 1.0,
                height: 2.0,
            },
        ),
        (
            "sphere",
            PrimitiveSpec::Sphere {
                origin: p(0.0, 0.0, 0.0),
                radius: 1.0,
            },
        ),
        (
            "cone",
            PrimitiveSpec::Cone {
                origin: p(0.0, 0.0, 0.0),
                radius_bottom: 1.0,
                radius_top: 0.5,
                height: 2.0,
            },
        ),
        (
            "torus",
            PrimitiveSpec::Torus {
                origin: p(0.0, 0.0, 0.0),
                major_radius: 2.0,
                minor_radius: 0.5,
            },
        ),
    ];
    for (name, spec) in specs {
        let brep = body(execute(&modeler, SolidOperation::Primitive { spec }));
        let outcome = tessellate_exchange(
            &SolidExchange::Brep(brep),
            TessellationTolerance::default(),
            TessellationBudget::default(),
        );
        // `Success` means the repository's own tessellator found no missing face,
        // no open edge and no dropped shell: a real watertightness probe, not a
        // triangle count.
        match outcome {
            TessellationOutcome::Success { geometry, .. } => {
                assert!(!geometry.is_empty(), "{name}: empty mesh");
                assert!(geometry.triangle_count() >= 2, "{name}: too few triangles");
            }
            other => panic!("{name}: tessellation was not Success: {other:?}"),
        }
    }
}

#[test]
fn probe_in_flight_cancellation_stops_a_long_boolean() {
    let modeler: Arc<dyn SolidModeler> = Arc::new(SolidCoreModeler::new());
    // A prism this size keeps the kernel busy for seconds; the baseline measured
    // below is what makes the cancellation assertion meaningful.
    let heavy = prism(200, 1.0);
    let cut = body(execute(
        modeler.as_ref(),
        SolidOperation::Primitive {
            spec: box_spec(p(-2.0, -2.0, 0.25), 4.0, 4.0, 0.5),
        },
    ));
    // Measure the uncancelled cost so the assertion below proves **mid-flight**
    // cancellation rather than a completed call that the post-check rejected.
    let baseline_start = Instant::now();
    let uncancelled = execute_solid(
        modeler.as_ref(),
        &SolidRequest {
            stamp: stamp(),
            operation: SolidOperation::Boolean {
                op: BooleanOp::Subtract,
                left: &heavy,
                right: &cut,
            },
        },
        &|| false,
    );
    let baseline = baseline_start.elapsed();
    assert!(
        uncancelled.is_ok(),
        "the baseline boolean must succeed: {uncancelled:?}"
    );
    // The watcher's detection granularity is 50 ms (the crate's `POLL`), so the
    // baseline has to dwarf it: a workload that can finish inside a poll or two
    // would make "cancelled" indistinguishable from "completed, then rejected".
    assert!(
        baseline > Duration::from_millis(500),
        "the workload is too small to test cancellation: {baseline:?}"
    );

    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let worker_modeler = Arc::clone(&modeler);
    let started = Instant::now();
    let worker = std::thread::spawn(move || {
        let request = SolidRequest {
            stamp: stamp(),
            operation: SolidOperation::Boolean {
                op: BooleanOp::Subtract,
                left: &heavy,
                right: &cut,
            },
        };
        let cancelled = move || worker_cancel.load(Ordering::Relaxed);
        execute_solid(worker_modeler.as_ref(), &request, &cancelled)
    });
    std::thread::sleep(Duration::from_millis(40));
    cancel.store(true, Ordering::Relaxed);
    let result = worker.join().expect("worker thread");
    let elapsed = started.elapsed();
    assert_eq!(result.unwrap_err(), CadError::Cancelled);
    // Mid-flight, not after completion: the cancel was requested at 40 ms, the
    // watcher polls every 50 ms, so a stopped call returns in ~2 polls — far
    // inside half the baseline. (`elapsed < baseline - 2*POLL` is the weaker
    // form; half the baseline is asserted because the baseline is > 500 ms.)
    assert!(
        elapsed < baseline / 2,
        "the call was not stopped mid-flight: {elapsed:?} of a {baseline:?} boolean"
    );

    // The modeler is usable afterwards (a cancelled call leaves no state).
    let brep = body(execute(
        modeler.as_ref(),
        SolidOperation::Primitive {
            spec: box_spec(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0),
        },
    ));
    assert!((volume(modeler.as_ref(), &brep) - 1.0).abs() < 1e-9);
}

#[test]
fn probe_mass_survives_scale_and_offset_sweeps() {
    let modeler = SolidCoreModeler::new();
    for scale in [1e-3, 1e-2, 1.0, 1e2, 1e3, 1e5] {
        let brep = body(execute(
            &modeler,
            SolidOperation::Primitive {
                spec: box_spec(p(0.0, 0.0, 0.0), scale, scale, scale),
            },
        ));
        let outcome = execute(
            &modeler,
            SolidOperation::MassProperties {
                brep: &brep,
                density: 1.0,
            },
        );
        let mass = outcome
            .mass()
            .unwrap_or_else(|| panic!("scale {scale}: mass refused: {outcome:?}"));
        let expected_volume = scale * scale * scale;
        let volume_error = (mass.volume - expected_volume).abs() / expected_volume;
        assert!(
            volume_error < 1e-6,
            "scale {scale}: volume {} vs {expected_volume} (relative {volume_error})",
            mass.volume
        );
        let expected_area = 6.0 * scale * scale;
        let area_error = (mass.surface_area - expected_area).abs() / expected_area;
        assert!(
            area_error < 1e-5,
            "scale {scale}: area {} vs {expected_area} (relative {area_error})",
            mass.surface_area
        );
        assert!(
            (mass.centroid.x - scale / 2.0).abs() < 1e-6 * scale.max(1.0)
                && (mass.centroid.z - scale / 2.0).abs() < 1e-6 * scale.max(1.0),
            "scale {scale}: centroid {:?}",
            mass.centroid
        );
    }

    // A unit box a million units from the origin: the offset must not leak into
    // the volume or the centroid.
    let offset = 1e6;
    let brep = body(execute(
        &modeler,
        SolidOperation::Primitive {
            spec: box_spec(p(offset, offset, offset), 1.0, 1.0, 1.0),
        },
    ));
    let mass = *execute(
        &modeler,
        SolidOperation::MassProperties {
            brep: &brep,
            density: 1.0,
        },
    )
    .mass()
    .expect("mass product");
    assert!((mass.volume - 1.0).abs() < 1e-6, "volume {}", mass.volume);
    assert!(
        (mass.centroid.x - (offset + 0.5)).abs() < 1e-6,
        "centroid {:?}",
        mass.centroid
    );
}
