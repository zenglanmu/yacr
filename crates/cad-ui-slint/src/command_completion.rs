//! Bounded command/alias completion and resolution for the typed dispatcher.
//!
//! This is not a CAD payload parser: it resolves a leading command token to a
//! canonical selector owned by the dispatcher, and it never executes anything or
//! promises that a document-dependent operation will succeed. Prefix matching
//! spans canonical selectors, full-word synonyms, and AutoCAD-style keyboard
//! aliases; the keyboard aliases only participate when the caller enables them.

use std::collections::BTreeSet;

const INPUT_BYTE_LIMIT: usize = 4096;
const SUGGESTION_LIMIT: usize = 8;

// Keep one lexically sorted vocabulary of canonical selectors the dispatcher
// supports today. The boolean marks Work-only selectors; runtime availability
// (selection, undo history, document state) remains the responsibility of the
// existing application callbacks.
pub(crate) const SELECTORS: &[(&str, bool)] = &[
    ("CANCEL", false),
    ("CIRCLE", true),
    ("CLEAR SELECTION", false),
    ("CONFIRM", true),
    ("DIAGNOSTICS", false),
    ("LINE", true),
    ("MEASURE", true),
    ("MEASURE ANGLE", true),
    ("MEASURE AREA", true),
    ("MEASURE DISTANCE", true),
    ("MEASURE POLYLINE", true),
    ("MODE", false),
    ("MOVE", true),
    ("NEW", false),
    ("OPEN", false),
    ("PAN", false),
    ("PANELS", false),
    ("PLOT", false),
    ("PROJECTION", false),
    ("REDO", true),
    ("SAVE", false),
    ("SELECTALL", false),
    ("TOOLS", false),
    ("TRIM", true),
    ("UNDO", true),
    ("VIEW BACK", false),
    ("VIEW BOTTOM", false),
    ("VIEW FRONT", false),
    ("VIEW ISOMETRIC", false),
    ("VIEW LEFT", false),
    ("VIEW MODE", false),
    ("VIEW RIGHT", false),
    ("VIEW TOP", false),
    ("ZOOM EXTENTS", false),
    ("ZOOM IN", false),
    ("ZOOM OUT", false),
];

/// AutoCAD/acad.pgp-style keyboard aliases.
///
/// Every alias maps onto a canonical [`SELECTORS`] entry the dispatcher already
/// executes. An alias for a command without an executor is forbidden: those
/// names belong to `command_line::UNSUPPORTED_COMMANDS` and must stay explicit,
/// never silently expand to the wrong operation. These aliases expand only when
/// the keyboard-shortcuts setting is enabled.
///
/// Bare `Z`/`ZOOM` are intentionally absent so they remain an ambiguous prefix
/// of the `ZOOM EXTENTS`/`ZOOM IN`/`ZOOM OUT` family instead of guessing one.
pub(crate) const ALIASES: &[(&str, &str)] = &[
    ("AA", "MEASURE AREA"),
    ("C", "CIRCLE"),
    ("DI", "MEASURE DISTANCE"),
    ("DIST", "MEASURE DISTANCE"),
    ("ESC", "CANCEL"),
    ("L", "LINE"),
    ("M", "MOVE"),
    ("P", "PAN"),
    ("TR", "TRIM"),
    ("U", "UNDO"),
    ("Z E", "ZOOM EXTENTS"),
    ("ZE", "ZOOM EXTENTS"),
    ("ZI", "ZOOM IN"),
    ("ZO", "ZOOM OUT"),
];

/// Full-word synonyms that are always available, independent of the
/// keyboard-shortcuts setting.
///
/// These are the dispatcher arms that exist as complete words rather than short
/// keystrokes. They map onto a canonical [`SELECTORS`] entry so completion and
/// dispatch agree on one name per operation.
pub(crate) const SYNONYMS: &[(&str, &str)] = &[
    ("ANGLE", "MEASURE ANGLE"),
    ("AREA", "MEASURE AREA"),
    ("DESELECT", "CLEAR SELECTION"),
    ("DISTANCE", "MEASURE DISTANCE"),
    ("ENTER", "CONFIRM"),
    ("EXPORT", "PLOT"),
    ("FIT", "ZOOM EXTENTS"),
    ("PRINT", "PLOT"),
    ("QSAVE", "SAVE"),
    ("RIBBON", "TOOLS"),
    ("SAVEAS", "SAVE"),
];

/// True when `selector` is a canonical selector restricted to Work mode.
fn is_work_only(selector: &str) -> bool {
    SELECTORS
        .iter()
        .find(|(name, _)| *name == selector)
        .is_some_and(|(_, work_only)| *work_only)
}

