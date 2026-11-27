//! `FILL` / `QTEXT` / `VIEWRES` display contracts at the **scene base** level.
//!
//! Each session token is part of [`SceneVersion`], so a flip must rebuild the
//! resident base, and the rebuilt batches must show the derived change the
//! provider documents: a hatch's fill mesh becomes its boundary lines, text
//! glyphs become one placeholder rectangle, and a `VIEWRES` ceiling changes the
//! chord sample count. None of this touches `cad-db` — every assertion also
//! pins that the drawing revision and history are untouched.
//!
//! These are CPU scene/batch contracts. They claim no pixel, GPU or performance
//! result.

use super::*;
use cad_db::DrawingDatabaseBuilder;
use cad_domain::{DatabaseId, ObjectId, Revision};

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn prepare(controller: &mut CadSceneController, db: &DrawingDatabase) {
    controller
        .prepare(
            Some(db),
            DocumentId(91),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
        )
        .unwrap();
}

/// A drawing holding one entity, on layer `0`.
fn drawing(geometry: cad_domain::SemanticGeometry, type_key: &str) -> DrawingDatabase {
    use cad_db::{DbEntity, DbObject, Layer};
    use cad_domain::{EntityId, LayerId, SpaceId};

    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(41));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: type_key.into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry,
            draw_order: 0,
        })
        .unwrap();
    builder.finish().unwrap()
}

fn solid_hatch() -> cad_domain::SemanticGeometry {
    use cad_domain::{
        HatchFill, HatchGeometry, HatchLoop, HatchLoopFlags, HatchPatternType, HatchPlane,
        HatchStyle,
    };
    let loop_ = HatchLoop::new(
        vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
        Vec::new(),
        HatchLoopFlags(1),
    );
    let hatch = HatchGeometry::new(
        vec![loop_],
        HatchPlane::default(),
        HatchStyle::Normal,
        HatchFill::Solid,
        Vec::new(),
        None,
        HatchPatternType::Predefined,
    )
    .expect("valid square hatch");
    cad_domain::SemanticGeometry::Hatch(Box::new(hatch))
}

fn mesh_batches(base: &ResidentScene) -> usize {
    base.metadata()
        .into_iter()
        .filter(|batch| batch.topology == cad_scene::RenderTopology::Mesh)
        .count()
}

fn line_batches(base: &ResidentScene) -> usize {
    base.metadata()
        .into_iter()
        .filter(|batch| batch.topology == cad_scene::RenderTopology::Lines)
        .count()
}

/// `FILL OFF` rebuilds the base and drops the hatch fill batch while keeping the
/// boundary line batch; the drawing itself is never written.
#[test]
fn fill_off_rebuilds_the_base_and_swaps_the_hatch_fill_for_its_outline() {
    let db = drawing(solid_hatch(), "AcDbHatch");
    let revision = db.revision();
    let mut controller = CadSceneController::default();
    prepare(&mut controller, &db);
    let filled = controller.ready.as_ref().unwrap().clone();
    assert_eq!(mesh_batches(&filled.base), 1, "FILL ON draws the fill mesh");
    assert_eq!(
        line_batches(&filled.base),
        1,
        "the hatch boundary is drawn beside the fill"
    );
    let base_revision = filled.base_revision;

    assert!(controller.set_fill(false), "the flip is a real change");
    assert!(
        !controller.set_fill(false),
        "an unchanged FILL is a no-op (no churn)"
    );
    prepare(&mut controller, &db);
    let outline = controller.ready.as_ref().unwrap().clone();
    assert!(
        outline.base_revision > base_revision,
        "FILL OFF removes the fill batch, so the base must be rebuilt"
    );
    assert_eq!(mesh_batches(&outline.base), 0, "no fill mesh survives");
    assert_eq!(
        line_batches(&outline.base),
        1,
        "the boundary loop is still one line batch"
    );
    assert_eq!(db.revision(), revision, "FILL never writes the drawing");

    // The stored geometry is the same in both builds: only the derived
    // primitive changed, so the boundary batch carries the hatch's corners in
    // both.
    let corners = |base: &ResidentScene| -> usize {
        base.resident_batches()
            .expect("resident payload")
            .into_iter()
            .filter(|batch| batch.topology == cad_scene::RenderTopology::Lines)
            .map(|batch| batch.vertices.len())
            .sum()
    };
    assert_eq!(
        corners(&outline.base),
        corners(&filled.base),
        "the outline keeps the filled draw's boundary vertices"
    );
}

