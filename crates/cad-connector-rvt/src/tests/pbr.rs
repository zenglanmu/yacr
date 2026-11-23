//! glTF PBR scalar contract tests for the RVT preview lane (ADR-0039 D4).
//!
//! `rvt` 0.24.0 writes `metallicFactor`/`roughnessFactor` derived from the
//! material **name**; these tests pin that the reader imports the declared
//! pair, never substitutes glTF's spec default for an absent one, never clamps
//! an unusable pair, and names the heuristic provenance once per document.
//!
//! Test helpers live in the parent [`super`] module.

use super::*;
use crate::pbr::validate_pbr_factors;
use cad_connector_core::MeshImportOutcome;

// ---- Unit 2: glTF PBR scalars (ADR-0039 D4) ------------------------------

#[test]
fn a_declared_metallic_and_roughness_factor_becomes_mesh_pbr() {
    // `rvt` 0.24.0 writes both factors for every material, derived from the
    // material name; the reader imports them and stores only those two scalars.
    let scene = read_glb(&materials_glb_fixture(), &ImportLimits::default(), &|| {
        false
    })
    .expect("the synthetic materials GLB reads");

    let opaque = placed(&scene, "opaque");
    let pbr = opaque
        .mesh
        .pbr
        .clone()
        .expect("the fixture's material 0 declares metallicFactor 0 / roughnessFactor 0.6");
    assert_eq!(pbr.metallic, 0.0, "metallicFactor 0 is exact in f32/f64");
    assert!(
        (pbr.roughness - 0.6).abs() < f64::from(f32::EPSILON),
        "{}",
        pbr.roughness
    );
    assert_eq!(
        pbr.emissive,
        [0.0, 0.0, 0.0],
        "no emissive is invented for a source that declares none"
    );
    assert_eq!(pbr.occlusion_strength, 0.0);
    assert!(pbr.metallic_roughness_texture.is_none());
    assert!(pbr.normal_texture.is_none());
    assert!(pbr.emissive_texture.is_none());
    assert!(pbr.occlusion_texture.is_none());
    // The colour/alpha path is unchanged by the PBR channel.
    assert_eq!(opaque.color, Some([200, 100, 50]));
    assert_eq!(opaque.opacity, None);

    let blend = placed(&scene, "blend")
        .mesh
        .pbr
        .clone()
        .expect("material 1 declares both factors");
    assert!(
        (blend.roughness - 0.6).abs() < f64::from(f32::EPSILON),
        "{}",
        blend.roughness
    );
    // One node mesh, two materials: the merged entity keeps no PBR either,
    // because picking one material's factors would apply them to the other
    // primitive's triangles (already counted as a material conflict).
    assert_eq!(placed(&scene, "conflict").mesh.pbr, None);
    assert_eq!(scene.pbr_meshes, 2, "opaque and blend");
    assert_eq!(scene.pbr_ignored, 0);

    let (completeness, codes) = scene_report(&scene);
    assert!(
        codes.contains(&DIAG_PBR_HEURISTIC.to_string()),
        "the provenance of the factors is named once: {codes:?}"
    );
    assert!(
        !codes.contains(&DIAG_PBR_IGNORED.to_string()),
        "a declared, usable pair is not a loss: {codes:?}"
    );
    assert_eq!(
        completeness,
        cad_domain::Completeness::Complete,
        "import.rvt.pbr_heuristic is a note, not a loss"
    );
}

