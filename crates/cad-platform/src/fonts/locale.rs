//! The drawing's text locale and the per-locale font fallback preference.
//!
//! A DWG/DXF declares a text code page (`$DWGCODEPAGE` / the DWG metadata code
//! page). A Chinese drawing therefore needs a CJK face to shape its text even
//! when its styles name only Latin faces, and a Latin drawing must not be handed
//! a CJK face first. This module owns the **single** code-page-to-locale table
//! and the per-locale fallback ordering; nothing else interprets a code page.
//!
//! The locale only *orders* candidates. Whether a face exists is decided by the
//! font package/catalog/system faces, and a locale with no available face is
//! reported as [`super::FontLoadReport::locale_missing`] (code
//! `font.locale_fallback_missing`) instead of being silently treated as Latin.

use cad_domain::Diagnostic;
use cad_representation::FontEngine;
use cad_resources::codes;

use super::DEFAULT_FALLBACK_NAMES;

/// The text locale a drawing's code page implies.
///
/// `Latin` is the honest default for "unknown/absent": it is the historical
/// behaviour and it never claims a Chinese/Japanese/Korean face is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextLocale {
    #[default]
    Latin,
    SimplifiedChinese,
    TraditionalChinese,
    Japanese,
    Korean,
}

/// The single code-page-to-locale mapping table.
///
/// Keys are compared after [`normalize_code_page`] (lowercased, `-`/space folded
/// to `_`), so `GB2312`, `gb2312` and `ANSI_936` are equivalent. Only the pages
/// below are recognised; every other value — including an absent value and a
/// numeric page this table does not name (for example `1252`) — maps to
/// [`TextLocale::Latin`]. An unknown page is never guessed to be CJK.
///
/// `korean` is included because acadrust reports the DWG Korean code page under
/// that name (`dwg_code_page_name`), not only as `ANSI_949`/`EUC-KR`.
const CODE_PAGE_TABLE: &[(&str, TextLocale)] = &[
    // Simplified Chinese
    ("936", TextLocale::SimplifiedChinese),
    ("gb2312", TextLocale::SimplifiedChinese),
    ("gbk", TextLocale::SimplifiedChinese),
    ("ansi_936", TextLocale::SimplifiedChinese),
    // Traditional Chinese
    ("950", TextLocale::TraditionalChinese),
    ("big5", TextLocale::TraditionalChinese),
    ("ansi_950", TextLocale::TraditionalChinese),
    // Japanese
    ("932", TextLocale::Japanese),
    ("shift_jis", TextLocale::Japanese),
    ("ansi_932", TextLocale::Japanese),
    // Korean
    ("949", TextLocale::Korean),
    ("euc_kr", TextLocale::Korean),
    ("ansi_949", TextLocale::Korean),
    ("korean", TextLocale::Korean),
];

/// Lowercase a code page and fold separators so the table above can be exact.
fn normalize_code_page(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_underscore = false;
    for ch in raw.trim().to_ascii_lowercase().chars() {
        let ch = match ch {
            '-' | ' ' | '\t' => '_',
            other => other,
        };
        if ch == '_' {
            if last_underscore {
                continue;
            }
            last_underscore = true;
        } else {
            last_underscore = false;
        }
        out.push(ch);
    }
    out.trim_matches('_').to_string()
}

impl TextLocale {
    /// The drawing's text locale for a declared code page.
    ///
    /// `None` (nothing declared) and unrecognised values are Latin, never CJK.
    pub fn from_code_page(code_page: Option<&str>) -> TextLocale {
        let Some(raw) = code_page else {
            return TextLocale::Latin;
        };
        let normalized = normalize_code_page(raw);
        CODE_PAGE_TABLE
            .iter()
            .find(|(key, _)| *key == normalized)
            .map(|(_, locale)| *locale)
            .unwrap_or(TextLocale::Latin)
    }

