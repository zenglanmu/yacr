//! Wipeout mask rendering (ADR 0022 C1): pixel contracts on a real device.
//!
//! Why these tests exist. The renderer is a **single depth-tested pass**: meshes
//! compare depth `Less` and write it, lines `LessEqual` and write it. A filled
//! quad drawn later at the same world z is therefore *rejected* by the depth
//! test, so "draw the background colour on top" cannot work. The mask gets its
//! own final pass with `depth_compare: Always` and no depth write; if that pass
//! is removed these tests must fail (verified by temporarily disabling it).
//!
//! The assertions are pixels read back from the target, not batch counts.

use super::*;
use cad_domain::Point3;
use cad_scene::MaskBatch;

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn mask(boundary: Vec<Point3>) -> MaskBatch {
    MaskBatch {
        polygon: boundary.into(),
        draw_order: 0,
        sources: vec![source()],
    }
}

/// A scene with both geometry batches and mask draw items.
fn mask_scene(batches: Vec<RenderBatch>, masks: Vec<MaskBatch>) -> SceneDelta {
    let mut delta = scene(batches);
    delta.masks = masks;
    delta
}

/// One horizontal line at world `y = 0` spanning most of the view. It is a
/// single 1 px anti-aliased stroke, so it paints the two pixel rows straddling
/// `y = 0` (31 and 32) with partial coverage.
fn crossing_line() -> RenderBatch {
    lines_batch(vec![[-0.9, 0.0, 0.0], [0.9, 0.0, 0.0]])
}

/// Pixel row indices the `y = 0` line paints: the target is 64 px over world
/// `[-1, 1]` (see [`camera_2d`]), so `y = 0` is the boundary between rows 31
/// and 32 and both receive the stroke's anti-aliased coverage.
const LINE_ROWS: [u32; 2] = [31, 32];

/// The pixel column containing world `x` (target spans world `[-1, 1]`).
fn column(x: f64) -> u32 {
    (((x + 1.0) / 2.0) * 64.0).floor().clamp(0.0, 63.0) as u32
}

fn is_clear(pixel: [u8; 4], background: [u8; 4]) -> bool {
    pixel == background
}

#[test]
fn mask_rectangle_hides_a_crossing_line_and_paints_the_clear_colour() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&mask_scene(
            vec![crossing_line()],
            vec![mask(vec![
                point(-0.5, -0.25),
                point(0.5, -0.25),
                point(0.5, 0.25),
                point(-0.5, 0.25),
            ])],
        ))
        .expect("upload line + mask");
    assert_eq!(renderer.mask_count(), 1);
    assert!(
        renderer.mask_skips().is_empty(),
        "the rectangle mask must tessellate: {:?}",
        renderer.mask_skips()
    );

    let target = RenderTarget::new(64, 64);
    let stats = renderer.render(camera_2d(), &target).expect("render frame");
    assert_eq!(
        stats.mask_draws, 1,
        "the frame must run the mask pass exactly once for one resident mask"
    );

    let image = renderer.read_target_rgba().expect("read back frame");
    let background = image.pixel(0, 0);
    assert_eq!(
        background,
        [0, 0, 0, 255],
        "the documented clear colour is opaque black"
    );

    // Outside the mask the line is visible: the stroke is still rasterized.
    for row in LINE_ROWS {
        let outside = image.pixel(column(-0.85), row);
        assert_ne!(
            outside,
            background,
            "line ink must be present outside the mask at column {} row {row}, got {outside:?}",
            column(-0.85)
        );
    }

    // Inside the mask the line is gone and the region equals the clear colour.
    for row in LINE_ROWS {
        for x in [-0.4, 0.0, 0.4] {
            let inside = image.pixel(column(x), row);
            assert_eq!(
                inside,
                background,
                "mask interior must equal the clear colour at column {} row {row}, got {inside:?}",
                column(x)
            );
        }
    }
}

/// A concave (L-shaped) mask must fill the contour **exactly**: the notch of the
/// L is outside the polygon, so the line crossing it stays visible. A naive
/// triangle fan over the boundary would cover the notch and fail this test.
#[test]
fn concave_mask_leaves_its_notch_visible() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    // L: a bottom bar (y in [-0.8, -0.1], x in [-0.8, 0.2]) plus a left bar
    // (x in [-0.8, -0.1], y in [-0.1, 0.2]). The notch is x in [-0.1, 0.2],
    // y in [-0.1, 0.2].
    renderer
        .upload(&mask_scene(
            vec![crossing_line()],
            vec![mask(vec![
                point(-0.8, -0.8),
                point(0.2, -0.8),
                point(0.2, -0.1),
                point(-0.1, -0.1),
                point(-0.1, 0.2),
                point(-0.8, 0.2),
            ])],
        ))
        .expect("upload line + concave mask");
    assert_eq!(renderer.mask_count(), 1);
    assert!(renderer.mask_skips().is_empty());

    let target = RenderTarget::new(64, 64);
    let stats = renderer.render(camera_2d(), &target).expect("render frame");
    assert_eq!(stats.mask_draws, 1);
    let image = renderer.read_target_rgba().expect("read back frame");
    let background = image.pixel(0, 0);

    // Inside the left bar the mask wipes the line.
    for row in LINE_ROWS {
        let inside = image.pixel(column(-0.5), row);
        assert_eq!(
            inside,
            background,
            "the L's left bar must be wiped at column {} row {row}, got {inside:?}",
            column(-0.5)
        );
    }
    // In the notch (outside the polygon) the line survives.
    for row in LINE_ROWS {
        let notch = image.pixel(column(0.05), row);
        assert!(
            !is_clear(notch, background),
            "the concave notch must not be wiped (naive fan leak) at column {} row {row}, got {notch:?}",
            column(0.05)
        );
    }
    // Outside the L entirely the line also survives.
    for row in LINE_ROWS {
        let outside = image.pixel(column(0.6), row);
        assert!(
            !is_clear(outside, background),
            "line ink must survive outside the L at column {} row {row}, got {outside:?}",
            column(0.6)
        );
    }
}
