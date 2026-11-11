//! **Regression fence: a background-only change must repaint the frame.**
//!
//! The UI theme writes the model/paper background (`#000000` dark,
//! `#ffffff` light) and the host republishes the frame's `clear_color`. Only the
//! **first page of a progressive cycle** clears the colour target, so a
//! background-only change has to restart that cycle. When the frame key omitted
//! the clear colour, the finished walk stayed at its end cursor, the page loaded
//! the retained colour and drew nothing new, and the presented frame kept the
//! old background. On a real host the theme then only appeared after a pan/zoom
//! restarted the cycle — the defect this test fences.
//!
//! The fence is the pixel: render the same camera twice, changing only the
//! target's clear colour, and the background pixel must follow it.
//!
//! A GPU is required; without an adapter every test skips with a clear message
//! rather than pretending to pass. Run serially:
//!
//! ```text
//! cargo test -p cad-render-wgpu --test headless_render --locked -- --test-threads=1
//! ```

use super::*;

/// Render until the progressive walk reports no pending page.
fn render_complete(renderer: &mut Renderer, target: &RenderTarget) {
    renderer.render(camera_2d(), target).expect("render frame");
    let mut pages = 0;
    while renderer.frame_pending() {
        renderer.render(camera_2d(), target).expect("render page");
        pages += 1;
        assert!(pages < 64, "the progressive walk never completed");
    }
}

#[test]
fn a_background_only_change_repaints_without_any_camera_change() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    // The native session renders progressively; a one-shot renderer always
    // clears, so the fence must run the same mode production uses.
    renderer.set_progressive_rendering(true);
    let delta = scene(vec![lines_batch(rectangle_lines())]);
    let staged = renderer
        .prepare_upload_batches(&delta.added)
        .expect("stage the rectangle outline");
    renderer
        .commit_upload(staged, 0)
        .expect("commit the rectangle outline");

    // Page 1 of the cycle clears the colour target, so the first frame is the
    // dark background the caller asked for.
    let dark = RenderTarget::new(64, 64).with_clear_color([0.0, 0.0, 0.0, 1.0]);
    render_complete(&mut renderer, &dark);
    let dark_background = renderer
        .read_target_rgba()
        .expect("read the dark frame")
        .pixel(2, 2);
    assert!(
        dark_background[0] <= 5 && dark_background[1] <= 5 && dark_background[2] <= 5,
        "the dark frame's background must be black, got {dark_background:?}"
    );

    // Same camera, same size, same scene — only the clear colour changes. This
    // is exactly the theme toggle's shape: no view change to restart the walk.
    let light = RenderTarget::new(64, 64).with_clear_color([1.0, 1.0, 1.0, 1.0]);
    render_complete(&mut renderer, &light);
    let light_background = renderer
        .read_target_rgba()
        .expect("read the light frame")
        .pixel(2, 2);
    assert!(
        light_background[0] >= 250 && light_background[1] >= 250 && light_background[2] >= 250,
        "a background-only change must clear the frame to the new colour; the corner still reads \
         {light_background:?}, which is the stale retained background"
    );
}
