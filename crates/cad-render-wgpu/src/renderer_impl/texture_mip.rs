//! CPU mip-chain generation for mesh material textures.
//!
//! Split out of `images.rs` to keep both files inside the 1000-line gate; the
//! upload path ([`super::images`]) and the GPU contract
//! (`crate::mesh_mip_gpu_tests`) share this one builder, so the test can never
//! generate levels with different rules than the renderer.
//!
//! Why the CPU: the bytes are already decoded here, the result is deterministic,
//! and there is no extra render pass, no readback and no per-backend variance.

use super::*;

/// How a generated mip level filters its source texels.
///
/// The role is a property of the **channel**, not of the upload: an sRGB albedo
/// must not be averaged in encoded space (the result is too dark), and a normal
/// map must not be averaged without renormalising (the result is not a normal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MipRole {
    /// Colour (base colour, emissive). The bytes are sRGB-encoded, so the 2×2
    /// footprint is decoded to **linear light**, averaged and re-encoded.
    /// Averaging the encoded bytes instead would darken every level.
    SrgbColour,
    /// Tangent-space normal. The footprint is averaged in `[-1, 1]` and then
    /// **renormalised**, so every level stays a unit normal with a sane
    /// (outward, non-zero) `z`.
    Normal,
    /// Linear scalar data (metallic-roughness, occlusion): plain box average.
    Linear,
}

/// One generated mip level (`1..`), tightly packed RGBA8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MipLevel {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
}

/// Full mip chain length for a `width × height` texture:
/// `floor(log2(max(width, height))) + 1`, so the last level is 1×1 and no level
/// is ever zero-sized.
pub(crate) fn mip_level_count(width: u32, height: u32) -> u32 {
    // `u32::BITS - leading_zeros` is `floor(log2(max)) + 1` for `max >= 1`.
    u32::BITS - width.max(height).max(1).leading_zeros()
}

/// Exact device texel bytes of an RGBA8 texture with the **full** mip chain:
/// `Σ max(1, w >> i) × max(1, h >> i) × 4`.
///
/// Used for the pre-flight budget request. Deliberately not
/// `gpu_memory::texture_alloc_bytes`, which treats every level as full size
/// (conservative for a one-level texture, wildly oversized for a chain).
pub(crate) fn mipped_rgba8_bytes(width: u32, height: u32) -> u64 {
    let mut total = 0u64;
    let (mut w, mut h) = (width.max(1), height.max(1));
    loop {
        total += u64::from(w) * u64::from(h) * 4;
        if w == 1 && h == 1 {
            return total;
        }
        w = (w >> 1).max(1);
        h = (h >> 1).max(1);
    }
}

