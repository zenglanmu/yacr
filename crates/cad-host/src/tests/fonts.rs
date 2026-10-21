//! Font-installation contracts for drawings whose text names no font.
//!
//! A created dimension (P5c–P5e) carries a `Standard` text style with no font
//! name, so `requested_fonts` is empty. The host must still install a shaping
//! engine (resolving through the fallback chain) or the measurement text is
//! never drawn; a drawing with no text must still install nothing.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer, Style};
use cad_domain::{
    DatabaseId, DimensionDefinition, DimensionGeometry, DimensionKind, DimensionPoints, EntityId,
    LayerId, ObjectId, Point3, Revision, SemanticGeometry, SpaceId, StyleId,
};
use std::path::Path;

fn options_with_package_fonts() -> HostOptions {
    let mut options = options();
    options.fonts_dir = Some(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fonts"),
    );
    options
}

fn core_with_desktop_resources() -> HostCore {
    let platform = HostPlatform {
        files: Rc::new(FakeFiles::default()),
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(DesktopResources),
        capabilities: capabilities(false),
    };
    HostCore::new(
        options_with_package_fonts(),
        platform,
        RenderSessionHandle::default(),
    )
    .unwrap()
}

fn install_drawing(core: &HostCore, drawing: Arc<DrawingDatabase>) {
    let mut controller = core.controller.borrow_mut();
    let document = controller.document_id;
    controller
        .application
        .workspace
        .documents
        .get_mut(&document)
        .expect("the host always has a document")
        .drawing = drawing;
}

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn base_builder() -> DrawingDatabaseBuilder {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    // A `Standard` text style with no resource keys: exactly what a newly
    // created drawing has. It cannot name a font.
    builder
        .insert_style(Style {
            id: StyleId(0),
            name: "Standard".into(),
            resource_keys: Vec::new(),
        })
        .unwrap();
    builder
}

fn dimension_definition() -> DimensionDefinition {
    DimensionDefinition {
        kind: DimensionKind::Aligned,
        points: DimensionPoints::Linear {
            first: p(-1.0, 0.0),
            second: p(1.0, 0.0),
            dimension_line: p(-1.0, -2.0),
        },
        text_middle: p(0.0, 0.0),
        text_rotation: 0.0,
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    }
}

fn dimension_drawing() -> DrawingDatabase {
    let values = cad_db::DimStyle::standard_metric(cad_domain::DimStyleId(0)).resolved(
        "Standard".into(),
        StyleId(0),
        None,
    );
    let dimension =
        DimensionGeometry::synthesized(dimension_definition(), "Standard".into(), values, None)
            .expect("synthesized dimension");
    let mut builder = base_builder();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbDimension".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Dimension(Box::new(dimension)),
            draw_order: 0,
        })
        .unwrap();
    builder.finish().unwrap()
}

fn line_drawing() -> DrawingDatabase {
    let mut builder = base_builder();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Line {
                start: p(0.0, 0.0),
                end: p(1.0, 0.0),
            },
            draw_order: 0,
        })
        .unwrap();
    builder.finish().unwrap()
}

#[test]
fn a_created_dimension_with_no_font_styles_installs_a_fallback_engine() {
    let core = core_with_desktop_resources();
    let drawing = dimension_drawing();
    assert!(
        cad_platform::fonts::requested_fonts(&drawing).is_empty(),
        "the defect precondition: the drawing names no font"
    );
    assert!(cad_platform::fonts::needs_text_shaping(&drawing));
    install_drawing(&core, Arc::new(drawing));

    core.install_fonts().expect("font installation");

    let fonts = core
        .render
        .fonts()
        .expect("a text-bearing drawing installs a fallback engine");
    assert!(
        fonts.contains("osifont.ttf") || !fonts.fallback_keys().is_empty(),
        "the fallback chain must hold a usable face: {:?}",
        fonts.fallback_keys()
    );
}

