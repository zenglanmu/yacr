//! Message catalogs and locale resolution (N01, spec v2.0 §3.5).
//!
//! The JSON files under `crates/cad-ui-core/i18n/` are the **single source of
//! truth** for user-facing text. A requested locale is resolved to one of the
//! stable locales ([`Locale::ZhCn`], [`Locale::En`]) and a [`MessageCatalog`] is
//! selected from it; the UI never embeds a second copy of the strings. This is
//! the shared, UI-framework-free half of the N01 contract (the Slint crate used
//! to own it; the catalogs now live here).
//!
//! Doctrine for N01 is documented in `docs/i18n.md`. In short:
//!
//! - catalogs are parsed from `include_str!` at startup (no per-frame parsing);
//! - `en-US`, `en_GB`, `zh-Hans`, … normalise onto the stable locales;
//! - an unsupported locale falls back to `en` with a *recorded reason*, not
//!   silently, and no call ever returns an empty string;
//! - key/placeholder parity between the two catalogs is enforced by unit tests
//!   and by `scripts/check-i18n.py` (CI).

use std::collections::BTreeMap;

/// Raw catalog sources. Kept `pub` so packaging/documentation tooling and the
/// i18n contract stay intact.
pub const ZH_CN_JSON: &str = include_str!("../i18n/zh-CN.json");
pub const EN_JSON: &str = include_str!("../i18n/en.json");

/// Locale tag used by [`Locale::ZhCn`].
pub const ZH_CN_TAG: &str = "zh-CN";
/// Locale tag used by [`Locale::En`].
pub const EN_TAG: &str = "en";
/// Locale used when the request cannot be satisfied: English is the shipped
/// fallback, so a system language this build does not have never lands on a
/// language the user did not ask for.
pub const DEFAULT_TAG: &str = EN_TAG;

/// A stable, selectable UI locale.
///
/// Only locales with a complete catalog are members. `en-US` and other English
/// variants are *inputs* that normalise onto [`Locale::En`]; they are not
/// separate members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Locale {
    /// Simplified Chinese.
    ZhCn,
    /// English (any region).
    En,
}

impl Locale {
    /// The canonical BCP-47-ish tag this locale is reported as.
    pub const fn tag(self) -> &'static str {
        match self {
            Locale::ZhCn => ZH_CN_TAG,
            Locale::En => EN_TAG,
        }
    }

    /// All locales that have a complete, shipped catalog.
    pub const ALL: [Locale; 2] = [Locale::ZhCn, Locale::En];

    /// The catalog for this locale.
    pub fn catalog(self) -> MessageCatalog {
        match self {
            Locale::ZhCn => MessageCatalog::parse(ZH_CN_JSON),
            Locale::En => MessageCatalog::parse(EN_JSON),
        }
    }
}

/// Why a requested locale tag did not resolve to itself.
///
/// Kept explicit so a host can log/surface it rather than showing a silently
/// wrong language (N01 §5.2.1 "可诊断策略").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    /// The tag carried a region/script (e.g. `en-US`) and was normalised.
    Normalised,
    /// The requested language or variant has no catalog (e.g. `de-DE`, `zh-TW`).
    Unsupported,
    /// The tag was empty or malformed (e.g. `""`, `"--"`).
    Malformed,
}

/// The outcome of resolving a requested locale tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleResolution {
    /// Requested tag, verbatim (trimmed) for diagnostics.
    pub requested: String,
    /// Locale actually selected.
    pub locale: Locale,
    /// `None` when the request mapped exactly; otherwise why it did not.
    pub fallback: Option<FallbackReason>,
}

/// Resolve a requested locale tag to a shipped catalog.
///
/// Accepts `en-US`, `en_US`, `EN-us`, `zh-Hans-CN`, `zh-CN`, … and the default
/// `en`. Unsupported or malformed input (including `zz`, `zh-TW`, `zh-Hant`)
/// resolves to [`Locale::En`] with a [`FallbackReason`]. This function never
/// fails and never returns "no locale".
pub fn resolve_locale(requested: &str) -> LocaleResolution {
    let trimmed = requested.trim();
    let malformed = split_primary(trimmed).is_none();
    let Some(locale) = supported_locale_for(trimmed) else {
        return LocaleResolution {
            requested: trimmed.to_string(),
            locale: Locale::En,
            fallback: Some(if malformed {
                FallbackReason::Malformed
            } else {
                FallbackReason::Unsupported
            }),
        };
    };
    // The canonical, stable tag resolves exactly; anything with a region or
    // script (e.g. `en-US`, `zh-Hans-CN`) is a normalisation, so the host can
    // tell "asked for en" apart from "asked for en-US and got en".
    let exact = canonical_tag_for(locale) == trimmed;
    LocaleResolution {
        requested: trimmed.to_string(),
        locale,
        fallback: (!exact).then_some(FallbackReason::Normalised),
    }
}

