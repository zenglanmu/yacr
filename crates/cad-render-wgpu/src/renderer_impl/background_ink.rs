//! Display-time ink flag derived from the frame's clear colour.
//!
//! When the canvas clear colour is a light background, the colour shaders flip
//! near-white ink to black so default ACI-7 geometry stays readable on a white
//! theme. This is a *display* transform: it never changes stored entity colours,
//! the display representation or the scene cache. The per-frame uniform word it
//! keys off is `light.a`; `light.rgb` (the light direction) is never repurposed.

use super::*;

/// The frame's light uniform for one slot: the camera's light direction with
/// `a` replaced by [`background_is_light`]'s answer for this frame's clear
/// colour. `rgb` (the direction) is copied verbatim.
///
/// Every slot of the shared uniform layout carries this value, so all colour
/// shaders see the same flag; the id-pass/mask/image shaders ignore `light`
/// entirely (the transform lives only in the seven colour shaders).
pub(crate) fn light_with_background_flag(light: [f32; 4], clear_color: [f32; 4]) -> [f32; 4] {
    [
        light[0],
        light[1],
        light[2],
        if background_is_light(clear_color) {
            1.0
        } else {
            0.0
        },
    ]
}
