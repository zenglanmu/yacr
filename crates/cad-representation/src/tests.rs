//! Unit tests.

use super::*;
use cad_db::{
    BlockDefinition, DbObject, DrawingDatabaseBuilder, EntityRenderAttributes, EntityTransparency,
    Layer,
};

fn entity(id: u128, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: Some("1A".into()),
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: 0,
    }
}

fn context() -> RepresentationContext {
    RepresentationContext::new(
        DocumentId(1),
        TolerancePolicy::default(),
        TaskStamp::new(DocumentId(1), 0),
    )
}

#[test]
fn line_becomes_a_lines_primitive() {
    let registry = ProviderRegistry::with_default_provider();
    let e = entity(
        1,
        SemanticGeometry::Line {
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
        },
    );
    let r = registry.build(&e, &context()).unwrap();
    assert_eq!(r.fragments.len(), 1);
    assert_eq!(r.completeness, Completeness::Complete);
    match &r.fragments[0].primitive {
        DisplayPrimitive::Lines(pts) => assert_eq!(pts.len(), 2),
        _ => panic!("expected lines"),
    }
}

#[test]
fn multi_ring_hatch_fill_flows_through_the_mesh_path() {
    // Donut: 10x10 outer with a 4x4 hole; the hole-aware fill from
    // cad-geometry is what the importer feeds into the compound.
    let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
    let hole = vec![[3.0, 3.0], [7.0, 3.0], [7.0, 7.0], [3.0, 7.0]];
    let fill = cad_geometry::fill_rings(&[outer.clone(), hole.clone()]).expect("donut fill");
    assert!((fill.area() - 84.0).abs() < 1e-6, "area {}", fill.area());
    let to_world = |p: [f64; 2]| Point3 {
        x: p[0],
        y: p[1],
        z: 0.0,
    };
    let vertices: Vec<Point3> = fill.vertices.iter().map(|p| to_world(*p)).collect();
    let normals = vec![
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        };
        vertices.len()
    ];
    let mut children = Vec::new();
    for ring in [&outer, &hole] {
        children.push(SemanticGeometry::Polyline {
            points: ring.iter().map(|p| to_world(*p)).collect(),
            bulges: Vec::new(),
            closed: true,
        });
    }
    children.push(SemanticGeometry::Mesh(Mesh {
        vertices,
        triangles: fill.triangles,
        normals,
        face_sources: Vec::new(),
        colors: Vec::new(),
    }));
    let e = entity(9, SemanticGeometry::Compound(children));
    let registry = ProviderRegistry::with_default_provider();
    let r = registry.build(&e, &context()).unwrap();
    assert_eq!(r.completeness, Completeness::Complete);
    let meshes = r
        .fragments
        .iter()
        .filter(|f| matches!(f.primitive, DisplayPrimitive::Mesh(_)))
        .count();
    let lines = r
        .fragments
        .iter()
        .filter(|f| matches!(f.primitive, DisplayPrimitive::Lines(_)))
        .count();
    assert_eq!(meshes, 1, "the hole-aware fill becomes one mesh primitive");
    assert_eq!(lines, 2, "both boundary loops stay outlined");
}

#[test]
fn opaque_geometry_is_reported_unsupported_not_empty_success() {
    let registry = ProviderRegistry::with_default_provider();
    let e = entity(
        2,
        SemanticGeometry::Opaque {
            type_key: "ACIS".into(),
            version: 1,
            payload: vec![1, 2, 3],
        },
    );
    let r = registry.build(&e, &context()).unwrap();
    assert!(matches!(r.completeness, Completeness::Missing(_)));
    assert!(!r.diagnostics.is_empty());
}

#[test]
fn duplicate_provider_type_key_is_rejected() {
    let mut registry = ProviderRegistry::new();
    registry
        .register(Box::new(DefaultRepresentationProvider))
        .unwrap();
    assert!(registry
        .register(Box::new(DefaultRepresentationProvider))
        .is_err());
}

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn line_entity(id: u128, space: SpaceId, a: Point3, b: Point3) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry: SemanticGeometry::Line { start: a, end: b },
        draw_order: id as i64,
    }
}

