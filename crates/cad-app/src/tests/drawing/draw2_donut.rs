//! `DONUT` contract tests: the filled disk / annulus mesh, its single
//! transaction and the refusal surface.

use super::draw2_batch::{
    assert_refused_unchanged, build, entity, exec, p3, revision, undo, undo_depth,
};
use super::*;

// ---------------------------------------------------------------- DONUT ----

#[test]
fn donut_creates_a_filled_annulus_in_one_undo() {
    let (mut app, mut session) = build(Vec::new());
    let before = revision(&app);
    let outcome = exec(
        &mut app,
        &mut session,
        CommandId::Donut,
        Draw2Payload::Donut {
            center: p3(2.0, 3.0, 0.0),
            inner: 3.0,
            outer: 5.0,
        },
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "one filled face, not two circles");
    let changes = outcome.changes.expect("change set");
    assert_eq!(changes.after.0, before.0 + 1, "one revision bump");
    assert_eq!(
        changes.changes.len(),
        1,
        "one entity insert in one transaction"
    );
    assert_eq!(undo_depth(&app), 1);

    let id = outcome.objects[0].0;
    let stored = entity(&app, id);
    assert_eq!(stored.object.type_key, "AcDbSolid");
    let SemanticGeometry::Mesh(mesh) = stored.geometry else {
        panic!("expected the SOLID triangle-mesh path");
    };
    assert_eq!(
        mesh.vertices.len(),
        128,
        "64 outer + 64 inner ring vertices"
    );
    assert_eq!(mesh.triangles.len(), 128, "two triangles per segment");
    assert_eq!(mesh.normals.len(), mesh.vertices.len());
    assert!(mesh.vertices.iter().all(|vertex| vertex.z == 0.0));

    undo(&mut app, &mut session);
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .is_none());
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn donut_refuses_bad_radii_and_a_non_finite_centre() {
    let (mut app, mut session) = build(Vec::new());
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::Donut,
        Draw2Payload::Donut {
            center: p3(0.0, 0.0, 0.0),
            inner: -1.0,
            outer: 2.0,
        },
        "donut.invalid_radius",
    );
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::Donut,
        Draw2Payload::Donut {
            center: p3(0.0, 0.0, 0.0),
            inner: 2.0,
            outer: 2.0,
        },
        "donut.radius_order",
    );
    assert_refused_unchanged(
        &mut app,
        &mut session,
        CommandId::Donut,
        Draw2Payload::Donut {
            center: p3(f64::NAN, 0.0, 0.0),
            inner: 1.0,
            outer: 2.0,
        },
        "donut.invalid_point",
    );
}

#[test]
fn donut_zero_inner_creates_a_filled_disk() {
    let (mut app, mut session) = build(Vec::new());
    let outcome = exec(
        &mut app,
        &mut session,
        CommandId::Donut,
        Draw2Payload::Donut {
            center: p3(0.0, 0.0, 0.0),
            inner: 0.0,
            outer: 4.0,
        },
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1);
    let id = outcome.objects[0].0;
    let SemanticGeometry::Mesh(mesh) = entity(&app, id).geometry else {
        panic!("expected the filled disk mesh");
    };
    assert_eq!(mesh.vertices.len(), 65, "the centre plus 64 ring vertices");
    assert_eq!(mesh.triangles.len(), 64, "one fan triangle per segment");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "drawing.donut"));
}
