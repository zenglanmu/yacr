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

// ---- Import-time tessellation of the four ACIS entity types ----

fn builder<'a>(req: &'a ImportRequest, acad: &'a acadrust::CadDocument) -> ImporterBuilder<'a> {
    ImporterBuilder::new(req, acad, ReadStats::default(), compute_identity(&[]))
}

fn partial_reasons(completeness: &Completeness) -> Vec<String> {
    match completeness {
        Completeness::Partial(reasons) => reasons.clone(),
        other => panic!("expected Partial, got {other:?}"),
    }
}

/// Signed volume of a closed triangle mesh (divergence theorem).
fn mesh_volume(mesh: &cad_domain::Mesh) -> f64 {
    let mut volume = 0.0;
    for triangle in &mesh.triangles {
        let a = mesh.vertices[triangle[0] as usize];
        let b = mesh.vertices[triangle[1] as usize];
        let c = mesh.vertices[triangle[2] as usize];
        volume += a.x * (b.y * c.z - b.z * c.y) - a.y * (b.x * c.z - b.z * c.x)
            + a.z * (b.x * c.y - b.y * c.x);
    }
    volume / 6.0
}

fn mesh_bounds(mesh: &cad_domain::Mesh) -> (Point3, Point3) {
    let mut min = mesh.vertices[0];
    let mut max = min;
    for p in &mesh.vertices {
        min = Point3 {
            x: min.x.min(p.x),
            y: min.y.min(p.y),
            z: min.z.min(p.z),
        };
        max = Point3 {
            x: max.x.max(p.x),
            y: max.y.max(p.y),
            z: max.z.max(p.z),
        };
    }
    (min, max)
}

#[test]
fn supported_acis_solid_becomes_an_exact_body_with_a_tessellated_display() {
    let sat = include_str!("../../../../fixtures/acis/cube.sat");
    let acad = acadrust::CadDocument::new();
    let req = request(Vec::new());
    let mut builder = builder(&req, &acad);

    let entity = EntityType::Solid3D(acadrust::entities::Solid3D::from_sat(sat));
    let common = entity.common();
    let (geometry, completeness) = builder.convert(&entity, common);

    let SemanticGeometry::Body3d(body) = geometry else {
        panic!("a tessellated 3DSOLID must become a Body3d, got {geometry:?}");
    };
    assert_eq!(body.kind, BodyKind::Solid);
    // The lifted ACIS B-Rep is kept as the exact geometry, never replaced by
    // the facets.
    let brep = body
        .exact_brep()
        .expect("the lifted B-Rep must be the exact geometry");
    assert_eq!(brep.face_count(), 6, "a box has six faces");

    // The kernel's facets are the display cache.
    assert_eq!(
        body.display.mesh.triangles.len(),
        12,
        "a box tessellates to 12 triangles"
    );
    // The unit cube fixture spans [-1, 1] on every axis.
    let (min, max) = mesh_bounds(&body.display.mesh);
    for (actual, expected) in [
        (min.x, -1.0),
        (min.y, -1.0),
        (min.z, -1.0),
        (max.x, 1.0),
        (max.y, 1.0),
        (max.z, 1.0),
    ] {
        assert!(
            (actual - expected).abs() < 1e-9,
            "bounds {min:?}..{max:?} do not match the fixture extent"
        );
    }
    let volume = mesh_volume(&body.display.mesh);
    assert!(
        (volume - 8.0).abs() < 1e-9,
        "closed-mesh volume {volume} != analytic 8.0"
    );

    // The boundary polylines are kept instead of dropped.
    assert!(
        !body.display.edges.is_empty(),
        "the tessellated body must keep its boundary edges"
    );
    assert!(
        body.display
            .edges
            .iter()
            .all(|polyline| polyline.len() >= 2),
        "every display edge polyline has at least two points"
    );

    // The tolerance rule is a deliberate fraction of the body diagonal, and the
    // deflection actually requested is recorded on the display.
    let (tolerance, diagonal) = crate::solid::acis_tolerance(brep);
    assert!(
        (diagonal - 2.0 * 3.0_f64.sqrt()).abs() < 1e-9,
        "cube diagonal {diagonal}"
    );
    assert!(
        (tolerance.linear_deflection - diagonal * 1e-3).abs() < 1e-12,
        "tolerance {} must be 0.1% of {diagonal}",
        tolerance.linear_deflection
    );
    assert_eq!(body.display.tolerance, Some(tolerance.linear_deflection));

    // A planar box tessellates analytically: nothing is approximated and
    // nothing is missing. The support status is therefore `Verified`, which a
    // tessellated-only (no exact B-Rep) body may never claim.
    assert_eq!(body.display.precision, Precision::Analytic);
    assert_eq!(body.display.completeness, Completeness::Complete);
    assert_eq!(completeness, Completeness::Complete);
    assert_eq!(
        display_support(&SemanticGeometry::Body3d(body.clone())),
        (SupportStatus::Verified, SupportStatus::Verified)
    );
    assert!(
        !builder
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "import.acis_tessellated"),
        "a planar exact body reports no approximation: {:?}",
        builder.diagnostics
    );
}