    /// Stable machine tag used in diagnostics and engine provenance.
    pub fn as_str(self) -> &'static str {
        match self {
            TextLocale::Latin => "latin",
            TextLocale::SimplifiedChinese => "zh-Hans",
            TextLocale::TraditionalChinese => "zh-Hant",
            TextLocale::Japanese => "ja",
            TextLocale::Korean => "ko",
        }
    }

    /// Locale-preferred faces, in preference order (empty for Latin).
    ///
    /// These are the faces that supply the locale's own script; the Latin tail
    /// ([`DEFAULT_FALLBACK_NAMES`]) is appended by [`Self::fallback_chain`] and
    /// is never reported as a missing locale candidate.
    ///
    /// Every CJK locale ends with `noto sans cjk sc`, the committed OFL
    /// `fonts/NotoSansCJKsc-Regular.otf` face and the unified last resort for
    /// all four scripts. It stays last so the lighter per-locale faces are
    /// preferred when a package actually ships them.
    pub fn preferred_faces(self) -> &'static [&'static str] {
        match self {
            TextLocale::Latin => &[],
            TextLocale::SimplifiedChinese => &[
                "gbcbig",
                "hztxt",
                "chineset",
                "bigfont",
                "simhei",
                "simsun",
                "noto sans cjk sc",
            ],
            TextLocale::TraditionalChinese => &["chineset", "bigfont", "noto sans cjk sc"],
            TextLocale::Japanese => &["bigfont", "noto sans cjk sc"],
            TextLocale::Korean => &["bigfont", "noto sans cjk sc"],
        }
    }

    /// The full ordered candidate list for this locale: locale-preferred faces
    /// first, then the existing Latin defaults as the last resort.
    pub fn fallback_chain(self) -> Vec<&'static str> {
        let mut chain: Vec<&'static str> = self.preferred_faces().to_vec();
        for name in DEFAULT_FALLBACK_NAMES {
            if !chain.contains(name) {
                chain.push(name);
            }
        }
        chain
    }

    /// Order the faces an engine already has for this locale.
    ///
    /// `available` is the engine's registered key list in registration order.
    /// For a non-Latin locale the locale-preferred keys move to the front (in
    /// preference order) and every remaining key keeps its relative order; the
    /// engine resolves glyphs per glyph, so a Latin key after the CJK key still
    /// supplies Latin characters the CJK face lacks. Latin returns `available`
    /// unchanged: a Latin drawing must keep the chain the loader built.
    pub fn ordered_fallback(self, available: &[String]) -> Vec<String> {
        if self == TextLocale::Latin {
            return available.to_vec();
        }
        let mut out: Vec<String> = Vec::new();
        for candidate in self.preferred_faces() {
            for key in available {
                if face_matches(candidate, key) && !out.iter().any(|existing| existing == key) {
                    out.push(key.clone());
                }
            }
        }
        for key in available {
            if !out.iter().any(|existing| existing == key) {
                out.push(key.clone());
            }
        }
        out
    }
}

/// Style suffixes a family file may append to the bare family name.
const STYLE_SUFFIXES: &[&str] = &[
    "regular", "bold", "italic", "oblique", "light", "medium", "book", "semibold", "black", "thin",
    "roman",
];

/// Case-folded alphanumeric form, ignoring separators (`Noto Sans CJK SC` ->
/// `notosanscjksc`).
fn face_compact(name: &str) -> String {
    name.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

/// Whether a registered key supplies the preferred face `candidate`.
///
/// A catalog name carries spaces (`noto sans cjk sc`) while the file it maps to
/// carries the family plus a style suffix (`NotoSansCJKsc-Regular.otf`), so
/// exact stem equality is not enough for the unified Noto fallback. Compare on
/// the compact form and accept a known style suffix appended to the family.
fn face_matches(candidate: &str, key: &str) -> bool {
    let candidate_stem = face_stem(candidate);
    let key_stem = face_stem(key);
    if key_stem == candidate_stem {
        return true;
    }
    let candidate_compact = face_compact(&candidate_stem);
    if candidate_compact.is_empty() {
        return false;
    }
    let key_compact = face_compact(&key_stem);
    key_compact == candidate_compact
        || key_compact
            .strip_prefix(&candidate_compact)
            .is_some_and(|suffix| STYLE_SUFFIXES.contains(&suffix))
}

/// Whether the engine has any face matching a preferred `name`.
///
/// `engine.contains` only probes the exact key/stem, so a registered file name
/// such as `NotoSansCJKsc-Regular.otf` would not answer for the catalog name
/// `noto sans cjk sc`. The loader sets the fallback chain to every registered
/// key, so that chain is checked with the same family match.
fn engine_has_face(engine: &FontEngine, name: &str) -> bool {
    engine.contains(name)
        || engine
            .fallback_keys()
            .iter()
            .any(|key| face_matches(name, key))
}

/// The bare, lowercased stem of a font key (`fonts/GBCBIG.SHX` -> `gbcbig`).
fn face_stem(name: &str) -> String {
    let bare = name.trim().trim_matches('"');
    let bare = bare.rsplit(['/', '\\']).next().unwrap_or(bare);
    let bare = bare.to_ascii_lowercase();
    match bare.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => bare,
    }
}

