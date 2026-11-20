//! Document-bundled external resources: fonts, xref sources and `IMAGE` rasters
//! (ADR-0040 D1/D2/D3).
//!
//! Three kinds of external asset can be packed into a `.ymodel` archive so one
//! file is self-contained: shaper fonts, the source bytes of an external
//! reference, and the raster bytes of an `IMAGE` entity. Mesh base-colour
//! textures are **not** part of this set — they already live in
//! `textures/<key>` ([`crate::is_valid_texture_key`]) and ADR-0040 leaves that
//! naming untouched.
//!
//! This module owns the two things every layer has to agree on:
//!
//! * [`BundledKind`] — the closed set of kinds, spelled `"font"`, `"xref"`,
//!   `"image"` in `tables/resources.json` and in the
//!   `resources/<kind>/<key>` entry name;
//! * [`is_valid_bundled_key`] — the only accepted key form, which is
//!   content-addressed and therefore contains no path component at all.
//!
//! # The one accepted key form
//!
//! `<32 lowercase hex>.<ext>`, where `<ext>` is in the kind's allowlist and is
//! **lowercase**: `font` takes `shx|ttf|otf|ttc`, `xref` takes `dwg|dxf`, and
//! `image` takes `png|jpg|jpeg|tif|tiff|bmp|webp|gif`. The hex prefix is the
//! first 32 characters of the payload's SHA-256, so identical bytes always name
//! one asset.
//!
//! Everything else is refused, and none of it is repaired:
//!
//! * an absolute path (`/tmp/x.ttf`), a Windows drive or a protocol
//!   (`C:x.png`, `file:x.png`) would make the document depend on the host's file
//!   system;
//! * `..` or a separator would let an archive entry escape `resources/<kind>/`;
//! * an uppercase spelling (`Arial.TTF`, `<32 upper hex>.ttf`) would give one
//!   asset two archive names, so the entry one writer emitted would not resolve
//!   through another reader — and the format deliberately does **not** normalise
//!   case (ADR-0040 D1);
//! * an extension outside the kind's list (`<key>.docx`) is "a resource this
//!   build cannot name", reported as `resource.unsupported_kind` rather than
//!   guessed at.
//!
//! # Self-containment
//!
//! Whether a document packs these resources at all is a **document property**
//! recorded in `document.json` as `DocumentHeader::embed_resources` (ADR-0040
//! D3, simplified to one toggle): absent means `false`, because documents
//! written before the field existed packed nothing and self-containment is
//! opt-in. That flag does not belong here — it is a header scalar, not a kind or
//! a key form — so this module deliberately carries no policy type.

use std::fmt;

/// Length of the SHA-256 prefix a bundled-resource key carries.
pub const RESOURCE_KEY_HEX_LEN: usize = 32;

/// Longest accepted key. A canonical key is at most 37 characters
/// (`32 hex` + `.` + 4); the ceiling only exists so an absurd string is refused
/// before it is used as an archive entry name.
pub const RESOURCE_KEY_MAX_LEN: usize = 80;

const FONT_EXTENSIONS: &[&str] = &["shx", "ttf", "otf", "ttc"];
const XREF_EXTENSIONS: &[&str] = &["dwg", "dxf"];
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "tif", "tiff", "bmp", "webp", "gif"];

/// The three kinds of asset a document may carry in `resources/<kind>/<key>`.
///
/// Deliberately **not** `cad_resources::ResourceKind`: `cad-db`/`cad-domain`
/// cannot depend on that crate (layering), and its enum also carries
/// resolve/decode behaviour. The carriers match between the two at the boundary
/// (ADR-0040 D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BundledKind {
    /// A shaper font file (`.shx`, `.ttf`, `.otf`, `.ttc`).
    Font,
    /// The source drawing of an external reference (`.dwg`, `.dxf`).
    Xref,
    /// A raster referenced by an `IMAGE` entity.
    Image,
}

impl BundledKind {
    /// Every kind, in the order this module documents them.
    pub const ALL: [BundledKind; 3] = [BundledKind::Font, BundledKind::Xref, BundledKind::Image];

