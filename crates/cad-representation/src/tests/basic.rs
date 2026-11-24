use super::*;

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
fn fragment_carries_the_source_entity_draw_order() {
    let registry = ProviderRegistry::with_default_provider();
    let mut e = entity(
        1,
        SemanticGeometry::Line {
            start: p(0.0, 0.0),
            end: p(10.0, 0.0),
        },
    );
    e.draw_order = 42;
    let r = registry.build(&e, &context()).unwrap();
    assert_eq!(r.fragments.len(), 1);
    assert_eq!(
        r.fragments[0].draw_order, 42,
        "a fragment must carry the source entity's display order, not a default"
    );
}

/// A POINT must not be drawn as the degenerate zero-length `[p, p]` chord the
/// generic fallback used to emit (ADR 0015 decision E). The omission is
/// reported honestly, and the shared tessellator's `[p, p]` output is kept for
/// the picking/bounds/snap consumers that rely on it.
#[test]
fn point_entity_emits_no_zero_length_line_in_the_base_scene() {
    let registry = ProviderRegistry::with_default_provider();
    let e = curve_entity(7, "AcDbPoint", SemanticGeometry::Point(p(3.0, 4.0)));
    let r = registry.build(&e, &context()).unwrap();
    assert!(r.fragments.is_empty(), "POINT must not draw base geometry");
    assert!(
        r.fragments.iter().all(|fragment| !matches!(
            &fragment.primitive,
            DisplayPrimitive::Lines(points) if points.len() == 2 && points[0] == points[1]
        )),
        "no zero-length line for a POINT"
    );
    assert!(r
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "representation.point_overlay"));
    assert!(matches!(r.completeness, Completeness::Partial(_)));

    // The skip is narrow: the shared tessellator is unchanged.
    let chord = cad_geometry::tessellate_geometry(
        &SemanticGeometry::Point(p(3.0, 4.0)),
        cad_geometry::TessellationParams::default(),
    );
    assert_eq!(chord, vec![p(3.0, 4.0), p(3.0, 4.0)]);
}

/// A POINT nested in a compound composes its `Partial` verdict with the
/// parent's instead of clobbering it: the Point arm uses `.combine` like every
/// sibling branch, so the overlay reason and diagnostic survive composition.
#[test]
fn point_inside_a_compound_keeps_its_partial_verdict_and_diagnostic() {
    let registry = ProviderRegistry::with_default_provider();
    let e = curve_entity(
        8,
        "AcDbCompound",
        SemanticGeometry::Compound(vec![
            SemanticGeometry::Line {
                start: p(0.0, 0.0),
                end: p(1.0, 0.0),
            },
            SemanticGeometry::Point(p(3.0, 4.0)),
        ]),
    );
    let r = registry.build(&e, &context()).unwrap();
    assert!(r
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "representation.point_overlay"));
    match &r.completeness {
        Completeness::Partial(reasons) => assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("overlay marker")),
            "the POINT reason survives composition: {reasons:?}"
        ),
        other => panic!("expected Partial, got {other:?}"),
    }
}

#[test]
fn sfnt_text_provider_emits_meshes_without_outline_lines() {
    let mut fonts = FontEngine::new();
    fonts
        .register(
            "osifont.ttf",
            Arc::from(include_bytes!("../../../../fonts/osifont.ttf").as_slice()),
        )
        .unwrap();
    let mut e = text_entity(91, 20.0);
    e.geometry = SemanticGeometry::Text {
        text: "O".into(),
        position: p(0.0, 0.0),
        style: StyleId(0),
        height: 20.0,
        rotation: 0.0,
        font: Some("osifont.ttf".into()),
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: None,
        width_factor: 1.0,
        line_spacing: cad_domain::TextLineSpacing::default(),
        mirrored: false,
    };
    let r = ProviderRegistry::with_default_provider()
        .build(&e, &context().with_fonts(Arc::new(fonts)))
        .unwrap();
    assert_eq!(r.completeness, Completeness::Complete);
    assert!(r.fragments.iter().any(|fragment| matches!(
        fragment.primitive,
        DisplayPrimitive::MeshWithoutEdgeOverlay(_)
    )));
    assert!(r
        .fragments
        .iter()
        .all(|fragment| !matches!(fragment.primitive, DisplayPrimitive::Lines(_))));
}

