//! Catalog-driven host font loading: plan → fetch → register → fallback.
//!
//! Spec v2.0 §7.3, §10: the core only builds logical keys and catalog URLs
//! (`cad-resources`); the host fetches the bytes through [`FontLoader`] and
//! registers them with the shaping engine (`cad-representation`). This module
//! holds the parts both browser and Android hosts share so the two platforms
//! cannot drift apart:
//!
//! 1. [`requested_fonts`] reads, read-only, the font keys a drawing references.
//! 2. [`load_font_engine`] fetches `fonts.json`, resolves those keys with
//!    `plan_fonts`, fetches each file and registers it with its encoding, then
//!    installs every registered key as a fallback.
//!
//! Fetching is the only host-specific step; it is expressed as [`FontLoader`],
//! whose future may resolve on a browser network stack or an Android asset
//! read. Unknown names are reported and may use explicit catalog fallbacks (never aliases).

use std::sync::Arc;

use std::collections::HashSet;

use cad_db::DrawingDatabase;
use cad_domain::{CadError, CadResult, SemanticGeometry};
use cad_representation::FontEngine;
use cad_resources::{face_url, plan_fonts, plan_fonts_report, FontCatalog};

use super::{FontLoader, HostFuture};

/// Reserved engine key for a host-supplied default fallback face.
///
/// A desktop host registers its system default font under this key so a
/// drawing font that is missing falls back to the platform default; see
/// [`register_default_face`].
pub const DEFAULT_FALLBACK_KEY: &str = "__yacr_default__";

/// Catalogue names tried, in order, for the last-resort default outline face.
///
/// `osifont` is the committed QCAD ISO 3098 face (web bundles ship it);
/// `arial`/`simplex` mirror the explicit catalog fallbacks below so a CDN-only
/// build still has a usable default when the drawing's own font is missing.
pub const DEFAULT_FALLBACK_NAMES: &[&str] = &["osifont", "arial", "simplex"];

/// Outcome of one catalog-driven loading pass.
///
/// Failures are recorded verbatim instead of being folded into a success;
/// callers surface them so a missing font is visible in diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontLoadReport {
    /// Number of entries in the fetched catalog.
    pub catalog_entries: usize,
    /// Font keys the drawing asked for (before catalog resolution).
    pub requested: Vec<String>,
    /// Catalog file names the plan selected (deduplicated by file).
    pub planned: Vec<String>,
    /// Keys successfully registered in the engine.
    pub registered: Vec<String>,
    /// `<file>: <reason>` for each planned font that could not be registered.
    pub failed: Vec<String>,
    /// Original names missing from the catalog, even when fallback rendering succeeds.
    pub unresolved: Vec<String>,
    /// The default outline face actually registered as the last-resort fallback
    /// (catalog name file when available, or [`DEFAULT_FALLBACK_KEY`] for a
    /// host-supplied system default), if any.
    pub default_face: Option<String>,
}

impl FontLoadReport {
    /// Whether no fonts were requested (the drawing uses no CAD text fonts).
    pub fn is_empty(&self) -> bool {
        self.requested.is_empty() && self.planned.is_empty() && self.registered.is_empty()
    }

    /// One-line, platform-neutral summary for diagnostics.
    pub fn summary(&self) -> String {
        format!(
            "catalog={} requested={} planned={} registered={} failed={} unresolved={}",
            self.catalog_entries,
            self.requested.len(),
            self.planned.len(),
            self.registered.len(),
            self.failed.len(),
            self.unresolved.len(),
        )
    }
}

/// The URL of the catalog JSON under a font base such as
/// `https://…/fonts/` or `asset://fonts/`.
pub fn catalog_url(base: &str) -> String {
    format!("{}/fonts.json", base.trim_end_matches('/'))
}

/// Font keys a drawing references, read from the public database API.
///
/// This only collects logical keys from `TEXT`/`MTEXT` geometry and the text
/// styles those entities point at (`Style::resource_keys`, which the importer
/// populates with the style's SHX/big-font/TTF names). It performs no catalog
/// lookup, no network access and no path resolution: a hostile drawing can
/// therefore only supply names, which `plan_fonts` later sanitises to catalog
/// entries. Names that resolve to nothing remain explicit in the loading report.
pub fn requested_fonts(database: &DrawingDatabase) -> Vec<String> {
    let mut out = Vec::new();
    for entity in database.entities() {
        collect_fonts(database, &entity.geometry, &mut out);
    }
    out
}