/// Map a POSIX/BCP-47 locale tag to a shipped catalog, or `None` when this
/// build has none.
///
/// This is the single mapping table shared by the React UI (`systemLocaleFor`)
/// and by [`locale_from_env`]. Codeset and modifier suffixes (`zh_CN.UTF-8`,
/// `sr_RS@latin`) are stripped first.
///
/// `en`/`en-*` select English; `zh`, `zh-CN`, `zh-Hans` and any other
/// Simplified-Chinese region select `zh-CN`. A Hant script or a Traditional
/// region (`zh-TW`, `zh-HK`, `zh-MO`) yields `None`: no Hant catalog exists, so
/// Traditional Chinese is not silently served Simplified.
pub fn supported_locale_for(tag: &str) -> Option<Locale> {
    let base = tag.trim().split(['.', '@']).next().unwrap_or("");
    let mut parts = base.split(['-', '_']);
    let primary = parts.next()?;
    if primary.is_empty() || !primary.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    match primary.to_ascii_lowercase().as_str() {
        "en" => Some(Locale::En),
        "zh" => {
            let traditional = parts.any(|part| {
                let part = part.to_ascii_lowercase();
                matches!(part.as_str(), "hant" | "tw" | "hk" | "mo")
            });
            (!traditional).then_some(Locale::ZhCn)
        }
        _ => None,
    }
}

/// Environment variables consulted for the system UI language, highest first.
///
/// This is the POSIX precedence order; an empty value is treated as unset.
pub const LOCALE_ENV_VARS: [&str; 4] = ["LC_ALL", "LC_MESSAGES", "LANGUAGE", "LANG"];

/// The locale a system environment selects, from a variable lookup.
///
/// Pure so the host wiring and its tests share one implementation. Only the
/// **highest-precedence non-empty** variable is consulted: if it names a
/// language this build does not ship (e.g. `fr_FR.UTF-8`) the answer is English,
/// it does not silently fall through to a different variable. `LANGUAGE` is a
/// colon-separated preference list, so its first entry wins. `C`, `POSIX`, an
/// empty value or a missing variable all resolve to [`Locale::En`].
pub fn locale_from_env(get: impl Fn(&str) -> Option<String>) -> Locale {
    for var in LOCALE_ENV_VARS {
        let Some(raw) = get(var) else { continue };
        let value = if var == "LANGUAGE" {
            raw.split(':').map(str::trim).find(|part| !part.is_empty())
        } else {
            let trimmed = raw.trim();
            (!trimmed.is_empty()).then_some(trimmed)
        };
        let Some(value) = value else { continue };
        return supported_locale_for(value).unwrap_or(Locale::En);
    }
    Locale::En
}

/// The exact canonical spelling of a locale tag, compared case-sensitively by
/// [`resolve_locale`] against the trimmed request.
fn canonical_tag_for(locale: Locale) -> &'static str {
    locale.tag()
}

/// Lower-case the primary subtag of `tag`.
///
/// Returns `None` when the primary subtag is absent or not alphabetic, which is
/// how `""`, `"-"` and `"--"` are rejected.
fn split_primary(tag: &str) -> Option<String> {
    let primary = tag.split(['-', '_']).next()?;
    if primary.is_empty() || !primary.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(primary.to_ascii_lowercase())
}

/// A parsed message catalog: stable key → display string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageCatalog {
    entries: BTreeMap<String, String>,
}

impl MessageCatalog {
    /// Parse a JSON object of `"key": "value"` pairs.
    ///
    /// Panics only on a malformed *embedded* catalog, which is a build-time bug
    /// caught by the unit tests; runtime input never goes through here.
    pub fn parse(json: &str) -> Self {
        Self {
            entries: parse_flat_json_object(json),
        }
    }

