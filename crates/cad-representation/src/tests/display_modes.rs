//! Session display modes that change the **derived** primitives: `FILL`,
//! `QTEXT` and `VIEWRES`.
//!
//! These are scene/batch-level contracts, not pixel or GPU evidence: every
//! assertion reads the fragments a provider builds (which primitive, how many
//! points, which boundary), so a mode that only stored a token would fail here.

use super::*;

fn display_context() -> RepresentationContext {
    context()
}

/// One filled square HATCH in the world XY plane, with no pattern rows.
fn solid_hatch() -> SemanticGeometry {
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
    .expect("the square hatch is a valid creation-path boundary");
    SemanticGeometry::Hatch(Box::new(hatch))
}

fn primitives(representation: &DisplayRepresentation) -> Vec<&DisplayPrimitive> {
    representation
        .fragments
        .iter()
        .map(|fragment| &fragment.primitive)
        .collect()
}

fn mesh_count(representation: &DisplayRepresentation) -> usize {
    primitives(representation)
        .iter()
        .filter(|primitive| matches!(primitive, DisplayPrimitive::Mesh(_)))
        .count()
}

fn all_line_points(representation: &DisplayRepresentation) -> Vec<Point3> {
    let mut points = Vec::new();
    for primitive in primitives(representation) {
        match primitive {
            DisplayPrimitive::Lines(line) | DisplayPrimitive::LineSegments(line) => {
                points.extend(line.iter().copied());
            }
            _ => {}
        }
    }
    points
}

/// `FILL OFF` drops the fill and keeps the hatch's own boundary vertices.
#[test]
fn fill_off_draws_a_hatch_as_its_boundary_loops_only() {
    let registry = ProviderRegistry::with_default_provider();
    let geometry = solid_hatch();

    let filled = registry
        .build(&entity(1, geometry.clone()), &display_context())
        .unwrap();
    assert_eq!(
        mesh_count(&filled),
        1,
        "FILL ON draws the solid hatch's fill mesh"
    );

    let outline = registry
        .build(&entity(1, geometry), &display_context().with_fill(false))
        .unwrap();
    assert_eq!(
        outline.fragments.len(),
        1,
        "one boundary loop, one fragment"
    );
    assert_eq!(
        mesh_count(&outline),
        0,
        "FILL OFF draws no fill mesh at all"
    );
    // The boundary is a closed polyline through the hatch's own corners, so the
    // outline keeps every snap point of the filled draw.
    let points = all_line_points(&outline);
    assert_eq!(points.len(), 5, "four corners plus the closing vertex");
    for corner in [p(0.0, 0.0), p(4.0, 0.0), p(4.0, 4.0), p(0.0, 4.0)] {
        assert!(
            points
                .iter()
                .any(|point| (point.x - corner.x).abs() < 1e-9
                    && (point.y - corner.y).abs() < 1e-9),
            "the FILL OFF outline keeps corner {corner:?}"
        );
    }
    assert_eq!(
        outline.completeness,
        Completeness::Complete,
        "an outline-only draw is not a degraded draw"
    );
}

/// A wide polyline under `FILL OFF` draws its band's closed outline, not the
/// centreline and not the filled band.
#[test]
fn fill_off_draws_a_wide_polyline_as_its_stroke_outline() {
    let geometry = SemanticGeometry::Polyline {
        points: vec![p(0.0, 0.0), p(4.0, 0.0)],
        bulges: vec![0.0],
        closed: false,
        width: 2.0,
    };
    let registry = ProviderRegistry::with_default_provider();
    let representation = registry
        .build(&entity(1, geometry), &display_context().with_fill(false))
        .unwrap();
    assert_eq!(
        mesh_count(&representation),
        0,
        "FILL OFF draws the band as an outline, not as triangles"
    );
    assert_eq!(representation.fragments.len(), 1);
    let points = all_line_points(&representation);
    // Two band corners per end, walked forward on one side and back on the
    // other, then closed: 2 + 2 + 1.
    assert_eq!(points.len(), 5, "the outline is one closed loop");
    let has = |x: f64, y: f64| {
        points
            .iter()
            .any(|point| (point.x - x).abs() < 1e-9 && (point.y - y).abs() < 1e-9)
    };
    assert!(
        has(0.0, -1.0) && has(0.0, 1.0),
        "the start cap is the ±half-width pair"
    );
    assert!(
        has(4.0, -1.0) && has(4.0, 1.0),
        "the end cap is the ±half-width pair"
    );
    assert!(
        !points.iter().any(|point| (point.y).abs() < 1e-9),
        "the centreline is not part of the outline"
    );
}