/// Collapse Unicode whitespace and ASCII-uppercase, exactly like the dispatcher.
fn normalize(input: &str) -> String {
    input
        .split_whitespace()
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The result of resolving a normalized key against the full vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrefixResolution {
    /// Exactly one canonical selector matches the key.
    Unique(&'static str),
    /// More than one canonical selector matches; the caller must not guess.
    Multiple,
    /// Nothing in the vocabulary matches the key.
    None,
}

/// Return up to eight canonical selectors matching the normalized input.
///
/// ASCII case is ignored; Unicode whitespace is trimmed and collapsed to one
/// space, just as in `command_line.rs`. A completed canonical selector is
/// omitted, but longer selectors sharing that prefix remain available. A key
/// that matches an alias/synonym name contributes that name's canonical target,
/// so `DI` suggests `MEASURE DISTANCE`. Empty input, inputs exceeding 4096
/// UTF-8 bytes, and text outside the vocabulary produce no suggestions. No
/// coordinates, paths, or free text payloads are interpreted. Viewer mode hides
/// Work-only operations.
pub(crate) fn suggestions(input: &str, work_mode: bool) -> Vec<&'static str> {
    if input.len() > INPUT_BYTE_LIMIT {
        return Vec::new();
    }
    let key = normalize(input);
    if key.is_empty() {
        return Vec::new();
    }
    let mut matches: BTreeSet<&'static str> = BTreeSet::new();
    for &(selector, work_only) in SELECTORS {
        if (work_mode || !work_only) && selector != key && selector.starts_with(&key) {
            matches.insert(selector);
        }
    }
    for &(name, target) in SYNONYMS.iter().chain(ALIASES.iter()) {
        // Skip a target equal to the key: a completed canonical selector is
        // never suggested, even when a longer synonym (e.g. SAVEAS -> SAVE)
        // shares its prefix.
        if name.starts_with(&key) && target != key && (work_mode || !is_work_only(target)) {
            matches.insert(target);
        }
    }
    matches.into_iter().take(SUGGESTION_LIMIT).collect()
}

/// Resolve an exact canonical selector, full-word synonym, or (when
/// `include_aliases`) gated keyboard alias to its canonical selector.
pub(crate) fn resolve_exact(key: &str, include_aliases: bool) -> Option<&'static str> {
    if let Some(target) = SELECTORS
        .iter()
        .find(|(selector, _)| *selector == key)
        .map(|(selector, _)| *selector)
    {
        return Some(target);
    }
    if let Some(target) = SYNONYMS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, target)| *target)
    {
        return Some(target);
    }
    if include_aliases {
        if let Some(target) = ALIASES
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, target)| *target)
        {
            return Some(target);
        }
    }
    None
}

/// Whether `key` names a keyboard alias (and is therefore gated by the
/// keyboard-shortcuts setting).
pub(crate) fn is_keyboard_alias(key: &str) -> bool {
    ALIASES.iter().any(|(name, _)| *name == key)
}

/// Resolve a normalized key by prefix against canonical selectors, full-word
/// synonyms, and (when `include_aliases`) keyboard aliases. Targets are
/// deduplicated, so two aliases that map to the same canonical selector stay a
/// unique match.
pub(crate) fn resolve_prefix(key: &str, include_aliases: bool) -> PrefixResolution {
    let mut targets: BTreeSet<&'static str> = BTreeSet::new();
    for &(selector, _) in SELECTORS {
        if selector.starts_with(key) {
            targets.insert(selector);
        }
    }
    for &(name, target) in SYNONYMS.iter().chain(ALIASES.iter()) {
        let gated = include_aliases || SYNONYMS.iter().any(|&(synonym, _)| synonym == name);
        if gated && name.starts_with(key) {
            targets.insert(target);
        }
    }
    let mut targets = targets.into_iter();
    let first = targets.next();
    match (first, targets.next()) {
        (None, _) => PrefixResolution::None,
        (Some(target), None) => PrefixResolution::Unique(target),
        _ => PrefixResolution::Multiple,
    }
}

#[cfg(test)]
mod contracts {
    use super::{
        normalize, resolve_exact, resolve_prefix, suggestions, PrefixResolution, ALIASES,
        INPUT_BYTE_LIMIT, SELECTORS, SUGGESTION_LIMIT, SYNONYMS,
    };
    use std::collections::BTreeSet;

    fn work_only(selector: &str) -> bool {
        SELECTORS
            .iter()
            .find(|(name, _)| *name == selector)
            .map(|(_, work_only)| *work_only)
            .unwrap_or(false)
    }

    #[test]
    fn ascii_case_and_unicode_whitespace_match_the_full_selector_prefix() {
        assert_eq!(
            suggestions(" \tMeAsUrE\u{2003}\u{00a0}dIs\r\n", true),
            vec!["MEASURE DISTANCE"]
        );
        assert_eq!(suggestions("zoom\t\ti", false), vec!["ZOOM IN"]);
        assert_eq!(suggestions("distance", true), vec!["MEASURE DISTANCE"]);
        assert!(suggestions("extents", true).is_empty());
        assert!(suggestions("vıew", true).is_empty());
    }

