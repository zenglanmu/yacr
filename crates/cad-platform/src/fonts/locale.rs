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
    pub fn preferred_faces(self) -> &'static [&'static str] {
        match self {
            TextLocale::Latin => &[],
            TextLocale::SimplifiedChinese => {
                &["gbcbig", "hztxt", "chineset", "bigfont", "simhei", "simsun"]
            }
            TextLocale::TraditionalChinese => &["chineset", "bigfont"],
            TextLocale::Japanese => &["bigfont"],
            TextLocale::Korean => &["bigfont"],
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
                if face_stem(key) == *candidate && !out.iter().any(|existing| existing == key) {
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
        .any(|name| engine.contains(name))
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
                "gbcbig", "hztxt", "chineset", "bigfont", "simhei", "simsun", "osifont", "arial",
                "simplex",
            ]
        );
        assert_eq!(
            TextLocale::TraditionalChinese.fallback_chain(),
            vec!["chineset", "bigfont", "osifont", "arial", "simplex"]
        );
        assert_eq!(
            TextLocale::Japanese.fallback_chain(),
            vec!["bigfont", "osifont", "arial", "simplex"]
        );
        assert_eq!(
            TextLocale::Korean.fallback_chain(),
            vec!["bigfont", "osifont", "arial", "simplex"]
        );
        // Latin is exactly the historical default list, unchanged.
        assert_eq!(
            TextLocale::Latin.fallback_chain(),
            DEFAULT_FALLBACK_NAMES.to_vec()
        );
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