/// A 2D SOLID (`AcDbSolid`) under `FILL OFF` draws its boundary edges, and the
/// shared internal diagonal of the two-triangle mesh is not one of them.
#[test]
fn fill_off_draws_a_2d_solid_as_its_boundary_edges() {
    use cad_domain::Mesh;
    // Two triangles sharing the (0,0)-(4,4) diagonal.
    let mut solid = entity(
        1,
        SemanticGeometry::Mesh(Mesh {
            uvs: Vec::new(),
            texture: None,
            pbr: None,
            vertices: vec![p(0.0, 0.0), p(4.0, 0.0), p(4.0, 4.0), p(0.0, 4.0)],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            normals: Vec::new(),
            face_sources: Vec::new(),
            colors: Vec::new(),
        }),
    );
    solid.object.type_key = "AcDbSolid".into();
    let registry = ProviderRegistry::with_default_provider();

    let filled = registry.build(&solid, &display_context()).unwrap();
    assert_eq!(mesh_count(&filled), 1, "FILL ON draws the SOLID's faces");

    let outline = registry
        .build(&solid, &display_context().with_fill(false))
        .unwrap();
    assert_eq!(mesh_count(&outline), 0);
    let points = all_line_points(&outline);
    assert_eq!(
        points.len(),
        8,
        "four boundary edges, as four endpoint pairs"
    );
    // The diagonal is used by both triangles, so it is not a boundary edge.
    let diagonal = points
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|pair| {
            (pair[0].x - 0.0).abs() < 1e-9
                && (pair[0].y - 0.0).abs() < 1e-9
                && (pair[1].x - 4.0).abs() < 1e-9
                && (pair[1].y - 4.0).abs() < 1e-9
        })
        .count();
    assert_eq!(diagonal, 0, "the internal diagonal must not be drawn");
}

/// A bare mesh (a kernel display mesh, a dimension arrowhead) is **not** a 2D
/// SOLID, so `FILL OFF` leaves it filled: the declared boundary of the rule.
#[test]
fn fill_off_keeps_a_non_solid_mesh_filled() {
    use cad_domain::Mesh;
    let geometry = SemanticGeometry::Mesh(Mesh {
        uvs: Vec::new(),
        texture: None,
        pbr: None,
        vertices: vec![p(0.0, 0.0), p(1.0, 0.0), p(0.0, 1.0)],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    });
    let registry = ProviderRegistry::with_default_provider();
    let representation = registry
        .build(&entity(1, geometry), &display_context().with_fill(false))
        .unwrap();
    assert_eq!(
        mesh_count(&representation),
        1,
        "only AcDbSolid is switched by FILL; a bare mesh stays filled (declared)"
    );
}

fn osifont_context() -> RepresentationContext {
    let mut engine = FontEngine::new();
    engine
        .register(
            "osifont.ttf",
            Arc::from(include_bytes!("../../../../fonts/osifont.ttf").as_slice()),
        )
        .unwrap();
    engine.set_fallback(vec!["osifont.ttf".into()]);
    display_context().with_fonts(Arc::new(engine))
}

fn text_entity() -> DbEntity {
    let mut text = entity(
        1,
        SemanticGeometry::Text {
            text: "OH".into(),
            position: p(0.0, 0.0),
            style: StyleId(0),
            height: 10.0,
            rotation: 0.0,
            font: Some("osifont.ttf".into()),
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            wrap_width: None,
            width_factor: 1.0,
            line_spacing: cad_domain::TextLineSpacing::default(),
        },
    );
    text.object.type_key = "AcDbText".into();
    text
}