    #[test]
    fn completed_selectors_are_omitted_without_hiding_longer_selectors() {
        for &(selector, _) in SELECTORS {
            assert!(!suggestions(selector, true).contains(&selector));
        }
        assert_eq!(
            suggestions("  mEaSuRe\u{2003}", true),
            vec![
                "MEASURE ANGLE",
                "MEASURE AREA",
                "MEASURE DISTANCE",
                "MEASURE POLYLINE",
            ]
        );
        assert!(suggestions("zoom in", true).is_empty());
    }

    #[test]
    fn empty_oversized_and_payload_input_cannot_offer_commands() {
        for input in [
            "",
            " \t\n\u{2003}\u{00a0}",
            "OPEN /home/user/Drawing.dwg",
            "MEASURE DISTANCE 0,0 10,10",
            "ANNOTATE TEXT MixedCase content",
            "VIEW TOP extra",
            "LINE 0,0",
            "0,0",
            ">OPEN",
            "NEW",
            "SAVE Drawing.dwg",
            "PUBLISH",
            "POLYLINE",
            "RECTANGLE",
        ] {
            assert!(suggestions(input, true).is_empty(), "{input:?}");
            assert!(suggestions(input, false).is_empty(), "{input:?}");
        }
        let at_limit = format!("{}O", " ".repeat(INPUT_BYTE_LIMIT - 1));
        assert_eq!(suggestions(&at_limit, false), vec!["OPEN"]);
        assert!(suggestions(&format!(" {at_limit}"), true).is_empty());
        let unicode_oversized = format!("{}O", "\u{2003}".repeat(1366));
        assert!(unicode_oversized.len() > INPUT_BYTE_LIMIT);
        assert!(suggestions(&unicode_oversized, true).is_empty());
    }

    #[test]
    fn viewer_filters_every_work_only_selector_before_applying_the_limit() {
        for &(selector, work_only) in SELECTORS {
            let prefix = &selector[..selector.len() - 1];
            assert!(suggestions(prefix, true).contains(&selector));
            assert_eq!(
                suggestions(prefix, false).contains(&selector),
                !work_only,
                "{selector}"
            );
        }
        assert_eq!(suggestions("c", false), vec!["CANCEL", "CLEAR SELECTION"]);
        assert_eq!(suggestions("o", false), vec!["OPEN"]);
        assert_eq!(suggestions("view", false).len(), SUGGESTION_LIMIT);
        assert!(suggestions("measure", false).is_empty());
        assert!(suggestions("annotate", false).is_empty());
    }

    #[test]
    fn aliases_resolve_to_supported_canonical_targets() {
        for &(name, target) in ALIASES.iter().chain(SYNONYMS.iter()) {
            assert!(
                SELECTORS.iter().any(|&(selector, _)| selector == target),
                "{name} -> {target} is not a canonical selector"
            );
            assert!(
                !SELECTORS.iter().any(|&(selector, _)| selector == name),
                "{name} shadows a canonical selector"
            );
        }
        // A prefix that matches an alias name contributes the canonical target.
        assert_eq!(suggestions("AA", true), vec!["MEASURE AREA"]);
        assert_eq!(suggestions("ZE", true), vec!["ZOOM EXTENTS"]);
        assert_eq!(suggestions("ESC", true), vec!["CANCEL"]);
        assert_eq!(suggestions("L", true), vec!["LINE"]);
        // Overlapping alias/synonym names dedup to one target.
        assert_eq!(suggestions("DIST", true), vec!["MEASURE DISTANCE"]);
        assert!(suggestions("SAVE", true).is_empty());
    }

    #[test]
    fn exact_resolution_prefers_canonical_then_synonyms_then_gated_aliases() {
        assert_eq!(resolve_exact("LINE", true), Some("LINE"));
        assert_eq!(resolve_exact("LINE", false), Some("LINE"));
        // Full-word synonyms never depend on the keyboard-shortcuts gate.
        assert_eq!(resolve_exact("FIT", false), Some("ZOOM EXTENTS"));
        assert_eq!(resolve_exact("DISTANCE", false), Some("MEASURE DISTANCE"));
        assert_eq!(resolve_exact("RIBBON", false), Some("TOOLS"));
        // Short aliases are gated.
        assert_eq!(resolve_exact("DI", true), Some("MEASURE DISTANCE"));
        assert_eq!(resolve_exact("DI", false), None);
        assert_eq!(resolve_exact("ZO", true), Some("ZOOM OUT"));
        assert_eq!(resolve_exact("ZOOM OUT", true), Some("ZOOM OUT"));
        assert_eq!(resolve_exact("NOPE", true), None);
    }

