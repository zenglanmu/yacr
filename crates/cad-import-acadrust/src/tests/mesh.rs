//! Mesh honesty contract (SUBD / POLYFACE / POLYGON MESH).
//!
//! All three kinds used to report `Complete` while dropping data. `Complete` is
//! a claim that the display is faithful; for a mesh that is false — a SubD
//! surface is smoothed by `subdivision_level`, a polyface edge can be marked
//! invisible by a negative index, and a polygon mesh can be closed/smoothed.
//! The verdict is now `Partial` with named, actionable reasons, while the
//! produced triangles are unchanged.

use super::*;

fn builder<'a>(req: &'a ImportRequest, acad: &'a acadrust::CadDocument) -> ImporterBuilder<'a> {
    ImporterBuilder::new(req, acad, ReadStats::default(), compute_identity(&[]))
}

fn partial_reasons(completeness: &Completeness) -> Vec<String> {
    match completeness {
        Completeness::Partial(reasons) => reasons.clone(),
        other => panic!("expected Partial, got {other:?}"),
    }
}

fn assert_reason(reasons: &[String], needles: &[&str]) {
    for needle in needles {
        assert!(
            reasons.iter().any(|reason| reason.contains(needle)),
            "missing reason containing {needle:?}: {reasons:?}"
        );
    }
}

fn unit_quad() -> Vec<acadrust::types::Vector3> {
    vec![
        acadrust::types::Vector3::new(0.0, 0.0, 0.0),
        acadrust::types::Vector3::new(1.0, 0.0, 0.0),
        acadrust::types::Vector3::new(1.0, 1.0, 0.0),
        acadrust::types::Vector3::new(0.0, 1.0, 0.5),
    ]
}

#[test]
fn subd_mesh_is_partial_because_the_base_cage_is_drawn() {
    let req = request(Vec::new());
    let acad = acadrust::CadDocument::new();
    let mut builder = builder(&req, &acad);

    let mut mesh = acadrust::entities::Mesh::new();
    mesh.version = 2;
    mesh.blend_crease = true;
    mesh.subdivision_level = 3;
    mesh.override_option = 1;
    mesh.vertices = unit_quad();
    mesh.faces = vec![acadrust::entities::MeshFace::quad(0, 1, 2, 3)];
    let entity = EntityType::Mesh(mesh);
    let common = entity.common();

    let (geometry, completeness) = builder.convert(&entity, common);
    let SemanticGeometry::Mesh(mesh) = geometry else {
        panic!("a SUBD MESH stays a triangle mesh");
    };
    // Geometry is unchanged: one quad fans into two triangles. The base cage is
    // what is drawn; the smoothing is not applied.
    assert_eq!(mesh.triangles.len(), 2, "{:?}", mesh.triangles);
    assert_eq!(mesh.vertices.len(), 4);

    let reasons = partial_reasons(&completeness);
    assert_eq!(reasons.len(), 3, "{reasons:?}");
    // The headline loss: the smoothing level is ignored, so the base cage is
    // drawn rather than the smoothed subdivision surface.
    assert_reason(
        &reasons,
        &[
            "subdivision_level is ignored",
            "base cage is drawn rather than the smoothed subdivision surface",
        ],
    );
    // Creases and the schema/override bookkeeping are named separately.
    assert_reason(&reasons, &["blend_crease", "edges[].crease"]);
    assert_reason(&reasons, &["version and override_option"]);

    let reported: Vec<&str> = builder
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "import.subd_mesh_partial")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    for reason in &reasons {
        assert!(
            reported.contains(&reason.as_str()),
            "{reason} must be reported once: {reported:?}"
        );
    }
}