fn insert_entity(id: u128, space: SpaceId, block: u128, dx: f64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbBlockReference".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry: SemanticGeometry::Insert {
            block: BlockId(block),
            transform: Transform3::translation(p(dx, 0.0)),
        },
        draw_order: id as i64,
    }
}

fn empty_db() -> DrawingDatabaseBuilder {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b
}

#[test]
fn insert_instances_expand_with_transform_and_instance_path() {
    let mut b = empty_db();
    // Block 2: one line (0,0)-(0,1).
    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(12)],
    })
    .unwrap();
    b.insert_entity(line_entity(
        12,
        SpaceId::Block(BlockId(2)),
        p(0.0, 0.0),
        p(0.0, 1.0),
    ))
    .unwrap();
    // Block 1: line (0,0)-(1,0) plus a nested insert of block 2 at y=5.
    b.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: vec![EntityId(11), EntityId(13)],
    })
    .unwrap();
    b.insert_entity(line_entity(
        11,
        SpaceId::Block(BlockId(1)),
        p(0.0, 0.0),
        p(1.0, 0.0),
    ))
    .unwrap();
    let mut nested = insert_entity(13, SpaceId::Block(BlockId(1)), 2, 0.0);
    nested.geometry = SemanticGeometry::Insert {
        block: BlockId(2),
        transform: Transform3::translation(p(0.0, 5.0)),
    };
    b.insert_entity(nested).unwrap();
    // Model: a single insert of block 1 at x=10.
    b.insert_entity(insert_entity(1, SpaceId::Model, 1, 10.0))
        .unwrap();
    let db = b.finish().unwrap();

    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert_eq!(rep.completeness, Completeness::Complete);
    assert_eq!(rep.fragments.len(), 2);

    let mut direct = None;
    let mut deep = None;
    for fragment in &rep.fragments {
        match &fragment.primitive {
            DisplayPrimitive::Lines(pts) => match fragment.source.instance.0.len() {
                1 => direct = Some((pts[0], pts[1])),
                2 => deep = Some((pts[0], pts[1])),
                other => panic!("unexpected instance path depth {other}"),
            },
            _ => panic!("expected lines"),
        }
    }
    let (a, z) = direct.expect("direct block line");
    assert_eq!((a.x, a.y), (10.0, 0.0));
    assert_eq!((z.x, z.y), (11.0, 0.0));
    // Nested line shifted by block 1's insert (0,5) then model insert (10,0).
    let (a, z) = deep.expect("nested block line");
    assert_eq!((a.x, a.y), (10.0, 5.0));
    assert_eq!((z.x, z.y), (10.0, 6.0));
}

#[test]
fn missing_block_definition_is_reported_missing() {
    let mut b = empty_db();
    b.insert_entity(insert_entity(1, SpaceId::Model, 99, 0.0))
        .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert!(matches!(rep.completeness, Completeness::Missing(_)));
    assert!(rep
        .diagnostics
        .iter()
        .any(|d| d.code == "representation.missing_block"));
    assert!(rep.fragments.is_empty());
}

#[test]
fn self_referencing_block_is_cut_with_partial_report() {
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(2)],
    })
    .unwrap();
    b.insert_entity(insert_entity(2, SpaceId::Block(BlockId(0)), 0, 1.0))
        .unwrap();
    b.insert_entity(insert_entity(1, SpaceId::Model, 0, 0.0))
        .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    assert!(rep
        .diagnostics
        .iter()
        .any(|d| d.code == "representation.instance_cycle"));
}

fn attrs(transparency: EntityTransparency, source: GeometrySource) -> EntityRenderAttributes {
    EntityRenderAttributes {
        transparency,
        color: cad_db::EntityColor::ByLayer,
        lineweight: cad_db::EntityLineWeight::ByLayer,
        geometry_source: source,
    }
}

