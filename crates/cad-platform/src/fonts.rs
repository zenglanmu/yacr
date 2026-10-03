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

use cad_db::DrawingDatabase;
use cad_domain::{CadError, SemanticGeometry};
use cad_representation::FontEngine;
use cad_resources::{plan_fonts, plan_fonts_report, FontCatalog};

use super::{FontLoader, HostFuture};

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
        // A registered face stands in for any missing one so text is not
        // silently dropped; the order follows the plan.
        engine.set_fallback(report.registered.clone());
        Ok((Arc::new(engine), report))
    })
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
        let mut bytes = b"AutoCAD-86 shapes 1.0\r\n\x1a".to_vec();
        let info = b"Synthetic\0\x15\x07\x02\0";
        for value in [0u16, 0, 1, 0, info.len() as u16] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(info);
        let loader = FakeLoader {
            catalog: Arc::from(
                br#"[{"file":"simplex.shx","name":["simplex"],"type":"shx"}]"#.as_slice(),
            ),
            files: Mutex::new(HashMap::from([(
                "https://example/fonts/simplex.shx".into(),
                Ok(Arc::from(bytes)),
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