fn collect_fonts(database: &DrawingDatabase, geometry: &SemanticGeometry, out: &mut Vec<String>) {
    match geometry {
        SemanticGeometry::Text { font, style, .. } => {
            if let Some(font) = font {
                out.push(font.clone());
            }
            if let Some(style) = database.style(*style) {
                out.extend(style.resource_keys.iter().cloned());
            }
        }
        SemanticGeometry::Compound(parts) => {
            for part in parts {
                collect_fonts(database, part, out);
            }
        }
        _ => {}
    }
}

/// Fetch the catalog, plan the `requested` names and register the bytes.
///
/// Returns the populated engine plus a [`FontLoadReport`]. An empty catalog or
/// an empty plan is not an error: the caller decides whether an empty engine
/// should be installed or `clear_fonts` used instead.
pub fn load_font_engine<'a>(
    loader: &'a dyn FontLoader,
    requested: &'a [String],
    base: &'a str,
) -> HostFuture<'a, (Arc<FontEngine>, FontLoadReport)> {
    Box::pin(async move {
        let catalog_bytes = loader.load_catalog(&catalog_url(base)).await?;
        let catalog_json = std::str::from_utf8(&catalog_bytes)
            .map_err(|e| CadError::InvalidInput(format!("font catalog is not UTF-8: {e}")))?;
        let catalog = FontCatalog::from_json(catalog_json)?;

        let resolution = plan_fonts_report(&catalog, requested, base);
        let mut plan = resolution.planned;
        if !resolution.unresolved.is_empty() || !resolution.unsupported.is_empty() {
            // Explicit, bounded fallbacks, never aliases masquerading as the original font.
            for fallback in plan_fonts(&catalog, &["arial".into(), "simplex".into()], base) {
                if !plan.iter().any(|font| font.file == fallback.file) {
                    plan.push(fallback);
                }
            }
        }
        let mut engine = FontEngine::new();
        let mut report = FontLoadReport {
            catalog_entries: catalog.len(),
            requested: requested.to_vec(),
            unresolved: resolution.unresolved,
            failed: resolution
                .unsupported
                .iter()
                .map(|key| format!("{key}: unsupported font technology"))
                .collect(),
            ..FontLoadReport::default()
        };
        for font in &plan {
            report.planned.push(font.file.clone());
            match loader.load_font(&font.url).await {
                Ok(bytes) => {
                    match engine.register_with_encoding(&font.file, bytes, font.encoding.as_deref())
                    {
                        Ok(()) => report.registered.push(font.file.clone()),
                        Err(e) => report.failed.push(format!("{}: {e}", font.file)),
                    }
                }
                Err(e) => report.failed.push(format!("{}: {e}", font.file)),
            }
        }
        // Guarantee a default outline face whenever a requested font could not
        // be resolved or fetched: the first catalogue name that resolves is
        // registered once (never re-fetching a file already attempted) and
        // reported. This is what lets a missing drawing font shape with a
        // default instead of being dropped.
        let attempted: HashSet<&String> = plan.iter().map(|font| &font.file).collect();
        let needs_default = !report.unresolved.is_empty() || !report.failed.is_empty();
        if !requested.is_empty() && needs_default {
            for name in DEFAULT_FALLBACK_NAMES {
                let Some(face) = catalog.get(name) else {
                    continue;
                };
                if report.registered.iter().any(|file| file == &face.file) {
                    report.default_face = Some(face.file.clone());
                    break;
                }
                if attempted.contains(&face.file) {
                    // Already planned (and either registered above or reported as
                    // failed); never fetch it a second time.
                    continue;
                }
                match loader.load_font(&face_url(base, face)).await {
                    Ok(bytes) => {
                        match engine.register_with_encoding(
                            &face.file,
                            bytes,
                            face.encoding.as_deref(),
                        ) {
                            Ok(()) => {
                                report.registered.push(face.file.clone());
                                report.default_face = Some(face.file.clone());
                                break;
                            }
                            Err(e) => report.failed.push(format!("{}: {e}", face.file)),
                        }
                    }
                    Err(e) => report.failed.push(format!("{}: {e}", face.file)),
                }
            }
        }
        // A registered face stands in for any missing one so text is not
        // silently dropped; the order follows the plan.
        engine.set_fallback(report.registered.clone());
        Ok((Arc::new(engine), report))
    })
}

