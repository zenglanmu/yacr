//! Bounded selector-only completion for the existing typed command dispatcher.
//!
//! This is not a CAD payload parser or a command registry: suggestions neither
//! execute commands nor promise that document-dependent operations will succeed.
//! Keyboard shortcut configuration intentionally has no effect on canonical
//! typed selectors. UI navigation and dispatch belong to the caller.

const INPUT_BYTE_LIMIT: usize = 4096;
const SUGGESTION_LIMIT: usize = 8;

// Keep one lexically sorted vocabulary. The boolean marks Work-only selectors;
// runtime availability (selection, undo history, document state) remains the
// responsibility of the existing application callbacks.
const SELECTORS: &[(&str, bool)] = &[
    ("ANNOTATE", true),
    ("ANNOTATE CLOUD", true),
    ("ANNOTATE ELLIPSE", true),
    ("ANNOTATE FREEHAND", true),
    ("ANNOTATE LEADER", true),
    ("ANNOTATE RECTANGLE", true),
    ("ANNOTATE TEXT", true),
    ("CANCEL", false),
    ("CIRCLE", true),
    ("CLEAR SELECTION", false),
    ("CONFIRM", true),
    ("DIAGNOSTICS", false),
    ("EXPORT", false),
    ("IMPORT", true),
    ("LINE", true),
    ("MEASURE", true),
    ("MEASURE ANGLE", true),
    ("MEASURE AREA", true),
    ("MEASURE DISTANCE", true),
    ("MEASURE POLYLINE", true),
    ("MODE", false),
    ("MOVE", true),
    ("OPEN", false),
    ("PAN", false),
    ("PANELS", false),
    ("PROJECTION", false),
    ("REDO", true),
    ("SAVE MEASUREMENT", true),
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

/// Return up to eight canonical selectors matching the entire normalized input.
///
/// ASCII case is ignored; Unicode whitespace is trimmed and collapsed to one
/// space, just as in `command_line.rs`. A completed selector is omitted, but
/// longer selectors sharing that prefix remain available. Empty input, inputs
/// exceeding 4096 UTF-8 bytes, and text outside the selector vocabulary produce
/// no suggestions. No aliases, substring search, coordinates, paths, or free
/// text payloads are interpreted. Viewer mode hides Work-only operations.
pub(crate) fn suggestions(input: &str, work_mode: bool) -> Vec<&'static str> {
    if input.len() > INPUT_BYTE_LIMIT {
        return Vec::new();
    }
    let key = input
        .split_whitespace()
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>()
        .join(" ");
    if key.is_empty() {
        return Vec::new();
    }
    SELECTORS
        .iter()
        .filter(|(selector, work_only)| {
            (work_mode || !work_only) && *selector != key && selector.starts_with(&key)
        })
        .take(SUGGESTION_LIMIT)
        .map(|(selector, _)| *selector)
        .collect()
}

#[cfg(test)]
mod contracts {
    use super::{suggestions, INPUT_BYTE_LIMIT, SELECTORS, SUGGESTION_LIMIT};
    use std::collections::BTreeSet;

    #[test]
    fn ascii_case_and_unicode_whitespace_match_the_full_selector_prefix() {
        assert_eq!(
            suggestions(" \tMeAsUrE\u{2003}\u{00a0}dIs\r\n", true),
            vec!["MEASURE DISTANCE"]
        );
        assert_eq!(suggestions("zoom\t\ti", false), vec!["ZOOM IN"]);
        assert!(suggestions("distance", true).is_empty());
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
            "PRINT",
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
        assert_eq!(suggestions("e", false), vec!["EXPORT"]);
        assert_eq!(suggestions("o", false), vec!["OPEN"]);
        assert_eq!(suggestions("view", false).len(), SUGGESTION_LIMIT);
        assert!(suggestions("measure", false).is_empty());
        assert!(suggestions("annotate", false).is_empty());
    }

    #[test]
    fn results_are_sorted_unique_bounded_and_deterministic_for_every_prefix() {
        assert!(SELECTORS.windows(2).all(|pair| pair[0].0 < pair[1].0));
        for &(selector, _) in SELECTORS {
            for end in 1..=selector.len() {
                let prefix = &selector[..end];
                for work_mode in [false, true] {
                    let result = suggestions(prefix, work_mode);
                    assert!(result.len() <= SUGGESTION_LIMIT);
                    assert!(result.windows(2).all(|pair| pair[0] < pair[1]));
                    assert_eq!(result, suggestions(prefix, work_mode));
                    assert!(result.iter().all(|entry| entry.starts_with(prefix)));
                }
            }
        }
    }

    #[test]
    fn aliases_are_not_suggested_or_expanded() {
        for alias in [
            "L", "C", "M", "TR", "ESC", "DI", "DIST", "AA", "ZE", "Z E", "ZI", "ZO", "U",
            "DISTANCE", "ANGLE", "AREA", "FIT", "DESELECT", "ENTER", "RIBBON",
        ] {
            assert!(!SELECTORS.iter().any(|&(selector, _)| selector == alias));
        }
        assert!(suggestions("AA", true).is_empty());
        assert!(suggestions("ZE", true).is_empty());
        assert!(suggestions("ESC", true).is_empty());
        // An alias-shaped prefix may still naturally match canonical names.
        assert_eq!(suggestions("L", true), vec!["LINE"]);
        assert_eq!(suggestions("SAVE", true), vec!["SAVE MEASUREMENT"]);
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
            .split("        match command {")
            .nth(1)
            .expect("typed selector dispatcher")
            .split("\n    });")
            .next()
            .expect("submission callback boundary");
        let aliases = [
            "DISTANCE", "ANGLE", "AREA", "FIT", "DESELECT", "ENTER", "RIBBON",
        ];
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
                    if !selector.is_empty() && !aliases.contains(&selector) {
                        supported.insert(selector);
                    }
                }
            }
        }
        assert!(!supported.is_empty());
        let vocabulary = SELECTORS.iter().map(|&(selector, _)| selector).collect();
        assert_eq!(supported, vocabulary);
    }
}
