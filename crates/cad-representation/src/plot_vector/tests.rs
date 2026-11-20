use super::*;
use cad_db::{
    LinetypePattern, PlotMargins, PlotPaperUnits, PlotProvenance, PlotRotation, PlotSettingsRecord,
    PlotType,
};
use cad_domain::{
    Completeness, DocumentId, EntityId, GeometrySource, InstancePath, LayoutId, Point3, Precision,
    SelectionRef,
};
use std::sync::Arc;

use crate::{
    DisplayFragment, DisplayPrimitive, DisplayRepresentation, Mesh, PlotPage, PlotScale,
    PlotTarget, MM_PER_INCH,
};

fn paper(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn fragment(primitive: DisplayPrimitive) -> DisplayFragment {
    DisplayFragment {
        source: SelectionRef {
            document: DocumentId(1),
            entity: EntityId(7),
            instance: InstancePath::default(),
            sub_element: None,
        },
        geometry_source: GeometrySource::Analytic,
        precision: Precision::Analytic,
        alpha: 1.0,
        color: [0.0, 0.0, 0.0],
        color_unresolved: false,
        lineweight: 0.25,
        lineweight_unresolved: false,
        linetype: LinetypePattern::continuous(),
        linetype_unresolved: false,
        linetype_scale: 1.0,
        // Synthetic plot fixture with no source entity order.
        draw_order: 0,
        primitive,
    }
}

fn lines(points: Vec<Point3>) -> DisplayFragment {
    fragment(DisplayPrimitive::Lines(Arc::from(
        points.into_boxed_slice(),
    )))
}

fn representation(fragments: Vec<DisplayFragment>) -> DisplayRepresentation {
    DisplayRepresentation {
        fragments,
        completeness: Completeness::Complete,
        diagnostics: Vec::new(),
    }
}

/// A 100 x 100 mm page at 1 px/mm with the planner's y-down pixel map.
fn simple_page() -> PlotPage {
    PlotPage {
        width: 100,
        height: 100,
        dpi: MM_PER_INCH,
        pixels_per_mm: 1.0,
        rotation_degrees: 0.0,
        printable_mm: (100.0, 100.0),
        paper_to_pixel: [1.0, 0.0, 0.0, -1.0, 0.0, 100.0],
    }
}

fn a4(rotation: PlotRotation) -> PlotSettingsRecord {
    PlotSettingsRecord {
        layout: LayoutId(1),
        paper_size_name: "ISO_A4_(210.00_x_297.00_MM)".into(),
        paper_width: 210.0,
        paper_height: 297.0,
        margins: PlotMargins::default(),
        rotation,
        scale_numerator: 1.0,
        scale_denominator: 1.0,
        plot_type: PlotType::Layout,
        paper_units: PlotPaperUnits::Millimeters,
        provenance: PlotProvenance::Imported,
    }
}

// ---- page transform ----------------------------------------------------

#[test]
fn vector_page_is_y_up_millimetres() {
    let page = VectorPage::from_plot_page(&simple_page()).unwrap();
    assert_eq!(page.width_mm, 100.0);
    assert_eq!(page.height_mm, 100.0);
    // Paper origin (y-up bottom-left) maps to the document origin.
    let origin = page.map_paper_point(paper(0.0, 0.0));
    assert!(origin[0].abs() < 1e-9 && origin[1].abs() < 1e-9);
    // Paper top-right maps to the document top-right.
    let top_right = page.map_paper_point(paper(100.0, 100.0));
    assert!((top_right[0] - 100.0).abs() < 1e-9);
    assert!((top_right[1] - 100.0).abs() < 1e-9);
    let quarter = page.map_paper_point(paper(25.0, 40.0));
    assert!((quarter[0] - 25.0).abs() < 1e-9);
    assert!((quarter[1] - 40.0).abs() < 1e-9);
}

#[test]
fn rotation_90_swaps_the_document_size() {
    // A4 portrait rotated 90° becomes landscape; at 254 dpi (10 px/mm).
    let page = crate::plan_plot(
        &a4(PlotRotation::Degrees90),
        PlotScale::ONE_TO_ONE,
        PlotTarget::Dpi(254.0),
    )
    .unwrap();
    let vector = VectorPage::from_plot_page(&page).unwrap();
    assert!(
        (vector.width_mm - 297.0).abs() < 1e-9,
        "{}",
        vector.width_mm
    );
    assert!(
        (vector.height_mm - 210.0).abs() < 1e-9,
        "{}",
        vector.height_mm
    );
    assert!(vector.width_mm > vector.height_mm);
}

#[test]
fn physical_size_is_dpi_independent() {
    let at_100 = VectorPage::from_plot_page(
        &crate::plan_plot(
            &a4(PlotRotation::None),
            PlotScale::ONE_TO_ONE,
            PlotTarget::Dpi(100.0),
        )
        .unwrap(),
    )
    .unwrap();
    let at_600 = VectorPage::from_plot_page(
        &crate::plan_plot(
            &a4(PlotRotation::None),
            PlotScale::ONE_TO_ONE,
            PlotTarget::Dpi(600.0),
        )
        .unwrap(),
    )
    .unwrap();
    // The two differ only by the plan's integer-canvas rounding, which is at
    // most one pixel divided by the DPI: 1/3.937 ≈ 0.254 mm at 100 dpi.
    assert!((at_100.width_mm - at_600.width_mm).abs() < 0.5);
    assert!((at_100.height_mm - at_600.height_mm).abs() < 0.5);
}

// ---- SVG ---------------------------------------------------------------

#[test]
fn svg_document_has_xml_header_and_svg_root() {
    let page = VectorPage::from_plot_page(&simple_page()).unwrap();
    let bytes = render_svg(&VectorDocument::empty(&page)).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    assert!(text.contains("<svg xmlns=\"http://www.w3.org/2000/svg\""));
    assert!(text.contains("width=\"100mm\" height=\"100mm\" viewBox=\"0 0 100 100\""));
    assert!(text.trim_end().ends_with("</svg>"));
}

#[test]
fn svg_flips_y_and_writes_points_in_millimetres() {
    let representation = representation(vec![lines(vec![paper(0.0, 0.0), paper(10.0, 0.0)])]);
    let document = build_vector_document(&representation, &simple_page()).unwrap();
    let text = String::from_utf8(render_svg(&document).unwrap()).unwrap();
    // y-up document (0,0) → SVG y-down (0,100).
    assert!(text.contains("points=\"0,100 10,100\""), "{text}");
    assert!(text.contains("stroke-width=\"0.25\""), "{text}");
    assert!(text.contains("fill=\"none\""), "{text}");
}

#[test]
fn packed_segment_export_keeps_gaps_and_reports_unmatched_endpoint() {
    let mut fragment = lines(vec![paper(0.0, 0.0), paper(2.0, 0.0)]);
    fragment.primitive = DisplayPrimitive::LineSegments(Arc::from(vec![
        paper(0.0, 0.0),
        paper(2.0, 0.0),
        paper(5.0, 0.0),
        paper(7.0, 0.0),
    ]));
    let document = build_vector_document(&representation(vec![fragment]), &simple_page()).unwrap();
    let text = String::from_utf8(render_svg(&document).unwrap()).unwrap();
    assert!(text.contains("points=\"0,100 2,100\""), "{text}");
    assert!(text.contains("points=\"5,100 7,100\""), "{text}");
    assert!(!text.contains("2,100 5,100"), "invented connector");
    let mut odd = lines(vec![]);
    odd.primitive = DisplayPrimitive::LineSegments(Arc::from(vec![paper(0.0, 0.0)]));
    let document = build_vector_document(&representation(vec![odd]), &simple_page()).unwrap();
    assert!(matches!(document.completeness, Completeness::Partial(_)));
}

#[test]
fn svg_is_self_contained() {
    let representation = representation(vec![lines(vec![paper(0.0, 0.0), paper(5.0, 5.0)])]);
    let document = build_vector_document(&representation, &simple_page()).unwrap();
    let text = String::from_utf8(render_svg(&document).unwrap()).unwrap();
    // No external references: the only URLs are the SVG namespace itself.
    assert!(!text.contains("href="), "{text}");
    assert!(!text.contains("<script"), "{text}");
}

// ---- PDF ---------------------------------------------------------------

#[test]
fn pdf_document_has_header_xref_and_eof() {
    let page = VectorPage::from_plot_page(&simple_page()).unwrap();
    let bytes = render_pdf(&VectorDocument::empty(&page)).unwrap();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert!(text.starts_with("%PDF-1.4\n"));
    assert!(text.contains("1 0 obj"), "{text}");
    assert!(text.contains("/Type /Catalog"), "{text}");
    assert!(text.contains("xref"), "{text}");
    assert!(text.contains("startxref"), "{text}");
    assert!(text.trim_end().ends_with("%%EOF"));
    // An opaque empty page has the free entry plus catalog/pages/page/content.
    assert!(text.contains("xref\n0 5\n"), "{text}");
}

#[test]
fn pdf_media_box_matches_the_physical_page() {
    let page = VectorPage::from_plot_page(&simple_page()).unwrap();
    let bytes = render_pdf(&VectorDocument::empty(&page)).unwrap();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    // 100 mm = 283.464567 pt.
    assert!(
        text.contains("/MediaBox [0 0 283.464567 283.464567]"),
        "{text}"
    );
}

#[test]
fn pdf_writes_path_operators_and_length() {
    let representation = representation(vec![lines(vec![paper(0.0, 0.0), paper(10.0, 0.0)])]);
    let document = build_vector_document(&representation, &simple_page()).unwrap();
    let text = String::from_utf8_lossy(&render_pdf(&document).unwrap()).into_owned();
    assert!(text.contains(" m\n"), "{text}");
    assert!(text.contains(" l\n"), "{text}");
    assert!(text.contains("S\n"), "{text}");
    // The stream length must equal the content byte count (non-zero here).
    assert!(text.contains("/Length "), "{text}");
}

// ---- empty page --------------------------------------------------------

#[test]
fn empty_page_is_a_valid_document_with_no_paths() {
    let representation = representation(Vec::new());
    let document = build_vector_document(&representation, &simple_page()).unwrap();
    assert!(document.paths.is_empty());
    assert_eq!(document.completeness, Completeness::Complete);
    assert!(document.diagnostics.is_empty());
    assert!(!render_svg(&document).unwrap().is_empty());
    assert!(!render_pdf(&document).unwrap().is_empty());
}

// ---- unsupported primitives -------------------------------------------

#[test]
fn unshaped_text_is_reported_not_dropped() {
    let representation = representation(vec![fragment(DisplayPrimitive::Text {
        text: "hello".into(),
        origin: paper(0.0, 0.0),
        font: cad_resources::ResourceKey("style:Standard".into()),
        height: 2.5,
    })]);
    let document = build_vector_document(&representation, &simple_page()).unwrap();
    assert!(document.paths.is_empty());
    assert_eq!(document.diagnostics.len(), 1);
    assert_eq!(
        document.diagnostics[0].code,
        vector_diagnostic::TEXT_UNSHAPED
    );
    assert_eq!(
        document.completeness,
        Completeness::Partial(vec![document.diagnostics[0].message.clone()])
    );
}

#[test]
fn image_is_reported_not_dropped() {
    let representation = representation(vec![fragment(DisplayPrimitive::Image {
        resource: cad_resources::ResourceKey("img:0".into()),
        transform: cad_domain::Transform3::identity(),
        clip: None,
        adjust: cad_domain::ImageAdjust::default(),
    })]);
    let document = build_vector_document(&representation, &simple_page()).unwrap();
    assert!(document.paths.is_empty());
    assert_eq!(
        document.diagnostics[0].code,
        vector_diagnostic::IMAGE_UNSUPPORTED
    );
}

#[test]
fn mask_fill_is_the_page_background_emitted_after_every_other_path() {
    // A `WIPEOUTFRAME 0` mask (no frame) still masks: its fill is a closed
    // page-background polygon, and because the raster mask pass paints last, the
    // path document paints it **after** the geometry it hides.
    let boundary = vec![
        paper(0.0, 0.0),
        paper(10.0, 0.0),
        paper(10.0, 10.0),
        paper(0.0, 10.0),
    ];
    let mut mask = lines(vec![]);
    mask.primitive = DisplayPrimitive::Mask {
        boundary: Arc::from(boundary.into_boxed_slice()),
        inverted: false,
        frame: false,
    };
    mask.draw_order = 1;
    let mut under = lines(vec![paper(-5.0, 5.0), paper(15.0, 5.0)]);
    under.draw_order = 0;
    let document =
        build_vector_document(&representation(vec![mask, under]), &simple_page()).unwrap();
    assert_eq!(document.paths.len(), 2, "the line and the mask fill");
    assert_eq!(
        document.paths[0].fill, None,
        "the ordinary line is painted first"
    );
    let fill = document.paths.last().expect("the mask fill");
    assert!(fill.closed, "the mask fill is a closed polygon");
    assert_eq!(
        fill.fill,
        Some(PAPER_BACKGROUND),
        "the fill is the page background colour, never a transparent hole"
    );
    assert_eq!(fill.stroke, None, "the fill alone paints no outline");
    assert_eq!(fill.points.len(), 4, "the boundary ring is mapped verbatim");
    assert!(
        document.diagnostics.is_empty(),
        "an ordinary mask is fully represented now: {:?}",
        document.diagnostics
    );

    // The SVG really carries the flood: a white polygon with the even-odd rule.
    let text = String::from_utf8(render_svg(&document).unwrap()).unwrap();
    assert!(text.contains("fill=\"#FFFFFF\""), "{text}");
    assert!(text.contains("fill-rule=\"evenodd\""), "{text}");
    // And the PDF uses the even-odd fill operator, not a stroke-only path.
    let pdf = String::from_utf8_lossy(&render_pdf(&document).unwrap()).into_owned();
    assert!(pdf.contains("f*\n"), "{pdf}");
}

#[test]
fn mask_fill_follows_display_order_and_a_later_mask_still_paints_last() {
    // Two masks: ascending `draw_order`, stable. Both fills land after ordinary
    // geometry, so neither can be overpainted by a line it must hide.
    let mask = |size: f64, draw_order: i64| {
        let mut fragment = lines(vec![]);
        fragment.primitive = DisplayPrimitive::Mask {
            boundary: Arc::from(
                vec![paper(0.0, 0.0), paper(size, 0.0), paper(size, size)].into_boxed_slice(),
            ),
            inverted: false,
            frame: false,
        };
        fragment.draw_order = draw_order;
        fragment
    };
    let first = mask(1.0, 7);
    let second = mask(2.0, 3);
    let line = lines(vec![paper(0.0, 0.0), paper(3.0, 0.0)]);
    let document =
        build_vector_document(&representation(vec![first, line, second]), &simple_page()).unwrap();
    assert_eq!(document.paths.len(), 3);
    let fills: Vec<&VectorPath> = document.paths.iter().filter(|p| p.fill.is_some()).collect();
    assert_eq!(fills.len(), 2, "both masks flood");
    assert_eq!(
        fills[0].points[2],
        [2.0, 2.0],
        "draw_order 3 paints before draw_order 7"
    );
    assert_eq!(fills[1].points[2], [1.0, 1.0]);
    assert!(
        document.paths.last().unwrap().fill.is_some(),
        "masks are the document's final stage"
    );
}

#[test]
fn mask_with_frame_plots_the_fill_then_the_closed_boundary() {
    // `WIPEOUTFRAME 1` (ON): the fill is still the page background, and the
    // boundary frame follows it as a stroked closed polyline whose last point
    // repeats the first — the renderer's per-mask fill-then-frame draw.
    let boundary = vec![paper(0.0, 0.0), paper(10.0, 0.0), paper(10.0, 10.0)];
    let mut fragment = lines(vec![]);
    fragment.primitive = DisplayPrimitive::Mask {
        boundary: Arc::from(boundary.into_boxed_slice()),
        inverted: false,
        frame: true,
    };
    let document = build_vector_document(&representation(vec![fragment]), &simple_page()).unwrap();
    assert_eq!(document.paths.len(), 2, "one fill, one frame");
    assert!(document.paths[0].fill.is_some(), "the fill is first");
    let frame = &document.paths[1];
    assert_eq!(frame.fill, None, "the frame is stroked, not filled");
    assert_eq!(
        frame.points.len(),
        4,
        "the closed ring repeats its first point"
    );
    assert_eq!(frame.points[0], frame.points[3]);
    assert!(
        document.diagnostics.is_empty(),
        "a framed mask is fully represented: {:?}",
        document.diagnostics
    );
}

/// Even-odd point-in-rings, the rule both writers fill with.
fn even_odd_contains(rings: &[Vec<[f64; 2]>], point: [f64; 2]) -> bool {
    let mut inside = false;
    for ring in rings {
        let mut crossed = false;
        for (index, a) in ring.iter().enumerate() {
            let b = ring[(index + 1) % ring.len()];
            if (a[1] > point[1]) != (b[1] > point[1]) {
                let x = a[0] + (point[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if point[0] < x {
                    crossed = !crossed;
                }
            }
        }
        if crossed {
            inside = !inside;
        }
    }
    inside
}

/// The rings one filled path paints with (its own ring first).
fn filled_rings(path: &VectorPath) -> Vec<Vec<[f64; 2]>> {
    std::iter::once(path.points.clone())
        .chain(path.extra_rings.iter().cloned())
        .collect()
}

#[test]
fn inverted_mask_floods_the_page_and_leaves_its_boundary_clear() {
    // `inverted` (clip mode Inside) is the complement: the raster renderer paints
    // it as one even-odd fill over [covering ring, boundary], so the path document
    // must do the same — the page rectangle minus the boundary — not just the
    // frame.
    let boundary = vec![
        paper(20.0, 20.0),
        paper(40.0, 20.0),
        paper(40.0, 40.0),
        paper(20.0, 40.0),
    ];
    let mut fragment = lines(vec![]);
    fragment.primitive = DisplayPrimitive::Mask {
        boundary: Arc::from(boundary.into_boxed_slice()),
        inverted: true,
        frame: true,
    };
    let mut under = lines(vec![paper(0.0, 5.0), paper(90.0, 5.0)]);
    under.draw_order = 0;
    let document =
        build_vector_document(&representation(vec![under, fragment]), &simple_page()).unwrap();
    assert_eq!(
        document.paths.len(),
        3,
        "the hidden line, then one fill and one frame"
    );
    let fill = &document.paths[1];
    assert_eq!(fill.fill, Some(PAPER_BACKGROUND));
    assert_eq!(
        fill.extra_rings.len(),
        1,
        "the covering page ring is a second ring of the same path"
    );
    let rings = filled_rings(fill);
    // Outside the boundary but on the page: flooded.
    assert!(
        even_odd_contains(&rings, [5.0, 5.0]),
        "the page outside the boundary must be flooded"
    );
    assert!(even_odd_contains(&rings, [95.0, 95.0]));
    // Inside the boundary: covered twice, so it stays clear (the mask shows).
    assert!(
        !even_odd_contains(&rings, [30.0, 30.0]),
        "the boundary interior must stay unflooded"
    );
    assert!(
        document.diagnostics.is_empty(),
        "an inverted mask is fully represented: {:?}",
        document.diagnostics
    );

    // SVG: one `<path>` with both subpaths and the even-odd rule.
    let svg = String::from_utf8(render_svg(&document).unwrap()).unwrap();
    let start = svg.find("<path").expect("a multi-ring path");
    let tag = &svg[start..start + svg[start..].find("/>").unwrap()];
    assert!(tag.contains("fill=\"#FFFFFF\""), "{tag}");
    assert!(tag.contains("fill-rule=\"evenodd\""), "{tag}");
    assert!(tag.contains("stroke=\"none\""), "{tag}");
    assert_eq!(tag.matches('M').count(), 2, "two subpaths: {tag}");
    assert_eq!(tag.matches('Z').count(), 2, "both subpaths close: {tag}");
    // PDF: one fill with two subpaths, painted by the even-odd operator.
    let pdf = String::from_utf8_lossy(&render_pdf(&document).unwrap()).into_owned();
    let fill_block = &pdf[..pdf.find("f*\n").expect("an even-odd fill")];
    assert!(
        fill_block.contains("20 20 m\n"),
        "the boundary subpath: {pdf}"
    );
    assert!(
        fill_block.contains("0 0 m\n"),
        "the page-ring subpath: {pdf}"
    );
    assert!(
        fill_block.rfind("100 100 l\n").is_some(),
        "the page ring is closed over the page size: {pdf}"
    );
}

#[test]
fn degenerate_mask_boundary_is_reported_not_dropped() {
    let mut fragment = lines(vec![]);
    fragment.primitive = DisplayPrimitive::Mask {
        boundary: Arc::from(vec![paper(0.0, 0.0), paper(1.0, 0.0)].into_boxed_slice()),
        inverted: false,
        frame: true,
    };
    let document = build_vector_document(&representation(vec![fragment]), &simple_page()).unwrap();
    assert!(document.paths.is_empty());
    assert_eq!(
        document.diagnostics[0].code,
        vector_diagnostic::MASK_BOUNDARY_DEGENERATE
    );
}

#[test]
fn unexpanded_instance_is_reported_not_dropped() {
    let representation = representation(vec![fragment(DisplayPrimitive::Instance {
        block: cad_domain::BlockId(1),
        transform: cad_domain::Transform3::identity(),
    })]);
    let document = build_vector_document(&representation, &simple_page()).unwrap();
    assert!(document.paths.is_empty());
    assert_eq!(
        document.diagnostics[0].code,
        vector_diagnostic::UNEXPANDED_INSTANCE
    );
}

#[test]
fn mesh_without_triangles_is_reported() {
    let representation = representation(vec![fragment(DisplayPrimitive::Mesh(Arc::new(
        Mesh::default(),
    )))]);
    let document = build_vector_document(&representation, &simple_page()).unwrap();
    assert!(document.paths.is_empty());
    assert_eq!(
        document.diagnostics[0].code,
        vector_diagnostic::MESH_WITHOUT_TRIANGLES
    );
}

#[test]
fn mesh_triangles_become_filled_polygons() {
    let mesh = Mesh {
        uvs: Vec::new(),
        texture: None,
        vertices: vec![paper(0.0, 0.0), paper(10.0, 0.0), paper(0.0, 10.0)],
        triangles: vec![[0, 1, 2]],
        ..Mesh::default()
    };
    let representation = representation(vec![fragment(DisplayPrimitive::Mesh(Arc::new(mesh)))]);
    let document = build_vector_document(&representation, &simple_page()).unwrap();
    assert_eq!(document.paths.len(), 1);
    assert!(document.paths[0].closed);
    assert_eq!(document.paths[0].fill, Some([0.0, 0.0, 0.0]));
    assert_eq!(document.paths[0].points.len(), 3);
}

// ---- degenerate / non-finite ------------------------------------------

#[test]
fn degenerate_polyline_is_reported() {
    let representation = representation(vec![lines(vec![paper(1.0, 1.0)])]);
    let document = build_vector_document(&representation, &simple_page()).unwrap();
    assert!(document.paths.is_empty());
    assert_eq!(
        document.diagnostics[0].code,
        vector_diagnostic::DEGENERATE_POLYLINE
    );
}

#[test]
fn non_finite_polyline_is_reported() {
    let representation = representation(vec![lines(vec![paper(0.0, 0.0), paper(f64::NAN, 1.0)])]);
    let document = build_vector_document(&representation, &simple_page()).unwrap();
    assert!(document.paths.is_empty());
    assert_eq!(document.diagnostics[0].code, vector_diagnostic::NON_FINITE);
}

#[test]
fn opacity_below_one_is_carried_to_svg() {
    let mut f = lines(vec![paper(0.0, 0.0), paper(10.0, 0.0)]);
    f.alpha = 0.5;
    let representation = representation(vec![f]);
    let document = build_vector_document(&representation, &simple_page()).unwrap();
    let text = String::from_utf8(render_svg(&document).unwrap()).unwrap();
    assert!(text.contains("stroke-opacity=\"0.5\""), "{text}");
}

#[test]
fn pdf_opacity_becomes_an_extgstate_resource() {
    let mut f = lines(vec![paper(0.0, 0.0), paper(10.0, 0.0)]);
    f.alpha = 0.25;
    let representation = representation(vec![f]);
    let document = build_vector_document(&representation, &simple_page()).unwrap();
    let text = String::from_utf8_lossy(&render_pdf(&document).unwrap()).into_owned();
    // A real ExtGState resource, referenced from the page and selected with
    // `gs`, not an ignored content operator.
    assert!(
        text.contains("/ExtGState << /GS0 5 0 R /GS_opaque 6 0 R >>"),
        "{text}"
    );
    assert!(
        text.contains("/Type /ExtGState /ca 0.25 /CA 0.25"),
        "{text}"
    );
    assert!(text.contains("/GS0 gs"), "{text}");
    assert!(text.contains("/GS_opaque gs"), "{text}");
    assert!(text.contains("xref\n0 7\n"), "{text}");
}

#[test]
fn fully_opaque_pdf_has_no_transparency_objects() {
    let representation = representation(vec![lines(vec![paper(0.0, 0.0), paper(10.0, 0.0)])]);
    let document = build_vector_document(&representation, &simple_page()).unwrap();
    let text = String::from_utf8_lossy(&render_pdf(&document).unwrap()).into_owned();
    assert!(!text.contains("/ExtGState"), "{text}");
    assert!(!text.contains(" gs\n"), "{text}");
}

// ---- paper-space plot of a real WIPEOUT --------------------------------

/// A 1:1 paper layout over model space: paper (0,0)-(40,40) shows model
/// (0,0)-(40,40), so a paper millimetre is a model unit.
fn one_to_one_layout() -> cad_db::Layout {
    use cad_domain::Transform3;
    cad_db::Layout {
        id: LayoutId(1),
        name: "Layout1".into(),
        viewports: vec![cad_db::PaperViewport {
            clip: vec![
                paper(0.0, 0.0),
                paper(40.0, 0.0),
                paper(40.0, 40.0),
                paper(0.0, 40.0),
            ],
            model_to_paper: Transform3::identity(),
            completeness: Completeness::Complete,
            layer_overrides: std::collections::BTreeMap::new(),
        }],
    }
}

/// Build the real paper-space plot of one model-space mask over a hidden line.
///
/// The chain is exactly what `cad-cli-tools plot` and the desktop `SAVE`/`PLOT`
/// run (they differ only in who calls it): `DrawingDatabase → build_paper_space`
/// (viewport transform applied) `→ plan_plot_for_record → build_vector_document`.
/// `inverted` selects the two mask modes the importer can produce.
fn paper_plot_with_mask(
    inverted: bool,
) -> (crate::DisplayRepresentation, VectorPage, VectorDocument) {
    use cad_db::{DbEntity, DbObject};
    use cad_domain::{LayerId, ObjectId, Revision, SpaceId};

    let mut builder = cad_db::DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1));
    builder
        .insert_layer(cad_db::Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    let mut entity = |id: u128, geometry: cad_domain::SemanticGeometry, order: i64| {
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(id),
                    type_key: "AcDbEntity".into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: EntityId(id),
                layer: LayerId(0),
                space: SpaceId::Model,
                geometry,
                draw_order: order,
            })
            .unwrap();
    };
    // A line runs through the mask region; the mask must hide it.
    entity(
        1,
        cad_domain::SemanticGeometry::Line {
            start: paper(5.0, 20.0),
            end: paper(35.0, 20.0),
        },
        0,
    );
    entity(
        2,
        cad_domain::SemanticGeometry::Mask {
            boundary: vec![
                paper(10.0, 10.0),
                paper(30.0, 10.0),
                paper(30.0, 30.0),
                paper(10.0, 30.0),
            ],
            inverted,
        },
        1,
    );
    builder.insert_layout(one_to_one_layout()).unwrap();
    let database = builder.finish().unwrap();

    let registry = crate::ProviderRegistry::with_default_provider();
    let context = crate::RepresentationContext::new(
        DocumentId(1),
        cad_domain::TolerancePolicy::default(),
        cad_domain::TaskStamp::new(DocumentId(1), 0),
    )
    .with_wipeout_frames(true);
    let representation =
        crate::build_paper_space(&registry, &database, LayoutId(1), &context, &|_| true).unwrap();

    let record = a4(PlotRotation::None);
    let planned = crate::plan_plot_for_record(&record, PlotTarget::Dpi(96.0)).unwrap();
    let page = VectorPage::from_plot_page(&planned).unwrap();
    let document = build_vector_document(&representation, &planned).unwrap();
    (representation, page, document)
}

