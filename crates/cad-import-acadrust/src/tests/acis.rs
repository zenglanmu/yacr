//! ACIS neutral lift and kernel tessellation (F15 reachable subset).

use super::*;
// ---- ACIS neutral lift and kernel tessellation (F15 reachable subset) ----

use cad_kernel_adapter::{
    BrepSurface, BrepTessellator, GeometryHandle, SolidExchange, SolidTessellator,
    TessellationBudget, TessellationOutcome, TessellationRequest, TessellationResult,
    TessellationTolerance,
};

fn tess_brep(exchange: SolidExchange) -> TessellationResult {
    let request = TessellationRequest {
        geometry: GeometryHandle::Resolved(ObjectId(1)),
        exchange,
        tolerance: TessellationTolerance::default(),
        budget: TessellationBudget::default(),
        stamp: TaskStamp::new(DocumentId(1), 0),
    };
    BrepTessellator.tessellate(&request, &|| false).unwrap()
}

#[test]
fn acis_box_lifts_to_six_planar_faces_and_tessellates_closed() {
    use acadrust::entities::acis::primitives::build_box;
    let doc = build_box([0.0, 0.0, 0.0], 2.0, 2.0, 2.0);
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 6, "a box has six faces");
    assert!(brep
        .shells
        .iter()
        .flat_map(|s| &s.faces)
        .all(|f| matches!(f.surface, BrepSurface::Plane { .. })));

    let result = tess_brep(SolidExchange::Brep(brep));
    match result.outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert_eq!(geometry.triangle_count(), 12);
            let area = cad_kernel_adapter::brep::mesh_area(&geometry.mesh);
            assert!((area - 24.0).abs() < 1e-9, "box area {area}");
        }
        other => panic!("box must tessellate as Success, got {other:?}"),
    }
}

#[test]
fn solid3d_entity_round_trips_through_sat_text() {
    use acadrust::entities::acis::primitives::build_box;
    use acadrust::entities::Solid3D;
    let sat = build_box([1.0, 2.0, 3.0], 2.0, 4.0, 6.0).to_sat_string();
    let solid = Solid3D::from_sat(&sat);
    let exchange = solid_exchange_from_entity(&EntityType::Solid3D(solid))
        .expect("3DSOLID must expose an exchange");
    let SolidExchange::Brep(brep) = exchange else {
        panic!("a valid SAT payload must lift to a neutral B-rep");
    };
    assert_eq!(brep.face_count(), 6);
    assert!(matches!(
        tess_brep(SolidExchange::Brep(brep)).outcome,
        TessellationOutcome::Success { .. }
    ));
}

#[test]
fn sab_payload_lifts_through_the_binary_reader() {
    use acadrust::entities::acis::primitives::build_box;
    use acadrust::entities::acis::SabWriter;
    use acadrust::entities::Solid3D;
    let doc = build_box([0.0, 0.0, 0.0], 2.0, 2.0, 2.0);
    let sab = SabWriter::write(&doc);
    let brep = sab_to_brep(&sab).expect("SAB must decode");
    assert_eq!(brep.face_count(), 6);
    let solid = Solid3D::from_sab(sab);
    let exchange = solid_exchange_from_entity(&EntityType::Solid3D(solid)).unwrap();
    assert!(matches!(exchange, SolidExchange::Brep(_)));
}

#[test]
fn cylinder_caps_and_side_tessellate_closed() {
    use acadrust::entities::acis::primitives::build_cylinder;
    let doc = build_cylinder([0.0, 0.0, 0.0], 1.0, 3.0);
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 3, "two caps plus the side");
    assert!(brep.shells[0]
        .faces
        .iter()
        .any(|f| matches!(f.surface, BrepSurface::Cylinder { .. })));
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() > 0);
            // The curved side makes it an approximation with a bound.
            assert!(matches!(
                geometry.precision,
                Precision::Approximate {
                    error_bound: Some(_)
                }
            ));
        }
        other => panic!("cylinder must be a closed Success, got {other:?}"),
    }
}