#[test]
fn text_with_no_named_font_shapes_through_the_fallback_chain() {
    // A Text child whose style names no font must not be silently dropped: the
    // registered fallback chain supplies the glyphs, and the fallback is
    // reported (never passed off as the requested face).
    let mut fonts = FontEngine::new();
    fonts
        .register(
            "osifont.ttf",
            Arc::from(include_bytes!("../../../../fonts/osifont.ttf").as_slice()),
        )
        .unwrap();
    fonts.set_fallback(vec!["osifont.ttf".into()]);
    let mut e = text_entity(94, 20.0);
    if let SemanticGeometry::Text { text, .. } = &mut e.geometry {
        *text = "12.5".into();
    }
    let r = ProviderRegistry::with_default_provider()
        .build(&e, &context().with_fonts(Arc::new(fonts)))
        .unwrap();
    assert!(!r.fragments.is_empty(), "fallback glyphs were not emitted");
    assert!(r.fragments.iter().all(|fragment| matches!(
        fragment.primitive,
        DisplayPrimitive::MeshWithoutEdgeOverlay(_) | DisplayPrimitive::Lines(_)
    )));
    assert!(
        r.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "text.font_fallback"),
        "{:?}",
        r.diagnostics
    );
}

#[test]
fn shx_text_provider_keeps_outline_lines_when_font_is_available() {
    let Ok(path) = std::env::var("YACR_TEST_SHX") else {
        return;
    };
    let mut fonts = FontEngine::new();
    fonts
        .register(
            "txt.shx",
            Arc::from(std::fs::read(path).unwrap().into_boxed_slice()),
        )
        .unwrap();
    let mut e = text_entity(92, 20.0);
    e.geometry = SemanticGeometry::Text {
        text: "AB".into(),
        position: p(0.0, 0.0),
        style: StyleId(0),
        height: 20.0,
        rotation: 0.0,
        font: Some("txt.shx".into()),
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: None,
        width_factor: 1.0,
        line_spacing: cad_domain::TextLineSpacing::default(),
        mirrored: false,
    };
    let r = ProviderRegistry::with_default_provider()
        .build(&e, &context().with_fonts(Arc::new(fonts)))
        .unwrap();
    assert!(!r.fragments.is_empty());
    assert!(r
        .fragments
        .iter()
        .all(|fragment| matches!(fragment.primitive, DisplayPrimitive::Lines(_))));
}

#[test]
fn whitespace_sfnt_text_is_a_valid_empty_representation() {
    let mut fonts = FontEngine::new();
    fonts
        .register(
            "osifont.ttf",
            Arc::from(include_bytes!("../../../../fonts/osifont.ttf").as_slice()),
        )
        .unwrap();
    let registry = ProviderRegistry::with_default_provider();
    let fonts = Arc::new(fonts);
    for raw in ["   ", "\\P", "\\~", "{\\H2; }"] {
        let mut e = text_entity(93, 20.0);
        if let SemanticGeometry::Text { text, font, .. } = &mut e.geometry {
            *text = raw.into();
            *font = Some("osifont.ttf".into());
        }
        let r = registry
            .build(&e, &context().with_fonts(fonts.clone()))
            .unwrap();
        assert!(r.fragments.is_empty(), "{raw:?} unexpectedly drew geometry");
        assert_eq!(r.completeness, Completeness::Complete, "{raw:?}");
        assert!(r.diagnostics.is_empty(), "{raw:?}");
    }
}

#[test]
fn missing_sfnt_glyphs_are_explicitly_reported() {
    let mut fonts = FontEngine::new();
    fonts
        .register(
            "osifont.ttf",
            Arc::from(include_bytes!("../../../../fonts/osifont.ttf").as_slice()),
        )
        .unwrap();
    let mut e = text_entity(94, 20.0);
    if let SemanticGeometry::Text { text, font, .. } = &mut e.geometry {
        *text = "\u{10FFFF}".into();
        *font = Some("osifont.ttf".into());
    }
    let r = ProviderRegistry::with_default_provider()
        .build(&e, &context().with_fonts(Arc::new(fonts)))
        .unwrap();
    assert!(matches!(
        r.completeness,
        Completeness::Partial(_) | Completeness::Missing(_)
    ));
    assert!(
        r.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "text.glyph_missing"),
        "{:?}",
        r.diagnostics
    );
}