/// A locale whose preferred fallback faces could not be made available.
///
/// Produced once per engine load/open (never per text entity) and surfaced as a
/// stable `font.locale_fallback_missing` diagnostic naming the locale and every
/// candidate that was tried. No user-visible prose is generated here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleFallbackGap {
    /// The locale whose candidates were missing.
    pub locale: TextLocale,
    /// Candidate names tried, in order, none of which registered.
    pub candidates: Vec<String>,
}

impl LocaleFallbackGap {
    /// The stable diagnostic for this gap.
    ///
    /// The message is ASCII machine detail (locale tag + candidate names), so a
    /// UI can localise by the code without parsing prose.
    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic {
            object: None,
            code: codes::FONT_LOCALE_FALLBACK_MISSING.to_string(),
            message: format!(
                "text locale '{}' has no preferred fallback face; tried {}",
                self.locale.as_str(),
                self.candidates.join(", ")
            ),
        }
    }
}

/// The locale gap of an already-built engine, if any.
///
/// `None` for Latin, for a drawing without text, or when at least one
/// locale-preferred face is registered. Used when an engine is reused/retargeted
/// rather than freshly loaded, so a reused Chinese engine still reports the
/// missing CJK face for each document that opens into it.
pub fn locale_gap(
    engine: &FontEngine,
    locale: TextLocale,
    text_needed: bool,
) -> Option<LocaleFallbackGap> {
    if locale == TextLocale::Latin || !text_needed {
        return None;
    }
    if locale
        .preferred_faces()
        .iter()
        .any(|name| engine_has_face(engine, name))
    {
        return None;
    }
    Some(LocaleFallbackGap {
        locale,
        candidates: locale
            .preferred_faces()
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_page_maps_to_the_documented_locale() {
        for (page, expected) in [
            ("936", TextLocale::SimplifiedChinese),
            ("GB2312", TextLocale::SimplifiedChinese),
            ("gbk", TextLocale::SimplifiedChinese),
            ("ANSI_936", TextLocale::SimplifiedChinese),
            ("ansi-936", TextLocale::SimplifiedChinese),
            ("950", TextLocale::TraditionalChinese),
            ("BIG5", TextLocale::TraditionalChinese),
            ("ANSI_950", TextLocale::TraditionalChinese),
            ("932", TextLocale::Japanese),
            ("SHIFT_JIS", TextLocale::Japanese),
            ("shift-jis", TextLocale::Japanese),
            ("ANSI_932", TextLocale::Japanese),
            ("949", TextLocale::Korean),
            ("EUC-KR", TextLocale::Korean),
            ("ANSI_949", TextLocale::Korean),
        ] {
            assert_eq!(
                TextLocale::from_code_page(Some(page)),
                expected,
                "code page {page}"
            );
        }
    }

    #[test]
    fn absent_unknown_and_non_cjk_code_pages_are_latin() {
        for page in [
            None,
            Some(""),
            Some("   "),
            Some("ANSI_1252"),
            Some("1252"),
            // Prefixes/suffixes are not substring matches.
            Some("9360"),
            Some("ansi_9360"),
            Some("utf-8"),
            Some("klingon"),
        ] {
            assert_eq!(
                TextLocale::from_code_page(page),
                TextLocale::Latin,
                "code page {page:?} must not be guessed as CJK"
            );
        }
    }

    #[test]
    fn locale_candidate_order_puts_the_locale_faces_before_the_latin_tail() {
        assert_eq!(
            TextLocale::SimplifiedChinese.fallback_chain(),
            vec![
                "gbcbig",
                "hztxt",
                "chineset",
                "bigfont",
                "simhei",
                "simsun",
                "noto sans cjk sc",
                "osifont",
                "arial",
                "simplex",
            ]
        );
        assert_eq!(
            TextLocale::TraditionalChinese.fallback_chain(),
            vec![
                "chineset",
                "bigfont",
                "noto sans cjk sc",
                "osifont",
                "arial",
                "simplex",
            ]
        );
        assert_eq!(
            TextLocale::Japanese.fallback_chain(),
            vec!["bigfont", "noto sans cjk sc", "osifont", "arial", "simplex"]
        );
        assert_eq!(
            TextLocale::Korean.fallback_chain(),
            vec!["bigfont", "noto sans cjk sc", "osifont", "arial", "simplex"]
        );
        // Latin is exactly the historical default list, unchanged.
        assert_eq!(
            TextLocale::Latin.fallback_chain(),
            DEFAULT_FALLBACK_NAMES.to_vec()
        );
    }

    #[test]
    fn every_cjk_locale_ends_with_the_noto_last_resort() {
        for locale in [
            TextLocale::SimplifiedChinese,
            TextLocale::TraditionalChinese,
            TextLocale::Japanese,
            TextLocale::Korean,
        ] {
            assert_eq!(
                locale.preferred_faces().last().copied(),
                Some("noto sans cjk sc"),
                "{} must fall back to Noto last",
                locale.as_str()
            );
        }
        // Latin has no locale preference; its chain is exactly the shared
        // default list, so Noto is only *fetched* when the default loop needs it.
        assert!(TextLocale::Latin.preferred_faces().is_empty());
        assert_eq!(
            TextLocale::Latin.fallback_chain(),
            DEFAULT_FALLBACK_NAMES.to_vec()
        );
    }

    #[test]
    fn a_registered_noto_file_satisfies_the_locale_gap_and_is_promoted() {
        // The catalog name has spaces; the committed file is
        // `NotoSansCJKsc-Regular.otf`. Matching must bridge the two.
        let mut engine = FontEngine::new();
        engine
            .register("NotoSansCJKsc-Regular.otf", synthetic_shx())
            .unwrap();
        engine.set_fallback(vec![
            "osifont.shx".to_string(),
            "NotoSansCJKsc-Regular.otf".into(),
        ]);

        assert!(
            locale_gap(&engine, TextLocale::SimplifiedChinese, true).is_none(),
            "a registered Noto face must clear the CJK locale gap"
        );
        assert_eq!(
            TextLocale::SimplifiedChinese.ordered_fallback(engine.fallback_keys()),
            vec![
                "NotoSansCJKsc-Regular.otf".to_string(),
                "osifont.shx".to_string()
            ],
            "Noto must be promoted ahead of the Latin default"
        );
    }

    /// Minimal but parseable compiled SHX shape font bytes.
    fn synthetic_shx() -> std::sync::Arc<[u8]> {
        let mut bytes = b"AutoCAD-86 shapes 1.0\r\n\x1a".to_vec();
        let info = b"Synthetic\0\x15\x07\x02\0";
        for value in [0u16, 0, 1, 0, info.len() as u16] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(info);
        std::sync::Arc::from(bytes)
    }

    #[test]
    fn ordered_fallback_moves_an_available_locale_face_first() {
        let available = vec!["osifont.ttf".to_string(), "gbcbig.shx".to_string()];
        assert_eq!(
            TextLocale::SimplifiedChinese.ordered_fallback(&available),
            vec!["gbcbig.shx".to_string(), "osifont.ttf".to_string()]
        );
        // Latin preserves the loader's order exactly (regression guard).
        assert_eq!(
            TextLocale::Latin.ordered_fallback(&available),
            available.clone()
        );
        // An unavailable locale face changes nothing but the order stays stable.
        assert_eq!(TextLocale::Japanese.ordered_fallback(&available), available);
    }
}