    #[test]
    fn prefix_resolution_is_unique_ambiguous_or_none() {
        assert_eq!(
            resolve_prefix("LIN", true),
            PrefixResolution::Unique("LINE")
        );
        assert_eq!(resolve_prefix("ZOOM", true), PrefixResolution::Multiple);
        assert_eq!(resolve_prefix("Z", true), PrefixResolution::Multiple);
        assert_eq!(resolve_prefix("NOPE", true), PrefixResolution::None);
        // The exact alias wins over the longer ambiguous prefix.
        assert_eq!(resolve_exact("ZO", true), Some("ZOOM OUT"));
        // A keyboard alias prefix resolves when enabled and is unknown otherwise.
        assert_eq!(
            resolve_prefix("AA", true),
            PrefixResolution::Unique("MEASURE AREA")
        );
        assert_eq!(resolve_prefix("AA", false), PrefixResolution::None);
        // A full-word synonym prefix is available even with aliases disabled.
        assert_eq!(
            resolve_prefix("FI", false),
            PrefixResolution::Unique("ZOOM EXTENTS")
        );
    }

    #[test]
    fn results_are_sorted_unique_bounded_and_deterministic_for_every_prefix() {
        assert!(SELECTORS.windows(2).all(|pair| pair[0].0 < pair[1].0));
        let names = SELECTORS
            .iter()
            .map(|&(selector, _)| selector)
            .chain(ALIASES.iter().map(|&(name, _)| name))
            .chain(SYNONYMS.iter().map(|&(name, _)| name));
        for name in names {
            for end in 1..=name.len() {
                let prefix = &name[..end];
                let key = normalize(prefix);
                for work_mode in [false, true] {
                    let result = suggestions(prefix, work_mode);
                    assert!(result.len() <= SUGGESTION_LIMIT);
                    assert!(result.windows(2).all(|pair| pair[0] < pair[1]), "{prefix}");
                    assert_eq!(result, suggestions(&key, work_mode), "{prefix}");
                    for entry in &result {
                        assert!(work_mode || !work_only(entry), "{prefix} -> {entry}");
                        let canonical_prefix = SELECTORS
                            .iter()
                            .any(|&(selector, _)| selector == *entry && selector.starts_with(&key));
                        let name_prefix = ALIASES
                            .iter()
                            .chain(SYNONYMS.iter())
                            .any(|&(alias, target)| target == *entry && alias.starts_with(&key));
                        assert!(canonical_prefix || name_prefix, "{prefix} -> {entry}");
                    }
                }
            }
        }
    }

    #[test]
    fn vocabulary_has_source_contract_parity_with_existing_dispatch_selectors() {
        // Source-level parity avoids importing the Slint dispatcher into this
        // pure module. Inspect only selector match arms, never callback labels
        // or test fixtures. If dispatch changes shape, fail rather than silently
        // treating an unrecognized source layout as an empty registry.
        let source = include_str!("command_line.rs");
        let catalog = source
            .split("fn catalog_command(")
            .nth(1)
            .expect("catalog selector dispatcher")
            .split("fn history_feedback_key(")
            .next()
            .expect("catalog boundary");
        let dispatch = source
            .split("match canonical {")
            .nth(1)
            .expect("typed selector dispatcher")
            .split("fn is_unsupported_command(")
            .next()
            .expect("submission boundary");
        let synonym_keys: Vec<&str> = SYNONYMS.iter().map(|&(name, _)| name).collect();
        let mut supported = BTreeSet::new();
        for section in [catalog, dispatch] {
            for line in section.lines() {
                let Some((pattern, _)) = line.trim().split_once("=>") else {
                    continue;
                };
                if !pattern.starts_with('"') {
                    continue;
                }
                for selector in pattern.split('|') {
                    let selector = selector.trim().trim_matches('"');
                    if !selector.is_empty() && !synonym_keys.contains(&selector) {
                        supported.insert(selector);
                    }
                }
            }
        }
        assert!(!supported.is_empty());
        let vocabulary: BTreeSet<&str> = SELECTORS.iter().map(|&(selector, _)| selector).collect();
        assert_eq!(supported, vocabulary);
        // The dispatcher arms that are words rather than canonical selectors must
        // all be covered by the synonyms table, so no supported word is lost.
        for arm in [
            "DISTANCE", "ANGLE", "AREA", "FIT", "DESELECT", "ENTER", "RIBBON", "EXPORT", "PRINT",
            "QSAVE", "SAVEAS",
        ] {
            assert!(synonym_keys.contains(&arm), "missing synonym {arm}");
        }
        assert_eq!(synonym_keys.len(), 11);
    }
}
