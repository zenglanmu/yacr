//! `LAYOUT COPY` contract tests: the duplicate form of the layout-table write.
//!
//! A copy must carry the source's **viewports with their clips, scales and
//! layer overrides**, its stored `PLOTSETTINGS` row (re-keyed) and its
//! paper-space entities (fresh ids), be exactly one revision / one undo step, and
//! leave the source layout and its entities untouched. `LAYOUT TEMPLATE` is a
//! named refusal at the command-line grammar (`layout.template_not_implemented`).
//!
//! Every refusal is asserted to write nothing: revision, undo depth and both
//! tables are read before and after.

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer, Layout, PaperViewport, PlotRotation};

const DOC: DocumentId = DocumentId(1);

fn layout_payload(payload: LayoutPayload) -> CommandPayload {
    CommandPayload::Layout(Box::new(payload))
}

fn layout_command(payload: LayoutPayload) -> Command {
    command(CommandId::Layout, layout_payload(payload))
}

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

/// One rectangular paper viewport of `100 x 50` at the origin, scaled `scale`.
fn rect_viewport(scale: f64) -> PaperViewport {
    PaperViewport {
        clip: vec![
            point(0.0, 0.0),
            point(100.0, 0.0),
            point(100.0, 50.0),
            point(0.0, 50.0),
        ],
        model_to_paper: Transform3::scale(scale),
        completeness: Completeness::Complete,
        layer_overrides: std::collections::BTreeMap::new(),
    }
}

/// The imported page setup of layout 1 (`ISO_A4`, 1:1, no rotation).
fn plot_settings(layout: u128) -> cad_db::PlotSettingsRecord {
    cad_db::PlotSettingsRecord {
        layout: LayoutId(layout),
        paper_size_name: "ISO_A4".into(),
        paper_width: 210.0,
        paper_height: 297.0,
        margins: cad_db::PlotMargins::uniform(5.0),
        rotation: PlotRotation::None,
        scale_numerator: 1.0,
        scale_denominator: 1.0,
        plot_type: cad_db::PlotType::Layout,
        paper_units: cad_db::PlotPaperUnits::Millimeters,
        provenance: cad_db::PlotProvenance::Imported,
    }
}

fn entity(id: u128, layout: u128, type_key: &str) -> cad_db::DbEntity {
    cad_db::DbEntity {
        object: cad_db::DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Paper(LayoutId(layout)),
        geometry: SemanticGeometry::Line {
            start: point(1.0, 2.0),
            end: point(3.0, 4.0),
        },
        draw_order: id as i64,
    }
}

/// A drawing with one paper layout (`Sheet1`, two viewports with different clips,
/// scales and layer overrides), one stored page setup, one paper line and one
/// imported `AcDbViewport` frame entity.
fn fixture() -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    let mut overridden = rect_viewport(100.0);
    overridden.layer_overrides.insert(LayerId(3), true);
    // The second viewport is the legacy three-point form (clip point 3 is the
    // model anchor), so a copy that only kept rectangular clips is caught.
    let legacy = PaperViewport {
        clip: vec![point(0.0, 0.0), point(100.0, 50.0), point(10.0, 20.0)],
        model_to_paper: Transform3::scale(100.0),
        completeness: Completeness::Complete,
        layer_overrides: std::collections::BTreeMap::new(),
    };
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet1".into(),
            viewports: vec![overridden, legacy],
        })
        .unwrap();
    builder.set_plot_settings(plot_settings(1)).unwrap();
    builder.insert_entity(entity(10, 1, "AcDbLine")).unwrap();
    builder
        .insert_entity(entity(11, 1, "AcDbViewport"))
        .unwrap();
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DOC,
        Document {
            id: DOC,
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
            compat: Vec::new(),
            sources: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DOC, [800.0, 600.0]),
    );
    let mut session = SessionState::new(DOC, AppMode::Work);
    session.active_space = SpaceId::Paper(LayoutId(1));
    (app, session)
}

fn drawing(app: &Application) -> &cad_db::DrawingDatabase {
    app.workspace.documents[&DOC].drawing.as_ref()
}

fn revision(app: &Application) -> Revision {
    drawing(app).revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DOC)
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn refusal(result: CadResult<CommandOutcome>) -> String {
    match result {
        Err(CadError::Unsupported(message)) | Err(CadError::InvalidInput(message)) => message,
        Err(other) => panic!("expected a named refusal, got {other:?}"),
        Ok(outcome) => panic!("expected a refusal, got {outcome:?}"),
    }
}

fn copy(source: &str, name: &str) -> Command {
    layout_command(LayoutPayload::Copy {
        source: source.into(),
        name: name.into(),
    })
}

fn paper_entities(app: &Application, layout: u128) -> Vec<(EntityId, String, i64)> {
    drawing(app)
        .entities()
        .filter(|entity| entity.space == SpaceId::Paper(LayoutId(layout)))
        .map(|entity| (entity.id, entity.object.type_key.clone(), entity.draw_order))
        .collect()
}