/// The unclipped-mask limit stays named (pinned by both paper tests).
fn assert_unclipped_mask_is_named(representation: &crate::DisplayRepresentation) {
    assert!(
        representation
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("cannot be clipped")),
        "the unclipped-mask limit must stay named: {:?}",
        representation.diagnostics
    );
}

/// The real plot chain, normal mask: the flood covers the mask and nothing else.
#[test]
fn paper_space_plot_floods_a_model_wipeout_after_the_geometry_it_hides() {
    let (representation, page, document) = paper_plot_with_mask(false);
    let svg = String::from_utf8(render_svg(&document).unwrap()).unwrap();

    // The line is plotted, then the mask floods over it with the page colour.
    let line_at = svg.find("points=").expect("the line path");
    let fill_at = svg
        .find("fill=\"#FFFFFF\"")
        .expect("the mask fill must be in the SVG");
    assert!(
        fill_at > line_at,
        "the mask fill must be painted after the geometry it hides:\n{svg}"
    );
    assert!(
        svg[fill_at..].contains("fill-rule=\"evenodd\""),
        "the flood uses the even-odd fill rule:\n{svg}"
    );
    let pdf = String::from_utf8_lossy(&render_pdf(&document).unwrap()).into_owned();
    assert!(
        pdf.contains("f*\n"),
        "the PDF fills the mask even-odd:\n{pdf}"
    );

    // The mask's own paper rectangle is what is flooded (page coordinates).
    let flooded = document
        .paths
        .iter()
        .find(|path| path.fill == Some(PAPER_BACKGROUND))
        .expect("the mask fill path");
    assert_eq!(flooded.points.len(), 4);
    assert!(flooded.extra_rings.is_empty(), "a normal mask is one ring");
    let rings = filled_rings(flooded);
    // The mask rectangle in *document* millimetres (paper 10..30 both axes).
    assert!(
        even_odd_contains(&rings, page.map_paper_point(paper(20.0, 20.0))),
        "the mask floods"
    );
    assert!(
        !even_odd_contains(&rings, page.map_paper_point(paper(35.0, 20.0))),
        "outside stays clear"
    );

    assert_unclipped_mask_is_named(&representation);
}

