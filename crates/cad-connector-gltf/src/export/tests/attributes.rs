//! Written vertex attributes (`NORMAL`, `COLOR_0`) and their reported losses.

use super::*;

use cad_domain::{Completeness, SupportStatus};
use gltf::Semantic;

#[test]
fn complete_normals_and_colors_are_written() {
    let normal = point(0.0, 0.0, 1.0);
    let colors = vec![[128, 128, 128], [255, 255, 255], [0, 0, 0]];
    let mesh = Mesh {
        normals: vec![normal; 3],
        colors: colors.clone(),
        ..triangle_mesh()
    };
    let database = mesh_database(mesh);
    let output = export(GltfContainer::Glb, &database);
    assert!(matches!(output.report.completeness, Completeness::Complete));

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let buffers = buffer_bytes(&gltf);
    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    let normal_accessor = primitive.get(&Semantic::Normals).expect("NORMAL");
    let normals = read_vec3_f32(&gltf, &buffers, normal_accessor.index());
    // The CAD +Z normal rotates to glTF +Y.
    assert_eq!(normals[0], [0.0, 1.0, 0.0]);
    let color_accessor = primitive.get(&Semantic::Colors(0)).expect("COLOR_0");
    let written = read_vec4_f32(&gltf, &buffers, color_accessor.index());
    // glTF COLOR_0 is linear: stored sRGB 128 is 0.21586..., not 128/255.
    assert!(
        (written[0][0] - 0.215_86).abs() < 1e-4,
        "sRGB 128 must decode to linear ~0.21586, got {}",
        written[0][0]
    );
    assert_eq!(written[0][1], written[0][0]);
    assert_eq!(written[0][2], written[0][0]);
    assert_eq!(written[0][3], 1.0, "vertex alpha is always opaque");
    assert_eq!(written[1], [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(written[2], [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn vertex_colors_survive_an_export_import_round_trip() {
    let colors = vec![[10u8, 128, 255]; 3];
    let mesh = Mesh {
        colors: colors.clone(),
        ..triangle_mesh()
    };
    let database = mesh_database(mesh);
    let output = export(GltfContainer::Glb, &database);

    let drawing = import_bytes(output.bytes);
    let imported = imported_mesh(&drawing);
    assert_eq!(imported.colors.len(), 3);
    for (written, expected) in imported.colors.iter().zip(colors.iter()) {
        for channel in 0..3 {
            let delta = i32::from(written[channel]) - i32::from(expected[channel]);
            assert!(
                delta.abs() <= 1,
                "sRGB channel drifted past rounding: wrote {written:?} for {expected:?}"
            );
        }
    }
}

#[test]
fn vertex_colors_with_explicit_opacity_are_reported_partial() {
    // `COLOR_0` alpha is always written opaque, so an explicit entity opacity
    // below 1.0 cannot be stored and must not be a silent `Complete`.
    let mesh = Mesh {
        colors: vec![[10, 20, 30]; 3],
        ..triangle_mesh()
    };
    let database = database_with_render(
        vec![entity(7, "GltfMesh", SemanticGeometry::Mesh(mesh))],
        vec![(
            EntityId(7),
            EntityRenderAttributes {
                color: EntityColor::ByLayer,
                transparency: EntityTransparency::Explicit(0.5),
                ..Default::default()
            },
        )],
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
        .find(|entry| entry.code == "export.gltf.vertex_color_opacity_not_written")
        .expect("the unwritten vertex opacity is reported");
    assert_eq!(entry.status, SupportStatus::Partial);
    assert!(
        entry.message.contains("#7"),
        "the entry must name the entity: {}",
        entry.message
    );
    assert_eq!(
        output.report.counts.converted, 1,
        "one entity, one conversion entry"
    );

    // The vertex colours are still written, at the documented opaque alpha.
    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let buffers = buffer_bytes(&gltf);
    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    let colors = primitive.get(&Semantic::Colors(0)).expect("COLOR_0");
    for written in read_vec4_f32(&gltf, &buffers, colors.index()) {
        assert_eq!(written[3], 1.0, "vertex alpha is written opaque");
    }
    assert!(primitive.material().index().is_none());
}

#[test]
fn vertex_colors_with_non_finite_opacity_are_reported_partial() {
    // A non-finite opacity cannot be stored either; it is reported through the
    // same conversion instead of being silently treated as authored opacity.
    let mesh = Mesh {
        colors: vec![[10, 20, 30]; 3],
        ..triangle_mesh()
    };
    let database = database_with_render(
        vec![entity(1, "GltfMesh", SemanticGeometry::Mesh(mesh))],
        vec![(
            EntityId(1),
            EntityRenderAttributes {
                color: EntityColor::ByLayer,
                transparency: EntityTransparency::Explicit(f32::NAN),
                ..Default::default()
            },
        )],
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
        .find(|entry| entry.code == "export.gltf.vertex_color_opacity_not_written")
        .expect("a non-finite opacity is reported");
    assert_eq!(entry.status, SupportStatus::Partial);
    assert!(entry.message.contains("#1"));

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let buffers = buffer_bytes(&gltf);
    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    let colors = primitive.get(&Semantic::Colors(0)).expect("COLOR_0");
    for written in read_vec4_f32(&gltf, &buffers, colors.index()) {
        assert_eq!(written[3], 1.0, "the written alpha stays opaque");
    }
}

#[test]
fn vertex_colors_with_layer_transparency_keep_the_layer_note() {
    // A layer-driven opacity has no glTF representation; it stays the one
    // evidence note and never becomes a `Partial` conversion.
    let mesh = Mesh {
        colors: vec![[10, 20, 30]; 3],
        ..triangle_mesh()
    };
    let database = database_with_render(
        vec![entity(1, "GltfMesh", SemanticGeometry::Mesh(mesh))],
        vec![(
            EntityId(1),
            EntityRenderAttributes {
                color: EntityColor::ByLayer,
                transparency: EntityTransparency::ByLayer,
                ..Default::default()
            },
        )],
    );
    let output = export(GltfContainer::Glb, &database);

    let notes: Vec<_> = output
        .report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.gltf.layer_appearance_not_written")
        .collect();
    assert_eq!(notes.len(), 1, "the layer gap is reported once");
    assert_eq!(notes[0].status, SupportStatus::Unverified);
    assert!(
        !output
            .report
            .entries
            .iter()
            .any(|entry| entry.code == "export.gltf.vertex_color_opacity_not_written"),
        "a layer opacity is a note, not an explicit unwritten opacity"
    );
    assert_eq!(output.report.counts.converted, 0);
    assert!(matches!(output.report.completeness, Completeness::Complete));
}

#[test]
fn vertex_colors_with_full_explicit_opacity_stay_complete() {
    // An explicit opacity of exactly 1.0 is what `COLOR_0` alpha already is, so
    // nothing was converted.
    let mesh = Mesh {
        colors: vec![[10, 20, 30]; 3],
        ..triangle_mesh()
    };
    let database = database_with_render(
        vec![entity(1, "GltfMesh", SemanticGeometry::Mesh(mesh))],
        vec![(
            EntityId(1),
            EntityRenderAttributes {
                color: EntityColor::ByLayer,
                transparency: EntityTransparency::Explicit(1.0),
                ..Default::default()
            },
        )],
    );
    let output = export(GltfContainer::Glb, &database);

    assert!(matches!(output.report.completeness, Completeness::Complete));
    assert_eq!(output.report.counts.converted, 0);
    assert!(!output
        .report
        .entries
        .iter()
        .any(|entry| entry.code == "export.gltf.vertex_color_opacity_not_written"));
}

#[test]
fn partially_present_normals_and_colors_are_dropped_and_reported() {
    let mesh = Mesh {
        // Only half of the vertices carry a normal, and only one has a colour:
        // the writer must not invent the missing values.
        vertices: vec![
            point(1.0, 2.0, 3.0),
            point(2.0, 2.0, 3.0),
            point(1.0, 3.0, 3.0),
            point(2.0, 3.0, 3.0),
        ],
        triangles: vec![[0, 1, 2], [1, 3, 2]],
        normals: vec![point(0.0, 0.0, 1.0), point(0.0, 0.0, 1.0)],
        face_sources: Vec::new(),
        colors: vec![[1, 2, 3]],
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    };
    let database = mesh_database(mesh);
    let output = export(GltfContainer::Glb, &database);

    let codes: Vec<&str> = output
        .report
        .entries
        .iter()
        .map(|entry| entry.code.as_str())
        .collect();
    assert!(codes.contains(&"export.gltf.normals_dropped"));
    assert!(codes.contains(&"export.gltf.colors_dropped"));
    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));
    assert_eq!(output.report.counts.converted, 2);

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    assert!(primitive.get(&Semantic::Normals).is_none());
    assert!(primitive.get(&Semantic::Colors(0)).is_none());
}
