//! The **font package** of the browser carrier.
//!
//! The Web host does not bundle fonts. The page learns what the open document
//! needs from `cad_fonts` (`requested`, `textNeeded`), lazily fetches the matching
//! entries of the catalogue + face bytes from the project's Pages CDN
//! (same-origin `fonts/`), and calls
//! [`set_font_package`](crate::set_font_package) with them; this module owns the
//! one in-memory [`MemoryFontLoader`] every subsequent install resolves from.
//! [`crate::apply_font_package`] then re-runs the resource install on the live
//! session, so the document's text is shaped without reopening it.
//!
//! # A **partial** package is the normal case
//!
//! The browser fetches only the faces the drawing can use (its own requested
//! names plus the default and locale fallbacks), so the package deliberately does
//! not have to carry every face the catalogue names. A face the catalogue names
//! but the package lacks is reported by [`load_package`] as a
//! [`CadError::ResourceMissing`] row when the plan selects it — a named failure,
//! never a silent empty face. Validation still refuses a package that could only
//! produce lies: an unparseable catalogue, an empty catalogue, a zero-length file
//! or a duplicate name.
//!
//! # Why an in-memory package and not a network loader
//!
//! `HostResources::install_fonts` is **synchronous**: the host core calls it from
//! the open funnel and expects the engine to be installed before the call
//! returns. A browser `fetch` inside it would either block (impossible on the
//! page thread) or force the shared contract to become async. The package is
//! therefore materialised by the page and installed through a synchronous wasm
//! call. This is the same split Android uses (assets read up front, then an
//! always-ready loader).
//!
//! # Honesty
//!
//! * A package that does not parse, or carries an empty file, is refused **before**
//!   the slot is swapped: the previously installed package (if any) stays in
//!   force.
//! * [`load_package`] returns `Ok((None, report))` when nothing registered, which
//!   is exactly what the desktop hosts do — an empty engine is never presented as
//!   a real shaping engine.
//! * Without an installed package [`crate::platform::BrowserResources`] keeps its
//!   explicit `unsupported-resources: fonts` refusal; there is no silent empty
//!   success.
//!
//! # Where the bytes come from
//!
//! [`crate::set_font_package`] is the wasm entry point. The host-target build has
//! no JS context, so it exposes the same refusal as the other browser-only
//! exports; the unit tests below drive [`install_package`] directly with the
//! committed files.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use cad_domain::CadResult;
use cad_platform::fonts::{
    load_font_engine_for_locale, FontLoadReport, MemoryFontLoader, TextLocale,
};
use cad_representation::FontEngine;
use cad_resources::FontCatalog;

/// The base the package's catalog and faces are addressed under.
///
/// It is never fetched: [`MemoryFontLoader`] serves every request from memory and
/// only uses the final path segment for its case-insensitive lookup. The scheme is
/// deliberate, so a diagnostic that prints it cannot be mistaken for a real URL.
pub const PACKAGE_BASE: &str = "memory://fonts/";

thread_local! {
    /// The package the current wasm instance installed.
    ///
    /// A thread-local, not a global: one wasm instance runs the page session on
    /// one thread, and a reload starts with an empty slot (there is no durable
    /// storage for a font package).
    static PACKAGE: RefCell<Option<Rc<MemoryFontLoader>>> = const { RefCell::new(None) };
}

/// What one successful [`install_package`] loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageSummary {
    /// Entries in the parsed catalog (the whole catalogue, not only the fetched
    /// subset — it is what every later plan resolves names against).
    pub catalog_entries: usize,
    /// Files actually carried by this (possibly partial) package.
    pub files: usize,
    /// Total bytes of the carried files.
    pub bytes: usize,
}

impl PackageSummary {
    /// The JSON the host returns from [`crate::set_font_package`].
    pub fn to_json(self) -> String {
        format!(
            "{{\"ok\":true,\"catalog_entries\":{},\"files\":{},\"bytes\":{}}}",
            self.catalog_entries, self.files, self.bytes
        )
    }
}

