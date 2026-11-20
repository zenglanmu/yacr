//! Raster-image loading contracts that need a document with real texture bytes.
//!
//! Split out of `tests.rs` (code-size gate): these drive
//! `ops::resources::load_images` through the document-aware loader (ADR-0038 D10).

use super::*;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_drawing_with_only_a_mesh_texture_still_loads_the_image_cache() {
    use cad_db::{DrawingDatabaseBuilder, Layer};
    use cad_domain::{DatabaseId, DocumentIdentity, LayerId, Revision, UnitContext};
    use std::sync::Arc;

    // ADR-0038 D10: a mesh texture is document bytes, not a file reference, so
    // `requested_images` is empty — the "any images?" branch must still fire or
    // a texture-only drawing would render (and report) as if it had none.
    const KEY: &str = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";
    // A 2x2 RGBA PNG (red, green, blue, dark), committed inline.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72,
        0xb6, 0x0d, 0x24, 0x00, 0x00, 0x00, 0x15, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x0c, 0x81, 0xf4, 0x7f, 0x2e, 0x11, 0xb9, 0xff, 0x00, 0x41, 0x71,
        0x07, 0x36, 0x40, 0xa9, 0x20, 0x1a, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
        0x42, 0x60, 0x82,
    ];

    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_texture(KEY, Arc::from(PNG))
        .expect("canonical key with real PNG bytes");
    let database = builder.finish().unwrap();
    let snapshot = database
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            Revision(0),
        )
        .unwrap();

    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller
        .install_snapshot(
            &snapshot,
            cad_app::PersistenceState::untitled(),
            Some("textured".into()),
        )
        .expect("the textured snapshot installs");

    assert!(
        images_requested(&controller).is_empty(),
        "the drawing references no image file"
    );
    assert!(
        cad_platform::images::has_document_textures(controller.drawing().unwrap().as_ref()),
        "the document owns a mesh texture"
    );

    let invocation = CliInvocation::new(CliOperation::FixedViewportRender, "demo.dwg");
    let (cache, report) = load_images(&invocation, &controller).unwrap();
    assert_eq!(report.requested, vec![KEY.to_string()]);
    assert_eq!(report.loaded, vec![KEY.to_string()]);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(report.bytes, 16, "2x2 RGBA decoded from the DB bytes");
    assert_eq!(
        cache.len(),
        1,
        "the document texture is cached under its canonical key"
    );
}
