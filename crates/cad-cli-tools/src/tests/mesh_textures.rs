//! Textured-mesh contracts for the headless GPU paths (ADR-0038 D9/D10).
//!
//! The CLI used to upload raster image quads only, so a textured mesh was drawn
//! with its batch colour while `images.loaded` still listed the decoded key —
//! a frame that was not the document's frame. These drive
//! `ops::render::render_document` (the GPU half of `render`) on a document
//! installed in memory, because the texture bytes belong to the database and
//! this crate cannot write the carrier-owned `.ymodel` they would come from.
//!
//! They bind a real headless adapter and auto-skip when there is none
//! (`SKIP ... NOT a pass`), exactly like the other GPU contracts here. A
//! software adapter's pixels are layout evidence only, never a GPU or vendor
//! claim.

use super::*;

use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_domain::{
    DatabaseId, DocumentIdentity, EntityId, LayerId, Mesh, ObjectId, Point3, Revision,
    SemanticGeometry, SpaceId, UnitContext,
};
use std::sync::Arc;

/// A canonical content-addressed key: 32 lowercase hex + `.png`.
const KEY: &str = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";

/// A 2x2 RGBA PNG (red, green, blue, dark), committed inline so the test needs
/// no binary fixture.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xb6, 0x0d,
    0x24, 0x00, 0x00, 0x00, 0x15, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x0c, 0x81, 0xf4, 0x7f, 0x2e, 0x11, 0xb9, 0xff, 0x00, 0x41, 0x71, 0x07, 0x36, 0x40, 0xa9,
    0x20, 0x1a, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// Payload a canonical key may carry that is not a decodable image: the database
/// accepts it (a non-empty payload under a valid key), the loader records the
/// decode failure and leaves the key out of the cache.
const CORRUPT_PNG: &[u8] = b"\x89PNG\r\n\x1a\nthis is not an image";

/// No adapter means this machine cannot answer the question; skip rather than
/// report a failure that is not about the code under test.
fn adapter_available() -> bool {
    use cad_render_wgpu::headless::enumerate_adapters;
    use cad_render_wgpu::BackendPreference;

    let available = !enumerate_adapters(BackendPreference::Auto).is_empty();
    if !available {
        eprintln!("no wgpu adapter; skipping mesh-texture render contract — NOT a pass");
    }
    available
}

/// One triangle at the origin, optionally carrying UV0 and a texture key.
fn triangle(texture: Option<&str>) -> Mesh {
    Mesh {
        vertices: vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
        // UV0 is kept in the untextured control too: a UV mapping is a true
        // surface property (ADR-0038 D1), and only the texture key is dropped.
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        texture: texture.map(str::to_owned),
    }
}

/// A controller holding a real database: one triangle mesh plus optional
/// document-owned texture bytes (the mesh names [`KEY`] either way).
fn controller(texture_bytes: Option<&[u8]>, textured: bool) -> HostController {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .expect("layer");
    if let Some(bytes) = texture_bytes {
        builder
            .insert_texture(KEY, Arc::from(bytes))
            .expect("canonical key with a non-empty payload");
    }
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbMesh".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Mesh(triangle(textured.then_some(KEY))),
            draw_order: 0,
        })
        .expect("mesh entity");
    let database = builder.finish().expect("database");
    let snapshot = database
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            Revision(0),
        )
        .expect("snapshot");

    let mut controller = HostController::with_demo_document([640.0, 480.0]).unwrap();
    controller
        .install_snapshot(
            &snapshot,
            cad_app::PersistenceState::untitled(),
            Some("textured".into()),
        )
        .expect("the textured snapshot installs");
    controller
}

fn invocation() -> CliInvocation {
    let mut invocation = CliInvocation::new(CliOperation::FixedViewportRender, "demo.dwg");
    invocation.render_width = 64;
    invocation.render_height = 64;
    invocation.locale = Locale::En;
    invocation
}