#[test]
fn a_dimension_committed_after_open_installs_fonts_before_the_rebuild() {
    let core = core_with_desktop_resources();
    install_drawing(&core, Arc::new(line_drawing()));
    core.install_fonts().expect("font installation");
    assert!(core.render.fonts().is_none(), "no text at open");

    // The commit funnel now sees a text-bearing drawing but no engine.
    install_drawing(&core, Arc::new(dimension_drawing()));
    core.ensure_fonts_for_text().expect("post-commit install");

    assert!(
        core.render.fonts().is_some(),
        "the new measurement text must be shapeable"
    );
}

#[test]
fn a_drawing_without_text_still_installs_no_font_engine() {
    let core = core_with_desktop_resources();
    let drawing = line_drawing();
    assert!(!cad_platform::fonts::needs_text_shaping(&drawing));
    install_drawing(&core, Arc::new(drawing));

    core.install_fonts().expect("font installation");

    assert!(
        core.render.fonts().is_none(),
        "no text and no named font must not load a font"
    );
}

/// P5 gate F3: the post-commit font install is not limited to the draw confirm.
///
/// A text-introducing commit that reaches the general commit funnel — here a
/// dimension created by its own command instead of the interactive confirm path,
/// which is the same shape as a pasted Dimension/Text or an undone erase — must
/// install the shaping engine too, or the new text stays unshaped in a drawing
/// that was opened text-less.
#[test]
fn a_committed_dimension_command_installs_fonts_without_the_draw_confirm() {
    let core = core_with_desktop_resources();
    install_drawing(&core, Arc::new(line_drawing()));
    core.install_fonts().expect("font installation");
    assert!(core.render.fonts().is_none(), "no text at open");

    core.command(
        CommandId::DimAligned,
        CommandPayload::Points(vec![p(0.0, 0.0), p(3.0, 4.0), p(-2.0, 2.0)]),
    )
    .expect("the dimension commits");

    let drawing = core.controller.borrow().drawing().expect("a drawing");
    assert!(
        cad_platform::fonts::needs_text_shaping(drawing.as_ref()),
        "the commit introduced the drawing's first text"
    );
    assert!(
        core.render.fonts().is_some(),
        "the general commit funnel must install the engine the open funnel could not"
    );
}

// ---------------------------------------------------------------------------
// Locale-driven fallback (drawing code page -> text locale)
// ---------------------------------------------------------------------------

/// A synthetic SHAPES font with one real outline for `A` (code 65).
fn synthetic_ascii_shx() -> Vec<u8> {
    let mut bytes = b"AutoCAD-86 shapes 1.0\r\n\x1a".to_vec();
    let info = b"Synthetic\0\x48\x08\x00\x00";
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
    let info = b"Big\0\x48\x08\x00\x00";
    let glyph = b"G\0\x01\x22\x28\x00";
    let table_len = 6 + 2 * 8;
    let info_offset = bytes.len() + table_len;
    let glyph_offset = info_offset + info.len();
    bytes.extend(0u16.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    bytes.extend((info.len() as u16).to_le_bytes());
    bytes.extend((info_offset as u32).to_le_bytes());
    bytes.extend(0xD6D0u16.to_le_bytes());
    bytes.extend((glyph.len() as u16).to_le_bytes());
    bytes.extend((glyph_offset as u32).to_le_bytes());
    bytes.extend(info);
    bytes.extend(glyph);
    bytes
}

fn locale_font_package(tag: &str, with_cjk: bool) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yacr-host-locale-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let catalog = if with_cjk {
        r#"[
            { "file": "osifont.shx", "name": ["osifont"], "type": "shx" },
            { "file": "gbcbig.shx", "name": ["gbcbig"], "type": "shx", "encoding": "gbk" }
        ]"#
    } else {
        r#"[{ "file": "osifont.shx", "name": ["osifont"], "type": "shx" }]"#
    };
    std::fs::write(dir.join("fonts.json"), catalog.as_bytes()).unwrap();
    std::fs::write(dir.join("osifont.shx"), synthetic_ascii_shx()).unwrap();
    if with_cjk {
        std::fs::write(dir.join("gbcbig.shx"), synthetic_gbk_bigfont()).unwrap();
    }
    dir
}

fn core_with_fonts_dir(dir: &Path) -> HostCore {
    let mut options = options();
    options.fonts_dir = Some(dir.to_path_buf());
    let platform = HostPlatform {
        files: Rc::new(FakeFiles::default()),
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(DesktopResources),
        capabilities: capabilities(false),
    };
    HostCore::new(options, platform, RenderSessionHandle::default()).unwrap()
}

