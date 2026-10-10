//! Raster image decoding (PNG/JPEG) and a decoded-image cache.
//!
//! Spec v2.0 §7.3, §16.2: a raster image referenced by a drawing is decoded
//! into a device-independent RGBA8 buffer behind the same resource-budget gate
//! as every other external resource. Only PNG and JPEG are decoded here; any
//! other container is an explicit [`codes::IMAGE_UNSUPPORTED_FORMAT`] diagnostic
//! and malformed bytes an explicit [`codes::IMAGE_DECODE_FAILED`] one, never a
//! silent empty image and never a panic.
//!
//! The header dimensions are read before any pixel buffer is allocated so the
//! pixel cap [`ResourceLimits::check_image_pixels`] and the running total-bytes
//! budget can reject an oversized image up front.

use super::*;
use std::io::Cursor;

/// A raster image encoding this crate can decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    /// PNG (Portable Network Graphics).
    Png,
    /// JPEG (JFIF/EXIF, ITU-T T.81).
    Jpeg,
}

impl ImageFormat {
    /// Detect the container from the leading magic bytes.
    ///
    /// Returns `None` for anything that is not PNG or JPEG rather than guessing
    /// a format from a file extension or a heuristic.
    pub fn detect(bytes: &[u8]) -> Option<ImageFormat> {
        // PNG signature: 89 50 4E 47 0D 0A 1A 0A.
        const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        if bytes.starts_with(&PNG_SIGNATURE) {
            return Some(ImageFormat::Png);
        }
        // JPEG start-of-image marker: 0xFF 0xD8.
        if bytes.starts_with(&[0xFF, 0xD8]) {
            return Some(ImageFormat::Jpeg);
        }
        None
    }
}

/// A decoded raster image in non-premultiplied RGBA8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8; invariant: `rgba.len() == width * height * 4`.
    pub rgba: Arc<[u8]>,
}

impl DecodedImage {
    /// Decoded byte length of the RGBA buffer (four bytes per pixel).
    pub fn byte_len(&self) -> usize {
        self.rgba.len()
    }
}

/// Decode a PNG or JPEG byte slice into a bounded RGBA8 image.
///
/// The format is detected from magic bytes; an unknown/unsupported container is
/// an [`codes::IMAGE_UNSUPPORTED_FORMAT`] error and malformed/truncated bytes an
/// [`codes::IMAGE_DECODE_FAILED`] error. Header dimensions are checked against
/// [`ResourceLimits::check_image_pixels`] and the decoded RGBA payload against
/// [`ResourceLimits::total_bytes`] before the pixel buffer is allocated.
pub fn decode_image(bytes: &[u8], limits: &ResourceLimits) -> Result<DecodedImage, ResourceIssue> {
    let format = ImageFormat::detect(bytes).ok_or_else(ResourceIssue::image_unsupported_format)?;
    let (width, height, rgba) = match format {
        ImageFormat::Png => decode_png(bytes, limits)?,
        ImageFormat::Jpeg => decode_jpeg(bytes, limits)?,
    };
    // The decoders are responsible for producing RGBA8, but verify the invariant
    // instead of trusting it: a mismatch is a decode failure, not a bad buffer.
    let expected = rgba_len(width, height).ok_or_else(ResourceIssue::image_decode_failed)?;
    if rgba.len() != expected {
        return Err(ResourceIssue::image_decode_failed());
    }
    Ok(DecodedImage {
        width,
        height,
        rgba: Arc::from(rgba),
    })
}

/// Bounded in-memory cache of decoded images, keyed by [`ResourceKey`].
///
/// A plain struct with no global state. Decoded RGBA bytes are accounted against
/// [`ResourceLimits::total_bytes`] (the existing caps have no image-specific
/// byte budget); re-inserting a key replaces the old image and adjusts the
/// running total, mirroring [`MapResolver`].
#[derive(Default)]
pub struct DecodedImageCache {
    entries: HashMap<ResourceKey, Arc<DecodedImage>>,
    used_bytes: usize,
}

impl DecodedImageCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a decoded image, enforcing the running byte budget.
    ///
    /// Returns a structured [`ResourceIssue`] when the resulting total would
    /// exceed [`ResourceLimits::total_bytes`]; the cache is left unchanged.
    pub fn insert(
        &mut self,
        key: ResourceKey,
        image: DecodedImage,
        limits: &ResourceLimits,
    ) -> Result<(), ResourceIssue> {
        let previous = self.entries.get(&key).map_or(0, |image| image.byte_len());
        let resulting = self.used_bytes - previous;
        let total = resulting.checked_add(image.byte_len());
        if total.is_none_or(|total| total > limits.total_bytes) {
            return Err(ResourceIssue::over_budget(
                Some(key.as_str()),
                Some(ResourceKind::Image),
                ResourceBudget::TotalBytes,
                resulting.saturating_add(image.byte_len()) as u64,
                limits.total_bytes as u64,
            ));
        }
        self.used_bytes = total.expect("total was checked against the budget");
        self.entries.insert(key, Arc::new(image));
        Ok(())
    }

    /// Look up a decoded image by key, returning a shared handle.
    pub fn get(&self, key: &ResourceKey) -> Option<Arc<DecodedImage>> {
        self.entries.get(key).cloned()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Bytes currently held against this cache's total budget.
    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }
}