/// `QTEXT ON` replaces the shaped glyph fragments with exactly one rectangle
/// that covers the same extent.
#[test]
fn qtext_replaces_glyphs_with_one_placeholder_rectangle() {
    let registry = ProviderRegistry::with_default_provider();
    let context = osifont_context();
    let entity = text_entity();

    let glyphs = registry.build(&entity, &context).unwrap();
    assert!(
        glyphs.fragments.len() > 1,
        "the glyph draw has more than one fragment"
    );
    assert!(
        glyphs
            .fragments
            .iter()
            .all(|fragment| !matches!(fragment.primitive, DisplayPrimitive::Lines(_))),
        "SFNT glyphs are filled faces, so the QTEXT box cannot be mistaken for them"
    );
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for fragment in &glyphs.fragments {
        // SFNT glyph faces reach the display as `MeshWithoutEdgeOverlay` (the
        // text-glyph display class), so both mesh variants are sampled here.
        let mesh = match &fragment.primitive {
            DisplayPrimitive::Mesh(mesh) | DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => mesh,
            _ => continue,
        };
        for vertex in &mesh.vertices {
            min[0] = min[0].min(vertex.x);
            min[1] = min[1].min(vertex.y);
            max[0] = max[0].max(vertex.x);
            max[1] = max[1].max(vertex.y);
        }
    }
    assert!(
        min[0].is_finite() && max[0] > min[0],
        "the glyphs have an extent"
    );

    let quick = registry
        .build(&entity, &context.clone().with_qtext(true))
        .unwrap();
    assert_eq!(
        quick.fragments.len(),
        1,
        "one placeholder for the whole text"
    );
    assert_eq!(mesh_count(&quick), 0, "no glyph faces survive QTEXT");
    let points = all_line_points(&quick);
    assert_eq!(points.len(), 5, "a closed rectangle");
    let xs: Vec<f64> = points.iter().map(|point| point.x).collect();
    let ys: Vec<f64> = points.iter().map(|point| point.y).collect();
    let (box_min_x, box_max_x) = (
        xs.iter().copied().fold(f64::INFINITY, f64::min),
        xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    );
    let (box_min_y, box_max_y) = (
        ys.iter().copied().fold(f64::INFINITY, f64::min),
        ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    );
    assert!(
        (box_min_x - min[0]).abs() < 1e-9,
        "the box starts where the glyphs start"
    );
    assert!(
        (box_max_x - max[0]).abs() < 1e-9,
        "the box ends where the glyphs end"
    );
    assert!(
        (box_min_y - min[1]).abs() < 1e-9,
        "the box baseline matches"
    );
    assert!(
        (box_max_y - max[1]).abs() < 1e-9,
        "the box cap height matches"
    );
}

/// Without a font engine the text has no derivable extent, so `QTEXT` keeps the
/// text primitive and names the gap instead of inventing a width.
#[test]
fn qtext_without_shaping_reports_the_missing_extent_and_draws_nothing_new() {
    let registry = ProviderRegistry::with_default_provider();
    let representation = registry
        .build(&text_entity(), &display_context().with_qtext(true))
        .unwrap();
    assert_eq!(
        representation.fragments.len(),
        1,
        "the unshaped text primitive is kept"
    );
    assert!(matches!(
        representation.fragments[0].primitive,
        DisplayPrimitive::Text { .. }
    ));
    assert_eq!(
        representation.diagnostics.len(),
        1,
        "the missing extent is reported, not silent"
    );
    assert_eq!(representation.diagnostics[0].code, "qtext.extent_unknown");
}