/// A text-bearing drawing whose declared code page is `code_page`.
fn text_drawing(code_page: Option<&str>) -> DrawingDatabase {
    let mut builder = base_builder();
    builder.set_code_page(code_page);
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbText".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Text {
                text: "A".into(),
                position: p(0.0, 0.0),
                style: StyleId(0),
                height: 2.5,
                rotation: 0.0,
                font: Some("osifont.shx".into()),
                h_align: cad_domain::TextAlignH::Left,
                v_align: cad_domain::TextAlignV::Baseline,
                wrap_width: None,
            },
            draw_order: 0,
        })
        .unwrap();
    builder.finish().unwrap()
}

fn empty_import_report() -> cad_app::host::ImportReport {
    cad_app::host::ImportReport {
        identity: cad_domain::DocumentIdentity::Temporary(1),
        capabilities: Vec::new(),
        completeness: cad_domain::Completeness::Complete,
        diagnostics: Vec::new(),
        parse_ms: None,
    }
}

#[test]
fn opening_a_drawing_retargets_the_chain_to_its_own_text_locale() {
    let dir = locale_font_package("retarget", true);
    let core = core_with_fonts_dir(&dir);

    install_drawing(&core, Arc::new(text_drawing(Some("ANSI_936"))));
    core.install_fonts().expect("font installation");
    let chinese = core.render.fonts().expect("a Chinese document gets fonts");
    assert_eq!(chinese.locale_tag(), Some("zh-Hans"));
    let keys = chinese.fallback_keys();
    let cjk = keys.iter().position(|key| key == "gbcbig.shx");
    let latin = keys.iter().position(|key| key == "osifont.shx");
    match (cjk, latin) {
        (Some(cjk), Some(latin)) => assert!(
            cjk < latin,
            "the CJK face must precede the Latin default: {keys:?}"
        ),
        other => panic!("expected both gbcbig and osifont, got {other:?} in {keys:?}"),
    }

    // Opening another Chinese document must reuse the engine, not rebuild it.
    install_drawing(&core, Arc::new(text_drawing(Some("GB2312"))));
    core.install_fonts().expect("font reinstallation");
    let reused = core.render.fonts().expect("reused engine");
    assert!(
        Arc::ptr_eq(&chinese, &reused),
        "the same locale must not rebuild the engine"
    );

    // A Latin document must not silently keep the Chinese chain.
    install_drawing(&core, Arc::new(text_drawing(Some("ANSI_1252"))));
    core.install_fonts().expect("font reinstallation");
    let latin_engine = core.render.fonts().expect("Latin engine");
    assert_eq!(latin_engine.locale_tag(), Some("latin"));
    assert!(
        !latin_engine.contains("gbcbig.shx"),
        "a Latin document must not inherit the Chinese chain: {:?}",
        latin_engine.fallback_keys()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_locale_face_is_one_named_diagnostic_per_document() {
    let dir = locale_font_package("no-cjk", false);
    let core = core_with_fonts_dir(&dir);
    install_drawing(&core, Arc::new(text_drawing(Some("ANSI_936"))));
    core.controller.borrow_mut().last_import_report = Some(empty_import_report());

    core.install_fonts().expect("font installation");
    // The post-commit path may run install_fonts again for the same document.
    core.install_fonts().expect("font reinstallation");

    let report = core
        .controller
        .borrow()
        .last_import_report
        .clone()
        .expect("import report");
    let gaps: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "font.locale_fallback_missing")
        .collect();
    assert_eq!(
        gaps.len(),
        1,
        "one gap per document open, not per install/entity"
    );
    assert!(gaps[0].message.contains("zh-Hans"));
    assert!(gaps[0].message.contains("gbcbig"));

    // The drawing is still installed with the Latin fallback: no panic and no
    // empty engine merely because the CJK face is missing.
    let engine = core.render.fonts().expect("Latin fallback engine");
    assert!(engine.contains("osifont.shx"));
    let _ = std::fs::remove_dir_all(&dir);
}
