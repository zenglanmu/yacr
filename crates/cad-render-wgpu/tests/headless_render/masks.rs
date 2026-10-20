//! Wipeout mask rendering (ADR 0022 C1/C2): pixel contracts on a real device.
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
    mask_with_mode(boundary, false)
}

/// `inverted` mirrors the DXF clip mode: `true` masks everything *outside* the
/// boundary.
fn mask_with_mode(boundary: Vec<Point3>, inverted: bool) -> MaskBatch {
    MaskBatch {
        polygon: boundary.into(),
        inverted,
        draw_order: 0,
        sources: vec![source()],
    }
}

/// The mask rectangle used by both crossing-line tests (identical geometry, so
/// the non-inverted and inverted assertions are mirror images).
fn mask_rectangle() -> Vec<Point3> {
    vec![
        point(-0.5, -0.25),
        point(0.5, -0.25),
        point(0.5, 0.25),
        point(-0.5, 0.25),
    ]
}

/// The concave L used by both concave tests: a bottom bar plus a left bar, with
/// the notch `x in [-0.1, 0.2], y in [-0.1, 0.2]` outside the polygon.
fn mask_l_shape() -> Vec<Point3> {
    vec![
        point(-0.8, -0.8),
        point(0.2, -0.8),
        point(0.2, -0.1),
        point(-0.1, -0.1),
        point(-0.1, 0.2),
        point(-0.8, 0.2),
    ]
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
            vec![mask(mask_rectangle())],
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
            vec![mask(mask_l_shape())],
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

/// The mirror image of the C1 rectangle assertion: an **inverted** mask wipes
/// everything *outside* its boundary and leaves the inside untouched.
///
/// The complement is produced without a stencil by adding a viewport-covering
/// ring to the same even-odd fill (ADR 0022 C2). A naive "fill the boundary"
/// (the non-inverted path) would leave the line visible outside and wipe it
/// inside, i.e. exactly invert these assertions.
#[test]
fn inverted_mask_wipes_outside_and_keeps_the_inside_visible() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&mask_scene(
            vec![crossing_line()],
            vec![mask_with_mode(mask_rectangle(), true)],
        ))
        .expect("upload line + inverted mask");
    assert_eq!(renderer.mask_count(), 1);

    let target = RenderTarget::new(64, 64);
    let stats = renderer.render(camera_2d(), &target).expect("render frame");
    assert_eq!(stats.mask_draws, 1);
    assert!(
        renderer.mask_skips().is_empty(),
        "the inverted mask must tessellate its complement: {:?}",
        renderer.mask_skips()
    );
    assert!(
        renderer.mask_vertex_count() > 0,
        "the first frame must build the inverted complement fill"
    );

    let image = renderer.read_target_rgba().expect("read back frame");
    let background = image.pixel(0, 0);
    assert_eq!(background, [0, 0, 0, 255]);

    // Outside the rectangle the line is erased: the region equals the clear
    // colour, on both rows the stroke painted before the mask pass.
    for row in LINE_ROWS {
        for x in [-0.85, 0.6] {
            let outside = image.pixel(column(x), row);
            assert_eq!(
                outside,
                background,
                "inverted mask must wipe outside at column {} row {row}, got {outside:?}",
                column(x)
            );
        }
    }

    // Inside the rectangle the line survives untouched.
    for row in LINE_ROWS {
        for x in [-0.4, 0.0, 0.4] {
            let inside = image.pixel(column(x), row);
            assert!(
                !is_clear(inside, background),
                "the inverted mask must leave its interior visible at column {} row {row}, got {inside:?}",
                column(x)
            );
        }
    }
}

/// The concave inverted case. Three distinct regions matter:
///
/// * inside the L's bars -> **visible** (inside the polygon: covered twice);
/// * outside the L entirely -> **wiped**;
/// * the notch -> **wiped**, because the notch is outside the polygon even
///   though it sits inside the L's bounding box.
///
/// A fan or an axis-aligned bounding-box approximation would get at least one of
/// these wrong.
#[test]
fn inverted_concave_mask_keeps_the_bars_and_wipes_the_notch() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut renderer = init(gpu);
    renderer
        .upload(&mask_scene(
            vec![crossing_line()],
            vec![mask_with_mode(mask_l_shape(), true)],
        ))
        .expect("upload line + inverted concave mask");
    let target = RenderTarget::new(64, 64);
    let stats = renderer.render(camera_2d(), &target).expect("render frame");
    assert_eq!(stats.mask_draws, 1);
    assert!(
        renderer.mask_skips().is_empty(),
        "the inverted L must tessellate its complement: {:?}",
        renderer.mask_skips()
    );

    let image = renderer.read_target_rgba().expect("read back frame");
    let background = image.pixel(0, 0);

    // Inside the left bar: visible.
    for row in LINE_ROWS {
        let bar = image.pixel(column(-0.5), row);
        assert!(
            !is_clear(bar, background),
            "the L's bar must stay visible at column {} row {row}, got {bar:?}",
            column(-0.5)
        );
    }
    // The notch is outside the polygon: wiped like the rest of the outside.
    for row in LINE_ROWS {
        let notch = image.pixel(column(0.05), row);
        assert_eq!(
            notch,
            background,
            "the notch is outside the polygon and must be wiped at column {} row {row}, got {notch:?}",
            column(0.05)
        );
    }
    // Outside the L entirely: wiped.
    for row in LINE_ROWS {
        for x in [0.6, -0.85] {
            let outside = image.pixel(column(x), row);
            assert_eq!(
                outside,
                background,
                "the inverted L must wipe outside at column {} row {row}, got {outside:?}",
                column(x)
            );
        }
    }
}