/// Parse and validate a catalog plus its files, then swap it in.
///
/// Validation happens **before** the slot is touched: a bad package cannot
/// replace a good one. The rules are the ones a browser bundle can actually
/// satisfy with a **partial** package (the page fetches only the faces the
/// drawing needs):
///
/// * `catalog_json` must parse as a [`FontCatalog`];
/// * the catalog and the file set must both be non-empty;
/// * every carried file must have a non-empty name and non-empty bytes;
/// * a carried file must not repeat another (case-insensitive bare name).
///
/// Coverage is deliberately **not** required: a face the catalogue names but the
/// caller did not fetch is reported by [`load_package`] as a named
/// [`cad_domain::CadError::ResourceMissing`] row when the plan selects it. Only a
/// package that could produce a lie (an empty or unparseable catalogue, empty
/// bytes) is refused here.
///
/// The package is keyed by bare file name (the catalog guarantees bare names and
/// [`MemoryFontLoader`] normalises lookup case).
pub fn install_package(
    catalog_json: &str,
    files: impl IntoIterator<Item = (String, Vec<u8>)>,
) -> Result<PackageSummary, String> {
    let catalog = FontCatalog::from_json(catalog_json).map_err(|error| error.to_string())?;
    if catalog.is_empty() {
        return Err("font package is empty: the catalog names no face".to_string());
    }

    let mut by_name: HashMap<String, Vec<u8>> = HashMap::new();
    let mut total_bytes = 0usize;
    for (name, bytes) in files {
        if name.trim().is_empty() {
            return Err("font package contains a file with an empty name".to_string());
        }
        if bytes.is_empty() {
            return Err(format!("font package file is empty: {name}"));
        }
        total_bytes = total_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| "font package byte count overflowed usize".to_string())?;
        let normalized = normalize_file_name(&name);
        if by_name.contains_key(&normalized) {
            return Err(format!("font package contains a duplicate file: {name}"));
        }
        by_name.insert(normalized, bytes);
    }
    if by_name.is_empty() {
        return Err("font package is empty: no font files were provided".to_string());
    }

    let file_count = by_name.len();
    let loader = MemoryFontLoader::from_bytes(catalog_json.as_bytes().to_vec(), by_name);
    PACKAGE.with(|slot| {
        *slot.borrow_mut() = Some(Rc::new(loader));
    });
    Ok(PackageSummary {
        catalog_entries: catalog.len(),
        files: file_count,
        bytes: total_bytes,
    })
}

/// The package installed on this thread, if any.
pub fn current() -> Option<Rc<MemoryFontLoader>> {
    PACKAGE.with(|slot| slot.borrow().clone())
}

/// Drop the installed package (a fresh page reload starts here).
pub fn clear() {
    PACKAGE.with(|slot| *slot.borrow_mut() = None);
}

/// Resolve the drawing's fonts from `package` and build the shaping engine.
///
/// This is the synchronous half of the desktop host's font install: the loader's
/// futures are always ready (`MemoryFontLoader`), so
/// [`cad_platform::block_on`] is sound here. The engine is `None` when nothing
/// registered — the caller installs `None` instead of an empty engine, exactly as
/// the desktop strategy does.
pub fn load_package(
    package: &MemoryFontLoader,
    requested: &[String],
    text_needed: bool,
    locale: TextLocale,
) -> CadResult<(Option<Arc<FontEngine>>, FontLoadReport)> {
    let (engine, report) = cad_platform::block_on(load_font_engine_for_locale(
        package,
        requested,
        text_needed,
        PACKAGE_BASE,
        locale,
    ))?;
    let engine = if report.registered.is_empty() {
        None
    } else {
        Some(engine)
    };
    Ok((engine, report))
}