    /// Number of keys in the catalog.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the catalog has no keys (a CI failure condition).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every key, sorted, for parity checks and tooling.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Look up a key verbatim.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// Look up a key, substituting `{name}` placeholders.
    ///
    /// Unknown placeholders are left untouched so a typo is visible instead of
    /// producing a silently truncated label. See [`format_message`].
    pub fn format(&self, key: &str, args: &[(&str, &str)]) -> Message {
        match self.get(key) {
            Some(template) => Message::Found(format_message(template, args)),
            None => Message::Missing(key.to_string()),
        }
    }
}

/// The result of a message lookup.
///
/// A missing key is modelled explicitly and renders as the key itself, never as
/// an empty string (N01 §5.2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// The key resolved; carries the formatted text.
    Found(String),
    /// The key is absent from the active catalog.
    Missing(String),
}

impl Message {
    /// Display text: the formatted value, or `"⟦key⟧"` for a missing key.
    ///
    /// Bracketing makes a missing key obvious in screenshots and logs instead of
    /// showing a blank control.
    pub fn text(&self) -> String {
        match self {
            Message::Found(text) => text.clone(),
            Message::Missing(key) => format!("\u{27e6}{key}\u{27e7}"),
        }
    }

    /// The missing key, if any.
    pub fn missing_key(&self) -> Option<&str> {
        match self {
            Message::Found(_) => None,
            Message::Missing(key) => Some(key),
        }
    }

    /// Whether the key resolved.
    pub fn is_found(&self) -> bool {
        matches!(self, Message::Found(_))
    }
}