#[test]
fn sphere_lifts_to_one_loopless_face_and_tessellates() {
    use acadrust::entities::acis::primitives::build_sphere;
    let doc = build_sphere([0.0, 0.0, 0.0], 2.0);
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 1);
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() >= 8);
        }
        other => panic!("sphere must be a closed Success, got {other:?}"),
    }
}

#[test]
fn cone_lifts_to_a_base_cap_and_a_lateral_face_that_tessellates_closed() {
    use acadrust::entities::acis::primitives::build_cone;
    let doc = build_cone([0.0, 0.0, 0.0], 1.0, 2.0);
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 2, "base cap plus lateral face");
    assert!(brep
        .shells
        .iter()
        .flat_map(|s| &s.faces)
        .any(|f| matches!(f.surface, BrepSurface::Cone { .. })));
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() > 0, "the cone must tessellate");
            // The curved lateral face makes it an approximation with a bound.
            assert!(matches!(
                geometry.precision,
                Precision::Approximate {
                    error_bound: Some(_)
                }
            ));
            // The planar base plus the lateral fan weld into a closed shell.
            let open = cad_kernel_adapter::brep::count_open_edges(&geometry.mesh);
            assert_eq!(open, 0, "cone shell must be watertight");
        }
        other => panic!("cone must be a closed Success, got {other:?}"),
    }
}

#[test]
fn torus_lifts_to_one_loopless_face_and_tessellates_closed() {
    use acadrust::entities::acis::primitives::build_torus;
    let doc = build_torus([0.0, 0.0, 0.0], 3.0, 1.0);
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 1);
    assert!(matches!(
        brep.shells[0].faces[0].surface,
        BrepSurface::Torus { .. }
    ));
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() > 0);
            assert!(matches!(
                geometry.precision,
                Precision::Approximate {
                    error_bound: Some(_)
                }
            ));
            let open = cad_kernel_adapter::brep::count_open_edges(&geometry.mesh);
            assert_eq!(open, 0, "torus seam must close");
        }
        other => panic!("torus must be a closed Success, got {other:?}"),
    }
}

#[test]
fn fixture_cone_is_a_closed_success() {
    let sat = include_str!("../../../../fixtures/acis/cone.sat");
    let doc = acadrust::entities::acis::SatDocument::parse(sat).expect("fixture parses");
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 2);
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() > 0);
            let open = cad_kernel_adapter::brep::count_open_edges(&geometry.mesh);
            assert_eq!(open, 0, "cone fixture must be watertight");
        }
        other => panic!("cone fixture must be a closed Success, got {other:?}"),
    }
}

#[test]
fn fixture_truncated_cone_is_a_closed_success() {
    let sat = include_str!("../../../../fixtures/acis/cone-truncated.sat");
    let doc = acadrust::entities::acis::SatDocument::parse(sat).expect("fixture parses");
    let brep = sat_to_brep(&doc);
    assert_eq!(
        brep.face_count(),
        3,
        "two circular caps plus the frustum side"
    );
    assert!(brep
        .shells
        .iter()
        .flat_map(|s| &s.faces)
        .any(|f| matches!(f.surface, BrepSurface::Cone { .. }) && f.loops.len() == 2));
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() > 0);
            let open = cad_kernel_adapter::brep::count_open_edges(&geometry.mesh);
            assert_eq!(open, 0, "truncated cone fixture must be watertight");
        }
        other => panic!("truncated cone fixture must be a closed Success, got {other:?}"),
    }
}

#[test]
fn fixture_torus_is_a_closed_success() {
    let sat = include_str!("../../../../fixtures/acis/torus.sat");
    let doc = acadrust::entities::acis::SatDocument::parse(sat).expect("fixture parses");
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 1);
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() > 0);
            let open = cad_kernel_adapter::brep::count_open_edges(&geometry.mesh);
            assert_eq!(open, 0, "torus fixture must be watertight");
        }
        other => panic!("torus fixture must be a closed Success, got {other:?}"),
    }
}