#[test]
fn curved_acis_body_keeps_its_exact_geometry_and_declares_the_approximation() {
    let sat = include_str!("../../../../fixtures/acis/cone.sat");
    let acad = acadrust::CadDocument::new();
    let req = request(Vec::new());
    let mut builder = builder(&req, &acad);

    let entity = EntityType::Solid3D(acadrust::entities::Solid3D::from_sat(sat));
    let common = entity.common();
    let (geometry, completeness) = builder.convert(&entity, common);

    let SemanticGeometry::Body3d(body) = geometry else {
        panic!("a tessellated 3DSOLID must become a Body3d, got {geometry:?}");
    };
    assert_eq!(body.kind, BodyKind::Solid);
    assert!(body.is_exact(), "the cone fixture lifts to a known B-Rep");
    assert!(matches!(
        body.display.precision,
        Precision::Approximate {
            error_bound: Some(bound)
        } if bound > 0.0
    ));
    // A curved body's facets are an approximation; the reason names the
    // deflection actually requested and the display stays `Partial`.
    let deflection = body.display.tolerance.expect("a requested deflection");
    let reason = crate::support::acis_approximation_reason(deflection);
    let reasons = partial_reasons(&completeness);
    assert_eq!(reasons, vec![reason.clone()]);
    assert_eq!(body.display.completeness, completeness);
    let count = builder
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "import.acis_tessellated")
        .count();
    assert_eq!(count, 1, "the approximation is reported once per body");
    // Exact geometry exists, so the display is drawable without over-claiming
    // the facets are exact.
    assert_eq!(
        display_support(&SemanticGeometry::Body3d(body.clone())),
        (SupportStatus::Verified, SupportStatus::Verified)
    );
}

#[test]
fn acis_entity_classes_map_to_the_body_kind_they_prove() {
    use acadrust::entities::acis::primitives::build_box;
    use acadrust::entities::{AcisData, Body, Region, Solid3D, Surface};
    let sat = build_box([0.0, 0.0, 0.0], 2.0, 2.0, 2.0).to_sat_string();
    let surface = Surface {
        acis_data: AcisData::from_sat(&sat),
        ..Surface::default()
    };
    let cases = [
        (
            EntityType::Solid3D(Solid3D::from_sat(&sat)),
            "AcDb3dSolid",
            BodyKind::Solid,
        ),
        (
            EntityType::Region(Region::from_sat(&sat)),
            "AcDbRegion",
            BodyKind::Sheet,
        ),
        (
            EntityType::Body(Body::from_sat(&sat)),
            "AcDbBody",
            BodyKind::Unknown,
        ),
        (
            EntityType::Surface(Box::new(surface)),
            "AcDbSurface",
            BodyKind::Sheet,
        ),
    ];
    let acad = acadrust::CadDocument::new();
    let req = request(Vec::new());
    let mut builder = builder(&req, &acad);
    for (entity, type_key, kind) in cases {
        // The source entity class (the stored type key) is unchanged.
        assert_eq!(entity_class_name(&entity), type_key);
        let common = entity.common();
        let (geometry, _completeness) = builder.convert(&entity, common);
        let SemanticGeometry::Body3d(body) = geometry else {
            panic!("{type_key} must become a Body3d, got {geometry:?}");
        };
        assert_eq!(body.kind, kind, "{type_key}");
        assert!(body.is_exact(), "{type_key} carries the lifted B-Rep");
        assert!(!body.display.mesh.triangles.is_empty(), "{type_key}");
    }
}

#[test]
fn importing_a_curved_acis_solid_keeps_the_body_and_reports_the_approximation() {
    use acadrust::entities::Solid3D;
    use acadrust::io::dwg::dwg_writer::DwgWriter;

    let sat = include_str!("../../../../fixtures/acis/cone.sat");
    let mut doc = acadrust::CadDocument::new();
    doc.add_entity(EntityType::Solid3D(Solid3D::from_sat(sat)))
        .unwrap();
    let bytes = DwgWriter::write_to_vec(&doc).unwrap();

    let drawing = AcadrustImporter::new()
        .import(&request(bytes), &|| false)
        .expect("synthetic DWG with a 3DSOLID imports");

    let bodies: Vec<&cad_domain::Body3d> = drawing
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Body3d(body) => Some(body.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(bodies.len(), 1);
    let body = bodies[0];
    assert_eq!(body.kind, BodyKind::Solid);
    assert!(body.is_exact());
    // A curved body is drawable and exact, but its display is an approximation.
    assert!(matches!(
        body.display.precision,
        Precision::Approximate { .. }
    ));
    assert!(matches!(
        body.display.completeness,
        Completeness::Partial(_)
    ));
    assert!(
        !body.display.edges.is_empty(),
        "the cone's circular cap/seam polylines must survive"
    );

    let reason = crate::support::acis_approximation_reason(
        body.display.tolerance.expect("a requested deflection"),
    );
    let count = drawing
        .report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message == reason)
        .count();
    assert_eq!(count, 1, "the approximation is reported once per import");
}

