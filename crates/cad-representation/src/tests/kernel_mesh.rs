use super::*;

#[test]
fn kernel_mesh_becomes_a_display_mesh_with_kernel_source() {
    // A lone planar square is an open sheet: usable facets, reported as
    // Partial with open edges, never as a complete solid.
    let result = tessellate(SolidExchange::Brep(square_brep(false)));
    let rep = DisplayRepresentation::from_tessellation(&result, selection(), 1.0);
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    assert_eq!(rep.fragments.len(), 1);
    assert_eq!(rep.fragments[0].geometry_source, GeometrySource::KernelMesh);
    // Planar facets are exact, so the kernel reports analytic precision.
    assert_eq!(rep.fragments[0].precision, Precision::Analytic);
    match &rep.fragments[0].primitive {
        DisplayPrimitive::Mesh(mesh) => assert_eq!(mesh.triangles.len(), 2),
        _ => panic!("expected a mesh primitive"),
    }
}

#[test]
fn unsupported_kernel_result_stays_missing_not_empty_success() {
    let result = tessellate(SolidExchange::Sat(b"ACIS payload".to_vec()));
    let rep = DisplayRepresentation::from_tessellation(&result, selection(), 1.0);
    assert!(matches!(rep.completeness, Completeness::Missing(_)));
    assert!(rep.fragments.is_empty());
    assert!(rep
        .diagnostics
        .iter()
        .any(|d| d.code == "kernel.unsupported"));
}

#[test]
fn partial_kernel_result_reports_degradation_and_keeps_facets() {
    let result = tessellate(SolidExchange::Brep(square_brep(true)));
    let rep = DisplayRepresentation::from_tessellation(&result, selection(), 1.0);
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    assert_eq!(rep.fragments.len(), 1);
    assert!(rep
        .diagnostics
        .iter()
        .any(|d| d.code == cad_kernel_adapter::codes::KERNEL_MISSING_FACE));
}