#[test]
fn build_expanded_carries_resolved_transparency_and_geometry_source() {
    let mut b = empty_db();
    // Model line 1: explicit 0.5 opacity, surviving from a proxy cache.
    b.insert_entity(line_entity(1, SpaceId::Model, p(0.0, 0.0), p(1.0, 0.0)))
        .unwrap();
    b.set_entity_render_attributes(
        EntityId(1),
        attrs(
            EntityTransparency::Explicit(0.5),
            GeometrySource::ProxyCache,
        ),
    )
    .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();

    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert_eq!(rep.fragments.len(), 1);
    assert_eq!(rep.fragments[0].alpha, 0.5);
    assert_eq!(rep.fragments[0].geometry_source, GeometrySource::ProxyCache);
    assert_eq!(
        rep.fragments[0].precision,
        Precision::Approximate { error_bound: None },
        "proxy-cache geometry is an approximation, not an exact source value"
    );
}

#[test]
fn build_expanded_resolves_byblock_from_the_containing_insert() {
    let mut b = empty_db();
    // Block 2 holds a ByBlock line; the INSERT carries 0.25.
    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(12)],
    })
    .unwrap();
    b.insert_entity(line_entity(
        12,
        SpaceId::Block(BlockId(2)),
        p(0.0, 0.0),
        p(0.0, 1.0),
    ))
    .unwrap();
    b.set_entity_render_attributes(
        EntityId(12),
        attrs(EntityTransparency::ByBlock, GeometrySource::Analytic),
    )
    .unwrap();
    b.insert_entity(insert_entity(2, SpaceId::Model, 2, 10.0))
        .unwrap();
    b.set_entity_render_attributes(
        EntityId(2),
        attrs(EntityTransparency::Explicit(0.25), GeometrySource::Analytic),
    )
    .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();

    let rep = registry
        .build_expanded(&db, db.entity(EntityId(2)).unwrap(), &context())
        .unwrap();
    assert_eq!(rep.fragments.len(), 1);
    // The child has no opacity of its own: it inherits the INSERT's.
    assert_eq!(rep.fragments[0].alpha, 0.25);
}

#[test]
fn byblock_at_the_model_root_falls_back_to_opaque() {
    let mut b = empty_db();
    b.insert_entity(line_entity(1, SpaceId::Model, p(0.0, 0.0), p(1.0, 0.0)))
        .unwrap();
    b.set_entity_render_attributes(
        EntityId(1),
        attrs(EntityTransparency::ByBlock, GeometrySource::Analytic),
    )
    .unwrap();
    let db = b.finish().unwrap();
    let registry = ProviderRegistry::with_default_provider();
    let rep = registry
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert_eq!(rep.fragments[0].alpha, 1.0);
}

fn attrs_styled(
    color: cad_db::EntityColor,
    lineweight: cad_db::EntityLineWeight,
) -> EntityRenderAttributes {
    EntityRenderAttributes {
        transparency: EntityTransparency::Explicit(1.0),
        color,
        lineweight,
        geometry_source: GeometrySource::Analytic,
    }
}

#[test]
fn build_expanded_carries_resolved_color_and_lineweight() {
    let mut b = empty_db();
    b.insert_entity(line_entity(1, SpaceId::Model, p(0.0, 0.0), p(1.0, 0.0)))
        .unwrap();
    b.set_entity_render_attributes(
        EntityId(1),
        attrs_styled(
            cad_db::EntityColor::Explicit([10, 20, 30]),
            cad_db::EntityLineWeight::Explicit(0.5),
        ),
    )
    .unwrap();
    let db = b.finish().unwrap();
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    assert_eq!(rep.fragments.len(), 1);
    let fragment = &rep.fragments[0];
    assert!((fragment.color[0] - 10.0 / 255.0).abs() < 1e-6);
    assert!((fragment.color[1] - 20.0 / 255.0).abs() < 1e-6);
    assert!((fragment.color[2] - 30.0 / 255.0).abs() < 1e-6);
    assert!(!fragment.color_unresolved, "explicit colour is resolved");
    assert_eq!(fragment.lineweight, 0.5);
    assert!(!fragment.lineweight_unresolved);
}