/// Decode one sRGB-encoded channel to linear light (the inverse of the
/// `Rgba8UnormSrgb` sample transfer).
fn srgb_to_linear(encoded: u8) -> f32 {
    let encoded = f32::from(encoded) / 255.0;
    if encoded <= 0.040_45 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

/// Encode one linear channel the way `Rgba8UnormSrgb` stores it.
fn linear_to_srgb_byte(linear: f32) -> u8 {
    let linear = linear.clamp(0.0, 1.0);
    let encoded = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Round a `0..=1` unit value to a byte.
fn unit_to_byte(value: f32) -> u8 {
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Round a `-1..=1` normal component to a byte (`0.5` bias, so 0 → 128).
fn normal_to_byte(value: f32) -> u8 {
    ((value * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Normalise an averaged tangent-space normal.
///
/// A footprint of opposing normals can cancel to (near) zero, and a normalised
/// `z <= 0` would point into the surface: both are replaced by the flat
/// `(0, 0, 1)` "no perturbation" normal rather than emitting a degenerate
/// direction into the shader.
fn renormalise_normal(x: f32, y: f32, z: f32) -> (f32, f32, f32) {
    let length = (x * x + y * y + z * z).sqrt();
    // `is_nan` first: a NaN footprint must not reach the shader either.
    if length.is_nan() || length <= 1e-6 {
        return (0.0, 0.0, 1.0);
    }
    let (nx, ny, nz) = (x / length, y / length, z / length);
    if nz <= 0.0 {
        return (0.0, 0.0, 1.0);
    }
    (nx, ny, nz)
}

/// Box-filter `src` (`width × height`, tightly packed RGBA8) into one level of
/// `max(1, width >> 1) × max(1, height >> 1)`, applying `role`'s filtering rule.
fn downsample_level(src: &[u8], width: u32, height: u32, role: MipRole) -> MipLevel {
    let out_width = (width >> 1).max(1);
    let out_height = (height >> 1).max(1);
    let mut rgba = vec![0u8; (out_width as usize) * (out_height as usize) * 4];
    for y in 0..out_height {
        // An odd row count leaves the last destination row a 1-tall footprint
        // instead of reading past the source; the same holds for columns.
        let y0 = (y * 2).min(height - 1);
        let y1 = ((y + 1) * 2).min(height);
        for x in 0..out_width {
            let x0 = (x * 2).min(width - 1);
            let x1 = ((x + 1) * 2).min(width);
            let mut sum = [0.0f32; 4];
            let mut samples = 0.0f32;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let at = (((sy * width) + sx) * 4) as usize;
                    let texel = [src[at], src[at + 1], src[at + 2], src[at + 3]];
                    match role {
                        MipRole::SrgbColour => {
                            // RGB is sRGB-encoded; alpha is always linear.
                            sum[0] += srgb_to_linear(texel[0]);
                            sum[1] += srgb_to_linear(texel[1]);
                            sum[2] += srgb_to_linear(texel[2]);
                            sum[3] += f32::from(texel[3]) / 255.0;
                        }
                        MipRole::Normal => {
                            // Bytes decode to `[-1, 1]` (128 → 0), and the
                            // decode is affine, so a byte average would already
                            // be the component average — but the sum stays in
                            // the decoded domain so the renormalisation below is
                            // unambiguous.
                            sum[0] += f32::from(texel[0]) / 127.5 - 1.0;
                            sum[1] += f32::from(texel[1]) / 127.5 - 1.0;
                            sum[2] += f32::from(texel[2]) / 127.5 - 1.0;
                            sum[3] += f32::from(texel[3]) / 255.0;
                        }
                        MipRole::Linear => {
                            for (slot, value) in sum.iter_mut().zip(texel) {
                                *slot += f32::from(value);
                            }
                        }
                    }
                    samples += 1.0;
                }
            }
            let at = (((y * out_width) + x) * 4) as usize;
            match role {
                MipRole::SrgbColour => {
                    rgba[at] = linear_to_srgb_byte(sum[0] / samples);
                    rgba[at + 1] = linear_to_srgb_byte(sum[1] / samples);
                    rgba[at + 2] = linear_to_srgb_byte(sum[2] / samples);
                    rgba[at + 3] = unit_to_byte(sum[3] / samples);
                }
                MipRole::Normal => {
                    let (nx, ny, nz) =
                        renormalise_normal(sum[0] / samples, sum[1] / samples, sum[2] / samples);
                    rgba[at] = normal_to_byte(nx);
                    rgba[at + 1] = normal_to_byte(ny);
                    rgba[at + 2] = normal_to_byte(nz);
                    rgba[at + 3] = unit_to_byte(sum[3] / samples);
                }
                MipRole::Linear => {
                    for (offset, value) in sum.iter().enumerate() {
                        rgba[at + offset] = (value / samples).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }
    MipLevel {
        width: out_width,
        height: out_height,
        rgba,
    }
}

/// Generate every mip level **after** level 0 on the CPU.
///
/// Returns `mip_level_count(width, height) - 1` levels: level 1 is
/// `max(1, width >> 1) × max(1, height >> 1)`, and the chain ends at a single
/// 1×1 level. CPU generation is deliberate: the bytes are already decoded here,
/// the result is deterministic, and no backend-dependent render pass is needed.
pub(crate) fn generate_mip_levels(
    rgba: &[u8],
    width: u32,
    height: u32,
    role: MipRole,
) -> Vec<MipLevel> {
    debug_assert_eq!(rgba.len(), (width as usize) * (height as usize) * 4);
    let mut levels: Vec<MipLevel> = Vec::new();
    let mut width = width.max(1);
    let mut height = height.max(1);
    while width > 1 || height > 1 {
        let source = levels.last().map_or(rgba, |level| level.rgba.as_slice());
        let level = downsample_level(source, width, height, role);
        width = level.width;
        height = level.height;
        levels.push(level);
    }
    levels
}

/// Write one tightly packed RGBA8 level with `write_texture`, padding each row to
/// wgpu's 256-byte copy alignment. Returns the padded byte count copied, which is
/// what the upload report accounts for.
pub(crate) fn write_texture_level(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    level: u32,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> usize {
    // `write_texture` needs the row stride padded to the 256-byte copy
    // alignment; the source rows are tightly packed, so copy through a padded
    // scratch buffer only when the stride is not already aligned.
    let row_bytes = width as usize * 4;
    let aligned = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded;
    let data: &[u8] = if aligned == row_bytes {
        rgba
    } else {
        let mut buffer = vec![0u8; aligned * height as usize];
        for row in 0..height as usize {
            buffer[row * aligned..row * aligned + row_bytes]
                .copy_from_slice(&rgba[row * row_bytes..(row + 1) * row_bytes]);
        }
        padded = buffer;
        &padded
    };
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(aligned as u32),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    aligned * height as usize
}

#[cfg(test)]
mod mip_tests {
    use super::*;

    /// A black/white 2×2 checkerboard, opaque.
    fn checkerboard_2x2() -> Vec<u8> {
        vec![
            0, 0, 0, 255, 255, 255, 255, 255, //
            255, 255, 255, 255, 0, 0, 0, 255, //
        ]
    }

    #[test]
    fn the_chain_has_floor_log2_of_the_largest_side_plus_one_levels() {
        for (width, height) in [
            (1u32, 1u32),
            (2, 2),
            (3, 5),
            (64, 64),
            (2048, 2048),
            (2048, 512),
        ] {
            let expected = u32::BITS - width.max(height).leading_zeros();
            assert_eq!(
                mip_level_count(width, height),
                expected,
                "chain length for {width}x{height}"
            );
            let rgba = vec![0u8; (width * height * 4) as usize];
            let levels = generate_mip_levels(&rgba, width, height, MipRole::Linear);
            // Level 0 lives in `decoded`; the builder returns the rest.
            assert_eq!(
                1 + levels.len() as u32,
                expected,
                "1 + generated levels == chain length for {width}x{height}"
            );
            if expected == 1 {
                // A 1×1 source is already the last level: there is nothing to
                // generate, and the chain length is still 1.
                assert!(levels.is_empty(), "a 1x1 texture has no extra levels");
            } else {
                let last = levels.last().expect("a chain longer than one level");
                assert_eq!(
                    (last.width, last.height),
                    (1, 1),
                    "the chain ends at 1x1 for {width}x{height}"
                );
            }
            let mut width = width;
            let mut height = height;
            for level in &levels {
                width = (width >> 1).max(1);
                height = (height >> 1).max(1);
                assert_eq!((level.width, level.height), (width, height));
                assert!(
                    level.width >= 1 && level.height >= 1,
                    "no zero-sized level for {width}x{height}"
                );
                assert_eq!(level.rgba.len(), (level.width * level.height * 4) as usize);
            }
        }
    }

    #[test]
    fn an_srgb_colour_level_averages_black_and_white_in_linear_light() {
        let levels = generate_mip_levels(&checkerboard_2x2(), 2, 2, MipRole::SrgbColour);
        let level = levels.first().expect("level 1 exists");
        assert_eq!((level.width, level.height), (1, 1));
        let r = level.rgba[0];
        assert_eq!(
            (level.rgba[1], level.rgba[2], level.rgba[3]),
            (r, r, 255),
            "the grey level is neutral and opaque"
        );
        assert!(
            (186..=190).contains(&r),
            "0.5 linear encodes to ~188 in sRGB, got {r}"
        );
        // The proof that the average happened in linear space: the mean linear
        // value is 0.5, not the ~0.216 a byte average (128) decodes to.
        let linear = srgb_to_linear(r);
        assert!(
            (linear - 0.5).abs() < 0.01,
            "level 1 must be the linear average (0.5), got {linear}"
        );
        // ... and an encoded-space average (the wrong rule) is a different byte:
        let naive = generate_mip_levels(&checkerboard_2x2(), 2, 2, MipRole::Linear);
        let naive = naive.first().expect("level 1 exists").rgba[0];
        assert_eq!(naive, 128, "a plain byte average is 128");
        assert!(
            r > naive + 40,
            "linear-space averaging must be much brighter than the byte average: \
             {r} vs {naive}"
        );
    }

    #[test]
    fn a_normal_level_stays_a_unit_normal() {
        // Two normals tilted 0.6 to opposite sides of +Z: the average (0, 0, 0.8)
        // is 0.8 long, so only a renormalise gives a unit normal. Bytes:
        // x = (-0.6 | +0.6) -> 51 / 204, y = 0 -> 128, z = 0.8 -> 230.
        let opposite = vec![
            51, 128, 230, 255, 204, 128, 230, 255, //
            51, 128, 230, 255, 204, 128, 230, 255, //
        ];
        let level = generate_mip_levels(&opposite, 2, 2, MipRole::Normal)
            .into_iter()
            .next()
            .expect("level 1 exists");
        let normal = [
            f32::from(level.rgba[0]) / 127.5 - 1.0,
            f32::from(level.rgba[1]) / 127.5 - 1.0,
            f32::from(level.rgba[2]) / 127.5 - 1.0,
        ];
        let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        assert!(
            (length - 1.0).abs() < 0.02,
            "the averaged normal must be renormalised to unit length, got {normal:?} \
             (length {length})"
        );
        assert!(
            normal[2] > 0.99,
            "opposing x cancels, so the level is the flat +Z normal, got {normal:?}"
        );
        // The plain average would have been 0.804 long, i.e. clearly not unit.
        let naive = generate_mip_levels(&opposite, 2, 2, MipRole::Linear)
            .into_iter()
            .next()
            .expect("level 1 exists");
        let naive_z = f32::from(naive.rgba[2]) / 127.5 - 1.0;
        assert!(
            (naive_z - 0.8).abs() < 0.01,
            "a box average keeps z at ~0.8 (the non-unit case this fixes), got {naive_z}"
        );
    }

    #[test]
    fn the_normal_guard_never_emits_a_degenerate_or_inward_normal() {
        // Directly on the guard: an exactly cancelling footprint (length 0) and
        // an inward one both become the flat +Z "no perturbation" normal.
        assert_eq!(renormalise_normal(0.0, 0.0, 0.0), (0.0, 0.0, 1.0));
        assert_eq!(renormalise_normal(0.0, 0.0, -2.0), (0.0, 0.0, 1.0));
        let (x, y, z) = renormalise_normal(0.0, 0.6, 0.8);
        assert!((x * x + y * y + z * z - 1.0).abs() < 1e-6 && z > 0.0);
    }

    #[test]
    fn an_inward_normal_footprint_becomes_flat_at_the_byte_level() {
        // Every texel stores z = -1 (byte 0), i.e. a normal pointing into the
        // surface. The level must not reproduce that: it is the flat normal.
        let inward = [128u8, 128, 0, 255].repeat(4);
        let level = generate_mip_levels(&inward, 2, 2, MipRole::Normal)
            .into_iter()
            .next()
            .expect("level 1 exists");
        assert_eq!(&level.rgba[..3], &[128, 128, 255], "the flat +Z normal");
    }

    #[test]
    fn the_pre_flight_buffer_request_covers_the_whole_chain() {
        // Level 0 alone for 2048² RGBA8 is 16 MiB; the full chain is 4/3 of it
        // (the earlier one-level request was exactly level 0).
        let level_zero = 2048u64 * 2048 * 4;
        let chain = mipped_rgba8_bytes(2048, 2048);
        assert_eq!(chain, 22_369_620);
        assert!(
            chain > level_zero + level_zero / 4,
            "the request must include every generated level: {chain} vs level 0 {level_zero}"
        );
        assert_eq!(mipped_rgba8_bytes(1, 1), 4);
        assert_eq!(mipped_rgba8_bytes(2, 2), 16 + 4);
        assert_eq!(mipped_rgba8_bytes(3, 5), 60 + 8 + 4);
    }

    #[test]
    fn odd_dimensions_halve_until_one_and_never_overshoot() {
        let rgba = vec![7u8; (3 * 5 * 4) as usize];
        let levels = generate_mip_levels(&rgba, 3, 5, MipRole::Linear);
        let sizes: Vec<(u32, u32)> = levels.iter().map(|l| (l.width, l.height)).collect();
        assert_eq!(sizes, vec![(1, 2), (1, 1)]);
    }
}
