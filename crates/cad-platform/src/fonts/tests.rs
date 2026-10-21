use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer, Style};
use cad_domain::{
    CadResult, DatabaseId, EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry,
    SpaceId, StyleId, TextAlignH, TextAlignV,
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
            wrap_width: None,
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
            result
                .unwrap_or_else(|| Err(CadError::InvalidInput(format!("no fake bytes for {url}"))))
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
        false,
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
        false,
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
        false,
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
        false,
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
fn a_resolved_named_font_does_not_fetch_the_unused_default() {
    // `text_needed` must not turn every text-bearing drawing into an extra
    // default-font fetch: the default is only for text with no usable named
    // font.
    let loader = FakeLoader {
        catalog: Arc::from(
            br#"[{"file":"simplex.shx","name":["simplex"],"type":"shx"},
                     {"file":"osifont.ttf","name":["osifont"],"type":"mesh"}]"#
                .as_slice(),
        ),
        files: Mutex::new(HashMap::from([(
            "https://example/fonts/simplex.shx".into(),
            Ok(synthetic_shx()),
        )])),
        seen: Mutex::new(Vec::new()),
    };
    let requested = vec!["simplex".to_string()];
    let (engine, report) = block_on(load_font_engine(
        &loader,
        &requested,
        true,
        "https://example/fonts",
    ))
    .unwrap();
    assert_eq!(report.registered, vec!["simplex.shx"]);
    assert_eq!(report.default_face, None);
    assert!(engine.contains("simplex.shx"));
    // Catalog + the one font: the unused `osifont` file was never requested.
    assert_eq!(loader.seen.lock().unwrap().len(), 2);
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
        false,
        "https://example/fonts",
    ));
    assert!(matches!(result, Err(CadError::ResourceMissing(_))));
}

// ---------------------------------------------------------------------------
// Code-page-driven locale fallback
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};

/// A synthetic SHAPES font with one real outline for `A` (code 65).
fn synthetic_ascii_shx() -> Vec<u8> {
    let mut bytes = b"AutoCAD-86 shapes 1.0\r\n\x1a".to_vec();
    let info = b"Synthetic\0\x48\x08\x00\x00"; // height = 72 + 8
    let glyph = b"A\0\x01\x22\x28\x00";
    for value in [0u16, 0, 2, 0, info.len() as u16, 65, glyph.len() as u16] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(info);
    bytes.extend(glyph);
    bytes
}

/// A synthetic BIGFONT with one glyph: `U+4E2D` in GBK (`0xD6D0`).
fn synthetic_gbk_bigfont() -> Vec<u8> {
    let mut bytes = b"AutoCAD-86 bigfont 1.0\r\n\x1a".to_vec();
    let info = b"Big\0\x48\x08\x00\x00"; // height = 72 + 8
    let glyph = b"G\0\x01\x22\x28\x00";
    // Two i16 header fields + count, then two (code, len, offset) entries.
    let table_len = 6 + 2 * 8;
    let info_offset = bytes.len() + table_len;
    let glyph_offset = info_offset + info.len();
    bytes.extend(0u16.to_le_bytes()); // reserved
    bytes.extend(2u16.to_le_bytes()); // count
    bytes.extend(0u16.to_le_bytes()); // skip
    bytes.extend(0u16.to_le_bytes()); // glyph code 0 = metrics
    bytes.extend((info.len() as u16).to_le_bytes());
    bytes.extend((info_offset as u32).to_le_bytes());
    bytes.extend(0xD6D0u16.to_le_bytes()); // U+4E2D in GBK
    bytes.extend((glyph.len() as u16).to_le_bytes());
    bytes.extend((glyph_offset as u32).to_le_bytes());
    bytes.extend(info);
    bytes.extend(glyph);
    bytes
}

fn temp_font_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yacr-locale-fonts-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_package(dir: &Path, catalog: &str, files: &[(&str, Vec<u8>)]) {
    std::fs::write(dir.join("fonts.json"), catalog.as_bytes()).unwrap();
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
}

const CJK_PACKAGE: &str = r#"[
    { "file": "osifont.shx", "name": ["osifont"], "type": "shx" },
    { "file": "gbcbig.shx", "name": ["gbcbig"], "type": "shx", "encoding": "gbk" }
]"#;

const LATIN_ONLY_PACKAGE: &str = r#"[
    { "file": "osifont.shx", "name": ["osifont"], "type": "shx" }
]"#;

fn load(
    dir: &Path,
    requested: &[&str],
    locale: TextLocale,
) -> (Option<Arc<FontEngine>>, FontLoadReport) {
    let requested: Vec<String> = requested.iter().map(|name| (*name).to_string()).collect();
    super::local::load_engine_report(Some(dir), &requested, true, &[], &[], locale).unwrap()
}

