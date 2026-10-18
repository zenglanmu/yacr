//! Fonts (production CLI loading shape).

use std::sync::Arc;

use cad_representation::FontEngine;

use super::repo_root;

pub(crate) struct FontProvenance {
    /// Number of font keys the drawing references (per-entity, not deduplicated).
    pub(crate) requested_count: usize,
    /// Unique requested keys, capped for display.
    pub(crate) requested_unique: Vec<String>,
    pub(crate) requested_unique_count: usize,
    pub(crate) dir: Option<String>,
    pub(crate) registered: usize,
    pub(crate) fallback_keys: Vec<String>,
    pub(crate) note: String,
}

/// Load fonts exactly like the native CLI: the sibling `fonts/` package next to
/// the executable, an explicit `--font` set, and a system default face.
///
/// The test binary lives under `target/debug/deps/`, so `resolve_font_dir(None)`
/// finds nothing; the committed repository package (`fonts/` at the repo root)
/// stands in for a packaged CLI's sibling directory and is labelled as such.
pub(crate) fn load_fonts(requested: &[String]) -> (Option<Arc<FontEngine>>, FontProvenance) {
    let exe = std::env::current_exe().unwrap_or_default();
    let committed = repo_root().join("fonts");
    let explicit_dir = committed.is_dir().then_some(committed.as_path());
    let dir = cad_platform::fonts::local::resolve_font_dir(explicit_dir, &exe)
        .ok()
        .flatten()
        .or_else(|| {
            cad_platform::fonts::local::resolve_font_dir(None, &exe)
                .ok()
                .flatten()
        });
    let engine = cad_platform::fonts::local::load_engine(
        dir.as_deref(),
        requested,
        &[],
        cad_platform::fonts::local::cached_system_default_candidates(),
    )
    .expect("font engine load");
    let (registered, fallback_keys) = match &engine {
        Some(engine) => (engine.len(), engine.fallback_keys().to_vec()),
        None => (0, Vec::new()),
    };
    let mut unique: Vec<String> = requested
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<String>>()
        .into_iter()
        .collect();
    let requested_unique_count = unique.len();
    unique.truncate(64);
    let note = if dir.as_deref().is_some_and(|d| d.starts_with(repo_root())) {
        "committed repository fonts/ package (osifont.ttf); the mlightcad full \
         catalogue from scripts/fetch-fonts.sh is NOT fetched offline"
            .to_string()
    } else if dir.is_some() {
        "sibling fonts/ directory next to the test executable".to_string()
    } else {
        "no fonts/ directory resolved; only explicit/system faces".to_string()
    };
    (
        engine,
        FontProvenance {
            requested_count: requested.len(),
            requested_unique: unique,
            requested_unique_count,
            dir: dir.map(|d| d.display().to_string()),
            registered,
            fallback_keys,
            note,
        },
    )
}