#[test]
fn build_expanded_resolves_byblock_color_and_lineweight_from_the_insert() {
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(12)],
    })
    .unwrap();
    b.insert_entity(line_entity(
        12,
        SpaceId::Block(BlockId(2)),
        p(0.0, 0.0),
        p(0.0, 1.0),
    ))
    .unwrap();
    b.set_entity_render_attributes(
        EntityId(12),
        attrs_styled(
            cad_db::EntityColor::ByBlock,
            cad_db::EntityLineWeight::ByBlock,
        ),
    )
    .unwrap();
    b.insert_entity(insert_entity(2, SpaceId::Model, 2, 10.0))
        .unwrap();
    b.set_entity_render_attributes(
        EntityId(2),
        attrs_styled(
            cad_db::EntityColor::Explicit([200, 100, 50]),
            cad_db::EntityLineWeight::Explicit(0.8),
        ),
    )
    .unwrap();
    let db = b.finish().unwrap();
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(2)).unwrap(), &context())
        .unwrap();
    assert_eq!(rep.fragments.len(), 1);
    let fragment = &rep.fragments[0];
    // The child is fully ByBlock: it inherits the INSERT's colour and weight.
    assert!((fragment.color[0] - 200.0 / 255.0).abs() < 1e-6);
    assert!((fragment.color[1] - 100.0 / 255.0).abs() < 1e-6);
    assert!((fragment.color[2] - 50.0 / 255.0).abs() < 1e-6);
    assert!(!fragment.color_unresolved);
    assert_eq!(fragment.lineweight, 0.8);
    assert!(!fragment.lineweight_unresolved);
}

#[test]
fn byblock_color_at_the_model_root_is_marked_unresolved() {
    let mut b = empty_db();
    b.insert_entity(line_entity(1, SpaceId::Model, p(0.0, 0.0), p(1.0, 0.0)))
        .unwrap();
    b.set_entity_render_attributes(
        EntityId(1),
        attrs_styled(
            cad_db::EntityColor::ByBlock,
            cad_db::EntityLineWeight::ByBlock,
        ),
    )
    .unwrap();
    let db = b.finish().unwrap();
    let rep = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(1)).unwrap(), &context())
        .unwrap();
    let fragment = &rep.fragments[0];
    // The fallback default is used, but the fragment is honest that it was
    // never resolved from a concrete source value.
    assert_eq!(fragment.color, DEFAULT_RENDER_COLOR);
    assert!(fragment.color_unresolved);
    assert_eq!(fragment.lineweight, DEFAULT_LINEWEIGHT_MM);
    assert!(fragment.lineweight_unresolved);
}

#[test]
fn build_without_import_attributes_marks_style_unresolved() {
    // A hand-built database (or the non-expanded `build`) has no import
    // attributes, so the style falls back to the documented defaults and is
    // explicitly marked unresolved rather than claiming a source value.
    let registry = ProviderRegistry::with_default_provider();
    let e = entity(
        7,
        SemanticGeometry::Line {
            start: p(0.0, 0.0),
            end: p(1.0, 0.0),
        },
    );
    let rep = registry.build(&e, &context()).unwrap();
    assert_eq!(rep.fragments.len(), 1);
    assert_eq!(rep.fragments[0].color, DEFAULT_RENDER_COLOR);
    assert!(rep.fragments[0].color_unresolved);
    assert!(rep.fragments[0].lineweight_unresolved);
}

