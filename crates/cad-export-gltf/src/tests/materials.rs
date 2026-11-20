//! Materials from explicit entity colours and the glTF round trip of them.

use super::*;

use cad_domain::{Completeness, SupportStatus};
use gltf::Semantic;

#[test]
fn explicit_entity_color_becomes_one_opaque_material() {
    let database = database_with_render(
        vec![entity(
            1,
            "GltfMesh",
            SemanticGeometry::Mesh(triangle_mesh()),
        )],
        vec![(EntityId(1), explicit([255, 0, 0], None))],
    );
    let output = export(GltfContainer::Glb, &database);
    assert!(matches!(output.report.completeness, Completeness::Complete));

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(
        gltf.materials().count(),
        1,
        "one explicit colour, one material"
    );
    let material = gltf.materials().next().unwrap();
    assert_eq!(material.alpha_mode(), gltf::material::AlphaMode::Opaque);
    let factor = material.pbr_metallic_roughness().base_color_factor();
    assert_eq!(factor, [1.0, 0.0, 0.0, 1.0]);

    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!(primitive.material().index(), Some(0));
    // No vertex colours, so the material is the only colour channel.
    assert!(primitive.get(&Semantic::Colors(0)).is_none());
}

#[test]
fn explicit_entity_color_with_half_opacity_becomes_a_blend_material() {
    let database = database_with_render(
        vec![entity(
            1,
            "GltfMesh",
            SemanticGeometry::Mesh(triangle_mesh()),
        )],
        vec![(EntityId(1), explicit([255, 0, 0], Some(0.5)))],
    );
    let output = export(GltfContainer::Glb, &database);
    assert!(matches!(output.report.completeness, Completeness::Complete));

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let material = gltf.materials().next().expect("one material");
    assert_eq!(material.alpha_mode(), gltf::material::AlphaMode::Blend);
    let factor = material.pbr_metallic_roughness().base_color_factor();
    assert_eq!(factor[0], 1.0);
    assert_eq!(factor[3], 0.5);
}

#[test]
fn identical_entity_colors_share_one_material() {
    let database = database_with_render(
        vec![
            entity(1, "GltfMesh", SemanticGeometry::Mesh(triangle_mesh())),
            entity(
                2,
                "GltfMesh",
                SemanticGeometry::Mesh(Mesh {
                    vertices: vec![
                        point(5.0, 0.0, 0.0),
                        point(6.0, 0.0, 0.0),
                        point(5.0, 1.0, 0.0),
                    ],
                    ..triangle_mesh()
                }),
            ),
        ],
        vec![
            (EntityId(1), explicit([255, 0, 0], None)),
            (EntityId(2), explicit([255, 0, 0], None)),
        ],
    );
    let output = export(GltfContainer::Glb, &database);

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(gltf.meshes().count(), 2);
    assert_eq!(gltf.materials().count(), 1, "the colour is deduplicated");
    for mesh in gltf.meshes() {
        let primitive = mesh.primitives().next().unwrap();
        assert_eq!(primitive.material().index(), Some(0));
    }
}

#[test]
fn by_layer_entity_color_writes_no_material_and_one_note() {
    // A database with no stored render attributes resolves to `ByLayer`.
    let database = mesh_database(triangle_mesh());
    let output = export(GltfContainer::Glb, &database);

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(gltf.materials().count(), 0);
    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    assert!(primitive.material().index().is_none());

    let notes: Vec<_> = output
        .report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.gltf.layer_appearance_not_written")
        .collect();
    assert_eq!(
        notes.len(),
        1,
        "the layer gap is reported once, not per entity"
    );
    assert_eq!(notes[0].status, SupportStatus::Unverified);
    // A note is evidence, not a loss: the verdict stays Complete.
    assert!(matches!(output.report.completeness, Completeness::Complete));
}

#[test]
fn vertex_colors_win_over_an_explicit_entity_color() {
    let mesh = Mesh {
        colors: vec![[10, 20, 30]; 3],
        ..triangle_mesh()
    };
    let database = database_with_render(
        vec![entity(1, "GltfMesh", SemanticGeometry::Mesh(mesh))],
        vec![(EntityId(1), explicit([255, 0, 0], None))],
    );
    let output = export(GltfContainer::Glb, &database);

    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));
    let entry = output
        .report
        .entries
        .iter()
        .find(|entry| entry.code == "export.gltf.entity_color_overridden")
        .expect("the overridden entity colour is reported");
    assert_eq!(entry.status, SupportStatus::Partial);

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(gltf.materials().count(), 0);
    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    assert!(primitive.get(&Semantic::Colors(0)).is_some());
    assert!(primitive.material().index().is_none());
}

#[test]
fn explicit_entity_color_survives_a_gltf_round_trip() {
    let database = database_with_render(
        vec![entity(
            1,
            "GltfMesh",
            SemanticGeometry::Mesh(triangle_mesh()),
        )],
        vec![(EntityId(1), explicit([10, 128, 255], Some(0.5)))],
    );
    let output = export(GltfContainer::Glb, &database);

    let drawing = import_bytes(output.bytes);
    let entity = drawing.database.entities().next().expect("one entity");
    let attributes = drawing.database.entity_render_attributes(entity.id);
    let EntityColor::Explicit(rgb) = attributes.color else {
        panic!(
            "the explicit colour was not restored: {:?}",
            attributes.color
        );
    };
    for (written, expected) in rgb.iter().zip([10u8, 128, 255].iter()) {
        let delta = i32::from(*written) - i32::from(*expected);
        assert!(
            delta.abs() <= 1,
            "sRGB channel drifted past rounding: wrote {rgb:?}"
        );
    }
    let EntityTransparency::Explicit(alpha) = attributes.transparency else {
        panic!(
            "the explicit opacity was not restored: {:?}",
            attributes.transparency
        );
    };
    assert!((alpha - 0.5).abs() < 1e-6, "opacity became {alpha}");
}