/// `VIEWRES` caps the chord paths it can reach and demonstrably leaves the
/// analytic conic path alone — the declared partiality.
#[test]
fn viewres_caps_chord_tessellation_and_not_the_analytic_conics() {
    let registry = ProviderRegistry::with_default_provider();
    // A big radius so the tolerance-driven segment count is far above the cap.
    let circle = |radius: f64| SemanticGeometry::Circle {
        center: p(0.0, 0.0),
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        radius,
    };
    let coarse = registry
        .build(
            &entity(1, circle(100_000.0)),
            &display_context().with_curve_segments(Some(12)),
        )
        .unwrap();
    assert_eq!(
        coarse.fragments.len(),
        1,
        "a circle is one analytic primitive"
    );
    assert!(
        matches!(coarse.fragments[0].primitive, DisplayPrimitive::Conic(_)),
        "circles stay analytic, so VIEWRES cannot change them (declared)"
    );

    // A bulge arc really is chord-tessellated: the ceiling changes its point
    // count. A half-circle of radius 100 000 needs far more than 6 chords at the
    // display tolerance, so both ceilings are exercised for real.
    let arc = SemanticGeometry::Polyline {
        points: vec![p(-100_000.0, 0.0), p(100_000.0, 0.0)],
        bulges: vec![1.0],
        closed: false,
        width: 0.0,
    };
    let fine = registry
        .build(
            &entity(2, arc.clone()),
            &display_context().with_curve_segments(Some(1000)),
        )
        .unwrap();
    let coarse = registry
        .build(
            &entity(2, arc),
            &display_context().with_curve_segments(Some(6)),
        )
        .unwrap();
    let fine_points = all_line_points(&fine).len();
    let coarse_points = all_line_points(&coarse).len();
    assert!(
        coarse_points < fine_points,
        "a 6-segment ceiling must coarsen the arc: {coarse_points} vs {fine_points}"
    );
    assert_eq!(coarse_points, 7, "6 segments means 7 samples");
}

/// A `VIEWRES` ceiling below the build's floor lowers the floor instead of
/// panicking in `clamp`, and never collapses a closed curve to nothing.
#[test]
fn viewres_floor_keeps_a_closed_curve_drawable() {
    use crate::viewres_segment_ceiling;
    assert_eq!(viewres_segment_ceiling(1), 3);
    assert_eq!(viewres_segment_ceiling(1000), 1000);

    let registry = ProviderRegistry::with_default_provider();
    // The chord fallback is what a low ceiling reaches; a dashed circle takes it
    // through the expansion path, so assert the tessellator itself here.
    let params = cad_geometry::TessellationParams {
        tolerance: 0.6,
        max_segments: 3,
        min_segments: 3,
    };
    let points = cad_geometry::tessellate_geometry(
        &SemanticGeometry::Circle {
            center: p(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 1000.0,
        },
        params,
    );
    assert_eq!(points.len(), 4, "a 3-segment circle is a closed triangle");
    let _ = registry;
}

/// The vector-plot path consumes the same `FILL` flag: `FILL ON` plots a filled
/// hatch area, `FILL OFF` plots its boundary as open stroked outlines.
#[test]
fn fill_off_plots_a_hatch_as_an_unfilled_outline() {
    let page = PlotPage {
        width: 100,
        height: 100,
        dpi: crate::MM_PER_INCH,
        pixels_per_mm: 1.0,
        rotation_degrees: 0.0,
        printable_mm: (100.0, 100.0),
        paper_to_pixel: [1.0, 0.0, 0.0, -1.0, 0.0, 100.0],
    };
    let registry = ProviderRegistry::with_default_provider();
    let geometry = solid_hatch();

    let filled = registry
        .build(&entity(1, geometry.clone()), &display_context())
        .unwrap();
    let filled_document = crate::build_vector_document(&filled, &page).unwrap();
    assert!(
        filled_document.paths.iter().any(|path| path.fill.is_some()),
        "FILL ON plots the hatch area filled"
    );

    let outline = registry
        .build(&entity(1, geometry), &display_context().with_fill(false))
        .unwrap();
    let outline_document = crate::build_vector_document(&outline, &page).unwrap();
    assert!(
        outline_document
            .paths
            .iter()
            .all(|path| path.fill.is_none()),
        "FILL OFF plots no filled path"
    );
    assert!(
        outline_document
            .paths
            .iter()
            .any(|path| path.stroke.is_some() && !path.closed),
        "the boundary is plotted as open stroked segments, not as a filled area"
    );
}
