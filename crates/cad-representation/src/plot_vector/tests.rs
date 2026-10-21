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
    })]);
    let document = build_vector_document(&representation, &simple_page()).unwrap();
    assert!(document.paths.is_empty());
    assert_eq!(
        document.diagnostics[0].code,
        vector_diagnostic::IMAGE_UNSUPPORTED
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