/// Substitute `{name}` placeholders in `template`.
///
/// `\{{` is an escape for a literal `{`. A placeholder whose value is not
/// supplied is left as-is; placeholders are never removed.
pub fn format_message(template: &str, args: &[(&str, &str)]) -> String {
    let bytes = template.as_bytes();
    let mut out = String::with_capacity(template.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if bytes.get(i + 1) == Some(&b'{') => {
                out.push('{');
                i += 2;
            }
            b'{' => {
                if let Some(close) = template[i..].find('}') {
                    let name = &template[i + 1..i + close];
                    if let Some((_, value)) = args.iter().find(|(candidate, _)| *candidate == name)
                    {
                        out.push_str(value);
                    } else {
                        // Unknown placeholder: keep it visible (usually a typo).
                        out.push_str(&template[i..=i + close]);
                    }
                    i += close + 1;
                } else {
                    out.push('{');
                    i += 1;
                }
            }
            _ => {
                let ch = template[i..].chars().next().expect("in-bounds char");
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

/// Collect the placeholder names present in `template`, in order, de-duplicated.
pub fn placeholders(template: &str) -> Vec<String> {
    let bytes = template.as_bytes();
    let mut names = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if bytes.get(i + 1) == Some(&b'{') => i += 2,
            b'{' => {
                if let Some(close) = template[i..].find('}') {
                    let name = &template[i + 1..i + close];
                    if !name.is_empty() && !names.iter().any(|n| n == name) {
                        names.push(name.to_string());
                    }
                    i += close + 1;
                } else {
                    i += 1;
                }
            }
            _ => {
                let ch = template[i..].chars().next().expect("in-bounds char");
                i += ch.len_utf8();
            }
        }
    }
    names
}

/// The runtime message source: a selected locale plus its catalog.
///
/// This is the object UI chrome should hold in a `Rc<RefCell<_>>` so the shell
/// can be rebuilt when [`MessageSource::set_locale`] changes the language.
#[derive(Debug, Clone)]
pub struct MessageSource {
    resolution: LocaleResolution,
    catalog: MessageCatalog,
}

impl Default for MessageSource {
    fn default() -> Self {
        Self::for_locale(Locale::En)
    }
}

impl MessageSource {
    /// Build a source for a request tag, applying fallback rules.
    pub fn from_request(requested: &str) -> Self {
        let resolution = resolve_locale(requested);
        let catalog = resolution.locale.catalog();
        Self {
            resolution,
            catalog,
        }
    }

    /// Build a source directly from a resolved locale.
    pub fn for_locale(locale: Locale) -> Self {
        Self {
            resolution: LocaleResolution {
                requested: locale.tag().to_string(),
                locale,
                fallback: None,
            },
            catalog: locale.catalog(),
        }
    }

    /// Switch the active locale at runtime; returns the resolution applied.
    pub fn set_locale(&mut self, requested: &str) -> LocaleResolution {
        *self = Self::from_request(requested);
        self.resolution.clone()
    }

    /// The locale currently in use.
    pub fn locale(&self) -> Locale {
        self.resolution.locale
    }

    /// The resolution that produced the current locale.
    pub fn resolution(&self) -> &LocaleResolution {
        &self.resolution
    }

    /// The active catalog.
    pub fn catalog(&self) -> &MessageCatalog {
        &self.catalog
    }

    /// Look up `key`, substituting placeholders.
    pub fn message(&self, key: &str, args: &[(&str, &str)]) -> Message {
        self.catalog.format(key, args)
    }

    /// Convenience: the display text for `key`.
    pub fn text(&self, key: &str, args: &[(&str, &str)]) -> String {
        self.message(key, args).text()
    }
}

/// Parse a flat JSON object of string values.
///
/// Deliberately minimal rather than adding a `serde_json` dependency to this
/// crate: the embedded catalogs are validated at test/CI time and this parser
/// handles comments, escapes and duplicate keys defensively.
fn parse_flat_json_object(json: &str) -> BTreeMap<String, String> {
    let chars: Vec<char> = json.chars().collect();
    let mut i = 0;
    let mut entries = BTreeMap::new();
    skip_ws(&chars, &mut i);
    if chars.get(i) != Some(&'{') {
        panic!("i18n catalog must be a JSON object");
    }
    i += 1;
    loop {
        skip_ws(&chars, &mut i);
        match chars.get(i) {
            Some('}') | None => break,
            Some(',') => {
                i += 1;
                continue;
            }
            Some('"') => {}
            other => panic!("i18n catalog: expected string key, found {other:?}"),
        }
        let key = parse_json_string(&chars, &mut i);
        skip_ws(&chars, &mut i);
        if chars.get(i) != Some(&':') {
            panic!("i18n catalog: expected ':' after key {key:?}");
        }
        i += 1;
        skip_ws(&chars, &mut i);
        if chars.get(i) != Some(&'"') {
            panic!("i18n catalog: value for {key:?} must be a string");
        }
        let value = parse_json_string(&chars, &mut i);
        entries.insert(key, value);
    }
    entries
}

fn skip_ws(chars: &[char], i: &mut usize) {
    while matches!(chars.get(*i), Some(' ' | '\t' | '\n' | '\r')) {
        *i += 1;
    }
}

fn parse_json_string(chars: &[char], i: &mut usize) -> String {
    debug_assert_eq!(chars.get(*i), Some(&'"'));
    *i += 1;
    let mut out = String::new();
    while let Some(&ch) = chars.get(*i) {
        *i += 1;
        match ch {
            '"' => return out,
            '\\' => {
                let escape = chars.get(*i).copied().unwrap_or('\\');
                *i += 1;
                match escape {
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'u' => {
                        let hex: String =
                            chars.get(*i..*i + 4).unwrap_or_default().iter().collect();
                        *i += 4.min(chars.len().saturating_sub(*i));
                        let code = u32::from_str_radix(&hex, 16).unwrap_or(0xFFFD);
                        out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                    }
                    other => out.push(other),
                }
            }
            other => out.push(other),
        }
    }
    panic!("i18n catalog: unterminated string");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_have_matching_keys() {
        let zh = Locale::ZhCn.catalog();
        let en = Locale::En.catalog();
        let zh_keys: Vec<_> = zh.keys().collect();
        let en_keys: Vec<_> = en.keys().collect();
        assert_eq!(zh_keys, en_keys, "catalog keys must match exactly");
        assert!(!zh.is_empty(), "zh-CN catalog must not be empty");
        assert!(!en.is_empty(), "en catalog must not be empty");
    }

    #[test]
    fn catalogs_have_matching_placeholders() {
        let zh = Locale::ZhCn.catalog();
        let en = Locale::En.catalog();
        for key in zh.keys() {
            let zh_ph = placeholders(zh.get(key).expect("zh key"));
            let en_ph = placeholders(en.get(key).expect("en key"));
            assert_eq!(zh_ph, en_ph, "placeholder mismatch for {key}");
        }
    }

    #[test]
    fn normalize_accepts_regions_and_case() {
        for tag in ["en-US", "en_US", "EN-us", "en-GB", "en"] {
            let resolved = resolve_locale(tag);
            assert_eq!(resolved.locale, Locale::En, "tag {tag}");
        }
        for tag in ["zh-CN", "zh_CN", "zh-Hans-CN", "ZH-cn", "zh"] {
            let resolved = resolve_locale(tag);
            assert_eq!(resolved.locale, Locale::ZhCn, "tag {tag}");
        }
        // A region means the tag was not already the stable form.
        assert_eq!(
            resolve_locale("en-US").fallback,
            Some(FallbackReason::Normalised)
        );
        assert_eq!(resolve_locale("en").fallback, None);
        assert_eq!(resolve_locale("zh-CN").fallback, None);
    }

    #[test]
    fn unsupported_and_malformed_fall_back_to_en() {
        for tag in [
            "de-DE", "fr", "ja-JP", "", "-", "--", "123", "zhx", "zh-TW", "zh-Hant",
        ] {
            let resolved = resolve_locale(tag);
            assert_eq!(resolved.locale, Locale::En, "tag {tag:?}");
            assert!(
                resolved.fallback.is_some(),
                "tag {tag:?} must record a fallback reason"
            );
        }
        assert_eq!(
            resolve_locale("de-DE").fallback,
            Some(FallbackReason::Unsupported)
        );
        assert_eq!(resolve_locale("").fallback, Some(FallbackReason::Malformed));
        assert_eq!(
            resolve_locale("123").fallback,
            Some(FallbackReason::Malformed)
        );
    }

    #[test]
    fn system_tag_mapping_is_one_table_for_hosts_and_env() {
        for tag in [
            "zh",
            "zh-CN",
            "zh_CN",
            "zh-Hans",
            "zh-Hans-CN",
            "zh-SG",
            "ZH-cn",
        ] {
            assert_eq!(supported_locale_for(tag), Some(Locale::ZhCn), "tag {tag:?}");
        }
        for tag in ["en", "en-US", "en_GB", "EN-us"] {
            assert_eq!(supported_locale_for(tag), Some(Locale::En), "tag {tag:?}");
        }
        // No Hant catalog exists, so Traditional Chinese is not served Simplified.
        for tag in ["zh-TW", "zh_TW.UTF-8", "zh-Hant", "zh-HK", "zh-MO"] {
            assert_eq!(supported_locale_for(tag), None, "tag {tag:?}");
        }
        for tag in ["fr", "fr_FR.UTF-8", "de", "ja-JP", "", "C", "POSIX"] {
            assert_eq!(supported_locale_for(tag), None, "tag {tag:?}");
        }
    }

    #[test]
    fn env_locale_follows_posix_precedence_and_falls_back_to_en() {
        let env = |pairs: &[(&str, &str)]| {
            let pairs: Vec<(String, String)> = pairs
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect();
            move |key: &str| {
                pairs
                    .iter()
                    .find(|(name, _)| name.as_str() == key)
                    .map(|(_, value)| value.clone())
            }
        };
        // LC_ALL wins over every lower variable.
        assert_eq!(
            locale_from_env(env(&[
                ("LC_ALL", "zh_CN.UTF-8"),
                ("LC_MESSAGES", "en_US.UTF-8"),
                ("LANG", "en_US.UTF-8"),
            ])),
            Locale::ZhCn
        );
        assert_eq!(
            locale_from_env(env(&[("LC_ALL", "en_US.UTF-8"), ("LANG", "zh_CN.UTF-8"),])),
            Locale::En
        );
        // LC_MESSAGES wins over LANGUAGE and LANG; empty values are unset.
        assert_eq!(
            locale_from_env(env(&[
                ("LC_ALL", ""),
                ("LC_MESSAGES", "zh_CN.UTF-8"),
                ("LANGUAGE", "en_US:de_DE"),
                ("LANG", "en_US.UTF-8"),
            ])),
            Locale::ZhCn
        );
        // LANGUAGE is a colon-separated preference list: the first entry wins.
        assert_eq!(
            locale_from_env(env(&[("LANGUAGE", "zh_CN:en_US"), ("LANG", "en_US.UTF-8")])),
            Locale::ZhCn
        );
        assert_eq!(
            locale_from_env(env(&[("LANGUAGE", "en_US:zh_CN"), ("LANG", "zh_CN.UTF-8")])),
            Locale::En
        );
        // A set-but-unsupported system language is English, not Chinese: the
        // higher-precedence variable is authoritative and does not fall through.
        assert_eq!(
            locale_from_env(env(&[("LC_ALL", "fr_FR.UTF-8"), ("LANG", "zh_CN.UTF-8"),])),
            Locale::En
        );
        assert_eq!(locale_from_env(env(&[("LANG", "C")])), Locale::En);
        assert_eq!(locale_from_env(env(&[("LANG", "POSIX")])), Locale::En);
        assert_eq!(locale_from_env(env(&[])), Locale::En);
        assert_eq!(locale_from_env(env(&[("LANG", "zh_TW.UTF-8")])), Locale::En);
    }

    #[test]
    fn missing_key_never_renders_empty() {
        let source = MessageSource::for_locale(Locale::En);
        let message = source.message("does.not.exist", &[]);
        assert!(!message.is_found());
        assert_eq!(message.missing_key(), Some("does.not.exist"));
        assert_eq!(message.text(), "\u{27e6}does.not.exist\u{27e7}");
        assert!(!message.text().is_empty());
    }

    #[test]
    fn known_keys_resolve_in_both_locales() {
        for locale in Locale::ALL {
            let source = MessageSource::for_locale(locale);
            let message = source.message("units.unknown", &[]);
            assert!(message.is_found(), "units.unknown missing in {locale:?}");
            assert!(!message.text().is_empty());
        }
        assert_eq!(
            MessageSource::for_locale(Locale::ZhCn).text("units.unknown", &[]),
            "图纸单位"
        );
        assert_eq!(
            MessageSource::for_locale(Locale::En).text("units.unknown", &[]),
            "Drawing units"
        );
    }

    #[test]
    fn runtime_switch_changes_language() {
        let mut source = MessageSource::default();
        assert_eq!(source.locale(), Locale::En);
        let resolution = source.set_locale("zh-Hans-CN");
        assert_eq!(source.locale(), Locale::ZhCn);
        assert_eq!(resolution.fallback, Some(FallbackReason::Normalised));
        assert_eq!(source.text("file.open", &[]), "打开图纸");
        // Switching to an unsupported language reports the fallback (English).
        let resolution = source.set_locale("de");
        assert_eq!(source.locale(), Locale::En);
        assert_eq!(resolution.fallback, Some(FallbackReason::Unsupported));
        assert_eq!(source.text("file.open", &[]), "Open drawing");
        // An unshipped Chinese variant is not silently served Simplified.
        let resolution = source.set_locale("zh-TW");
        assert_eq!(source.locale(), Locale::En);
        assert_eq!(resolution.fallback, Some(FallbackReason::Unsupported));
    }

    #[test]
    fn placeholders_are_substituted_and_unknown_ones_survive() {
        assert_eq!(
            format_message(
                "已打开 {name}: {state}",
                &[("name", "a.dwg"), ("state", "ok")]
            ),
            "已打开 a.dwg: ok"
        );
        assert_eq!(format_message("{a}{b}", &[("a", "1")]), "1{b}");
        assert_eq!(format_message("literal \\{brace}", &[]), "literal {brace}");
        assert_eq!(format_message("\\{a}{b}", &[("b", "2")]), "{a}2");
        assert_eq!(format_message("no placeholders", &[]), "no placeholders");
    }

    #[test]
    fn placeholder_collection_is_ordered_and_unique() {
        assert_eq!(placeholders("{a} {b} {a}"), vec!["a", "b"]);
        assert_eq!(placeholders("none"), Vec::<String>::new());
        assert_eq!(placeholders("\\{x}"), Vec::<String>::new());
    }

    #[test]
    fn embedded_catalogs_parse_as_flat_json() {
        // A brace containing a real placeholder must not confuse the parser.
        let catalog = MessageCatalog::parse(r#"{ "k": "v {p}", "n": "a\\b" }"#);
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog.get("k"), Some("v {p}"));
        assert_eq!(catalog.get("n"), Some("a\\b"));
    }
}