#[test]
fn an_undeclared_metallic_and_roughness_factor_is_read_from_the_json() {
    // The rule: the presence check reads the raw JSON chunk, because `gltf`
    // 1.4.1's typed view substitutes the spec default (1.0) for an absent key.
    // A material that declares neither factor stays `pbr: None` — glTF's
    // metallic default of 1.0 would draw an unlit CAD body as black metal.
    let (mut json, bin) = glb_parts(&materials_glb_fixture());
    let factors = json["materials"][0]["pbrMetallicRoughness"]
        .as_object_mut()
        .expect("the fixture's material 0 has a pbrMetallicRoughness object");
    factors.remove("metallicFactor");
    factors.remove("roughnessFactor");
    let scene = read_glb(&rebuild_glb(&json, &bin), &ImportLimits::default(), &|| {
        false
    })
    .expect("a material with no PBR scalar still reads");

    let opaque = placed(&scene, "opaque");
    assert_eq!(opaque.mesh.pbr, None);
    // The colour is still read: PBR presence is independent of the colour path.
    assert_eq!(opaque.color, Some([200, 100, 50]));
    // Material 1 still declares both, so the common `rvt` case is unaffected.
    assert_eq!(
        placed(&scene, "blend").mesh.pbr.as_ref().unwrap().metallic,
        0.0
    );
    assert_eq!(scene.pbr_meshes, 1);
    assert_eq!(scene.pbr_ignored, 0);
}

#[test]
fn a_half_declared_pbr_pair_keeps_the_gltf_default_for_the_other() {
    // Declaring one scalar is enough to create a `MeshPbr`; the other keeps the
    // glTF spec default the writer meant (`roughnessFactor = 1.0`), which is a
    // format value rather than an invented one.
    let (mut json, bin) = glb_parts(&materials_glb_fixture());
    let factors = json["materials"][0]["pbrMetallicRoughness"]
        .as_object_mut()
        .expect("the fixture's material 0 has a pbrMetallicRoughness object");
    factors.insert("metallicFactor".into(), json!(0.25));
    factors.remove("roughnessFactor");
    let scene = read_glb(&rebuild_glb(&json, &bin), &ImportLimits::default(), &|| {
        false
    })
    .expect("a half-declared pair still reads");
    assert_eq!(
        placed(&scene, "opaque").mesh.pbr,
        Some(MeshPbr {
            metallic: 0.25,
            roughness: 1.0,
            ..MeshPbr::default()
        })
    );
    assert_eq!(scene.pbr_ignored, 0);
}

#[test]
fn a_quarter_and_three_quarter_pair_reaches_the_imported_entity() {
    // The exact pair from ADR-0039 D4's contract: 0.25/0.75 is representable in
    // f32, so it must arrive in the stored mesh unchanged (no clamping, no
    // rounding through the domain's f64).
    let (mut json, bin) = glb_parts(&materials_glb_fixture());
    json["materials"][0]["pbrMetallicRoughness"]["metallicFactor"] = json!(0.25);
    json["materials"][0]["pbrMetallicRoughness"]["roughnessFactor"] = json!(0.75);
    let scene = read_glb(&rebuild_glb(&json, &bin), &ImportLimits::default(), &|| {
        false
    })
    .expect("the patched materials GLB reads");
    assert_eq!(
        placed(&scene, "opaque").mesh.pbr,
        Some(MeshPbr {
            metallic: 0.25,
            roughness: 0.75,
            ..MeshPbr::default()
        })
    );

    // And the database accepts it for a mesh with no UVs and no textures.
    let outcome = import_scene(&scene);
    let mut checked = 0;
    for entity in outcome.database.entities() {
        let cad_domain::SemanticGeometry::Mesh(mesh) = &entity.geometry else {
            continue;
        };
        if mesh.pbr
            == Some(MeshPbr {
                metallic: 0.25,
                roughness: 0.75,
                ..MeshPbr::default()
            })
        {
            checked += 1;
        }
    }
    assert_eq!(checked, 1, "exactly the `opaque` entity carries the pair");
    assert!(
        diagnostic_codes(&outcome).contains(&DIAG_PBR_HEURISTIC.to_string()),
        "the entity's factors are reported as rvt heuristics"
    );
}

