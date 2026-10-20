//! Contract tests for the WIPEOUT mask primitive (ADR 0022 C1/C2).
//!
//! These are CPU contracts only: the pixel half of the decision lives in
//! `cad-render-wgpu`'s headless readback tests.

use super::*;

fn mask_entity(id: u128, boundary: Vec<Point3>, inverted: bool) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbWipeout".into(),
            revision: Revision(0),
            source_handle: Some("1A".into()),
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Mask { boundary, inverted },
        draw_order: 0,
    }
}

fn rectangle() -> Vec<Point3> {
    vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 5.0), p(0.0, 5.0)]
}

#[test]
fn mask_becomes_a_colourless_mask_primitive_with_the_boundary() {
    let registry = ProviderRegistry::with_default_provider();
    let boundary = rectangle();
    let r = registry
        .build(&mask_entity(1, boundary.clone(), false), &context())
        .unwrap();
    assert_eq!(
        r.fragments.len(),
        1,
        "a mask is one colourless primitive, not a polyline"
    );
    match &r.fragments[0].primitive {
        DisplayPrimitive::Mask {
            boundary: polygon,
            inverted,
        } => {
            assert_eq!(
                polygon.as_ref(),
                boundary.as_slice(),
                "the mask carries the boundary polygon unchanged"
            );
            assert!(!inverted, "clip mode Outside is not inverted");
        }
        other => panic!("expected a Mask primitive, got {}", primitive_kind(other)),
    }
    assert_eq!(
        r.completeness,
        Completeness::Complete,
        "the non-inverted mask fill is rendered, so nothing is missing"
    );
    // The primitive is colourless: the renderer owns the background colour. The
    // fragment keeps the default colour only because every fragment carries one.
    assert_eq!(r.fragments[0].color, DEFAULT_RENDER_COLOR);
    assert!(r.fragments[0].color_unresolved);
}

#[test]
fn inverted_mask_emits_a_mask_primitive_carrying_the_flag() {
    let registry = ProviderRegistry::with_default_provider();
    let boundary = rectangle();
    let r = registry
        .build(&mask_entity(2, boundary.clone(), true), &context())
        .unwrap();
    assert_eq!(
        r.fragments.len(),
        1,
        "the inverted mask is one colourless primitive, not a boundary polyline"
    );
    match &r.fragments[0].primitive {
        DisplayPrimitive::Mask {
            boundary: polygon,
            inverted,
        } => {
            assert!(*inverted, "clip mode Inside must reach the renderer");
            assert_eq!(
                polygon.as_ref(),
                boundary.as_slice(),
                "the inverted mask carries the same boundary polygon"
            );
        }
        other => panic!("expected a Mask primitive, got {}", primitive_kind(other)),
    }
    assert_eq!(
        r.completeness,
        Completeness::Complete,
        "the inverted fill is rendered too, so nothing is missing"
    );
    assert!(
        r.diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "representation.mask_inverted"),
        "the former not-rendered diagnostic must be gone"
    );
}

#[test]
fn degenerate_mask_boundary_keeps_the_polyline_and_names_the_reason() {
    let registry = ProviderRegistry::with_default_provider();
    let r = registry
        .build(
            &mask_entity(3, vec![p(0.0, 0.0), p(1.0, 0.0)], false),
            &context(),
        )
        .unwrap();
    assert!(r
        .fragments
        .iter()
        .all(|fragment| !matches!(fragment.primitive, DisplayPrimitive::Mask { .. })));
    match &r.completeness {
        Completeness::Partial(reasons) => assert!(reasons
            .iter()
            .any(|reason| reason == MASK_BOUNDARY_DEGENERATE)),
        other => panic!("a two-vertex mask boundary must be Partial, got {other:?}"),
    }
}

#[test]
fn mask_boundary_transforms_with_an_insert() {
    // A WIPEOUT inside a block: the instance transform must reach the mask
    // boundary polygon, otherwise a masked block would erase the wrong region.
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(9),
        entities: vec![EntityId(4)],
        dynamic_visibility: None,
    })
    .unwrap();
    let mut mask = mask_entity(4, rectangle(), false);
    mask.space = SpaceId::Block(BlockId(9));
    b.insert_entity(mask).unwrap();
    b.insert_entity(insert_entity(11, SpaceId::Model, 9, 5.0))
        .unwrap();
    let db = b.finish().unwrap();

    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(11)).unwrap(), &context())
        .unwrap();
    let polygons: Vec<&Arc<[Point3]>> = rep
        .fragments
        .iter()
        .filter_map(|fragment| match &fragment.primitive {
            DisplayPrimitive::Mask { boundary, .. } => Some(boundary),
            _ => None,
        })
        .collect();
    assert_eq!(polygons.len(), 1, "the nested mask survives expansion");
    let expected: Vec<Point3> = rectangle()
        .iter()
        .map(|point| p(point.x + 5.0, point.y))
        .collect();
    assert_eq!(
        polygons[0].as_ref(),
        expected.as_slice(),
        "the mask boundary follows the instance transform"
    );
}

fn primitive_kind(primitive: &DisplayPrimitive) -> &'static str {
    match primitive {
        DisplayPrimitive::Lines(_) => "Lines",
        DisplayPrimitive::LineSegments(_) => "LineSegments",
        DisplayPrimitive::Conic(_) => "Conic",
        DisplayPrimitive::Mesh(_) => "Mesh",
        DisplayPrimitive::MeshWithoutEdgeOverlay(_) => "MeshWithoutEdgeOverlay",
        DisplayPrimitive::Text { .. } => "Text",
        DisplayPrimitive::Image { .. } => "Image",
        DisplayPrimitive::Instance { .. } => "Instance",
        DisplayPrimitive::Mask { .. } => "Mask",
    }
}
