use super::*;

// ---------------------------------------------------------------------------
// The session IBL switch (ADR-0044) is a live frame input.
//
// Two levels are contracted here:
//   * PURE-STATE (no GPU): a toggle repaints through the ordinary view path and
//     is **not** a camera change, so the runtime never arms the navigation pause
//     (`pause_after_page`) or the deferred grid refresh for it, and it never
//     rebuilds the scene.
//   * `#[ignore]` hardware: through the whole session facade (progressive walk
//     included), the next presented frame after a toggle is re-shaded without a
//     scene rebuild. Run it with
//     `cargo test -p cad-engine --locked -- --ignored`.
// ---------------------------------------------------------------------------

/// PURE-STATE (no GPU): the IBL toggle is a frame input, never a camera-only
/// change. `RenderSessionHandle::render` hands the renderer exactly
/// `navigation_pause_pending` as `pause_after_page`, so an armed pause would make
/// the toggled frame reuse the retained base page (invisible in the app).
#[test]
fn an_ibl_toggle_is_a_live_frame_input_that_never_pauses_the_walk() {
    let session = session_with_published_base();
    let view_revision = session.view_revision();
    let input_revision = session.input_revision();
    let mut view = session.view_snapshot();
    assert!(!view.ibl_enabled, "the legacy lighting is the default");

    view.ibl_enabled = true;
    assert!(session.apply_view_snapshot(view).unwrap());

    assert!(
        session.cad_frame_dirty(),
        "the toggled frame must repaint without any other input"
    );
    assert!(
        !session.navigation_pause_pending(),
        "an IBL toggle is not a camera gesture: pausing the progressive walk \
         after one page would present a retained base shaded with the old flag"
    );
    assert!(
        !session.grid_refresh_pending(),
        "an IBL toggle changes no camera-derived bounds, so no grid refresh is \
         deferred"
    );
    assert_eq!(
        session.input_revision(),
        input_revision,
        "an IBL toggle must never rebuild the scene"
    );
    assert_eq!(
        session.view_revision(),
        view_revision + 1,
        "the toggle advances the presented view revision exactly once"
    );
    assert!(
        session.preparation_current(),
        "the published scene stays current across the toggle"
    );

    // Round-trips back to the legacy lighting through the same path.
    let mut back = session.view_snapshot();
    back.ibl_enabled = false;
    assert!(session.apply_view_snapshot(back).unwrap());
    assert!(!session.view_snapshot().ibl_enabled);
    assert!(session.cad_frame_dirty());
    assert!(!session.navigation_pause_pending());
}

/// A filled quad whose material is an unlit metal: `metallic = 1` with a `-Z`
/// vertex normal makes the 2D headlight's `n_dot_l = 0`, so the legacy ambient
/// floor is `base * (1 - metallic) * AMBIENT = 0` (black). Only the environment
/// term can light it, which makes the toggle pixel-visible.
#[cfg(not(target_arch = "wasm32"))]
fn pbr_metal_quad_entity(id: u128) -> DbEntity {
    let half = 15.0;
    let vertices = vec![
        Point3 {
            x: -half,
            y: -half,
            z: 0.0,
        },
        Point3 {
            x: -half,
            y: half,
            z: 0.0,
        },
        Point3 {
            x: half,
            y: half,
            z: 0.0,
        },
        Point3 {
            x: half,
            y: -half,
            z: 0.0,
        },
    ];
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbMesh".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Mesh(Mesh {
            vertices,
            triangles: vec![[0, 2, 1], [0, 3, 2]],
            normals: vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: -1.0,
                };
                4
            ],
            face_sources: vec![None; 2],
            colors: Vec::new(),
            uvs: Vec::new(),
            texture: None,
            pbr: Some(cad_domain::MeshPbr {
                metallic: 1.0,
                roughness: 0.35,
                ..cad_domain::MeshPbr::default()
            }),
        }),
        draw_order: id as i64,
    }
}

/// DETERMINISTIC HARDWARE: the session presents a re-shaded frame on the toggle,
/// with the resident scene untouched (no identity rebuild, no re-upload, no
/// batch-set change) and one extra presented frame.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "hardware GPU: run explicitly on a real adapter (P620/Vulkan)"]
fn ibl_toggle_reshades_the_next_presented_frame_without_a_scene_rebuild() {
    let gpu = headless_gpu();
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    session.attach_device(gpu.device, gpu.queue).unwrap();

    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(pbr_metal_quad_entity(1)).unwrap();
    session.sync_drawing(Some(b.finish().unwrap().into()));
    // The sample must be geometry, not the reference grid/axes: both are overlay
    // batches drawn over the base.
    let mut overlays = session.overlays();
    overlays.visibility.grid = false;
    overlays.visibility.axes = false;
    session.set_overlays(overlays);
    session.prepare().unwrap();
    drive_uploads(&session);

    let size = [800.0, 600.0];
    let batches = renderer_batch_count(&session);
    let input_revision = session.input_revision();
    let frames = session.frames_rendered();

    // A point inside the quad and away from the origin's world axes (the camera
    // default is a 1 world-unit-per-pixel orthographic view centred on (0, 0)).
    let (px, py) = world_to_pixel_2d(size[0] as u32, size[1] as u32, [5.0, 5.0]);
    let at = ((py * size[0] as u32 + px) * 4) as usize;
    let sample = |pixels: &[u8]| [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]];
    let luma = |pixel: [u8; 4]| {
        0.2126 * f32::from(pixel[0]) + 0.7152 * f32::from(pixel[1]) + 0.0722 * f32::from(pixel[2])
    };

    assert!(!session.view_snapshot().ibl_enabled);
    let legacy_frame = session_frame(&session, size);
    let legacy = sample(&legacy_frame);
    assert!(
        luma(legacy) < 4.0,
        "the unlit metal is black under the legacy lighting (pixel {legacy:?})"
    );

    let mut view = session.view_snapshot();
    view.ibl_enabled = true;
    assert!(session.apply_view_snapshot(view).unwrap());
    assert!(session.cad_frame_dirty());
    let ibl_frame = session_frame(&session, size);
    let ibl = sample(&ibl_frame);

    assert!(
        luma(ibl) > luma(legacy) + 5.0,
        "the toggled frame must be re-shaded: legacy={legacy:?} ibl={ibl:?}"
    );
    assert_eq!(
        session.frames_rendered(),
        frames + 2,
        "both frames are real renders, so the toggle cannot present a reused one"
    );
    assert_eq!(
        session.input_revision(),
        input_revision,
        "the toggle must not rebuild the published scene"
    );
    assert_eq!(
        renderer_batch_count(&session),
        batches,
        "the resident batch set must be untouched by the toggle"
    );
}