/// The string entries of a JSON array (empty for a non-array / empty value).
fn names(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Regression test for the bug: a drawing with a textured mesh **and** its
/// document texture bytes must report the key as uploaded and resolved, not
/// merely as a decoded image.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_textured_mesh_uploads_its_document_texture() {
    // This test owns a device: see `gpu_test_guard`.
    let _gpu = gpu_test_guard();
    if !adapter_available() {
        return;
    }

    let result = render_document(&invocation(), &controller(Some(PNG), true))
        .expect("the textured render must succeed");
    let mesh = &result["mesh_textures"];

    assert_eq!(
        names(&mesh["requested"]),
        vec![KEY.to_string()],
        "the batch names its texture key: {mesh}"
    );
    assert_eq!(mesh["uploaded"], 1, "the texture became resident: {mesh}");
    assert!(
        mesh["bytes"].as_u64().unwrap() > 0,
        "uploaded texture bytes are reported: {mesh}"
    );
    assert!(
        names(&mesh["unresolved"]).is_empty(),
        "the key decoded from the document: {mesh}"
    );
    assert!(
        names(&mesh["fallbacks"]).is_empty(),
        "a resident texture is sampled, not fallen back from: {mesh}"
    );
    // The decode the `images` object already reported is the same texture.
    assert_eq!(names(&result["images"]["loaded"]), vec![KEY.to_string()]);

    // Pixel regression: an untextured batch is one flat fill, so sampling the
    // 2x2 texture must add colours the control frame does not have. This is the
    // assertion that fails when only `upload_images` runs.
    let control = render_document(&invocation(), &controller(Some(PNG), false))
        .expect("the untextured control render must succeed");
    let textured_colors = result["pixels"]["distinct_colors"].as_u64().unwrap();
    let control_colors = control["pixels"]["distinct_colors"].as_u64().unwrap();
    assert!(
        textured_colors > control_colors,
        "the textured frame must sample texels (distinct_colors {textured_colors} vs \
         control {control_colors})"
    );
    assert!(
        names(&control["mesh_textures"]["requested"]).is_empty(),
        "the control mesh carries no texture, so no mesh texture work is requested"
    );
}

/// A key the document cannot decode is never a silent fallback: it is requested,
/// not uploaded, and named by the upload report and/or the draw-time fallbacks.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn an_unresolvable_mesh_texture_is_named_not_silently_fallen_back() {
    // This test owns a device: see `gpu_test_guard`.
    let _gpu = gpu_test_guard();
    if !adapter_available() {
        return;
    }

    let result = render_document(&invocation(), &controller(Some(CORRUPT_PNG), true))
        .expect("the rest of the drawing must still render");
    let mesh = &result["mesh_textures"];

    assert_eq!(
        names(&mesh["requested"]),
        vec![KEY.to_string()],
        "the batch still asks for its texture: {mesh}"
    );
    assert_eq!(mesh["uploaded"], 0, "nothing could be uploaded: {mesh}");
    let unresolved = names(&mesh["unresolved"]);
    let fallbacks = names(&mesh["fallbacks"]);
    assert!(
        unresolved.contains(&KEY.to_string()) || fallbacks.contains(&KEY.to_string()),
        "the untextured batch must report its lost key in `unresolved` or \
         `fallbacks`: {mesh}"
    );
    // The decode failure itself is the `images` object's fact, not a hidden one.
    let failed = names(&result["images"]["failed"]);
    assert!(
        failed.iter().any(|entry| entry.contains(KEY)),
        "the decode failure is reported by the loader: {failed:?}"
    );
}

/// A drawing with no textured mesh asks for no mesh texture work at all: the
/// empty slice must be a no-op, not an error, and nothing may be uploaded.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_drawing_without_mesh_textures_does_no_texture_work() {
    // This test owns a device: see `gpu_test_guard`.
    let _gpu = gpu_test_guard();
    if !adapter_available() {
        return;
    }

    let result = render_document(&invocation(), &controller(None, false))
        .expect("an untextured drawing renders as before");
    let mesh = &result["mesh_textures"];

    assert!(names(&mesh["requested"]).is_empty(), "{mesh}");
    assert_eq!(mesh["uploaded"], 0);
    assert_eq!(mesh["bytes"], 0);
    assert!(names(&mesh["unresolved"]).is_empty(), "{mesh}");
    assert!(names(&mesh["fallbacks"]).is_empty(), "{mesh}");
}

/// The paper-space texture keys of a controller's model space, built through the
/// real provider (no GPU).
#[cfg(not(target_arch = "wasm32"))]
fn paper_texture_keys(controller: &HostController) -> Vec<String> {
    let drawing = controller.drawing().expect("an open drawing");
    let context = representation_context(controller, None);
    let registry = cad_representation::ProviderRegistry::with_default_provider();
    let mut keys = Vec::new();
    for entity in drawing.model_space() {
        let representation = registry
            .build_expanded(&drawing, entity, &context)
            .expect("the mesh builds a representation");
        keys.extend(representation_texture_keys(&representation));
    }
    keys
}

/// The vector plot (SVG/PDF) has no texture channel, so it must be able to name
/// the key it cannot write. This is the helper behind that diagnostic entry.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_vector_plot_can_name_a_mesh_texture_it_cannot_write() {
    assert_eq!(
        paper_texture_keys(&controller(Some(PNG), true)),
        vec![KEY.to_string()]
    );
    assert!(
        paper_texture_keys(&controller(Some(PNG), false)).is_empty(),
        "an untextured mesh names no texture, so the plot reports no loss"
    );
}
