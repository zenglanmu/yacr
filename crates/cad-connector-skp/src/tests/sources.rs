//! Contract tests for SketchUp **source face identity** carried into
//! [`cad_domain::Mesh::face_sources`] (P-MESH-2).
//!
//! The fixtures are synthetic `skpwright::Model`s built in memory and walked by
//! [`crate::read_scene`]: no `.skp` file and no vendor product is involved.

use super::*;

/// Two unpainted, coplanar quads in one run: one material group with two
/// distinct source faces, four triangles.
fn two_face_model() -> skpwright::Model {
    skpwright::Model {
        geometry: vec![skpwright::GeometryRun {
            mesh: mesh_with(
                vec![
                    [0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [1.0, 1.0, 0.0],
                    [0.0, 1.0, 0.0],
                    [2.0, 0.0, 0.0],
                    [3.0, 0.0, 0.0],
                    [3.0, 1.0, 0.0],
                    [2.0, 1.0, 0.0],
                ],
                vec![face(vec![0, 1, 2, 3]), face(vec![4, 5, 6, 7])],
            ),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn every_triangle_carries_its_source_face_identity() {
    let model = two_face_model();
    let mut import = MeshImport::new(&request(Vec::new()), "SkpPreviewMesh").expect("import");
    let scene = read_scene(&model, &ImportLimits::default(), &|| false, &mut import)
        .expect("the model walks");

    assert_eq!(
        scene.placed.len(),
        1,
        "both unpainted quads share one group"
    );
    let mesh = &scene.placed[0].mesh;
    assert_eq!(mesh.triangles.len(), 4, "each quad fills two triangles");
    assert_eq!(
        mesh.face_sources.len(),
        mesh.triangles.len(),
        "one source entry per emitted triangle"
    );

    let keys: Vec<&str> = mesh
        .face_sources
        .iter()
        .map(|source| {
            source
                .as_ref()
                .expect("every emitted triangle names its source face")
                .source_key
                .as_str()
        })
        .collect();
    assert_eq!(
        keys,
        vec!["skp:face:0", "skp:face:0", "skp:face:1", "skp:face:1"],
        "the two triangles of a face share its key and the two faces differ"
    );
    for source in &mesh.face_sources {
        assert_eq!(
            source.as_ref().expect("source").topology_revision,
            cad_domain::Revision(0)
        );
    }
}

#[test]
fn a_skipped_face_adds_no_source_entries() {
    // The second face is collinear, so the filler refuses it: its absence must
    // not desynchronise `face_sources` from `triangles`.
    let mesh = mesh_with(
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
        ],
        vec![face(vec![0, 1, 2, 3]), face(vec![4, 5, 6])],
    );
    let model = skpwright::Model {
        geometry: vec![skpwright::GeometryRun {
            mesh,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut import = MeshImport::new(&request(Vec::new()), "SkpPreviewMesh").expect("import");
    let scene = read_scene(&model, &ImportLimits::default(), &|| false, &mut import)
        .expect("the model walks");

    let mesh = &scene.placed[0].mesh;
    assert_eq!(mesh.triangles.len(), 2, "only the quad fills");
    assert_eq!(mesh.face_sources.len(), mesh.triangles.len());
    assert!(mesh.face_sources.iter().all(|source| source
        .as_ref()
        .is_some_and(|s| s.source_key == "skp:face:0")));
    assert_eq!(scene.stats.faces_untriangulable, 1);
}

/// The same contract on a **real** SketchUp file when one is supplied.
///
/// ```text
/// YACR_TEST_SKP=/path/to/model.skp cargo test -p cad-connector-skp \
///   a_real_sketchup_model_carries_face_identity_when_supplied -- --nocapture
/// ```
///
/// An unset variable returns early; that skip is **not** evidence.
#[test]
fn a_real_sketchup_model_carries_face_identity_when_supplied() {
    let Ok(path) = std::env::var("YACR_TEST_SKP") else {
        return;
    };
    let bytes = std::fs::read(&path).expect("YACR_TEST_SKP points at a readable file");
    let drawing = SkpImporter::new()
        .import(&request(bytes), &|| false)
        .expect("a real model with visible geometry must import");

    let mut meshes = 0usize;
    let mut triangles = 0usize;
    let mut sources = 0usize;
    let mut keys = std::collections::BTreeSet::new();
    for entity in drawing.database.entities() {
        let mesh = match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => mesh,
            SemanticGeometry::Body3d(body) => &body.display.mesh,
            _ => continue,
        };
        if mesh.triangles.is_empty() {
            continue;
        }
        meshes += 1;
        triangles += mesh.triangles.len();
        assert_eq!(
            mesh.face_sources.len(),
            mesh.triangles.len(),
            "entity {}: every triangle must name one source face",
            entity.id.0
        );
        sources += mesh.face_sources.len();
        for source in &mesh.face_sources {
            let source = source
                .as_ref()
                .unwrap_or_else(|| panic!("entity {}: a triangle has no source", entity.id.0));
            assert_eq!(source.topology_revision, cad_domain::Revision(0));
            keys.insert(source.source_key.clone());
        }
    }

    eprintln!(
        "real SKP face identity: meshes={meshes} triangles={triangles} sources={sources} \
         distinct_keys={}",
        keys.len()
    );
    assert!(meshes > 0, "the real sample must yield preview meshes");
    assert_eq!(sources, triangles, "one source per triangle");
    assert!(!keys.is_empty(), "a real model must carry source face keys");
}