/// The real plot chain, **inverted** mask (DXF clip mode Inside): the page is
/// flooded and the boundary interior stays clear — the complement, not the frame
/// alone.
#[test]
fn paper_space_plot_of_an_inverted_wipeout_floods_the_page_minus_the_boundary() {
    let (representation, page, document) = paper_plot_with_mask(true);
    let svg = String::from_utf8(render_svg(&document).unwrap()).unwrap();

    // One multi-ring path, not a single polygon and not a bare frame.
    let path_at = svg
        .find("<path ")
        .expect("the inverted flood is a multi-subpath path");
    let tag = &svg[path_at..path_at + svg[path_at..].find("/>").unwrap()];
    assert!(tag.contains("fill=\"#FFFFFF\""), "{tag}");
    assert!(tag.contains("fill-rule=\"evenodd\""), "{tag}");
    assert_eq!(tag.matches('M').count(), 2, "boundary + page ring: {tag}");

    let pdf = String::from_utf8_lossy(&render_pdf(&document).unwrap()).into_owned();
    assert!(pdf.contains("f*\n"), "one even-odd fill in the PDF:\n{pdf}");

    let flooded = document
        .paths
        .iter()
        .find(|path| path.fill == Some(PAPER_BACKGROUND))
        .expect("the inverted mask fill path");
    assert_eq!(
        flooded.extra_rings.len(),
        1,
        "the covering page ring is a second ring"
    );
    let rings = filled_rings(flooded);
    // The page corners (far outside the boundary) are hidden...
    assert!(
        even_odd_contains(&rings, page.map_paper_point(paper(1.0, 1.0))),
        "the page outside the boundary is flooded"
    );
    assert!(
        even_odd_contains(&rings, page.map_paper_point(paper(200.0, 290.0))),
        "every page region outside the mask boundary is flooded"
    );
    // ...while the boundary interior stays visible (covered twice).
    assert!(
        !even_odd_contains(&rings, page.map_paper_point(paper(20.0, 20.0))),
        "the boundary interior must stay clear"
    );
    // The line's covered end (paper x = 20, inside the boundary) sits in the
    // clear part and its far end (x = 35) in the flooded part: the complement.
    assert!(!even_odd_contains(
        &rings,
        page.map_paper_point(paper(20.0, 20.0))
    ));
    assert!(even_odd_contains(
        &rings,
        page.map_paper_point(paper(35.0, 20.0))
    ));

    assert_unclipped_mask_is_named(&representation);
}
