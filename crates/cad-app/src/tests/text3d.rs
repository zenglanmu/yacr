//! `TEXT3D` contracts: one extruded glyph mesh, one transaction/undo step, and
//! every named refusal writing nothing.
//!
//! The shaping engine is the same installed outline hook `TXTEXP` uses; these
//! tests install the repository's open font (the carrier's role in production).

use super::*;
use cad_representation::FontEngine;
use std::sync::Arc;

fn install_fonts() {
    let mut engine = FontEngine::new();
    engine
        .register(
            "osifont.ttf",
            Arc::from(include_bytes!("../../../../fonts/osifont.ttf").as_slice()),
        )
        .unwrap();
    // The first installation wins; every outline test installs the same engine.
    let _ = crate::install_outline_fonts(Arc::new(engine));
}

fn input(text: &str, height: f64, extrusion: f64) -> Text3dInput {
    Text3dInput {
        origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        height,
        extrusion,
        font: Some("osifont.ttf".into()),
        text: text.into(),
    }
}

fn run(
    app: &mut Application,
    session: &mut SessionState,
    input: Text3dInput,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Text3d, CommandPayload::Text3d(input)),
    )
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map_or(0, cad_history::History::undo_depth)
}

#[test]
fn text3d_writes_one_extruded_mesh_and_one_undo_step() {
    install_fonts();
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = run(&mut app, &mut session, input("A", 10.0, 2.0)).expect("TEXT3D");
    assert_eq!(revision(&app), Revision(before.0 + 1), "one revision");
    assert_eq!(undo_depth(&app), 1, "one undo step");
    assert_eq!(outcome.objects.len(), 1);
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "text3d.created")
        .expect("the success is named");
    assert!(
        diagnostic.message.contains("extruded by 2"),
        "{diagnostic:?}"
    );

    let document = &app.workspace.documents[&DocumentId(1)];
    let entity = document
        .drawing
        .entity(EntityId(outcome.objects[0].0))
        .expect("the new row");
    assert_eq!(entity.object.type_key, "AcDbSubDMesh");
    let SemanticGeometry::Mesh(mesh) = &entity.geometry else {
        panic!("TEXT3D must store one mesh, got {:?}", entity.geometry);
    };
    assert!(!mesh.vertices.is_empty());
    assert!(!mesh.triangles.is_empty());
    assert_eq!(mesh.normals.len(), mesh.vertices.len());
    // The solid spans the requested extrusion.
    let zs: Vec<f64> = mesh.vertices.iter().map(|p| p.z).collect();
    let min = zs.iter().copied().fold(f64::INFINITY, f64::min);
    let max = zs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!(
        (min - 0.0).abs() < 1e-9 && (max - 2.0).abs() < 1e-9,
        "{min}..{max}"
    );

    // Undo removes exactly the row.
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo");
    assert_eq!(entity_count(&app), 0);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn text3d_refuses_bad_sizes_and_text_before_writing() {
    install_fonts();
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    for (input, marker) in [
        (input("A", 0.0, 2.0), "text3d.invalid_size"),
        (input("A", 10.0, -1.0), "text3d.invalid_size"),
        (input("A", f64::NAN, 2.0), "text3d.invalid_size"),
        (input("   ", 10.0, 2.0), "text3d.invalid_text"),
        (input("A\nB", 10.0, 2.0), "text3d.invalid_text"),
    ] {
        let error = run(&mut app, &mut session, input).expect_err(marker);
        let text = match error {
            CadError::InvalidInput(text) | CadError::Unsupported(text) => text,
            other => panic!("expected a named refusal, got {other:?}"),
        };
        assert!(text.contains(marker), "{marker}: {text}");
    }
    assert_eq!(revision(&app), before, "refusals write nothing");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn text3d_is_refused_in_viewer_mode() {
    install_fonts();
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).expect("Viewer");
    let error = run(&mut app, &mut session, input("A", 10.0, 2.0))
        .expect_err("a document write is work-only");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(entity_count(&app), 0);
}