/// Enforce the pixel cap and the decoded-byte budget for header dimensions.
///
/// `check_image_pixels` runs first so its `ImagePixels` diagnostics propagate
/// unchanged; the decoded RGBA payload is then counted against `total_bytes`
/// before the caller allocates anything.
fn enforce_image_budget(
    width: u32,
    height: u32,
    limits: &ResourceLimits,
) -> Result<(), ResourceIssue> {
    limits.check_image_pixels(u64::from(width), u64::from(height))?;
    let decoded = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| size_overflow_issue(limits))?;
    if decoded > limits.total_bytes as u64 {
        return Err(ResourceIssue::over_budget(
            None,
            Some(ResourceKind::Image),
            ResourceBudget::TotalBytes,
            decoded,
            limits.total_bytes as u64,
        ));
    }
    Ok(())
}

/// The decoded RGBA byte length for `width`x`height`, or `None` on overflow.
fn rgba_len(width: u32, height: u32) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
}

fn size_overflow_issue(limits: &ResourceLimits) -> ResourceIssue {
    let mut issue = ResourceIssue::new(codes::RESOURCE_SIZE_OVERFLOW);
    issue.kind = Some(ResourceKind::Image);
    issue.budget = Some(ResourceBudget::TotalBytes);
    issue.actual = u64::MAX;
    issue.limit = limits.total_bytes as u64;
    issue
}

fn decode_failed() -> ResourceIssue {
    ResourceIssue::image_decode_failed()
}

/// Decode PNG bytes to RGBA8.
fn decode_png(bytes: &[u8], limits: &ResourceLimits) -> Result<(u32, u32, Vec<u8>), ResourceIssue> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    // Normalise every input to 8-bit samples with an alpha channel so the
    // expansion below only has to handle Rgba(8)/GrayscaleAlpha(8) (and the
    // no-alpha fallbacks if a future reader ignores the transform).
    decoder.set_transformations(
        png::Transformations::normalize_to_color8() | png::Transformations::ALPHA,
    );
    let mut reader = decoder.read_info().map_err(|_| decode_failed())?;
    // Header dimensions are available before any pixel buffer is allocated.
    let (width, height) = reader.info().size();
    enforce_image_budget(width, height, limits)?;
    let buffer_size = reader.output_buffer_size().ok_or_else(decode_failed)?;
    let mut buffer = vec![0u8; buffer_size];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|_| decode_failed())?;
    buffer.truncate(info.buffer_size());
    let rgba = expand_to_rgba(
        info.color_type,
        info.bit_depth,
        &buffer,
        info.width,
        info.height,
    )?;
    Ok((info.width, info.height, rgba))
}

/// Expand a PNG output frame to non-premultiplied RGBA8.
fn expand_to_rgba(
    color_type: png::ColorType,
    bit_depth: png::BitDepth,
    data: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, ResourceIssue> {
    // The transform above guarantees 8-bit samples.
    if bit_depth != png::BitDepth::Eight {
        return Err(decode_failed());
    }
    let plane = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(decode_failed)?;
    let samples = color_type.samples();
    if data.len() != plane.checked_mul(samples).ok_or_else(decode_failed)? {
        return Err(decode_failed());
    }
    let mut out = Vec::with_capacity(plane.checked_mul(4).ok_or_else(decode_failed)?);
    match color_type {
        png::ColorType::Rgba => out.extend_from_slice(data),
        png::ColorType::GrayscaleAlpha => {
            for px in data.chunks_exact(2) {
                out.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
            }
        }
        png::ColorType::Rgb => {
            for px in data.chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 0xFF]);
            }
        }
        png::ColorType::Grayscale => {
            for &gray in data {
                out.extend_from_slice(&[gray, gray, gray, 0xFF]);
            }
        }
        // The transform expands indexed images; seeing one here is a bug.
        png::ColorType::Indexed => return Err(decode_failed()),
    }
    Ok(out)
}

/// Decode JPEG bytes to RGBA8.
fn decode_jpeg(
    bytes: &[u8],
    limits: &ResourceLimits,
) -> Result<(u32, u32, Vec<u8>), ResourceIssue> {
    use zune_jpeg::zune_core::options::DecoderOptions;
    use zune_jpeg::JpegDecoder;

    // Raise the decoder's own dimension cap to the JPEG maximum so the pixel
    // budget below (not an arbitrary library default) is the authority.
    let options = DecoderOptions::default()
        .set_max_width(u16::MAX as usize)
        .set_max_height(u16::MAX as usize);
    let mut decoder = JpegDecoder::new_with_options(Cursor::new(bytes), options);
    decoder.decode_headers().map_err(|_| decode_failed())?;
    let info = decoder.info().ok_or_else(decode_failed)?;
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    enforce_image_budget(width, height, limits)?;
    // RGB/gray/CMYK output; the decoder converts to the requested output
    // colorspace (RGB by default), which `output_colorspace` reports.
    let pixels = decoder.decode().map_err(|_| decode_failed())?;
    let components = decoder
        .output_colorspace()
        .map(|space| space.num_components())
        .ok_or_else(decode_failed)?;
    let rgba = jpeg_to_rgba(&pixels, components, width, height)?;
    Ok((width, height, rgba))
}

/// Expand decoded JPEG samples (1/3/4 components) to RGBA8.
fn jpeg_to_rgba(
    pixels: &[u8],
    components: usize,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, ResourceIssue> {
    let plane = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(decode_failed)?;
    let expected = plane.checked_mul(components).ok_or_else(decode_failed)?;
    if pixels.len() != expected {
        return Err(decode_failed());
    }
    let mut out = Vec::with_capacity(plane.checked_mul(4).ok_or_else(decode_failed)?);
    match components {
        4 => out.extend_from_slice(pixels),
        3 => {
            for px in pixels.chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 0xFF]);
            }
        }
        1 => {
            for &gray in pixels {
                out.extend_from_slice(&[gray, gray, gray, 0xFF]);
            }
        }
        _ => return Err(decode_failed()),
    }
    Ok(out)
}