#[test]
fn a_solid_without_exact_geometry_is_demoted_to_unknown() {
    use acadrust::entities::acis::primitives::build_box;
    use acadrust::entities::Solid3D;

    let sat_doc = build_box([0.0, 0.0, 0.0], 2.0, 2.0, 2.0);
    let sat = sat_doc.to_sat_string();
    let entity = EntityType::Solid3D(Solid3D::from_sat(&sat));

    // A `Solid` claim needs an exact B-rep; without one the class is demoted
    // (cad-db would reject a solid whose exact geometry is unavailable).
    assert_eq!(
        crate::support::acis_body_class(
            &entity,
            &BodyExact::Unavailable {
                reason: "test".into()
            }
        ),
        BodyKind::Unknown
    );
    assert_eq!(
        crate::support::acis_body_class(
            &entity,
            &BodyExact::Exact(Box::new(sat_to_brep(&sat_doc)))
        ),
        BodyKind::Solid
    );

    // A sheet never depends on exact geometry to keep its class.
    let region = EntityType::Region(acadrust::entities::Region::from_sat(&sat));
    assert_eq!(
        crate::support::acis_body_class(
            &region,
            &BodyExact::Unavailable {
                reason: "test".into()
            }
        ),
        BodyKind::Sheet
    );
}

#[test]
fn raw_unparseable_acis_stays_opaque_with_the_kernel_reason_and_no_mesh() {
    let acad = acadrust::CadDocument::new();
    let req = request(Vec::new());
    let mut builder = builder(&req, &acad);

    let entity = EntityType::Solid3D(acadrust::entities::Solid3D::from_sat(
        "this is not an ACIS payload",
    ));
    let common = entity.common();
    let (geometry, completeness) = builder.convert(&entity, common);

    assert!(
        matches!(geometry, SemanticGeometry::Opaque { .. }),
        "an unparseable payload must stay Opaque, got {geometry:?}"
    );
    let reasons = partial_reasons(&completeness);
    assert!(
        reasons
            .iter()
            .any(|reason| reason.starts_with("kernel.no_acis_kernel:")),
        "the kernel's own named reason must survive: {reasons:?}"
    );
    assert!(
        !matches!(completeness, Completeness::Complete),
        "a refusal is never Complete"
    );
    assert!(
        builder
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "import.acis_unsupported"),
        "the refusal must be reported as an import diagnostic"
    );
}

#[test]
fn importing_two_acis_solids_keeps_each_exact_body_and_its_source_class() {
    use acadrust::io::dwg::dwg_writer::DwgWriter;

    let sat = include_str!("../../../../fixtures/acis/cube.sat");
    let mut doc = acadrust::CadDocument::new();
    for _ in 0..2 {
        doc.add_entity(EntityType::Solid3D(acadrust::entities::Solid3D::from_sat(
            sat,
        )))
        .unwrap();
    }
    let bytes = DwgWriter::write_to_vec(&doc).unwrap();

    let drawing = AcadrustImporter::new()
        .import(&request(bytes), &|| false)
        .expect("synthetic DWG with 3DSOLIDs imports");

    let bodies: Vec<(&str, &cad_domain::Body3d)> = drawing
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Body3d(body) => {
                Some((entity.object.type_key.as_str(), body.as_ref()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(bodies.len(), 2, "both 3DSOLIDs must be imported as bodies");
    for (type_key, body) in bodies {
        // The source entity class survives as the stored type key.
        assert_eq!(type_key, "AcDb3dSolid");
        assert_eq!(body.kind, BodyKind::Solid);
        assert!(body.is_exact(), "the lifted B-Rep must be retained");
        assert_eq!(body.exact_brep().map(|brep| brep.face_count()), Some(6));
        assert_eq!(body.display.mesh.triangles.len(), 12);
        assert!(!body.display.edges.is_empty());
        // A planar solid tessellates analytically, so nothing is approximated.
        assert_eq!(body.display.precision, Precision::Analytic);
        assert_eq!(body.display.completeness, Completeness::Complete);
    }

    // The exact B-Rep is retained for a planar solid, and the approximation is
    // not among the reported import losses. (The report can still be `Partial`
    // for unrelated drawing-wide reasons such as an annotation scale.)
    assert!(
        !matches!(drawing.report.completeness, Completeness::Missing(_)),
        "{:?}",
        drawing.report.completeness
    );
    assert!(
        !drawing
            .report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "import.acis_tessellated"),
        "{:?}",
        drawing.report.diagnostics
    );
}
