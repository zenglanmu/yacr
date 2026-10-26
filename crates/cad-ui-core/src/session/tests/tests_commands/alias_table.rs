//! The AutoCAD-style alias table's invariants.
//!
//! Aliases are a **convenience layer only**: a typed alias must normalize onto a
//! canonical `COMMAND_TABLE` selector through the ordinary
//! [`canonical_command_name`]/[`normalize_command`] path, so nothing downstream
//! (session dispatch, the command log, the host) ever has to know an alias
//! exists. These tests pin the three ways that guarantee can rot: an alias that
//! shadows a real name, a duplicate alias pointing somewhere else, and an alias
//! whose target does not exist.

use super::super::super::*;

/// Resolve an alias exactly the way the router does.
fn alias_target(alias: &str) -> Option<&'static str> {
    COMMAND_ALIASES
        .iter()
        .find(|(candidate, _)| *candidate == alias)
        .and_then(|(_, target)| canonical_command_name(target))
}

#[test]
fn no_alias_collides_with_a_canonical_or_interaction_only_name() {
    for (alias, target) in COMMAND_ALIASES {
        // A name in an earlier lookup table would win in
        // `canonical_command_name`, silently re-pointing the alias.
        assert!(
            !COMMAND_TABLE.iter().any(|(name, _)| name == alias),
            "{alias} is both an alias and a canonical command name"
        );
        assert!(
            !MEASUREMENT_SELECTORS.iter().any(|(name, _)| name == alias),
            "{alias} is both an alias and a measurement selector"
        );
        assert!(
            !ZOOM_STEP_SELECTORS.iter().any(|(name, _)| name == alias),
            "{alias} is both an alias and a zoom-step selector"
        );
        // The target side must be a real canonical name, never another alias and
        // never an interaction-only selector (aliases map onto `CommandId`s).
        assert!(
            COMMAND_TABLE.iter().any(|(name, _)| name == target),
            "{alias} targets {target}, which is not a COMMAND_TABLE name"
        );
        assert_ne!(alias, target, "{alias} aliases itself");
    }
}

#[test]
fn no_alias_is_defined_twice() {
    let mut seen: std::collections::BTreeMap<&str, &str> = std::collections::BTreeMap::new();
    for (alias, target) in COMMAND_ALIASES {
        if let Some(previous) = seen.insert(alias, target) {
            assert_eq!(
                previous, *target,
                "{alias} is defined twice with conflicting targets ({previous} / {target})"
            );
        }
    }
    // A definition that repeats the same target is still a redundant row: the
    // table is a lookup, so the count must equal the unique alias count.
    assert_eq!(
        seen.len(),
        COMMAND_ALIASES.len(),
        "duplicate alias row in COMMAND_ALIASES"
    );
}

#[test]
fn every_alias_canonicalises_onto_its_target() {
    for (alias, target) in COMMAND_ALIASES {
        let canonical = canonical_command_name(alias);
        assert_eq!(canonical, Some(*target), "{alias} does not canonicalise");
        // A `CommandId` target normalizes through the real funnel; an alias is
        // never a second dispatch path.
        if let Some(id) = normalize_command(target) {
            assert_eq!(normalize_command(alias), Some(id), "{alias}");
        }
        // Case-insensitive and trimmed, exactly like a canonical name.
        assert_eq!(
            canonical_command_name(&format!("  {} ", alias.to_ascii_lowercase())),
            Some(*target),
            "{alias} must accept lower-case typed input"
        );
    }
}

