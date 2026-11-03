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
/// first: next to the binary, next to its `bin/` parent, under a macOS
/// `.app` bundle's `Contents/Resources/`, then the working directory.
pub fn font_dir_candidates(exe: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = exe.parent() {
        out.push(dir.join("fonts"));
        if let Some(parent) = dir.parent() {
            out.push(parent.join("fonts"));
            // macOS `.app` layout: `Contents/MacOS/<exe>` with the font
            // package at `Contents/Resources/fonts`. Harmless on other
            // platforms (the path simply does not exist).
            out.push(parent.join("Resources").join("fonts"));
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
/// Unix (and Android) only: Windows and macOS resolve their own system fonts
/// from the font directories instead of `fc-match`.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
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

/// One system default face candidate: its bare file name and bytes.
///
/// The name is what [`FontLoadReport::faces`] reports as the system default
/// row's `file`; the bytes are what the engine registers.
#[derive(Clone)]
pub struct SystemFontCandidate {
    pub name: String,
    pub bytes: Arc<[u8]>,
}

/// The final path segment of `path` (either separator).
fn file_basename(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// The font technology an explicit file name suggests, for the face report.
fn kind_for_file(path: &Path) -> String {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("shx") => "shx".to_string(),
        Some("ttf" | "otf" | "woff") => "mesh".to_string(),
        _ => "unknown".to_string(),
    }
}

/// Best-effort system default faces, most specific first.
///
/// Entries the shaping engine cannot parse (for example a `.ttc` collection)
/// are skipped by the caller; this only collects candidates. Duplicate
/// `fc-match` answers for different families are collapsed.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn system_default_fonts() -> Vec<SystemFontCandidate> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |path: &str| {
        if !seen.insert(path.to_string()) {
            return;
        }
        if let Ok(bytes) = std::fs::read(path) {
            out.push(SystemFontCandidate {
                name: file_basename(path),
                bytes: Arc::from(bytes.into_boxed_slice()),
            });
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
/// `%LOCALAPPDATA%\Microsoft\Windows\Fonts` holds per-user installs. The
/// shaping engine skips anything it cannot parse (for example a `.ttc`
/// collection); this only collects candidates plus their file names.
#[cfg(target_os = "windows")]
pub fn system_default_fonts() -> Vec<SystemFontCandidate> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |path: &Path| {
        let shown = path.to_string_lossy();
        if !seen.insert(shown.to_string()) {
            return;
        }
        if let Ok(bytes) = std::fs::read(path) {
            out.push(SystemFontCandidate {
                name: file_basename(&shown),
                bytes: Arc::from(bytes.into_boxed_slice()),
            });
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

/// macOS: read the usual UI faces from the system font directories.
///
/// There is no `fc-match`; the machine-wide faces live in
/// `/System/Library/Fonts` (with supplemental faces under
/// `Supplemental/`) and the administrator-installed faces in `/Library/Fonts`.
/// The shaping engine skips anything it cannot parse (for example the many
/// `.ttc` collections and the variable-axis `SFNS.ttf`); this only collects
/// candidates plus their file names. System Integrity Protection guarantees
/// these paths exist and are readable on a stock install.
#[cfg(target_os = "macos")]
pub fn system_default_fonts() -> Vec<SystemFontCandidate> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |path: &str| {
        if !seen.insert(path.to_string()) {
            return;
        }
        if let Ok(bytes) = std::fs::read(path) {
            out.push(SystemFontCandidate {
                name: file_basename(path),
                bytes: Arc::from(bytes.into_boxed_slice()),
            });
        }
    };
    for path in [
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/System/Library/Fonts/Helvetica.ttf",
        "/System/Library/Fonts/Geneva.ttf",
        "/System/Library/Fonts/Supplemental/Verdana.ttf",
        "/Library/Fonts/Arial.ttf",
    ] {
        push(path);
    }
    out
}

/// Cached, file-name-carrying [`system_default_fonts`] for the process.
///
/// Reading a large system collection (`NotoSansCJK.ttc` is ~19 MiB) on every
/// document open would be wasteful, so the candidates are materialised once.
pub fn cached_system_default_fonts() -> &'static Vec<SystemFontCandidate> {
    static CACHE: std::sync::OnceLock<Vec<SystemFontCandidate>> = std::sync::OnceLock::new();
    CACHE.get_or_init(system_default_fonts)
}

/// Best-effort system default face bytes, most specific first.
///
/// The bytes are clones of [`cached_system_default_fonts`], so a caller that
/// later passes them to [`load_engine_report`] keeps the file name.
pub fn system_default_candidates() -> Vec<Arc<[u8]>> {
    cached_system_default_fonts()
        .iter()
        .map(|candidate| Arc::clone(&candidate.bytes))
        .collect()
}

/// Cached [`system_default_candidates`] for the process.
pub fn cached_system_default_candidates() -> &'static Vec<Arc<[u8]>> {
    static CACHE: std::sync::OnceLock<Vec<Arc<[u8]>>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        cached_system_default_fonts()
            .iter()
            .map(|candidate| Arc::clone(&candidate.bytes))
            .collect()
    })
}

/// The bare file name of a cached system candidate, when `bytes` came from
/// [`system_default_candidates`]/[`cached_system_default_candidates`].
///
/// Identity is by `Arc` pointer, which is exact for the cached clones; a
/// caller passing unrelated bytes simply gets `None` (the report keeps the
/// row with `file: None` rather than inventing a name).
fn system_face_name(bytes: &Arc<[u8]>) -> Option<String> {
    cached_system_default_fonts()
        .iter()
        .find(|candidate| Arc::ptr_eq(&candidate.bytes, bytes))
        .map(|candidate| candidate.name.clone())
}

/// Build a shaping engine from the `fonts/` package, explicit entries and a
/// system default, or `None` when nothing is needed.
///
/// `requested` are the drawing's font keys, `text_needed` whether the drawing
/// contains text that must be shaped even though it names no font, `explicit`
/// the `--font` entries and `system_default` the ordered default-face
/// candidates. A drawing with neither text nor a named font installs nothing;
/// a text-bearing drawing always gets an engine so its glyphs resolve through
/// the fallback chain instead of being silently dropped.
///
/// This variant assumes [`TextLocale::Latin`]; use [`load_engine_report`]
/// with the locale derived from the drawing's code page.
pub fn load_engine(
    font_dir: Option<&Path>,
    requested: &[String],
    text_needed: bool,
    explicit: &[(String, PathBuf)],
    system_default: &[Arc<[u8]>],
) -> CadResult<Option<Arc<FontEngine>>> {
    Ok(load_engine_report(
        font_dir,
        requested,
        text_needed,
        explicit,
        system_default,
        TextLocale::Latin,
    )?
    .0)
}

/// Like [`load_engine`], but for a document whose text locale is known, and
/// with the load report (locale gap included) returned to the caller.
///
/// A locale with no preferred face available still installs the Latin chain
/// and reports the gap; it never fails the open and never silently pretends
/// the locale was satisfied.
pub fn load_engine_report(
    font_dir: Option<&Path>,
    requested: &[String],
    text_needed: bool,
    explicit: &[(String, PathBuf)],
    system_default: &[Arc<[u8]>],
    locale: TextLocale,
) -> CadResult<(Option<Arc<FontEngine>>, FontLoadReport)> {
    let mut report = FontLoadReport {
        locale,
        ..FontLoadReport::default()
    };
    if requested.is_empty() && explicit.is_empty() && !text_needed {
        return Ok((None, report));
    }
    let mut engine = match font_dir {
        Some(dir) => {
            let base = dir.to_string_lossy().to_string();
            let loader = DirFontLoader::new(dir.to_path_buf());
            let (loaded, loaded_report) = block_on(load_font_engine_for_locale(
                &loader,
                requested,
                text_needed,
                &base,
                locale,
            ))?;
            report = loaded_report;
            // `load_font_engine_for_locale` returns a uniquely owned engine;
            // unwrapping it lets the explicit/system registrations below
            // extend the same set.
            Arc::try_unwrap(loaded)
                .map_err(|_| CadError::Invariant("font engine unexpectedly shared".into()))?
        }
        None => {
            // No package to resolve the locale's faces from: the gap is
            // known here rather than after a fetch attempt.
            if locale != TextLocale::Latin && text_needed {
                report.locale_missing = Some(LocaleFallbackGap {
                    locale,
                    candidates: locale
                        .preferred_faces()
                        .iter()
                        .map(|name| (*name).to_string())
                        .collect(),
                });
            }
            FontEngine::new()
        }
    };
    let mut fallback = engine.fallback_keys().to_vec();
    for (name, path) in explicit {
        let kind = kind_for_file(path);
        let file = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string());
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                report.push_face(FontFaceRecord {
                    key: name.clone(),
                    file: file.clone(),
                    kind: kind.clone(),
                    source: FontFaceSource::Explicit,
                    status: FontFaceStatus::Failed,
                    detail: Some(e.to_string()),
                });
                return Err(CadError::InvalidInput(format!(
                    "font '{}': {e}",
                    path.display()
                )));
            }
        };
        if let Err(e) = engine.register(name, Arc::from(bytes.into_boxed_slice())) {
            report.push_face(FontFaceRecord {
                key: name.clone(),
                file,
                kind,
                source: FontFaceSource::Explicit,
                status: FontFaceStatus::Failed,
                detail: Some(e.to_string()),
            });
            return Err(e);
        }
        report.push_face(FontFaceRecord {
            key: name.clone(),
            file,
            kind,
            source: FontFaceSource::Explicit,
            status: FontFaceStatus::Loaded,
            detail: None,
        });
        if !fallback.iter().any(|key| key == name) {
            fallback.push(name.clone());
        }
    }
    engine.set_fallback(fallback);
    // The first parseable system face becomes the preferred default; a `.ttc`
    // that the engine rejects is simply skipped.
    for bytes in system_default {
        if register_default_face(&mut engine, bytes.clone()).is_ok() {
            report.push_face(FontFaceRecord {
                key: DEFAULT_FALLBACK_KEY.to_string(),
                file: system_face_name(bytes),
                kind: "system".to_string(),
                source: FontFaceSource::System,
                status: FontFaceStatus::Loaded,
                detail: None,
            });
            break;
        }
    }
    if engine.is_empty() {
        return Ok((None, report));
    }
    engine.set_locale_tag(locale.as_str());
    // An explicit/`--font` CJK face also satisfies the locale; the gap is
    // only real when the engine truly has none of the preferred faces.
    if report.locale_missing.is_some() && locale_gap(&engine, locale, text_needed).is_none() {
        report.locale_missing = None;
    }
    Ok((Some(Arc::new(engine)), report))
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
        // macOS `.app` layout: `Contents/MacOS/<exe>` -> `Contents/Resources/fonts`.
        let app_exe = Path::new("/opt/Yacr.app/Contents/MacOS/yacr-macos");
        let app_candidates = font_dir_candidates(app_exe);
        assert_eq!(
            app_candidates[0],
            PathBuf::from("/opt/Yacr.app/Contents/MacOS/fonts")
        );
        assert_eq!(
            app_candidates[1],
            PathBuf::from("/opt/Yacr.app/Contents/fonts")
        );
        assert_eq!(
            app_candidates[2],
            PathBuf::from("/opt/Yacr.app/Contents/Resources/fonts")
        );
    }

    #[test]
    fn explicit_missing_font_dir_is_an_error_not_a_silent_fallback() {
        let missing = Path::new("/definitely/not/a/font/dir");
        let result = resolve_font_dir(Some(missing), Path::new("/opt/yacr/bin/yacr-linux"));
        assert!(matches!(result, Err(CadError::InvalidInput(_))));
    }

    #[test]
    fn nothing_requested_and_no_explicit_fonts_installs_nothing() {
        let engine = load_engine(None, &[], false, &[], &[]).unwrap();
        assert!(engine.is_none());
    }

    #[test]
    fn text_with_no_named_font_installs_a_fallback_engine() {
        // A drawing can hold text whose style names no font. It still needs
        // shaping, so the default face must be installed instead of returning
        // `None`; the glyphs then come from the fallback chain.
        let engine = load_engine(None, &[], true, &[], &[Arc::from(synthetic_shx())])
            .unwrap()
            .expect("text without a named font installs an engine");
        assert_eq!(
            engine.fallback_keys().first().map(String::as_str),
            Some(DEFAULT_FALLBACK_KEY)
        );
    }

    #[test]
    fn text_without_a_named_font_loads_the_font_package_default() {
        // `--fonts-dir` (or the sibling package) alone must rescue text with
        // no named font: the catalog default is registered even though
        // `requested` is empty.
        let dir =
            std::env::temp_dir().join(format!("yacr-font-package-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("fonts.json"),
            br#"[{ "file": "osifont.shx", "name": ["osifont"], "type": "shx" }]"#,
        )
        .unwrap();
        std::fs::write(dir.join("osifont.shx"), synthetic_shx()).unwrap();
        let engine = load_engine(Some(&dir), &[], true, &[], &[])
            .unwrap()
            .expect("the package default installs an engine");
        assert!(engine.contains("osifont.shx"));
        let _ = std::fs::remove_dir_all(&dir);
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
            false,
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
            false,
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
