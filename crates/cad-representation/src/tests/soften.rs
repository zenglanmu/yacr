//! SketchUp **Soften Edges** at display time.
//!
//! The provider reads the entity's `render_attributes.soften` angle and drops a
//! stored edge whose two adjacent display triangles meet at or below it. A
//! boundary edge, a non-manifold edge and an edge the mesh cannot explain stay
//! visible; a body whose every stored edge is softened keeps its fill and draws
//! no wireframe (the `derive_when_empty: false` contract).

use super::*;
use cad_db::EntityRenderAttributes;

fn attrs_with_soften(angle: f64) -> EntityRenderAttributes {
    EntityRenderAttributes {
        soften: Some(angle),
        ..EntityRenderAttributes::default()
    }
}

fn body_entity(id: u128, kind: BodyKind, display: BodyDisplay) -> DbEntity {
    let mut entity = entity(
        id,
        SemanticGeometry::Body3d(Box::new(Body3d {
            kind,
            exact: BodyExact::Unavailable {
                reason: "soften display fixture".into(),
            },
            display,
        })),
    );
    entity.object.type_key = "AcDb3dSubDMesh".into();
    entity
}

fn expanded_with(entity: DbEntity, attributes: EntityRenderAttributes) -> DisplayRepresentation {
    let id = entity.id;
    let mut builder = empty_db();
    builder.insert_entity(entity).unwrap();
    builder
        .set_entity_render_attributes(id, attributes)
        .unwrap();
    let db = builder.finish().unwrap();
    ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(id).unwrap(), &context())
        .unwrap()
}

/// A flat square with its four boundary edges and the shared diagonal stored.
fn flat_square() -> BodyDisplay {
    BodyDisplay {
        mesh: Mesh {
            uvs: Vec::new(),
            texture: None,
            pbr: None,
            vertices: vec![
                p3(0.0, 0.0, 0.0),
                p3(1.0, 0.0, 0.0),
                p3(1.0, 1.0, 0.0),
                p3(0.0, 1.0, 0.0),
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            normals: Vec::new(),
            face_sources: Vec::new(),
            colors: Vec::new(),
        },
        edges: vec![
            vec![p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)],
            vec![p3(1.0, 0.0, 0.0), p3(1.0, 1.0, 0.0)],
            vec![p3(1.0, 1.0, 0.0), p3(0.0, 1.0, 0.0)],
            vec![p3(0.0, 1.0, 0.0), p3(0.0, 0.0, 0.0)],
            vec![p3(0.0, 0.0, 0.0), p3(1.0, 1.0, 0.0)],
        ],
        tolerance: None,
        precision: Precision::Analytic,
        completeness: Completeness::Complete,
    }
}

fn body_fragment(rep: &DisplayRepresentation) -> &DisplayPrimitive {
    &rep.fragments
        .first()
        .expect("the body expands to one fragment")
        .primitive
}

#[test]
fn a_coplanar_diagonal_is_softened_away_at_any_threshold() {
    for angle in [0.0, 20.0, 30.0] {
        let rep = expanded_with(
            body_entity(61, BodyKind::Sheet, flat_square()),
            attrs_with_soften(angle),
        );
        match body_fragment(&rep) {
            DisplayPrimitive::MeshWithStoredEdges {
                edges,
                derive_when_empty,
                ..
            } => {
                assert_eq!(edges.len(), 8, "four boundary edges, diagonal hidden");
                assert!(!derive_when_empty, "a softened body owns its edge set");
            }
            _ => panic!("expected a mesh with stored edges"),
        }
    }
}

#[test]
fn a_ridge_above_the_threshold_stays_and_below_it_hides() {
    // Two triangles folded by 90°: shared edge is the ridge.
    let display = BodyDisplay {
        mesh: Mesh {
            uvs: Vec::new(),
            texture: None,
            pbr: None,
            vertices: vec![
                p3(0.0, 0.0, 0.0),
                p3(1.0, 0.0, 0.0),
                p3(0.0, 1.0, 0.0),
                p3(0.0, 0.0, 1.0),
            ],
            triangles: vec![[0, 1, 2], [0, 1, 3]],
            normals: Vec::new(),
            face_sources: Vec::new(),
            colors: Vec::new(),
        },
        edges: vec![vec![p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)]],
        tolerance: None,
        precision: Precision::Analytic,
        completeness: Completeness::Complete,
    };
    let kept = expanded_with(
        body_entity(62, BodyKind::Sheet, display.clone()),
        attrs_with_soften(30.0),
    );
    match body_fragment(&kept) {
        DisplayPrimitive::MeshWithStoredEdges { edges, .. } => {
            assert_eq!(edges.len(), 2, "90° ridge stays at a 30° threshold");
        }
        _ => panic!("expected a mesh with stored edges"),
    }
    let hidden = expanded_with(
        body_entity(63, BodyKind::Sheet, display),
        attrs_with_soften(120.0),
    );
    match body_fragment(&hidden) {
        DisplayPrimitive::MeshWithStoredEdges {
            edges,
            derive_when_empty,
            ..
        } => {
            assert!(edges.is_empty(), "the 90° ridge is softened at 120°");
            assert!(!derive_when_empty, "no triangle overlay may reappear");
        }
        _ => panic!("expected a mesh with stored edges"),
    }
}

#[test]
fn an_edge_the_mesh_cannot_explain_stays_visible() {
    // The stored edge does not coincide with any triangle edge: it is kept
    // rather than guessed away.
    let mut display = flat_square();
    display
        .edges
        .push(vec![p3(0.25, 0.25, 0.0), p3(0.5, 0.5, 0.0)]);
    let rep = expanded_with(
        body_entity(64, BodyKind::Sheet, display),
        attrs_with_soften(180.0),
    );
    match body_fragment(&rep) {
        DisplayPrimitive::MeshWithStoredEdges { edges, .. } => {
            // Four boundary pairs + the unmatched pair; only the coplanar
            // diagonal is hidden.
            assert_eq!(edges.len(), 10, "the unmatched segment survives");
            assert_eq!(edges[edges.len() - 2], p3(0.25, 0.25, 0.0));
            assert_eq!(edges[edges.len() - 1], p3(0.5, 0.5, 0.0));
        }
        _ => panic!("expected a mesh with stored edges"),
    }
}

#[test]
fn wireframe_soften_drops_the_diagonal_too() {
    let context = {
        let mut context = context();
        context.body_wireframe = true;
        context
    };
    let entity = body_entity(65, BodyKind::Sheet, flat_square());
    let id = entity.id;
    let mut builder = empty_db();
    builder.insert_entity(entity).unwrap();
    builder
        .set_entity_render_attributes(id, attrs_with_soften(20.0))
        .unwrap();
    let db = builder.finish().unwrap();
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(id).unwrap(), &context)
        .unwrap();
    match body_fragment(&rep) {
        DisplayPrimitive::LineSegments(points) => {
            assert_eq!(points.len(), 8, "four boundary pairs, diagonal hidden");
        }
        _ => panic!("expected line segments"),
    }
}

#[test]
fn an_unsoftened_body_keeps_the_legacy_fragment() {
    let rep = expanded_with(
        body_entity(66, BodyKind::Sheet, flat_square()),
        EntityRenderAttributes::default(),
    );
    match body_fragment(&rep) {
        DisplayPrimitive::MeshWithStoredEdges {
            edges,
            derive_when_empty,
            ..
        } => {
            assert_eq!(edges.len(), 10, "all five stored pairs are drawn");
            assert!(derive_when_empty, "the legacy overlay fallback stays on");
        }
        _ => panic!("expected a mesh with stored edges"),
    }
}