#[test]
fn layout_copy_duplicates_the_row_the_viewports_the_entities_and_the_page_setup() {
    let (mut app, mut session) = fixture();
    let before = revision(&app);
    let depth = undo_depth(&app);
    let source_viewports = drawing(&app)
        .layout(LayoutId(1))
        .expect("the source layout exists")
        .viewports
        .clone();

    let outcome = app
        .execute(&mut session, copy("Sheet1", "Sheet2"))
        .expect("LAYOUT COPY writes the duplicate");
    assert_eq!(
        codes(&outcome),
        vec!["layout.copied", "layout.copy.viewport_frame"],
        "the copy reports itself and the one skipped viewport frame"
    );
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert_eq!(undo_depth(&app), depth + 1, "one undo step");

    let copied = drawing(&app)
        .layouts()
        .find(|layout| layout.name == "Sheet2")
        .expect("the copy exists");
    assert_eq!(copied.id, LayoutId(2), "the next monotonic id");
    assert_eq!(
        copied.viewports, source_viewports,
        "clips, scales and layer overrides are copied verbatim"
    );
    assert_eq!(
        copied.viewports[0].layer_overrides.get(&LayerId(3)),
        Some(&true),
        "the per-viewport layer override survives the copy"
    );

    let source_plot = drawing(&app).plot_settings(LayoutId(1)).cloned();
    let mut expected = source_plot.expect("the source keeps its page setup");
    expected.layout = LayoutId(2);
    expected.provenance = cad_db::PlotProvenance::Authored;
    assert_eq!(
        drawing(&app).plot_settings(LayoutId(2)).cloned(),
        Some(expected),
        "the stored page setup is re-keyed and marked authored"
    );

    let source_entities = paper_entities(&app, 1);
    let copied_entities = paper_entities(&app, 2);
    assert_eq!(
        source_entities,
        vec![
            (EntityId(10), "AcDbLine".into(), 10),
            (EntityId(11), "AcDbViewport".into(), 11)
        ],
        "the source layout is untouched"
    );
    assert_eq!(
        copied_entities.len(),
        1,
        "only the user geometry is duplicated, not the viewport frame"
    );
    assert_ne!(
        copied_entities[0].0,
        EntityId(10),
        "the copy gets a fresh entity id"
    );
    assert_eq!(copied_entities[0].1, "AcDbLine");
    assert_eq!(copied_entities[0].2, 10, "the source draw order is kept");
    let copied_geometry = drawing(&app)
        .entity(copied_entities[0].0)
        .expect("the copied entity exists")
        .geometry
        .clone();
    assert_eq!(
        copied_geometry,
        drawing(&app).entity(EntityId(10)).unwrap().geometry,
        "the geometry is copied verbatim, not re-derived"
    );

    // One undo step removes the row and the copied entity together.
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo replays the copy");
    assert!(
        drawing(&app)
            .layouts()
            .all(|layout| layout.name != "Sheet2"),
        "undo removes the copied layout"
    );
    assert_eq!(paper_entities(&app, 2), Vec::new(), "…and its entities");

    app.execute(&mut session, command(CommandId::Redo, CommandPayload::None))
        .expect("redo re-applies the copy");
    assert!(
        drawing(&app)
            .layouts()
            .any(|layout| layout.name == "Sheet2"),
        "redo restores the copied layout"
    );
    assert_eq!(paper_entities(&app, 2).len(), 1);
}

#[test]
fn layout_copy_refusals_write_nothing() {
    let (mut app, mut session) = fixture();
    let before = revision(&app);
    let layouts = drawing(&app).layouts().count();
    let entities = drawing(&app).entities().count();

    let duplicate = refusal(app.execute(&mut session, copy("Sheet1", "SHEET1")));
    assert!(duplicate.contains("layout.duplicate_name"), "{duplicate}");

    let unknown = refusal(app.execute(&mut session, copy("Nope", "Sheet2")));
    assert!(unknown.contains("layout.unknown"), "{unknown}");

    let invalid = refusal(app.execute(&mut session, copy("Sheet1", "bad<name>")));
    assert!(invalid.contains("layout.invalid_name"), "{invalid}");

    // A one-token form is a grammar refusal, not a copy of the source name.
    let grammar = refusal(app.execute(
        &mut session,
        layout_command(LayoutPayload::Copy {
            source: "Sheet1".into(),
            name: String::new(),
        }),
    ));
    assert!(grammar.contains("layout.invalid_name"), "{grammar}");

    assert_eq!(revision(&app), before, "a refusal writes nothing");
    assert_eq!(drawing(&app).layouts().count(), layouts);
    assert_eq!(drawing(&app).entities().count(), entities);
    assert_eq!(undo_depth(&app), 0, "a refusal records no history");
}

#[test]
fn layout_copy_is_work_only_and_copying_onto_the_same_name_refuses() {
    let (mut app, mut session) = fixture();
    let before = revision(&app);
    // Copying a layout onto its own name is a duplicate, never a self-overwrite.
    let same = refusal(app.execute(&mut session, copy("Sheet1", "Sheet1")));
    assert!(same.contains("layout.duplicate_name"), "{same}");
    assert_eq!(revision(&app), before);

    session.switch_mode(AppMode::Viewer).unwrap();
    let viewer = app
        .execute(&mut session, copy("Sheet1", "Sheet2"))
        .expect_err("a Viewer session cannot write the layout table");
    assert_eq!(viewer, CadError::PermissionDenied);
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DOC));
}