    /// The persisted spelling: the `kind` field of a `tables/resources.json`
    /// entry and the `resources/<kind>/` path component.
    pub const fn as_str(self) -> &'static str {
        match self {
            BundledKind::Font => "font",
            BundledKind::Xref => "xref",
            BundledKind::Image => "image",
        }
    }

    /// The extensions this kind accepts, lowercase only.
    pub const fn extensions(self) -> &'static [&'static str] {
        match self {
            BundledKind::Font => FONT_EXTENSIONS,
            BundledKind::Xref => XREF_EXTENSIONS,
            BundledKind::Image => IMAGE_EXTENSIONS,
        }
    }

    /// Whether `ext` (the text after the final `.`) is allowed for this kind.
    ///
    /// Case-sensitive on purpose: an uppercase extension is refused, never
    /// folded to lowercase (ADR-0040 D1).
    pub fn allows_extension(self, ext: &str) -> bool {
        self.extensions().contains(&ext)
    }

    /// The kind a persisted token names, or `None` when it names nothing this
    /// build models. Strict: there is no "assume image" fallback.
    pub fn parse(value: &str) -> Option<BundledKind> {
        match value {
            "font" => Some(BundledKind::Font),
            "xref" => Some(BundledKind::Xref),
            "image" => Some(BundledKind::Image),
            _ => None,
        }
    }
}

impl fmt::Display for BundledKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A resource-kind token that names no [`BundledKind`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownBundledKind {
    /// The token that was refused, verbatim.
    pub value: String,
}

impl fmt::Display for UnknownBundledKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown resource kind {:?} (expected font, xref or image)",
            self.value
        )
    }
}

impl std::error::Error for UnknownBundledKind {}

impl TryFrom<&str> for BundledKind {
    type Error = UnknownBundledKind;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        BundledKind::parse(value).ok_or_else(|| UnknownBundledKind {
            value: value.to_string(),
        })
    }
}

impl std::str::FromStr for BundledKind {
    type Err = UnknownBundledKind;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        BundledKind::try_from(value)
    }
}

/// The kind whose allowlist contains `ext`, or `None`.
///
/// Lowercase only, like [`is_valid_bundled_key`]: this is the helper a carrier
/// uses to classify an on-disk file before hashing it, and it never guesses a
/// media type for an extension no kind lists.
pub fn bundled_kind_of_extension(ext: &str) -> Option<BundledKind> {
    BundledKind::ALL
        .into_iter()
        .find(|kind| kind.allows_extension(ext))
}

