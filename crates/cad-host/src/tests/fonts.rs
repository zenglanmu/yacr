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