/// `QTEXT ON` rebuilds the base and replaces every glyph batch with one
/// placeholder rectangle.
#[test]
fn qtext_rebuilds_the_base_and_replaces_glyph_batches() {
    use cad_representation::FontEngine;
    use std::sync::Arc;

    let mut fonts = FontEngine::new();
    fonts
        .register(
            "osifont.ttf",
            Arc::from(include_bytes!("../../../../fonts/osifont.ttf").as_slice()),
        )
        .unwrap();
    fonts.set_fallback(vec!["osifont.ttf".into()]);
    let fonts = Arc::new(fonts);

    let db = drawing(
        cad_domain::SemanticGeometry::Text {
            text: "OH".into(),
            position: p(0.0, 0.0),
            style: cad_domain::StyleId(0),
            height: 10.0,
            rotation: 0.0,
            font: Some("osifont.ttf".into()),
            h_align: cad_domain::TextAlignH::Left,
            v_align: cad_domain::TextAlignV::Bottom,
            wrap_width: None,
            width_factor: 1.0,
            line_spacing: cad_domain::TextLineSpacing::default(),
            mirrored: false,
        },
        "AcDbText",
    );
    let mut controller = CadSceneController::default();
    controller
        .prepare(
            Some(&db),
            DocumentId(91),
            Some(Arc::clone(&fonts)),
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
        )
        .unwrap();
    let glyphs = controller.ready.as_ref().unwrap().clone();
    assert!(
        glyphs.base.batch_len() > 1,
        "two glyphs shape into more than one batch"
    );
    assert_eq!(
        mesh_batches(&glyphs.base),
        glyphs.base.batch_len(),
        "shaped SFNT text draws filled glyph faces"
    );

    assert!(controller.set_qtext(true));
    controller
        .prepare(
            Some(&db),
            DocumentId(91),
            Some(fonts),
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
        )
        .unwrap();
    let quick = controller.ready.as_ref().unwrap().clone();
    assert!(
        quick.base_revision > glyphs.base_revision,
        "QTEXT changes the primitives, so the base must be rebuilt"
    );
    assert_eq!(quick.base.batch_len(), 1, "one placeholder rectangle");
    assert_eq!(mesh_batches(&quick.base), 0, "no glyph face survives");
    let batch = quick.base.resident_batches().expect("resident payload")[0].clone();
    assert_eq!(batch.topology, cad_scene::RenderTopology::Lines);
    // A `Lines` batch carries independent endpoint pairs, so the rectangle's
    // four sides are eight vertices.
    assert_eq!(batch.vertices.len(), 8, "a closed rectangle of four sides");
}

/// `VIEWRES` is a real tessellation ceiling: the same arc builds a coarser
/// chord batch, and the flip rebuilds the base.
#[test]
fn viewres_ceiling_changes_the_chord_batch_and_rebuilds_the_base() {
    let db = drawing(
        cad_domain::SemanticGeometry::Polyline {
            points: vec![p(-100_000.0, 0.0), p(100_000.0, 0.0)],
            bulges: vec![1.0],
            closed: false,
            width: 0.0,
        },
        "AcDbPolyline",
    );
    let mut controller = CadSceneController::default();
    controller.set_viewres(Some(1000));
    prepare(&mut controller, &db);
    let fine = controller.ready.as_ref().unwrap().clone();
    let fine_vertices: usize = fine
        .base
        .resident_batches()
        .expect("resident payload")
        .into_iter()
        .map(|batch| batch.vertices.len())
        .sum();
    assert!(fine_vertices > 7, "the fine arc has many chords");

    assert!(controller.set_viewres(Some(6)));
    prepare(&mut controller, &db);
    let coarse = controller.ready.as_ref().unwrap().clone();
    assert!(
        coarse.base_revision > fine.base_revision,
        "a VIEWRES flip changes the batch vertices, so the base is rebuilt"
    );
    let coarse_vertices: usize = coarse
        .base
        .resident_batches()
        .expect("resident payload")
        .into_iter()
        .map(|batch| batch.vertices.len())
        .sum();
    // A `Lines` batch carries independent endpoint pairs: 6 chords = 12
    // vertices (the 7-sample polyline, split into its segments).
    assert_eq!(
        coarse_vertices, 12,
        "6 chords means 7 samples / 12 vertices"
    );
}

/// One batched sync of every token rebuilds once, and a repeat sync is free.
#[test]
fn display_settings_batch_is_one_rebuild_and_a_repeat_is_a_no_op() {
    let db = drawing(solid_hatch(), "AcDbHatch");
    let mut controller = CadSceneController::default();
    prepare(&mut controller, &db);
    let baseline = controller.ready.as_ref().unwrap().base_revision;

    let changed = DisplaySettings {
        wipeout_frames: false,
        frames_visible: false,
        fill: false,
        qtext: true,
        attdisp: cad_representation::AttributeDisplay::Off,
        curve_segments: Some(250),
        body_wireframe: true,
        body_edges: true,
        edge_color: None,
    };
    assert!(controller.set_display_settings(changed));
    prepare(&mut controller, &db);
    let after = controller.ready.as_ref().unwrap().base_revision;
    assert!(after > baseline, "the batch rebuilds the base once");
    assert!(
        !controller.set_display_settings(changed),
        "the same settings again schedule nothing"
    );
    prepare(&mut controller, &db);
    assert_eq!(
        controller.ready.as_ref().unwrap().base_revision,
        after,
        "a repeated sync must not churn the base"
    );
}
