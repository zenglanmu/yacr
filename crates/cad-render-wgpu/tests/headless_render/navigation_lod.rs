//! Navigation-time text LOD: text-glyph meshes disappear while the camera is
//! moving and return when it settles.
//!
//! The glyph class is derived from the representation primitive: a mesh with an
//! explicit index list and no edge overlay. This test uses the real software
//! Vulkan path (lavapipe on CI) and probes pixels, not a font shape.

use super::*;

/// A horizontal line at world `y = 0`, x in `[0.2, 0.8]`, from `source()`.
fn right_line() -> RenderBatch {
    lines_batch(vec![[0.2, 0.0, 0.0], [0.8, 0.0, 0.0]])
}

/// A text-glyph mesh on the left half (`x` in `[-1, 0]`): `quad_mesh` is a mesh
/// with a non-empty index list and no edge overlay, which is exactly the derived
/// glyph class.
fn left_glyph() -> RenderBatch {
    let mut batch = quad_mesh();
    batch.local_origin.x = -0.5;
    batch
}

fn pixel(image: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
    image.pixel(x, y)
}

fn is_lit(rgba: [u8; 4]) -> bool {
    rgba[0] > 8 || rgba[1] > 8 || rgba[2] > 8
}

/// Render the mixed scene and return `(glyph_pixel, line_pixel)`.
fn render_probes(renderer: &mut Renderer, target: &RenderTarget) -> ([u8; 4], [u8; 4]) {
    renderer
        .render(camera_2d(), target)
        .expect("render navigation-lod scene");
    let image = renderer
        .read_target_rgba()
        .expect("read navigation-lod scene");
    // World (-0.5, 0) -> pixel (16, 32); world (0.5, 0) -> pixel (48, 32).
    (pixel(&image, 16, 32), pixel(&image, 48, 32))
}

#[test]
fn text_glyphs_hide_during_navigation_and_return_when_settled() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&scene(vec![left_glyph(), right_line()]))
        .expect("upload glyph + line scene");
    let target = RenderTarget::new(64, 64);

    let (glyph, line) = render_probes(&mut renderer, &target);
    assert!(is_lit(glyph), "the glyph must be drawn before navigation");
    assert!(is_lit(line), "the non-glyph line must always be drawn");

    assert!(
        renderer.set_navigation_lod(true),
        "turning the LOD on must report a change"
    );
    assert!(
        !renderer.set_navigation_lod(true),
        "setting the same LOD state must not report a change"
    );
    let (glyph, line) = render_probes(&mut renderer, &target);
    assert!(
        !is_lit(glyph),
        "the glyph pixel must be absent while the camera is moving"
    );
    assert!(
        is_lit(line),
        "non-text geometry must stay visible during navigation"
    );

    assert!(
        renderer.set_navigation_lod(false),
        "turning the LOD off must report a change"
    );
    let (glyph, line) = render_probes(&mut renderer, &target);
    assert!(
        is_lit(glyph),
        "full text must return on the first settled frame"
    );
    assert!(
        is_lit(line),
        "the non-glyph line must survive the LOD cycle"
    );
}