/// Register a host-supplied default face and make it the **first** fallback.
///
/// Desktop hosts use this to prefer a system font over the drawing/catalog
/// fallbacks when a requested font is missing. The bytes must parse exactly as
/// in [`FontEngine::register`]; a host that cannot obtain a system font simply
/// does not call this and the catalog default is used instead.
pub fn register_default_face(engine: &mut FontEngine, bytes: Arc<[u8]>) -> CadResult<()> {
    engine.register(DEFAULT_FALLBACK_KEY, bytes)?;
    let mut keys = vec![DEFAULT_FALLBACK_KEY.to_string()];
    for key in engine.fallback_keys() {
        if key != DEFAULT_FALLBACK_KEY {
            keys.push(key.clone());
        }
    }
    engine.set_fallback(keys);
    Ok(())
}

/// Local (native) font-package loader shared by desktop hosts.
///
/// This is the "sibling `fonts/` package" half of the font design: a directory
/// next to the executable holds `fonts.json` plus the committed faces, and the
/// host loads exactly the faces a drawing requests (plus a default) from it, so
/// a packaged client needs no network. Kept out of wasm builds, which use the
/// browser network stack instead.
#[cfg(not(target_arch = "wasm32"))]
pub mod local {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use cad_domain::{CadError, CadResult};

    use super::*;
    use crate::block_on;

    /// A [`FontLoader`] rooted at a font directory.
    ///
    /// The base handed to [`load_font_engine`] is the directory itself, so the
    /// catalog and face URLs `cad-resources` builds are already paths under it.
    pub struct DirFontLoader {
        root: PathBuf,
    }

    impl DirFontLoader {
        pub fn new(root: PathBuf) -> Self {
            Self { root }
        }

        fn path_for(&self, url: &str) -> CadResult<PathBuf> {
            let root = self.root.to_string_lossy();
            // `face_url` percent-encodes, but catalog parsing guarantees a bare
            // file name; the prefix check is defence in depth against a caller
            // passing a URL that points outside the granted directory.
            if !url.starts_with(root.as_ref()) {
                return Err(CadError::InvalidInput(format!(
                    "font path escapes the font directory: {url}"
                )));
            }
            Ok(PathBuf::from(url))
        }
    }

