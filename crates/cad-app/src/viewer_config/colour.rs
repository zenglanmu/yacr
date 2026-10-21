use super::*;

/// A canvas colour written as `#rrggbb`.
///
/// Exactly six hexadecimal digits after a leading `#` (either case) are
/// accepted. The three-digit shorthand (`#abc`), CSS colour names (`red`) and
/// every other spelling are refused, never guessed or expanded: this type is
/// the configuration's parse authority, so a wrong value is an explicit error.
///
/// The channels are stored as the display (sRGB) bytes a user typed. The
/// renderer's colour channels are linear sRGB (see `cad-render-wgpu`'s packed
/// instance colour), so [`HexColor::to_linear_rgba`] decodes them once, at the
/// seam where this crate talks to the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HexColor([u8; 3]);

impl HexColor {
    /// Opaque black, the renderer's documented clear colour.
    pub const BLACK: Self = Self([0, 0, 0]);

    /// Parse `#rrggbb` (lower or upper case). `None` is a refusal.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        // The length and the hex check above make these slices exact.
        let channel = |range: std::ops::Range<usize>| {
            u8::from_str_radix(&digits[range], 16).expect("two hex digits parse as a byte")
        };
        Some(Self([channel(0..2), channel(2..4), channel(4..6)]))
    }

    /// The three display bytes, in `r, g, b` order.
    pub fn channels(self) -> [u8; 3] {
        self.0
    }

    /// The canonical lower-case `#rrggbb` spelling (what is persisted).
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0[0], self.0[1], self.0[2])
    }

    /// The colour in the renderer's linear channel space, as opaque RGBA.
    ///
    /// Each sRGB byte is decoded with the standard transfer function, so a
    /// configured `#808080` writes the same pixel an sRGB target encodes as
    /// mid grey instead of the brighter value a raw `128 / 255` would produce.
    pub fn to_linear_rgba(self) -> [f32; 4] {
        let decode = |byte: u8| {
            let srgb = f64::from(byte) / 255.0;
            let linear = if srgb <= 0.040_45 {
                srgb / 12.92
            } else {
                ((srgb + 0.055) / 1.055).powf(2.4)
            };
            linear as f32
        };
        [decode(self.0[0]), decode(self.0[1]), decode(self.0[2]), 1.0]
    }
}

impl Default for HexColor {
    fn default() -> Self {
        Self::BLACK
    }
}

impl fmt::Display for HexColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for HexColor {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for HexColor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "colour {text:?} is not #rrggbb (exactly six hex digits)"
            ))
        })
    }
}