#[test]
fn polyface_mesh_is_partial_and_its_hidden_edge_flag_is_named() {
    let req = request(Vec::new());
    let acad = acadrust::CadDocument::new();
    let mut builder = builder(&req, &acad);

    let mut polyface = acadrust::entities::PolyfaceMesh::new();
    polyface.elevation = 0.25;
    polyface.smooth_surface = acadrust::entities::PolyfaceSmoothType::Cubic;
    polyface.vertices = unit_quad()
        .into_iter()
        .map(acadrust::entities::PolyfaceVertex::new)
        .collect();
    // A negative index marks the first edge invisible. The importer takes the
    // absolute value, so the edge is still drawn — an honest loss, not a fix.
    let mut face = acadrust::entities::PolyfaceFace::quad(1, 2, 3, 4);
    face.index1 = -1;
    face.color = Some(acadrust::types::Color::from_index(3));
    polyface.faces = vec![face];
    let entity = EntityType::PolyfaceMesh(polyface);
    let common = entity.common();

    let (geometry, completeness) = builder.convert(&entity, common);
    let SemanticGeometry::Mesh(mesh) = geometry else {
        panic!("a POLYFACE MESH stays a triangle mesh");
    };
    // Same triangles as before: the face still references vertex 1 (index 0),
    // which is exactly why the invisibility declaration must be reported.
    assert_eq!(mesh.triangles, vec![[0, 1, 2], [0, 2, 3]]);

    let reasons = partial_reasons(&completeness);
    assert_eq!(reasons.len(), 4, "{reasons:?}");
    assert_reason(
        &reasons,
        &[
            "face-edge visibility",
            "negative face index",
            "invisible edge",
            "hidden edges are drawn",
        ],
    );
    assert_reason(&reasons, &["per-face colour"]);
    assert_reason(
        &reasons,
        &["flags, smooth_surface, normal, elevation and thickness"],
    );
    assert_reason(&reasons, &["bulge, start/end width and id"]);

    let reported: Vec<&str> = builder
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "import.polyface_mesh_partial")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    for reason in &reasons {
        assert!(
            reported.contains(&reason.as_str()),
            "{reason} must be reported once: {reported:?}"
        );
    }
}

#[test]
fn polygon_mesh_is_partial_because_closed_and_smoothed_forms_are_flattened() {
    let req = request(Vec::new());
    let acad = acadrust::CadDocument::new();
    let mut builder = builder(&req, &acad);

    let mut grid = acadrust::entities::PolygonMeshEntity::new();
    grid.m_vertex_count = 2;
    grid.n_vertex_count = 2;
    grid.m_smooth_density = 8;
    grid.n_smooth_density = 8;
    grid.smooth_type = acadrust::entities::SurfaceSmoothType::Cubic;
    grid.flags |= acadrust::entities::PolygonMeshFlags::CLOSED_M
        | acadrust::entities::PolygonMeshFlags::CLOSED_N;
    grid.elevation = 0.5;
    grid.vertices = unit_quad()
        .into_iter()
        .map(acadrust::entities::PolygonMeshVertex::at)
        .collect();
    let entity = EntityType::PolygonMesh(grid);
    let common = entity.common();

    let (geometry, completeness) = builder.convert(&entity, common);
    let SemanticGeometry::Mesh(mesh) = geometry else {
        panic!("a POLYGON MESH stays a triangle mesh");
    };
    // A 2x2 grid is one quad, i.e. two triangles, whether or not it is closed:
    // the closed direction is not stitched and the smoothing is not applied.
    assert_eq!(mesh.triangles.len(), 2, "{:?}", mesh.triangles);

    let reasons = partial_reasons(&completeness);
    assert_eq!(reasons.len(), 3, "{reasons:?}");
    assert_reason(&reasons, &["CLOSED_M/CLOSED_N", "not stitched closed"]);
    assert_reason(
        &reasons,
        &[
            "smooth_type and m_smooth_density/n_smooth_density",
            "flat M x N grid",
        ],
    );
    assert_reason(&reasons, &["elevation and normal"]);

    let reported: Vec<&str> = builder
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "import.polygon_mesh_partial")
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    for reason in &reasons {
        assert!(
            reported.contains(&reason.as_str()),
            "{reason} must be reported once: {reported:?}"
        );
    }
}

#[test]
fn a_second_mesh_does_not_duplicate_the_reported_reasons() {
    let req = request(Vec::new());
    let acad = acadrust::CadDocument::new();
    let mut builder = builder(&req, &acad);

    for _ in 0..2 {
        let mut mesh = acadrust::entities::Mesh::new();
        mesh.subdivision_level = 1;
        mesh.vertices = unit_quad();
        mesh.faces = vec![acadrust::entities::MeshFace::quad(0, 1, 2, 3)];
        let entity = EntityType::Mesh(mesh);
        let common = entity.common();
        let (_, completeness) = builder.convert(&entity, common);
        assert_eq!(partial_reasons(&completeness).len(), 3);
    }

    assert_eq!(
        builder
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "import.subd_mesh_partial")
            .count(),
        3,
        "the three reasons are reported once per import, not once per mesh: {:?}",
        builder.diagnostics
    );
}
