//! The document-owned base-colour texture key (ADR-0038 D2/D4).
//!
//! A mesh's texture is a **document resource**, not a path: [`crate::Mesh::texture`]
//! holds a logical key and the bytes live in the drawing database (and, on disk,
//! in a `textures/<key>` archive entry). The key form is the only thing the
//! domain can enforce on its own, so it is enforced here and reused by every
//! layer that names a texture (`cad-db` validation, `cad-store-zip` entry names).
//!
//! # The one accepted form
//!
//! `<32 lowercase hex>.<png|jpg>`, for example
//! `4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png`. The hex prefix is the first 32
//! characters of the image's SHA-256, so two identical images always name one
//! asset; the extension is the sniffed container form.
//!
//! Everything else is refused, and the refusals matter:
//!
//! * an absolute path (`/tmp/x.png`), a Windows drive or protocol (`C:x.png`,
//!   `file:x.png`) would make the document depend on the host's file system;
//! * `..` or a separator would let an archive entry escape `textures/`;
//! * uppercase hex would make two spellings of one key, so a zip entry written
//!   by one code path would not resolve through another;
//! * any other length or extension (`x.png`, `<32 hex>.jpeg`, `<64 hex>.png`) is
//!   a key this build could not reproduce from the bytes.

/// Length of the SHA-256 prefix a texture key carries.
pub const TEXTURE_KEY_HEX_LEN: usize = 32;

/// Longest accepted key. The canonical form is 36 characters; the ceiling only
/// exists so an absurd string is refused before it is used as an entry name.
pub const TEXTURE_KEY_MAX_LEN: usize = 80;

/// `true` for `<32 lowercase hex><.png|.jpg>` — the only accepted texture key form.
pub fn is_valid_texture_key(key: &str) -> bool {
    if key.is_empty() || key.len() > TEXTURE_KEY_MAX_LEN {
        return false;
    }
    let stem = match key
        .strip_suffix(".png")
        .or_else(|| key.strip_suffix(".jpg"))
    {
        Some(stem) => stem,
        None => return false,
    };
    stem.len() == TEXTURE_KEY_HEX_LEN
        && stem
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71";

    #[test]
    fn the_canonical_forms_are_accepted() {
        assert_eq!(HEX.len(), TEXTURE_KEY_HEX_LEN);
        assert!(is_valid_texture_key(&format!("{HEX}.png")));
        assert!(is_valid_texture_key(&format!("{HEX}.jpg")));
        // All-zero is a real (if unlikely) digest prefix, not a placeholder.
        assert!(is_valid_texture_key(&format!("{}.png", "0".repeat(32))));
    }

    #[test]
    fn paths_and_traversal_are_refused() {
        assert!(!is_valid_texture_key("/tmp/x.png"));
        assert!(!is_valid_texture_key(&format!("/textures/{HEX}.png")));
        assert!(!is_valid_texture_key(&format!("textures/{HEX}.png")));
        assert!(!is_valid_texture_key(&format!("../{HEX}.png")));
        assert!(!is_valid_texture_key(&format!("{HEX}/../x.png")));
        assert!(!is_valid_texture_key(&format!("C:\\textures\\{HEX}.png")));
        assert!(!is_valid_texture_key(&format!("file:{HEX}.png")));
        assert!(!is_valid_texture_key(&format!("{HEX}.png\0")));
        assert!(!is_valid_texture_key(&format!("{HEX}.png\n")));
    }

    #[test]
    fn the_hex_prefix_has_one_spelling() {
        assert!(
            !is_valid_texture_key(&format!("{}.png", HEX.to_uppercase())),
            "uppercase hex is a second spelling of one key"
        );
        assert!(!is_valid_texture_key(&format!("{}A.png", &HEX[..31])));
        // Wrong lengths: the key must reproduce the digest prefix exactly.
        assert!(!is_valid_texture_key(&format!("{}.png", &HEX[..31])));
        assert!(!is_valid_texture_key(&format!("{HEX}0.png")));
        assert!(!is_valid_texture_key(""));
        assert!(!is_valid_texture_key(".png"));
    }

    #[test]
    fn only_png_and_jpg_extensions_are_accepted() {
        assert!(!is_valid_texture_key(HEX));
        assert!(!is_valid_texture_key(&format!("{HEX}.jpeg")));
        assert!(!is_valid_texture_key(&format!("{HEX}.PNG")));
        assert!(!is_valid_texture_key(&format!("{HEX}.bmp")));
        assert!(!is_valid_texture_key(&format!("{HEX}.png.png")));
        assert!(!is_valid_texture_key(&format!("{HEX}.pn")));
    }

    #[test]
    fn the_length_ceiling_is_enforced() {
        let overlong = format!("{}{}.png", HEX, "0".repeat(TEXTURE_KEY_MAX_LEN));
        assert!(overlong.len() > TEXTURE_KEY_MAX_LEN);
        assert!(!is_valid_texture_key(&overlong));
        // A 64-hex key is refused by the exact-prefix rule, not the ceiling.
        assert!(!is_valid_texture_key(&format!("{HEX}{HEX}.png")));
        assert_eq!(format!("{HEX}.png").len(), 36);
    }
}