#[test]
fn multi_ring_hatch_fill_flows_through_the_mesh_path() {
    // Donut: 10x10 outer with a 4x4 hole; the hole-aware fill from
    // cad-domain is what the importer feeds into the compound.
    let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
    let hole = vec![[3.0, 3.0], [7.0, 3.0], [7.0, 7.0], [3.0, 7.0]];
    let fill = cad_domain::hatch::fill_rings(&[outer.clone(), hole.clone()]).expect("donut fill");
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
            width: 0.0,
        });
    }
    children.push(SemanticGeometry::Mesh(Mesh {
        uvs: Vec::new(),
        texture: None,
        pbr: None,
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
fn large_compound_preserves_every_child_and_parent_selection_identity() {
    // Pattern hatches can contain tens of thousands of line segments. Keep a
    // large fixture here to exercise borrowed traversal rather than repeated
    // deep copies of the complete compound.
    let count = 20_000;
    let e = entity(
        42,
        SemanticGeometry::Compound(
            (0..count)
                .map(|i| SemanticGeometry::Line {
                    start: p(i as f64, 0.0),
                    end: p(i as f64, 1.0),
                })
                .collect(),
        ),
    );
    let r = ProviderRegistry::with_default_provider()
        .build(&e, &context())
        .unwrap();
    assert_eq!(r.fragments.len(), count);
    assert_eq!(r.completeness, Completeness::Complete);
    assert!(r.diagnostics.is_empty());
    for (i, fragment) in r.fragments.iter().enumerate() {
        assert_eq!(fragment.source.entity, e.id);
        assert_eq!(fragment.source.document, DocumentId(1));
        match &fragment.primitive {
            DisplayPrimitive::Lines(points) => {
                assert_eq!(points.as_ref(), &[p(i as f64, 0.0), p(i as f64, 1.0)]);
            }
            _ => panic!("expected compound line"),
        }
    }
}

#[test]
fn nested_compound_preserves_child_order_and_unsupported_diagnostics() {
    let e = entity(
        43,
        SemanticGeometry::Compound(vec![
            SemanticGeometry::Line {
                start: p(0.0, 0.0),
                end: p(1.0, 0.0),
            },
            SemanticGeometry::Compound(vec![
                SemanticGeometry::Opaque {
                    type_key: "ACIS".into(),
                    version: 1,
                    payload: vec![1, 2, 3],
                },
                SemanticGeometry::Line {
                    start: p(2.0, 0.0),
                    end: p(3.0, 0.0),
                },
            ]),
        ]),
    );
    let r = ProviderRegistry::with_default_provider()
        .build(&e, &context())
        .unwrap();
    assert_eq!(r.fragments.len(), 2);
    assert!(matches!(r.completeness, Completeness::Missing(_)));
    assert_eq!(r.diagnostics.len(), 1);
    assert_eq!(r.diagnostics[0].object, Some(ObjectId(e.id.0)));
    assert_eq!(r.diagnostics[0].code, "representation.opaque");
    for (fragment, start) in r.fragments.iter().zip([0.0, 2.0]) {
        assert_eq!(fragment.source.entity, e.id);
        match &fragment.primitive {
            DisplayPrimitive::Lines(points) => assert_eq!(points[0], p(start, 0.0)),
            _ => panic!("expected nested compound line"),
        }
    }
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

#[test]
fn mirrored_text_reflects_glyph_geometry_across_its_vertical_axis() {
    // `MIRRTEXT=1` is display-real: the same shaped run with `mirrored: true`
    // must be the exact reflection of the readable run across the local Y axis
    // through the anchor, with the fill triangle winding reversed.
    let mut fonts = FontEngine::new();
    fonts
        .register(
            "osifont.ttf",
            Arc::from(include_bytes!("../../../../fonts/osifont.ttf").as_slice()),
        )
        .unwrap();
    let fonts = Arc::new(fonts);
    let registry = ProviderRegistry::with_default_provider();
    let build = |mirrored: bool| {
        let mut e = text_entity(95, 20.0);
        e.geometry = SemanticGeometry::Text {
            text: "O".into(),
            position: p(0.0, 0.0),
            style: StyleId(0),
            height: 20.0,
            rotation: 0.0,
            font: Some("osifont.ttf".into()),
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            wrap_width: None,
            width_factor: 1.0,
            line_spacing: cad_domain::TextLineSpacing::default(),
            mirrored,
        };
        registry
            .build(&e, &context().with_fonts(fonts.clone()))
            .unwrap()
    };
    let readable = build(false);
    let mirrored = build(true);
    let meshes = |r: &DisplayRepresentation| -> Vec<(Vec<Point3>, Vec<[u32; 3]>)> {
        r.fragments
            .iter()
            .filter_map(|fragment| match &fragment.primitive {
                DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => {
                    Some((mesh.vertices.clone(), mesh.triangles.clone()))
                }
                _ => None,
            })
            .collect()
    };
    let plain = meshes(&readable);
    let reflected = meshes(&mirrored);
    assert_eq!(plain.len(), reflected.len());
    assert!(!plain.is_empty(), "the test needs filled glyph geometry");
    for ((plain_vertices, plain_tris), (mirror_vertices, mirror_tris)) in
        plain.iter().zip(reflected.iter())
    {
        assert_eq!(plain_vertices.len(), mirror_vertices.len());
        for (a, b) in plain_vertices.iter().zip(mirror_vertices.iter()) {
            assert!((a.x + b.x).abs() < 1e-9, "{a:?} vs {b:?}");
            assert!((a.y - b.y).abs() < 1e-9, "{a:?} vs {b:?}");
        }
        assert_eq!(plain_tris.len(), mirror_tris.len());
        for (a, b) in plain_tris.iter().zip(mirror_tris.iter()) {
            assert_eq!([a[0], a[2], a[1]], *b, "winding is reversed");
        }
    }
}