#[test]
fn chinese_document_prefers_a_cjk_face_before_the_latin_defaults() {
    let dir = temp_font_dir("zh-prefers-cjk");
    write_package(
        &dir,
        CJK_PACKAGE,
        &[
            ("osifont.shx", synthetic_ascii_shx()),
            ("gbcbig.shx", synthetic_gbk_bigfont()),
        ],
    );
    let (engine, report) = load(&dir, &["osifont"], TextLocale::SimplifiedChinese);
    let engine = engine.expect("the package installs an engine");
    let keys = engine.fallback_keys();
    let cjk = keys
        .iter()
        .position(|key| key == "gbcbig.shx")
        .unwrap_or_else(|| panic!("gbcbig missing from fallback chain: {keys:?}"));
    let latin = keys
        .iter()
        .position(|key| key == "osifont.shx")
        .unwrap_or_else(|| panic!("osifont missing from fallback chain: {keys:?}"));
    assert!(
        cjk < latin,
        "the CJK face must come before the Latin default: {keys:?}"
    );
    assert_eq!(report.locale_faces, vec!["gbcbig.shx".to_string()]);
    assert!(report.locale_missing.is_none());
    assert!(report.diagnostics().is_empty());
    assert_eq!(engine.locale_tag(), Some("zh-Hans"));

    // The CJK glyph actually shapes through the bigfont face, not a notdef or
    // a Latin fallback marker.
    let shaped = engine
        .shape(
            "gbcbig.shx",
            "\u{4e2d}",
            p(0.0, 0.0),
            10.0,
            0.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            None,
        )
        .unwrap();
    assert!(
        !shaped.outlines.is_empty(),
        "the CJK face must supply the glyph outline"
    );
    assert!(
        !shaped
            .issues
            .iter()
            .any(|issue| issue.code == "text.glyph_missing"),
        "the CJK glyph must not be reported missing: {:?}",
        shaped.issues
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn chinese_without_a_cjk_face_reports_one_named_gap_and_shapes_latin_anyway() {
    let dir = temp_font_dir("zh-no-cjk");
    write_package(
        &dir,
        LATIN_ONLY_PACKAGE,
        &[("osifont.shx", synthetic_ascii_shx())],
    );
    let (engine, report) = load(&dir, &["osifont"], TextLocale::SimplifiedChinese);
    let engine = engine.expect("the package still installs an engine");

    let diagnostics = report.diagnostics();
    assert_eq!(diagnostics.len(), 1, "exactly one gap per load/open");
    assert_eq!(diagnostics[0].code, "font.locale_fallback_missing");
    assert!(diagnostics[0].message.contains("zh-Hans"));
    for candidate in ["gbcbig", "hztxt", "chineset", "bigfont", "simhei", "simsun"] {
        assert!(
            diagnostics[0].message.contains(candidate),
            "the diagnostic must name the candidates tried: {}",
            diagnostics[0].message
        );
    }

    // Latin text still shapes through the available fallback: no panic, no
    // empty result disguised as success.
    let latin = engine
        .shape(
            "osifont.shx",
            "A",
            p(0.0, 0.0),
            10.0,
            0.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            None,
        )
        .unwrap();
    assert!(
        !latin.outlines.is_empty(),
        "Latin fallback must still shape"
    );

    // A CJK character has no glyph in this package: that is an explicit
    // `text.glyph_missing` issue, never silently rendered as Latin.
    let cjk = engine
        .shape(
            "osifont.shx",
            "\u{4e2d}",
            p(0.0, 0.0),
            10.0,
            0.0,
            TextAlignH::Left,
            TextAlignV::Baseline,
            None,
        )
        .unwrap();
    assert!(cjk.outlines.is_empty());
    assert!(cjk
        .issues
        .iter()
        .any(|issue| issue.code == "text.glyph_missing"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn latin_document_keeps_the_historical_fallback_order() {
    // Pin the pre-locale behaviour: registration order, no CJK face fetched.
    let dir = temp_font_dir("latin-order");
    write_package(
        &dir,
        CJK_PACKAGE,
        &[
            ("osifont.shx", synthetic_ascii_shx()),
            ("gbcbig.shx", synthetic_gbk_bigfont()),
        ],
    );
    let (engine, report) = load(&dir, &["osifont"], TextLocale::Latin);
    let engine = engine.expect("the package installs an engine");
    assert_eq!(engine.fallback_keys(), &["osifont.shx".to_string()]);
    assert!(
        !engine.contains("gbcbig.shx"),
        "a Latin document must not load a CJK face"
    );
    assert_eq!(report.locale, TextLocale::Latin);
    assert!(report.locale_faces.is_empty());
    assert!(report.diagnostics().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn locale_gap_reports_a_reused_engine_without_a_locale_face() {
    let dir = temp_font_dir("gap-reuse");
    write_package(
        &dir,
        LATIN_ONLY_PACKAGE,
        &[("osifont.shx", synthetic_ascii_shx())],
    );
    let (engine, _) = load(&dir, &["osifont"], TextLocale::SimplifiedChinese);
    let engine = engine.expect("engine");
    let gap = locale_gap(&engine, TextLocale::SimplifiedChinese, true)
        .expect("a reused Chinese engine without a CJK face still reports the gap");
    assert_eq!(gap.locale, TextLocale::SimplifiedChinese);
    assert_eq!(gap.candidates.first().map(String::as_str), Some("gbcbig"));
    // No text means no shaping requirement and therefore no gap.
    assert!(locale_gap(&engine, TextLocale::SimplifiedChinese, false).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