#[test]
fn the_shipped_table_covers_the_implemented_autocad_abbreviations() {
    // A representative slice of AutoCAD's default `acad.pgp` abbreviations,
    // including the two "same target, two keys" pairs (`CO`/`CP`, and `E`).
    for (alias, target) in [
        ("L", "LINE"),
        ("C", "CIRCLE"),
        ("A", "ARC"),
        ("PL", "PLINE"),
        ("REC", "RECTANG"),
        ("EL", "ELLIPSE"),
        ("POL", "POLYGON"),
        ("PO", "POINT"),
        ("SPL", "SPLINE"),
        ("T", "TEXT"),
        ("M", "MOVE"),
        ("CO", "COPY"),
        ("CP", "COPY"),
        ("E", "ERASE"),
        ("TR", "TRIM"),
        ("EX", "EXTEND"),
        ("O", "OFFSET"),
        ("RO", "ROTATE"),
        ("SC", "SCALE"),
        ("MI", "MIRROR"),
        ("AR", "ARRAYRECT"),
        ("AL", "ALIGN"),
        ("S", "STRETCH"),
        ("X", "EXPLODE"),
        ("H", "HATCH"),
        ("U", "UNDO"),
        ("Z", "ZOOM"),
        ("P", "PAN"),
        ("F", "FILLET"),
        ("CHA", "CHAMFER"),
        ("OS", "OSNAP"),
        ("GR", "GRID"),
        ("OR", "ORTHO"),
        ("DLI", "DIMLINEAR"),
        ("DAL", "DIMALIGNED"),
        ("DRA", "DIMRADIUS"),
        ("DDI", "DIMDIAMETER"),
        ("DAN", "DIMANGULAR"),
        ("DCE", "DIMCENTER"),
    ] {
        assert_eq!(alias_target(alias), Some(target), "{alias}");
        assert_eq!(
            normalize_command(alias),
            normalize_command(target),
            "{alias}"
        );
    }
    // `MA` is AutoCAD's MATCHPROP abbreviation. MATCHPROP is implemented here as
    // the selection-based property painter, so the abbreviation now resolves onto
    // it like every other implemented-command row (it was rejected while the
    // command itself was absent).
    assert_eq!(alias_target("MA"), Some("MATCHPROP"));
    assert_eq!(canonical_command_name("MA"), Some("MATCHPROP"));
    assert_eq!(normalize_command("MA"), Some(CommandId::MatchProp));
    // The pre-existing word synonyms stay in the same table.
    for (alias, target) in [
        ("EXPORT", "PLOT"),
        ("PRINT", "PLOT"),
        ("SAVEAS", "SAVE"),
        ("QSAVE", "SAVE"),
        ("ZOOM EXTENTS", "FIT"),
        ("ENTER", "CONFIRM"),
        ("DBA", "DIMBASELINE"),
        ("DCO", "DIMCONTINUE"),
        // Layer isolation's off switch. There is one "clear every override" path
        // (`RESTORELAYERS`); `LAYERP` is **not** an alias of it — it is its own
        // previous-state command (`CommandId::LayerPrevious`, asserted in
        // `command_vocabulary`), so it is absent from this table.
        ("LAYUNISO", "RESTORELAYERS"),
        // `LAYON` is the same single restore path by its AutoCAD name ("turn all
        // layers on"): no second restore exists.
        ("LAYON", "RESTORELAYERS"),
    ] {
        assert_eq!(alias_target(alias), Some(target), "{alias}");
    }
    // The old `LAYERP` alias row is gone: it must not resolve through the alias
    // table, and must not point at the clear-all restore.
    assert_eq!(alias_target("LAYERP"), None);
    assert_eq!(normalize_command("LAYERP"), Some(CommandId::LayerPrevious));
}

#[test]
fn admissible_names_pick_up_every_alias_and_alias_pairs_is_the_same_table() {
    let names = admissible_names();
    let pairs = alias_pairs();
    assert_eq!(
        pairs.len(),
        COMMAND_ALIASES.len(),
        "alias_pairs must not drop a row"
    );
    for ((alias, target), (pair_alias, pair_target)) in
        COMMAND_ALIASES.iter().copied().zip(pairs.iter().copied())
    {
        assert_eq!((alias, target), (pair_alias, pair_target));
        assert!(
            names.contains(&alias),
            "{alias} is admitted but not advertised"
        );
        assert_eq!(
            canonical_command_name(pair_alias),
            Some(pair_target),
            "the published pair must round-trip through the normalizer"
        );
    }
    // The frontend learns aliases from this one helper; a name it publishes must
    // be admitted by the same vocabulary it also publishes (`cad_commands`).
    for (alias, _) in &pairs {
        assert!(names.contains(alias), "{alias}");
    }
}