#[test]
fn fixture_cube_parses_to_a_closed_six_face_solid() {
    let sat = include_str!("../../../../fixtures/acis/cube.sat");
    let doc = acadrust::entities::acis::SatDocument::parse(sat).expect("fixture parses");
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 6);
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert_eq!(geometry.triangle_count(), 12);
            let area = cad_kernel_adapter::brep::mesh_area(&geometry.mesh);
            assert!((area - 24.0).abs() < 1e-9, "area {area}");
        }
        other => panic!("cube fixture must be a closed Success, got {other:?}"),
    }
}

#[test]
fn fixture_box_with_square_hole_tessellates_closed_with_holes() {
    let sat = include_str!("../../../../fixtures/acis/box-with-square-hole.sat");
    let doc = acadrust::entities::acis::SatDocument::parse(sat).expect("fixture parses");
    let brep = sat_to_brep(&doc);
    assert_eq!(brep.face_count(), 10, "two annuli plus eight walls");
    assert!(
        brep.shells
            .iter()
            .flat_map(|s| &s.faces)
            .any(|f| f.loops.len() == 2),
        "the annulus faces must carry an inner loop"
    );
    match tess_brep(SolidExchange::Brep(brep)).outcome {
        TessellationOutcome::Success { geometry, .. } => {
            assert!(geometry.triangle_count() > 0);
            // 2*(10*10) + 4*(10*4) - 2*(4*4) + 16*4 = 392.
            let area = cad_kernel_adapter::brep::mesh_area(&geometry.mesh);
            assert!((area - 392.0).abs() < 1e-9, "area {area}");
        }
        other => panic!("holed box must be a closed Success, got {other:?}"),
    }
}

#[test]
fn non_solid_entities_have_no_acis_exchange() {
    let line = EntityType::Line(acadrust::entities::Line::new());
    assert!(solid_exchange_from_entity(&line).is_none());
}

#[test]
fn region_body_and_surface_entities_share_the_acis_lift() {
    use acadrust::entities::acis::primitives::build_box;
    use acadrust::entities::{AcisData, Body, Region, Surface};
    let sat = build_box([0.0, 0.0, 0.0], 2.0, 2.0, 2.0).to_sat_string();
    let surface = Surface {
        acis_data: AcisData::from_sat(&sat),
        ..Surface::default()
    };
    let entities = [
        EntityType::Region(Region::from_sat(&sat)),
        EntityType::Body(Body::from_sat(&sat)),
        EntityType::Surface(Box::new(surface)),
    ];
    for entity in entities {
        match solid_exchange_from_entity(&entity) {
            Some(SolidExchange::Brep(brep)) => assert_eq!(brep.face_count(), 6),
            other => panic!("{entity:?} must lift to a B-rep, got {other:?}"),
        }
    }
}

#[test]
fn empty_acis_data_stays_raw_and_empty() {
    let empty = acadrust::entities::AcisData::new();
    let exchange = acis_exchange(&empty);
    assert!(exchange.is_empty());
    let result = tess_brep(exchange);
    assert!(matches!(result.outcome, TessellationOutcome::Failed { .. }));
}

#[test]
fn opaque_payload_retains_the_raw_acis_bytes() {
    let sat = "700 0 1 0\n@8 acadrust @8 ACIS 7.0 @24 Thu Jan 01 00:00:00 2023\n1e-06 1e-06\n-1 body $-1 $-1 $-1 $-1 #\n";
    let acis = acadrust::entities::AcisData::from_sat(sat);
    let (version, payload) = acis_raw_payload(&acis);
    assert_eq!(version, 1);
    assert!(!payload.is_empty());
}