/// A hand-built database (or the non-expanded `build`) has no import
/// attributes, so it must stay fully opaque rather than guessing.
#[test]
fn build_without_import_attributes_is_opaque_analytic() {
    let registry = ProviderRegistry::with_default_provider();
    let e = entity(
        7,
        SemanticGeometry::Line {
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
    );
    let r = registry.build(&e, &context()).unwrap();
    assert_eq!(r.fragments[0].alpha, 1.0);
    assert_eq!(r.fragments[0].geometry_source, GeometrySource::Analytic);
    assert_eq!(r.fragments[0].precision, Precision::Analytic);
}

// ---- Kernel-mesh display path (F15 seam) ----

use cad_kernel_adapter::{
    BrepCurve, BrepData, BrepFace, BrepLoop, BrepShell, BrepSurface, BrepTessellator,
    GeometryHandle, SolidExchange, SolidTessellator, TessellationBudget, TessellationRequest,
    TessellationTolerance,
};

fn selection() -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(42),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn square_brep(with_unsupported: bool) -> BrepData {
    let loop_ = BrepLoop {
        edges: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
            .iter()
            .map(|q| BrepCurve::Line {
                start: Point3 {
                    x: q[0],
                    y: q[1],
                    z: 0.0,
                },
                end: Point3 {
                    x: q[0],
                    y: q[1],
                    z: 0.0,
                },
            })
            .collect(),
    };
    let mut faces = vec![BrepFace {
        id: 0,
        surface: BrepSurface::Plane {
            origin: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            u_dir: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
        reversed: false,
        loops: vec![loop_],
    }];
    if with_unsupported {
        faces.push(BrepFace {
            id: 9,
            surface: BrepSurface::Unsupported {
                type_key: "nurbs-surface".into(),
            },
            reversed: false,
            loops: Vec::new(),
        });
    }
    BrepData {
        shells: vec![BrepShell { id: 0, faces }],
        placement: None,
    }
}

fn tessellate(exchange: SolidExchange) -> cad_kernel_adapter::TessellationResult {
    let request = TessellationRequest {
        geometry: GeometryHandle::Resolved(ObjectId(7)),
        exchange,
        tolerance: TessellationTolerance::default(),
        budget: TessellationBudget::default(),
        stamp: TaskStamp::new(DocumentId(1), 0),
    };
    BrepTessellator.tessellate(&request, &|| false).unwrap()
}

#[test]
fn kernel_mesh_becomes_a_display_mesh_with_kernel_source() {
    // A lone planar square is an open sheet: usable facets, reported as
    // Partial with open edges, never as a complete solid.
    let result = tessellate(SolidExchange::Brep(square_brep(false)));
    let rep = DisplayRepresentation::from_tessellation(&result, selection(), 1.0);
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    assert_eq!(rep.fragments.len(), 1);
    assert_eq!(rep.fragments[0].geometry_source, GeometrySource::KernelMesh);
    // Planar facets are exact, so the kernel reports analytic precision.
    assert_eq!(rep.fragments[0].precision, Precision::Analytic);
    match &rep.fragments[0].primitive {
        DisplayPrimitive::Mesh(mesh) => assert_eq!(mesh.triangles.len(), 2),
        _ => panic!("expected a mesh primitive"),
    }
}

#[test]
fn unsupported_kernel_result_stays_missing_not_empty_success() {
    let result = tessellate(SolidExchange::Sat(b"ACIS payload".to_vec()));
    let rep = DisplayRepresentation::from_tessellation(&result, selection(), 1.0);
    assert!(matches!(rep.completeness, Completeness::Missing(_)));
    assert!(rep.fragments.is_empty());
    assert!(rep
        .diagnostics
        .iter()
        .any(|d| d.code == "kernel.unsupported"));
}

#[test]
fn partial_kernel_result_reports_degradation_and_keeps_facets() {
    let result = tessellate(SolidExchange::Brep(square_brep(true)));
    let rep = DisplayRepresentation::from_tessellation(&result, selection(), 1.0);
    assert!(matches!(rep.completeness, Completeness::Partial(_)));
    assert_eq!(rep.fragments.len(), 1);
    assert!(rep
        .diagnostics
        .iter()
        .any(|d| d.code == cad_kernel_adapter::codes::KERNEL_MISSING_FACE));
}