/// Match [`MemoryFontLoader`]'s key normalisation when validating coverage.
fn normalize_file_name(name: &str) -> String {
    let path = name
        .split(['?', '#'])
        .next()
        .unwrap_or(name)
        .trim_end_matches('/');
    path.rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cad_platform::fonts::FontFaceStatus;

    use super::*;

    fn committed_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fonts")
    }

    fn committed_package() -> (String, Vec<(String, Vec<u8>)>) {
        let dir = committed_dir();
        let catalog_path = dir.join("fonts.json");
        let catalog = std::fs::read_to_string(&catalog_path).expect("committed font catalog");
        let parsed = FontCatalog::from_json(&catalog).expect("committed catalog parses");
        let files = parsed
            .faces()
            .map(|face| {
                let path = dir.join(&face.file);
                let bytes = std::fs::read(&path)
                    .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
                (face.file.clone(), bytes)
            })
            .collect();
        (catalog, files)
    }

    #[test]
    fn installs_the_committed_package_and_reports_its_size() {
        clear();
        let (catalog, files) = committed_package();
        let expected_bytes: usize = files.iter().map(|(_, bytes)| bytes.len()).sum();
        let summary = install_package(&catalog, files).expect("committed package installs");
        assert_eq!(summary.catalog_entries, 2, "osifont + Noto Sans CJK SC");
        assert_eq!(summary.files, 2);
        assert_eq!(summary.bytes, expected_bytes);
        assert!(summary.to_json().contains("\"ok\":true"));
        let package = current().expect("the package is installed");
        assert_eq!(package.file_count(), 2);
        clear();
    }

    #[test]
    fn an_unparseable_catalog_is_refused_before_the_slot_is_swapped() {
        clear();
        let error = install_package("[not json", Vec::new()).unwrap_err();
        assert!(error.contains("JSON"), "{error}");
        assert!(current().is_none(), "a refused package installs nothing");
    }

    #[test]
    fn an_empty_package_is_refused() {
        let error = install_package("[]", Vec::new()).unwrap_err();
        assert!(error.contains("empty"), "{error}");

        let catalog = br#"[{"file":"a.shx","name":["a"],"type":"shx"}]"#;
        let error = install_package(std::str::from_utf8(catalog).unwrap(), Vec::new()).unwrap_err();
        assert!(error.contains("no font files"), "{error}");
    }

    #[test]
    fn a_zero_length_file_is_refused() {
        let catalog = br#"[{"file":"a.shx","name":["a"],"type":"shx"}]"#;
        let error = install_package(
            std::str::from_utf8(catalog).unwrap(),
            [("a.shx".to_string(), Vec::new())],
        )
        .unwrap_err();
        assert!(error.contains("empty"), "{error}");
    }

    #[test]
    fn a_partial_package_installs_and_a_missing_face_is_a_named_row_when_planned() {
        clear();
        // The browser fetches only the faces the drawing can use, so "the
        // catalogue names two faces, the package carries one" is the normal case.
        // The carried file is a real font so the shaping engine can register it.
        let (catalog, files) = committed_package();
        let noto = files
            .iter()
            .find(|(name, _)| name == "NotoSansCJKsc-Regular.otf")
            .expect("the committed package carries the Noto face")
            .clone();
        let summary =
            install_package(&catalog, [noto]).expect("a partial package is a valid package");
        assert_eq!(summary.catalog_entries, 2, "the whole catalogue is kept");
        assert_eq!(summary.files, 1, "only the fetched face is carried");

        let package = current().expect("the partial package is installed");
        // The drawing's own face was not fetched: a named ResourceMissing row, and
        // the default fallback (which *was* fetched) still shapes the text.
        let (engine, report) = load_package(
            package.as_ref(),
            &["osifont".to_string()],
            true,
            TextLocale::Latin,
        )
        .expect("a missing face is reported, not a hard error");
        assert!(
            report
                .failed
                .iter()
                .any(|row| row.contains("osifont.ttf")
                    && row.contains("not in the in-memory bundle")),
            "the missing face is a ResourceMissing row: {:?}",
            report.failed
        );
        assert!(
            report.faces.iter().any(|face| {
                face.file.as_deref() == Some("osifont.ttf") && face.status == FontFaceStatus::Failed
            }),
            "the face list carries the failed file: {:?}",
            report.faces
        );
        assert!(
            report
                .registered
                .iter()
                .any(|file| file == "NotoSansCJKsc-Regular.otf"),
            "the fetched default face still registers: {:?}",
            report.registered
        );
        assert!(
            engine.is_some_and(|engine| engine.contains("NotoSansCJKsc-Regular.otf")),
            "the partial package still yields a real shaping engine"
        );
        clear();
    }

    #[test]
    fn a_text_request_registers_the_noto_default_and_the_cjk_fallback() {
        clear();
        let (catalog, files) = committed_package();
        let summary = install_package(&catalog, files).expect("committed package installs");
        assert_eq!(summary.files, 2);
        let package = current().expect("the package is installed");

        // A text-bearing CJK document with no named font: the platform's default
        // face is Noto Sans CJK SC (it covers Latin, kana and CJK), and it is also
        // the locale face for zh-Hans, so text is never silently dropped or
        // Latin-substituted.
        let (engine, report) =
            load_package(package.as_ref(), &[], true, TextLocale::SimplifiedChinese)
                .expect("the in-memory package resolves");
        let engine = engine.expect("a text-bearing drawing installs an engine");
        assert!(
            report
                .registered
                .iter()
                .any(|file| file == "NotoSansCJKsc-Regular.otf"),
            "the default/CJK face registered: {:?}",
            report.registered
        );
        assert_eq!(
            report.default_face.as_deref(),
            Some("NotoSansCJKsc-Regular.otf"),
            "the platform default face is Noto Sans CJK SC"
        );
        assert!(
            engine.contains("NotoSansCJKsc-Regular.otf"),
            "the engine can resolve the Noto face"
        );

        assert_eq!(
            report.locale_faces.first().map(String::as_str),
            Some("NotoSansCJKsc-Regular.otf")
        );
        clear();
    }

    #[test]
    fn nothing_requested_returns_no_engine() {
        clear();
        let (catalog, files) = committed_package();
        install_package(&catalog, files).expect("committed package installs");
        let package = current().expect("the package is installed");
        let (engine, report) =
            load_package(package.as_ref(), &[], false, TextLocale::Latin).expect("no-op load");
        assert!(engine.is_none(), "an empty plan installs no engine");
        assert!(report.registered.is_empty());
        clear();
    }
}