    impl FontLoader for DirFontLoader {
        fn load_font(&self, url: &str) -> HostFuture<'_, Arc<[u8]>> {
            let path = self.path_for(url);
            Box::pin(async move {
                let path = path?;
                std::fs::read(&path)
                    .map(|bytes| Arc::from(bytes.into_boxed_slice()))
                    .map_err(|e| {
                        CadError::ResourceMissing(format!(
                            "font read failed for {}: {e}",
                            path.display()
                        ))
                    })
            })
        }
    }

    /// Candidate `fonts/` directories for an executable path, most specific
    /// first: next to the binary, next to its `bin/` parent, then the working
    /// directory.
    pub fn font_dir_candidates(exe: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(dir) = exe.parent() {
            out.push(dir.join("fonts"));
            if let Some(parent) = dir.parent() {
                out.push(parent.join("fonts"));
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            out.push(cwd.join("fonts"));
        }
        out
    }

    /// Resolve the font directory to use.
    ///
    /// An explicit path must exist (a typo is an error, not a silent fallback);
    /// otherwise the first existing candidate for `exe` is used, or `None`.
    pub fn resolve_font_dir(explicit: Option<&Path>, exe: &Path) -> CadResult<Option<PathBuf>> {
        match explicit {
            Some(dir) => {
                if dir.is_dir() {
                    Ok(Some(dir.to_path_buf()))
                } else {
                    Err(CadError::InvalidInput(format!(
                        "font directory does not exist: {}",
                        dir.display()
                    )))
                }
            }
            None => Ok(font_dir_candidates(exe)
                .into_iter()
                .find(|dir| dir.is_dir())),
        }
    }

    /// Ask fontconfig for the file backing `family`; `None` when fontconfig is
    /// absent or the family resolves to nothing.
    ///
    /// Unix (and Android) only: Windows resolves its own system fonts from the
    /// font directory instead of `fc-match`.
    #[cfg(not(target_os = "windows"))]
    fn fc_match(family: &str) -> Option<String> {
        let output = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", family])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let path = String::from_utf8(output.stdout).ok()?;
        let path = path.trim();
        (!path.is_empty()).then(|| path.to_string())
    }

    /// Best-effort system default face bytes, most specific first.
    ///
    /// Entries the shaping engine cannot parse (for example a `.ttc` collection)
    /// are skipped by the caller; this only collects candidate bytes.
    #[cfg(not(target_os = "windows"))]
    pub fn system_default_candidates() -> Vec<Arc<[u8]>> {
        let mut out = Vec::new();
        let mut push = |path: &str| {
            if let Ok(bytes) = std::fs::read(path) {
                out.push(Arc::from(bytes.into_boxed_slice()));
            }
        };
        for family in [
            "sans-serif",
            "DejaVu Sans",
            "Liberation Sans",
            "Noto Sans",
            "Arial",
        ] {
            if let Some(path) = fc_match(family) {
                push(&path);
            }
        }
        for path in [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
        ] {
            push(path);
        }
        out
    }

    /// Windows: read the usual UI faces from the system font directory.
    ///
    /// There is no `fc-match`; `%WINDIR%\Fonts` is the machine-wide directory and
    /// `%LOCALAPPDATA%\Microsoft\Windows\Fonts` holds per-user installs. Only the
    /// byte candidates are collected here; the shaping engine skips anything it
    /// cannot parse (for example a `.ttc` collection).
    #[cfg(target_os = "windows")]
    pub fn system_default_candidates() -> Vec<Arc<[u8]>> {
        let mut out = Vec::new();
        let mut push = |path: &Path| {
            if let Ok(bytes) = std::fs::read(path) {
                out.push(Arc::from(bytes.into_boxed_slice()));
            }
        };
        let mut dirs = Vec::new();
        if let Some(windir) = std::env::var_os("WINDIR") {
            dirs.push(PathBuf::from(windir).join("Fonts"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(
                PathBuf::from(local)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Fonts"),
            );
        }
        for dir in dirs {
            for name in [
                "segoeui.ttf",
                "arial.ttf",
                "calibri.ttf",
                "tahoma.ttf",
                "verdana.ttf",
                "consola.ttf",
            ] {
                push(&dir.join(name));
            }
        }
        out
    }

    /// Cached [`system_default_candidates`] for the process.
    ///
    /// Reading a large system collection (`NotoSansCJK.ttc` is ~19 MiB) on every
    /// document open would be wasteful, so the candidates are materialised once.
    pub fn cached_system_default_candidates() -> &'static Vec<Arc<[u8]>> {
        static CACHE: std::sync::OnceLock<Vec<Arc<[u8]>>> = std::sync::OnceLock::new();
        CACHE.get_or_init(system_default_candidates)
    }

    /// Build a shaping engine from the `fonts/` package, explicit entries and a
    /// system default, or `None` when nothing is needed.
    ///
    /// `requested` are the drawing's font keys, `explicit` the `--font` entries
    /// and `system_default` the ordered default-face candidates.
    pub fn load_engine(
        font_dir: Option<&Path>,
        requested: &[String],
        explicit: &[(String, PathBuf)],
        system_default: &[Arc<[u8]>],
    ) -> CadResult<Option<Arc<FontEngine>>> {
        if requested.is_empty() && explicit.is_empty() {
            return Ok(None);
        }
        let mut engine = match font_dir {
            Some(dir) => {
                let base = dir.to_string_lossy().to_string();
                let loader = DirFontLoader::new(dir.to_path_buf());
                let (loaded, _report) = block_on(load_font_engine(&loader, requested, &base))?;
                // `load_font_engine` returns a uniquely owned engine; unwrapping it
                // lets the explicit/system registrations below extend the same set.
                Arc::try_unwrap(loaded)
                    .map_err(|_| CadError::Invariant("font engine unexpectedly shared".into()))?
            }
            None => FontEngine::new(),
        };
        let mut fallback = engine.fallback_keys().to_vec();
        for (name, path) in explicit {
            let bytes = std::fs::read(path)
                .map_err(|e| CadError::InvalidInput(format!("font '{}': {e}", path.display())))?;
            engine.register(name, Arc::from(bytes.into_boxed_slice()))?;
            if !fallback.iter().any(|key| key == name) {
                fallback.push(name.clone());
            }
        }
        engine.set_fallback(fallback);
        // The first parseable system face becomes the preferred default; a `.ttc`
        // that the engine rejects is simply skipped.
        for bytes in system_default {
            if register_default_face(&mut engine, bytes.clone()).is_ok() {
                break;
            }
        }
        if engine.is_empty() {
            return Ok(None);
        }
        Ok(Some(Arc::new(engine)))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn candidates_prefer_the_binary_side_fonts_dir() {
            let exe = Path::new("/opt/yacr/bin/yacr-linux");
            let candidates = font_dir_candidates(exe);
            assert_eq!(candidates[0], PathBuf::from("/opt/yacr/bin/fonts"));
            assert_eq!(candidates[1], PathBuf::from("/opt/yacr/fonts"));
        }

        #[test]
        fn explicit_missing_font_dir_is_an_error_not_a_silent_fallback() {
            let missing = Path::new("/definitely/not/a/font/dir");
            let result = resolve_font_dir(Some(missing), Path::new("/opt/yacr/bin/yacr-linux"));
            assert!(matches!(result, Err(CadError::InvalidInput(_))));
        }

        #[test]
        fn nothing_requested_and_no_explicit_fonts_installs_nothing() {
            let engine = load_engine(None, &[], &[], &[]).unwrap();
            assert!(engine.is_none());
        }

        #[test]
        fn explicit_font_is_registered_and_used_as_fallback() {
            let dir = std::env::temp_dir().join(format!("yacr-font-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("explicit.shx");
            std::fs::write(&path, synthetic_shx()).unwrap();
            let engine = load_engine(
                None,
                &["missing.shx".to_string()],
                &[("MyFont".to_string(), path.clone())],
                &[],
            )
            .unwrap()
            .expect("explicit font installs an engine");
            assert!(engine.contains("MyFont"));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn system_default_becomes_the_first_fallback() {
            let engine = load_engine(
                None,
                &["missing.shx".to_string()],
                &[],
                &[Arc::from(synthetic_shx())],
            )
            .unwrap()
            .expect("a system default installs an engine");
            assert_eq!(
                engine.fallback_keys().first().map(String::as_str),
                Some(DEFAULT_FALLBACK_KEY)
            );
            assert!(engine.contains(DEFAULT_FALLBACK_KEY));
        }

        /// Minimal but parseable compiled SHX shape font bytes.
        fn synthetic_shx() -> Vec<u8> {
            let mut bytes = b"AutoCAD-86 shapes 1.0\r\n\x1a".to_vec();
            let info = b"Synthetic\0\x15\x07\x02\0";
            for value in [0u16, 0, 1, 0, info.len() as u16] {
                bytes.extend(value.to_le_bytes());
            }
            bytes.extend(info);
            bytes
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer, Style};
    use cad_domain::{
        CadResult, DatabaseId, EntityId, LayerId, ObjectId, Point3, Revision, SpaceId, StyleId,
        TextAlignH, TextAlignV,
    };
    use std::collections::HashMap;
    use std::future::Future;
    use std::sync::Mutex;
    use std::task::{Context, Poll, Waker};

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    /// Drive a future to completion on a no-op waker (all steps are ready).
    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = Box::pin(future);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            match future.as_mut().poll(&mut cx) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn text_entity(id: u128, font: Option<&str>, style: StyleId) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbText".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Text {
                text: "hello".into(),
                position: p(0.0, 0.0),
                style,
                height: 2.5,
                rotation: 0.0,
                font: font.map(str::to_string),
                h_align: TextAlignH::Left,
                v_align: TextAlignV::Baseline,
            },
            draw_order: 0,
        }
    }

    /// Minimal but parseable compiled SHX shape font bytes.
    fn synthetic_shx() -> Arc<[u8]> {
        let mut bytes = b"AutoCAD-86 shapes 1.0\r\n\x1a".to_vec();
        let info = b"Synthetic\0\x15\x07\x02\0";
        for value in [0u16, 0, 1, 0, info.len() as u16] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(info);
        Arc::from(bytes)
    }

    fn database_with_text() -> DrawingDatabase {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
        builder
            .insert_layer(Layer {
                id: LayerId(0),
                name: "0".into(),
                visible: true,
            })
            .unwrap();
        builder
            .insert_style(Style {
                id: StyleId(7),
                name: "Standard".into(),
                resource_keys: vec!["bigfont.shx".into()],
            })
            .unwrap();
        builder
            .insert_entity(text_entity(1, Some("simplex.shx"), StyleId(7)))
            .unwrap();
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(2),
                    type_key: "AcDbLine".into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: EntityId(2),
                layer: LayerId(0),
                space: SpaceId::Model,
                geometry: SemanticGeometry::Line {
                    start: p(0.0, 0.0),
                    end: p(1.0, 1.0),
                },
                draw_order: 1,
            })
            .unwrap();
        builder.finish().unwrap()
    }

    #[test]
    fn collects_font_and_style_resource_keys() {
        let db = database_with_text();
        let mut names = requested_fonts(&db);
        names.sort();
        assert_eq!(names, vec!["bigfont.shx".to_string(), "simplex.shx".into()]);
    }

    #[test]
    fn catalog_url_normalises_trailing_slash() {
        assert_eq!(catalog_url("https://x/fonts"), "https://x/fonts/fonts.json");
        assert_eq!(
            catalog_url("https://x/fonts/"),
            "https://x/fonts/fonts.json"
        );
        assert_eq!(catalog_url("asset://fonts/"), "asset://fonts/fonts.json");
    }

    /// A loader that serves a canned catalog plus per-URL font bytes.
    struct FakeLoader {
        catalog: Arc<[u8]>,
        files: Mutex<HashMap<String, CadResult<Arc<[u8]>>>>,
        seen: Mutex<Vec<String>>,
    }

    impl FontLoader for FakeLoader {
        fn load_font(&self, url: &str) -> HostFuture<'_, Arc<[u8]>> {
            self.seen.lock().unwrap().push(url.to_string());
            let url = url.to_string();
            if url.ends_with("fonts.json") {
                let catalog = self.catalog.clone();
                return Box::pin(async move { Ok(catalog) });
            }
            let result = self.files.lock().unwrap().remove(&url);
            Box::pin(async move {
                result.unwrap_or_else(|| {
                    Err(CadError::InvalidInput(format!("no fake bytes for {url}")))
                })
            })
        }
    }

    #[test]
    fn loads_catalog_and_reports_registration_failure() {
        let catalog: Arc<[u8]> = Arc::from(
            br#"[{ "file": "simplex.shx", "name": ["simplex"], "type": "shx" }]"#
                .to_vec()
                .into_boxed_slice(),
        );
        // Deliberately invalid font bytes: registration must be reported as a
        // failure, never silently counted as success.
        let mut files = HashMap::new();
        files.insert(
            "https://example/fonts/simplex.shx".to_string(),
            Ok(Arc::from(b"not-a-font".to_vec().into_boxed_slice())),
        );
        let loader = FakeLoader {
            catalog,
            files: Mutex::new(files),
            seen: Mutex::new(Vec::new()),
        };
        let requested = vec!["simplex".to_string()];
        let (engine, report) = block_on(load_font_engine(
            &loader,
            &requested,
            "https://example/fonts",
        ))
        .unwrap();
        assert_eq!(report.catalog_entries, 1);
        assert_eq!(report.planned, vec!["simplex.shx".to_string()]);
        assert!(report.registered.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert!(engine.is_empty());
        assert!(report.summary().contains("failed=1"));
    }

    #[test]
    fn unknown_names_are_skipped_not_faked() {
        let catalog: Arc<[u8]> = Arc::from(
            br#"[{"file":"a.shx","name":["a"],"type":"shx"}]"#
                .to_vec()
                .into_boxed_slice(),
        );
        let loader = FakeLoader {
            catalog,
            files: Mutex::new(HashMap::new()),
            seen: Mutex::new(Vec::new()),
        };
        let requested = vec!["does-not-exist.ttf".to_string()];
        let (_engine, report) = block_on(load_font_engine(
            &loader,
            &requested,
            "https://example/fonts",
        ))
        .unwrap();
        assert!(report.planned.is_empty());
        assert!(report.registered.is_empty());
        assert!(report.failed.is_empty());
        assert_eq!(report.unresolved, requested);
        // The catalog was fetched (one URL) but no font URL was requested.
        assert_eq!(loader.seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn missing_names_load_explicit_catalog_fallback_without_claiming_resolution() {
        let loader = FakeLoader {
            catalog: Arc::from(
                br#"[{"file":"simplex.shx","name":["simplex"],"type":"shx"}]"#.as_slice(),
            ),
            files: Mutex::new(HashMap::from([(
                "https://example/fonts/simplex.shx".into(),
                Ok(synthetic_shx()),
            )])),
            seen: Mutex::new(Vec::new()),
        };
        let requested = vec!["missing-original.shx".into()];
        let (engine, report) = block_on(load_font_engine(
            &loader,
            &requested,
            "https://example/fonts",
        ))
        .unwrap();
        assert_eq!(report.unresolved, requested);
        assert_eq!(report.registered, vec!["simplex.shx"]);
        assert!(!engine.contains("missing-original.shx"));
        assert_eq!(engine.fallback_keys(), &["simplex.shx"]);
    }

    #[test]
    fn missing_font_registers_the_default_catalog_face() {
        // The drawing's font is absent; `osifont` (the committed default) is in
        // the catalog and must be registered and reported as the default face.
        let loader = FakeLoader {
            catalog: Arc::from(
                br#"[{"file":"simplex.shx","name":["simplex"],"type":"shx"},
                     {"file":"osifont.ttf","name":["osifont"],"type":"mesh"}]"#
                    .as_slice(),
            ),
            files: Mutex::new(HashMap::from([
                (
                    "https://example/fonts/simplex.shx".into(),
                    Ok(synthetic_shx()),
                ),
                (
                    "https://example/fonts/osifont.ttf".into(),
                    Ok(synthetic_shx()),
                ),
            ])),
            seen: Mutex::new(Vec::new()),
        };
        let requested = vec!["missing.shx".into()];
        let (engine, report) = block_on(load_font_engine(
            &loader,
            &requested,
            "https://example/fonts",
        ))
        .unwrap();
        assert_eq!(report.default_face.as_deref(), Some("osifont.ttf"));
        assert!(report.registered.contains(&"osifont.ttf".to_string()));
        assert!(engine.fallback_keys().iter().any(|k| k == "osifont.ttf"));
    }

    #[test]
    fn host_default_face_is_prepended_to_the_fallback_chain() {
        let mut engine = FontEngine::new();
        engine.register("arial.woff", synthetic_shx()).unwrap();
        engine.set_fallback(vec!["arial.woff".into()]);
        register_default_face(&mut engine, synthetic_shx()).unwrap();
        assert_eq!(
            engine.fallback_keys().first().map(String::as_str),
            Some(DEFAULT_FALLBACK_KEY)
        );
        assert!(engine.contains(DEFAULT_FALLBACK_KEY));
        assert!(engine.fallback_keys().contains(&"arial.woff".to_string()));
    }

    #[test]
    fn catalog_fetch_error_is_propagated() {
        struct NoCatalog;
        impl FontLoader for NoCatalog {
            fn load_font(&self, url: &str) -> HostFuture<'_, Arc<[u8]>> {
                let url = url.to_string();
                Box::pin(async move { Err(CadError::ResourceMissing(format!("offline: {url}"))) })
            }
        }
        let requested = vec!["simplex".to_string()];
        let result = block_on(load_font_engine(
            &NoCatalog,
            &requested,
            "https://example/fonts",
        ));
        assert!(matches!(result, Err(CadError::ResourceMissing(_))));
    }
}