/// `true` for `<32 lowercase hex>.<ext>` with `ext` in `kind`'s allowlist.
pub fn is_valid_bundled_key(kind: BundledKind, key: &str) -> bool {
    if key.is_empty() || key.len() > RESOURCE_KEY_MAX_LEN {
        return false;
    }
    let Some((stem, ext)) = key.rsplit_once('.') else {
        return false;
    };
    // A stem containing another `.` (or a separator, or a drive prefix) never
    // reaches `RESOURCE_KEY_HEX_LEN` lowercase hex digits, so this single check
    // also refuses `..`, `/`, `\` and an absolute path.
    stem.len() == RESOURCE_KEY_HEX_LEN
        && stem
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && kind.allows_extension(ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical hex prefix: 32 lowercase hex digits.
    const HEX: &str = "9f3a1c0d5e7b2468ace013579bdf2468";

    fn key(ext: &str) -> String {
        format!("{HEX}.{ext}")
    }

    #[test]
    fn the_canonical_form_is_accepted_for_every_allowed_extension() {
        assert_eq!(HEX.len(), RESOURCE_KEY_HEX_LEN);
        for kind in BundledKind::ALL {
            for ext in kind.extensions() {
                assert!(
                    is_valid_bundled_key(kind, &key(ext)),
                    "{kind} should accept {ext}"
                );
                assert_eq!(bundled_kind_of_extension(ext), Some(kind));
            }
            // All-zero is a real (if unlikely) digest prefix, not a placeholder.
            assert!(is_valid_bundled_key(
                kind,
                &format!("{}.{}", "0".repeat(32), kind.extensions()[0])
            ));
        }
    }

    #[test]
    fn an_extension_belongs_to_exactly_one_kind() {
        assert!(is_valid_bundled_key(BundledKind::Font, &key("ttf")));
        assert!(!is_valid_bundled_key(BundledKind::Image, &key("ttf")));
        assert!(!is_valid_bundled_key(BundledKind::Xref, &key("png")));
        assert!(!is_valid_bundled_key(BundledKind::Font, &key("dwg")));
        // A `.shx` font is not an image and a `.dxf` reference is not a font.
        assert!(!is_valid_bundled_key(BundledKind::Image, &key("shx")));
        assert!(!is_valid_bundled_key(BundledKind::Font, &key("dxf")));
    }

    #[test]
    fn paths_traversal_and_uppercase_are_refused() {
        for bad in [
            String::new(),
            "/tmp/arial.ttf".to_string(),
            format!("/resources/{HEX}.ttf"),
            format!("resources/{HEX}.ttf"),
            format!("../{HEX}.ttf"),
            format!("{HEX}/../arial.ttf"),
            format!("C:\\fonts\\{HEX}.ttf"),
            format!("file:{HEX}.ttf"),
            format!("{HEX}.ttf\0"),
            format!("{HEX}.ttf\n"),
            "arial.ttf".to_string(),
            HEX.to_string(),
            format!("{HEX}.ttf.extra"),
            format!("{HEX}.ttf/"),
            format!(".{}", "ttf"),
        ] {
            for kind in BundledKind::ALL {
                assert!(!is_valid_bundled_key(kind, &bad), "accepted {bad:?}");
            }
        }
    }

    #[test]
    fn case_is_never_normalised() {
        // Uppercase extension: refused, not folded.
        assert!(!is_valid_bundled_key(BundledKind::Font, &key("TTF")));
        assert!(!is_valid_bundled_key(BundledKind::Font, &key("Ttf")));
        assert!(!is_valid_bundled_key(BundledKind::Image, &key("PNG")));
        // Uppercase hex: one asset would get two names.
        assert!(!is_valid_bundled_key(
            BundledKind::Font,
            &format!("{}.ttf", HEX.to_uppercase())
        ));
        assert!(!is_valid_bundled_key(
            BundledKind::Font,
            &format!("{}A.ttf", &HEX[..31])
        ));
        // `bundled_kind_of_extension` follows the same rule.
        assert_eq!(bundled_kind_of_extension("TTF"), None);
        assert_eq!(bundled_kind_of_extension("ttf"), Some(BundledKind::Font));
    }

    #[test]
    fn the_hex_prefix_must_be_exactly_32_digits() {
        assert!(!is_valid_bundled_key(
            BundledKind::Font,
            &format!("{}.ttf", &HEX[..31])
        ));
        assert!(!is_valid_bundled_key(
            BundledKind::Font,
            &key("ttf").replacen(HEX, &format!("{HEX}0"), 1)
        ));
        assert!(!is_valid_bundled_key(
            BundledKind::Font,
            &format!("{HEX}{HEX}.ttf")
        ));
        assert!(!is_valid_bundled_key(
            BundledKind::Font,
            &format!("{}g.ttf", &HEX[..31])
        ));
        // The length ceiling only bounds absurd input.
        let overlong = format!("{}{}.ttf", HEX, "0".repeat(RESOURCE_KEY_MAX_LEN));
        assert!(overlong.len() > RESOURCE_KEY_MAX_LEN);
        assert!(!is_valid_bundled_key(BundledKind::Font, &overlong));
    }

    #[test]
    fn kinds_round_trip_their_tokens_and_refuse_unknown_ones() {
        for (kind, token) in [
            (BundledKind::Font, "font"),
            (BundledKind::Xref, "xref"),
            (BundledKind::Image, "image"),
        ] {
            assert_eq!(kind.as_str(), token);
            assert_eq!(BundledKind::parse(token), Some(kind));
            assert_eq!(BundledKind::try_from(token), Ok(kind));
            assert_eq!(token.parse::<BundledKind>(), Ok(kind));
        }
        for bad in ["", "Font", "FONT", "texture", "mesh", "font "] {
            assert_eq!(BundledKind::parse(bad), None, "accepted {bad:?}");
            assert_eq!(
                bad.parse::<BundledKind>(),
                Err(UnknownBundledKind {
                    value: bad.to_string()
                })
            );
        }
        assert_eq!(
            BundledKind::try_from("Font").unwrap_err().to_string(),
            "unknown resource kind \"Font\" (expected font, xref or image)"
        );
    }
}