#[test]
fn an_out_of_range_metallic_factor_drops_the_pbr_channel_and_marks_partial() {
    // ADR-0039 D4: never clamp. Material 0 is named by the `opaque` node and by
    // the `conflict` mesh's first primitive, so both primitives lose the pair.
    let (mut json, bin) = glb_parts(&materials_glb_fixture());
    json["materials"][0]["pbrMetallicRoughness"]["metallicFactor"] = json!(2.0);
    let scene = read_glb(&rebuild_glb(&json, &bin), &ImportLimits::default(), &|| {
        false
    })
    .expect("an out-of-range factor is a loss, not a refusal");

    assert_eq!(placed(&scene, "opaque").mesh.pbr, None);
    assert_eq!(
        placed(&scene, "opaque").color,
        Some([200, 100, 50]),
        "the colour path is untouched by a dropped PBR pair"
    );
    assert_eq!(
        scene.pbr_ignored, 2,
        "opaque and conflict's first primitive"
    );
    assert_eq!(scene.pbr_meshes, 1, "material 1's pair is still usable");

    let (completeness, codes) = scene_report(&scene);
    assert!(codes.contains(&DIAG_PBR_IGNORED.to_string()), "{codes:?}");
    assert!(
        matches!(completeness, cad_domain::Completeness::Partial(_)),
        "an unusable metallic/roughness pair is a named loss: {completeness:?}"
    );
}

#[test]
fn a_non_finite_metallic_or_roughness_factor_is_never_accepted() {
    // JSON cannot spell NaN, so the non-finite arm is pinned on the validator
    // the reader uses for every pair.
    assert_eq!(validate_pbr_factors(0.25, 0.75), Some((0.25, 0.75)));
    assert_eq!(validate_pbr_factors(0.0, 1.0), Some((0.0, 1.0)));
    assert_eq!(validate_pbr_factors(f32::NAN, 0.5), None);
    assert_eq!(validate_pbr_factors(0.5, f32::INFINITY), None);
    assert_eq!(validate_pbr_factors(0.5, f32::NEG_INFINITY), None);
    // Out of range on either scalar drops the whole pair, never one half.
    assert_eq!(validate_pbr_factors(2.0, 0.5), None);
    assert_eq!(validate_pbr_factors(0.5, -0.25), None);
}

// ---- Helpers -------------------------------------------------------------

/// Record a scene into a throwaway import and return its outcome.
///
/// A scene with meshes pushes each of them (so the database write is part of the
/// contract); an empty one pushes a single triangle, because
/// `MeshImport::finish` needs at least one entity.
fn import_scene(scene: &GlbScene) -> MeshImportOutcome {
    let req = request(fixture("upstream-metadata-scaffold-2024-no-geometry.rvt"));
    let mut import = MeshImport::new(&req, RVT_MESH_TYPE_KEY).expect("a scaffold request imports");
    if scene.meshes.is_empty() {
        let object = import.reserve_object().expect("reserve an object");
        import
            .push_mesh(
                object,
                cad_domain::Mesh {
                    vertices: vec![
                        cad_domain::Point3 {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        },
                        cad_domain::Point3 {
                            x: 1.0,
                            y: 0.0,
                            z: 0.0,
                        },
                        cad_domain::Point3 {
                            x: 0.0,
                            y: 1.0,
                            z: 0.0,
                        },
                    ],
                    triangles: vec![[0, 1, 2]],
                    ..cad_domain::Mesh::default()
                },
                cad_db::EntityColor::ByLayer,
            )
            .expect("push a triangle");
    } else {
        for placed in &scene.meshes {
            let object = import.reserve_object().expect("reserve an object");
            import
                .push_mesh_with_attributes(
                    object,
                    placed.mesh.clone(),
                    cad_db::EntityRenderAttributes {
                        clip: None,
                        color: placed
                            .color
                            .map_or(cad_db::EntityColor::ByLayer, cad_db::EntityColor::Explicit),
                        transparency: placed.opacity.map_or(
                            cad_db::EntityTransparency::ByLayer,
                            cad_db::EntityTransparency::Explicit,
                        ),
                        ..Default::default()
                    },
                )
                .expect("push an imported mesh");
        }
    }
    record_scene(&mut import, scene);
    import.finish().expect("finish the import")
}

fn diagnostic_codes(outcome: &MeshImportOutcome) -> Vec<String> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect()
}

/// Record a scene's diagnostics into a throwaway import.
fn scene_report(scene: &GlbScene) -> (cad_domain::Completeness, Vec<String>) {
    let outcome = import_scene(scene);
    let codes = diagnostic_codes(&outcome);
    (outcome.completeness, codes)
}
