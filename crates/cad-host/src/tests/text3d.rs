//! `TEXT3D` on the host boundary: the payload reaches the drawing as one
//! extruded `AcDbSubDMesh` row in one revision, with one undo step.
//!
//! The shaping engine is the same installed outline hook `TXTEXP` uses; this test
//! installs the repository's open font (the carrier's role in production). No GUI
//! or GPU is involved.
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
    let _ = cad_app::install_outline_fonts(Arc::new(engine));
}

#[test]
fn the_text3d_command_writes_one_extruded_mesh_row() {
    install_fonts();
    let core = core();
    let before = core.controller.borrow().drawing().unwrap().revision();
    let entities_before = core
        .controller
        .borrow()
        .drawing()
        .unwrap()
        .model_space()
        .len();
    core.command(
        CommandId::Text3d,
        CommandPayload::Text3d(cad_app::Text3dInput {
            origin: p(0.0, 0.0),
            height: 10.0,
            extrusion: 2.0,
            font: Some("osifont.ttf".into()),
            text: "A".into(),
        }),
    )
    .expect("TEXT3D is admitted");

    let controller = core.controller.borrow();
    let drawing = controller.drawing().unwrap();
    assert_eq!(drawing.revision().0, before.0 + 1, "one revision");
    let model = drawing.model_space();
    assert_eq!(model.len(), entities_before + 1);
    let entity = model.last().expect("the new entity");
    assert_eq!(entity.object.type_key, "AcDbSubDMesh");
    let cad_domain::SemanticGeometry::Mesh(mesh) = &entity.geometry else {
        panic!("TEXT3D must store one mesh, got {:?}", entity.geometry);
    };
    assert!(!mesh.triangles.is_empty());
    assert!(!mesh.vertices.is_empty());
    let zs: Vec<f64> = mesh.vertices.iter().map(|vertex| vertex.z).collect();
    let min = zs.iter().copied().fold(f64::INFINITY, f64::min);
    let max = zs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!(
        (min - 0.0).abs() < 1e-9 && (max - 2.0).abs() < 1e-9,
        "{min}..{max}"
    );
}
