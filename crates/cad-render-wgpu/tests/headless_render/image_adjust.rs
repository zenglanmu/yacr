//! Per-image display transfer (`IMAGEADJUST`) on a real offscreen frame.
//!
//! These are **software-Vulkan (lavapipe)** contract tests: they prove the
//! declared shader order realises the documented intent (monochrome luma →
//! contrast → brightness → fade toward the background), not that the result is
//! pixel-identical to AutoCAD's transfer curve. A real-GPU/vendor result is NOT
//! VERIFIED by this file.

use super::*;

/// Render one solid image with `adjust` and return the centre pixel plus the
/// background (top-left) pixel. The quad spans world `[-0.8, 0.8]`, so the
/// centre at `(32, 32)` is inside it.
fn render_adjusted(rgba: [u8; 4], adjust: ImageAdjust) -> Option<([u8; 4], [u8; 4])> {
    let gpu = gpu()?;
    let mut renderer = init(gpu);
    let mut cache = DecodedImageCache::new();
    cache
        .insert(
            ResourceKey("adjust".into()),
            solid_image(2, 2, rgba),
            &ResourceLimits::default(),
        )
        .expect("insert the solid image");
    let mut batch = image_batch("adjust");
    batch.adjust = adjust;
    renderer
        .upload_images(&[batch], &cache)
        .expect("upload the adjusted image");
    renderer
        .render(camera_2d(), &RenderTarget::new(64, 64))
        .expect("render the adjusted frame");
    let image = renderer.read_target_rgba().expect("read the frame");
    Some((image.pixel(32, 32), image.pixel(0, 0)))
}

/// `fade = 100` blends the image fully into the target's clear colour, so the
/// centre pixel equals the untouched background.
#[test]
fn fade_100_reaches_the_background_exactly() {
    let Some((centre, background)) = render_adjusted(
        [255, 0, 0, 255],
        ImageAdjust {
            fade: 100,
            ..ImageAdjust::default()
        },
    ) else {
        return;
    };
    assert_ne!(
        background,
        [255, 0, 0, 255],
        "the background is not the image"
    );
    assert_eq!(
        centre, background,
        "fade 100 must land exactly on the clear colour"
    );
}

/// `contrast = 0` collapses every channel to the same mid value, whatever the
/// texture's hue was. A red 255 texel proves the hue is gone.
#[test]
fn contrast_zero_flattens_to_mid_grey() {
    let Some((centre, _background)) = render_adjusted(
        [255, 0, 0, 255],
        ImageAdjust {
            contrast: 0,
            ..ImageAdjust::default()
        },
    ) else {
        return;
    };
    assert_eq!(
        (centre[0], centre[1], centre[2]),
        (centre[0], centre[0], centre[0]),
        "contrast 0 must flatten R=G=B, got {centre:?}"
    );
    // 0.5 linear encoded to an 8-bit sRGB target. The exact byte is the target
    // format's rounding, so the band is generous on purpose.
    assert!(
        (150..=220).contains(&centre[0]),
        "contrast 0 must land near mid grey, got {centre:?}"
    );
}

/// `MONOCHROME` converts the sampled colour to luma, so a saturated red texel
/// renders as a grey (`R = G = B`) that is not black.
#[test]
fn monochrome_renders_grey() {
    let Some((centre, _background)) = render_adjusted(
        [255, 0, 0, 255],
        ImageAdjust {
            monochrome: true,
            ..ImageAdjust::default()
        },
    ) else {
        return;
    };
    assert_eq!(
        (centre[0], centre[1], centre[2]),
        (centre[0], centre[0], centre[0]),
        "MONO must render R=G=B, got {centre:?}"
    );
    assert!(
        centre[0] > 20,
        "the red luma must not collapse to black, got {centre:?}"
    );
}

/// Brightness is a plain scale on the sampled colour: a dimmer image is darker
/// on every channel, and `brightness = 100` is brighter than the neutral 50.
#[test]
fn brightness_scales_the_sampled_colour() {
    let Some((neutral, _)) = render_adjusted([128, 128, 128, 255], ImageAdjust::default()) else {
        return;
    };
    let Some((dimmer, _)) = render_adjusted(
        [128, 128, 128, 255],
        ImageAdjust {
            brightness: 25,
            ..ImageAdjust::default()
        },
    ) else {
        return;
    };
    let Some((brighter, _)) = render_adjusted(
        [128, 128, 128, 255],
        ImageAdjust {
            brightness: 100,
            ..ImageAdjust::default()
        },
    ) else {
        return;
    };
    assert!(
        dimmer[0] < neutral[0] && neutral[0] < brighter[0],
        "brightness must scale monotonically: dim {dimmer:?} neutral {neutral:?} bright {brighter:?}"
    );
    assert_eq!(
        (brighter[0], brighter[1], brighter[2]),
        (brighter[0], brighter[0], brighter[0]),
        "a grey texel stays grey"
    );
}

/// The neutral transfer (50/50/0/off) is an exact identity on the sampled texel:
/// it must not introduce a visible tint or darkening.
#[test]
fn neutral_transfer_is_an_identity() {
    let Some((centre, background)) = render_adjusted([255, 0, 0, 255], ImageAdjust::default())
    else {
        return;
    };
    assert!(
        centre[0] > 240 && centre[1] < 15 && centre[2] < 15,
        "a neutral transfer draws the texture unchanged, got {centre:?}"
    );
    assert_eq!(background, [0, 0, 0, 255], "the CAD target clears to black");
}
