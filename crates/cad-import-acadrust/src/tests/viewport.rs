//! Viewport content transforms and clipping semantics.

use super::*;
use crate::export::{AcisVersion, DxfTarget};

/// Build a top/plan VIEWPORT entity with the given paper rectangle, scale,
/// and model view centre.
///
/// Both the DXF view centre (group 12/22) and the 3D view target (group
/// 17/27/37) are set to the same model point, so the fixture does not depend on
/// which field the importer anchors on.
fn top_viewport(
    center: (f64, f64),
    w: f64,
    h: f64,
    view_height: f64,
    view_target: (f64, f64),
) -> acadrust::entities::Viewport {
    let mut v = acadrust::entities::Viewport::new();
    v.id = 2;
    v.center = acadrust::types::Vector3::new(center.0, center.1, 0.0);
    v.width = w;
    v.height = h;
    v.view_height = view_height;
    v.view_direction = acadrust::types::Vector3::UNIT_Z;
    v.view_center = acadrust::types::Vector3::new(view_target.0, view_target.1, 0.0);
    v.view_target = acadrust::types::Vector3::new(view_target.0, view_target.1, 0.0);
    v
}

fn paper_corner(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point3> {
    vec![
        paper_corner(x0, y0),
        paper_corner(x1, y0),
        paper_corner(x1, y1),
        paper_corner(x0, y1),
    ]
}

/// A closed LWPOLYLINE clip boundary through the given paper points.
fn closed_polyline(points: &[(f64, f64)]) -> acadrust::entities::LwPolyline {
    let mut polyline = acadrust::entities::LwPolyline::new();
    polyline.is_closed = true;
    polyline.vertices = points
        .iter()
        .map(|(x, y)| acadrust::entities::LwVertex::from_coords(*x, *y))
        .collect();
    polyline
}

/// Attach a clip boundary handle to a viewport and resolve it against a fresh
/// empty document (the handle then dangles).
fn viewport_with_dangling_clip() -> acadrust::entities::Viewport {
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.clip_boundary_handle = acadrust::types::Handle::new(0x1A);
    vp
}

#[test]
fn one_to_one_hundred_viewport_has_four_corners_and_the_real_transform() {
    // 1:100: a 10000×5000 model region fits the 100×50 paper window, view
    // centre (10, 20).
    let vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    let doc = acadrust::CadDocument::new();
    let pv = paper_viewport(&vp, &doc).expect("id 2 is a content viewport");
    assert_eq!(pv.completeness, Completeness::Complete);
    assert_eq!(pv.clip, rectangle(0.0, 0.0, 100.0, 50.0));
    // The stored paper→model transform maps the paper centre to the view
    // target; 100 model units per paper unit.
    let m = &pv.model_to_paper.matrix;
    assert!((m[0][0] - 100.0).abs() < 1e-9, "{m:?}");
    assert!((m[1][1] - 100.0).abs() < 1e-9, "{m:?}");
    let model_centre = pv.model_to_paper.apply_point(Point3 {
        x: 50.0,
        y: 25.0,
        z: 0.0,
    });
    assert!((model_centre.x - 10.0).abs() < 1e-9, "{model_centre:?}");
    assert!((model_centre.y - 20.0).abs() < 1e-9, "{model_centre:?}");
    // One paper unit right of centre is 100 model units.
    let right = pv.model_to_paper.apply_point(Point3 {
        x: 51.0,
        y: 25.0,
        z: 0.0,
    });
    assert!((right.x - 110.0).abs() < 1e-9, "{right:?}");
}

#[test]
fn view_center_is_used_when_view_target_is_zero() {
    // QCAD-style VIEWPORT: the real 2D centre lives in group 12/22 and group
    // 17/27/37 is left at the origin. The importer must anchor on the view
    // centre, not collapse the viewport to "model origin at viewport centre".
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (0.0, 0.0));
    vp.view_center = acadrust::types::Vector3::new(10.0, 20.0, 0.0);
    let doc = acadrust::CadDocument::new();
    let pv = paper_viewport(&vp, &doc).expect("id 2 is a content viewport");
    assert_eq!(pv.completeness, Completeness::Complete);
    assert_eq!(pv.clip, rectangle(0.0, 0.0, 100.0, 50.0));
    let m = &pv.model_to_paper.matrix;
    assert!((m[0][0] - 100.0).abs() < 1e-9, "{m:?}");
    assert!((m[1][1] - 100.0).abs() < 1e-9, "{m:?}");
    let model_centre = pv.model_to_paper.apply_point(Point3 {
        x: 50.0,
        y: 25.0,
        z: 0.0,
    });
    assert!((model_centre.x - 10.0).abs() < 1e-9, "{model_centre:?}");
    assert!((model_centre.y - 20.0).abs() < 1e-9, "{model_centre:?}");
}

#[test]
fn view_target_is_the_fallback_anchor_when_view_center_is_zero() {
    // Files that populate only group 17/27 must keep working: a zero view
    // centre falls back to the 3D target's x/y as the model anchor.
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.view_center = acadrust::types::Vector3::ZERO;
    let doc = acadrust::CadDocument::new();
    let pv = paper_viewport(&vp, &doc).expect("id 2 is a content viewport");
    assert_eq!(pv.completeness, Completeness::Complete);
    let model_centre = pv.model_to_paper.apply_point(Point3 {
        x: 50.0,
        y: 25.0,
        z: 0.0,
    });
    assert!((model_centre.x - 10.0).abs() < 1e-9, "{model_centre:?}");
    assert!((model_centre.y - 20.0).abs() < 1e-9, "{model_centre:?}");
}

#[test]
fn sheet_viewport_and_off_viewport_are_not_content_viewports() {
    let doc = acadrust::CadDocument::new();
    let mut sheet = top_viewport((0.0, 0.0), 100.0, 100.0, 100.0, (0.0, 0.0));
    sheet.id = 1;
    assert!(paper_viewport(&sheet, &doc).is_none(), "id 1 is the sheet");
    let mut off = top_viewport((0.0, 0.0), 100.0, 100.0, 100.0, (0.0, 0.0));
    off.status.is_on = false;
    assert!(
        paper_viewport(&off, &doc).is_none(),
        "an off viewport draws nothing"
    );
}

#[test]
fn rotated_viewport_is_partial_with_a_twist_reason() {
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.twist_angle = std::f64::consts::FRAC_PI_4;
    let doc = acadrust::CadDocument::new();
    let pv = paper_viewport(&vp, &doc).expect("still a content viewport");
    match &pv.completeness {
        Completeness::Partial(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("twist")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
    // The four-corner rectangle is still present (the representation layer
    // will refuse it with `viewport.twisted_transform`, never square it off).
    assert_eq!(pv.clip.len(), 4);
}

#[test]
fn non_perpendicular_view_is_partial_with_an_off_plane_reason() {
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.view_direction = acadrust::types::Vector3::new(1.0, 0.0, 1.0);
    let doc = acadrust::CadDocument::new();
    let pv = paper_viewport(&vp, &doc).unwrap();
    match &pv.completeness {
        Completeness::Partial(reasons) => {
            assert!(
                reasons.iter().any(|r| r.contains("perpendicular")),
                "{reasons:?}"
            );
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}
// ---- VPCLIP / MVIEW clip boundary: the file's own polygon ----

/// A paper polygon as the point list a viewport clip stores.
fn polygon(points: &[(f64, f64)]) -> Vec<Point3> {
    points.iter().map(|(x, y)| paper_corner(*x, *y)).collect()
}

/// The L-shaped boundary used by the polygon tests: five vertices, one concave
/// corner, counter-clockwise.
fn l_shape() -> Vec<Point3> {
    polygon(&[
        (0.0, 0.0),
        (100.0, 0.0),
        (100.0, 25.0),
        (50.0, 60.0),
        (0.0, 50.0),
    ])
}

/// The stored refusal code of a `Partial` viewport: the reason's code prefix.
fn refusal_code(viewport: &cad_db::PaperViewport) -> &str {
    match &viewport.completeness {
        Completeness::Partial(reasons) => reasons[0]
            .split_once(": ")
            .map(|(code, _)| code)
            .unwrap_or_else(|| panic!("a refusal reason names its code: {reasons:?}")),
        other => panic!("expected Partial, got {other:?}"),
    }
}

/// A document holding the given boundary entity (reached by handle) plus one
/// content viewport on `Layout1`'s paper block.
fn document_with_viewport(
    boundary: Option<acadrust::EntityType>,
    mut viewport: acadrust::entities::Viewport,
) -> acadrust::CadDocument {
    let mut doc = acadrust::CadDocument::new();
    if let Some(entity) = boundary {
        let handle = doc
            .add_entity(entity)
            .expect("the boundary entity is accepted");
        viewport.clip_boundary_handle = handle;
    }
    doc.add_entity_to_layout(EntityType::Viewport(Box::new(viewport)), "Layout1")
        .expect("the viewport is added to the paper layout");
    doc
}

/// Import an in-memory acadrust document.
///
/// Bytes are irrelevant here: the contract under test is
/// `read_layouts`/`read_entities`, and a writer/parser round trip would add a
/// second system whose behaviour could mask a clip bug (the same reason the
/// layout-order tests build the document in memory).
fn import_document(doc: &acadrust::CadDocument) -> ImportedDrawing {
    let request = request(Vec::new());
    ImporterBuilder::new(&request, doc, ReadStats::default(), compute_identity(&[]))
        .run()
        .expect("the in-memory document imports")
}

/// The single content viewport of the imported drawing.
fn only_viewport(drawing: &ImportedDrawing) -> &cad_db::PaperViewport {
    let mut viewports = drawing
        .database
        .layouts()
        .flat_map(|layout| layout.viewports.iter());
    let viewport = viewports.next().expect("one content viewport was imported");
    assert!(viewports.next().is_none(), "exactly one content viewport");
    viewport
}

#[test]
fn polygonal_clip_boundary_imports_as_that_polygon_and_is_drawn() {
    // A five-vertex L-shaped VPCLIP boundary: the file's own polygon, not its
    // envelope and not the viewport rectangle.
    let doc = document_with_viewport(
        Some(EntityType::LwPolyline(closed_polyline(&[
            (0.0, 0.0),
            (100.0, 0.0),
            (100.0, 25.0),
            (50.0, 60.0),
            (0.0, 50.0),
        ]))),
        top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0)),
    );
    let drawing = import_document(&doc);
    let viewport = only_viewport(&drawing);

    assert_eq!(viewport.completeness, Completeness::Complete);
    assert_eq!(viewport.clip, l_shape());
    // Nothing about the clip is downgraded in the report.
    let codes: Vec<&str> = drawing
        .report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert!(
        !codes.iter().any(|code| code.contains("viewport")),
        "a stored polygon must not report a viewport loss: {codes:?}"
    );

    // ...and the representation layer reads the stored points as a supported
    // simple polygon, not as a rectangle and not as `viewport.complex_clip`.
    let state = cad_representation::layout::viewport_transform(viewport);
    let cad_representation::layout::ViewportState::Supported(transform) = state else {
        panic!("the polygon clip is drawable, got {state:?}");
    };
    assert_eq!(transform.paper_clip.as_rect(), None, "not squared off");
    assert_eq!(
        transform.paper_clip.boundary(),
        l_shape()
            .iter()
            .map(|point| [point.x, point.y])
            .collect::<Vec<_>>()
            .as_slice()
    );
}

#[test]
fn axis_aligned_clip_boundary_is_byte_identical_to_the_plain_rectangle() {
    // The pre-polygon reader stored the four canonical corners; a rectangular
    // VPCLIP boundary must keep producing exactly those bytes.
    let plain = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    let empty = acadrust::CadDocument::new();
    let expected = paper_viewport(&plain, &empty)
        .expect("a content viewport")
        .clip;
    assert_eq!(expected, rectangle(0.0, 0.0, 100.0, 50.0));

    let doc = document_with_viewport(
        Some(EntityType::LwPolyline(closed_polyline(&[
            (0.0, 0.0),
            (100.0, 0.0),
            (100.0, 50.0),
            (0.0, 50.0),
        ]))),
        plain,
    );
    let drawing = import_document(&doc);
    let viewport = only_viewport(&drawing);
    assert_eq!(viewport.completeness, Completeness::Complete);
    assert_eq!(
        viewport.clip, expected,
        "a rectangular boundary stores the same four corners as the viewport rectangle"
    );
    let state = cad_representation::layout::viewport_transform(viewport);
    let cad_representation::layout::ViewportState::Supported(transform) = state else {
        panic!("the rectangle clip is drawable, got {state:?}");
    };
    assert_eq!(
        transform.paper_clip.as_rect(),
        Some(([0.0, 0.0], [100.0, 50.0])),
        "a rectangle keeps the dedicated half-plane clipper path"
    );
}

#[test]
fn clockwise_rectangular_clip_boundary_still_canonicalizes() {
    // Files do not promise a winding order; the stored bytes are the canonical
    // counter-clockwise corners either way.
    let doc = document_with_viewport(
        Some(EntityType::LwPolyline(closed_polyline(&[
            (0.0, 0.0),
            (0.0, 50.0),
            (100.0, 50.0),
            (100.0, 0.0),
        ]))),
        top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0)),
    );
    let drawing = import_document(&doc);
    let viewport = only_viewport(&drawing);
    assert_eq!(viewport.completeness, Completeness::Complete);
    assert_eq!(viewport.clip, rectangle(0.0, 0.0, 100.0, 50.0));
}

#[test]
fn rotated_clip_boundary_imports_as_a_polygon_not_a_rectangle() {
    // A rotated square uses no corner of its envelope that a rectangle check
    // could accept, so it must arrive as four polygon vertices.
    let doc = document_with_viewport(
        Some(EntityType::LwPolyline(closed_polyline(&[
            (50.0, 0.0),
            (100.0, 50.0),
            (50.0, 100.0),
            (0.0, 50.0),
        ]))),
        top_viewport((50.0, 50.0), 100.0, 100.0, 5000.0, (10.0, 20.0)),
    );
    let drawing = import_document(&doc);
    let viewport = only_viewport(&drawing);
    assert_eq!(viewport.completeness, Completeness::Complete);
    assert_eq!(
        viewport.clip,
        polygon(&[(50.0, 0.0), (100.0, 50.0), (50.0, 100.0), (0.0, 50.0)])
    );
    let state = cad_representation::layout::viewport_transform(viewport);
    let cad_representation::layout::ViewportState::Supported(transform) = state else {
        panic!("a rotated square is a simple polygon, got {state:?}");
    };
    assert_eq!(transform.paper_clip.as_rect(), None);
}

#[test]
fn dangling_clip_boundary_handle_is_refused_by_name() {
    // The handle references no entity. The viewport keeps its own rectangle as
    // data but is `Partial`, so the representation refuses it instead of
    // drawing a full rectangle over a clipped window.
    let vp = viewport_with_dangling_clip();
    let doc = acadrust::CadDocument::new();
    let pv = paper_viewport(&vp, &doc).expect("still a content viewport");
    assert_eq!(refusal_code(&pv), viewport_clip_reason::UNRESOLVED);
    assert_eq!(pv.clip, rectangle(0.0, 0.0, 100.0, 50.0));
    assert!(matches!(
        cad_representation::layout::viewport_transform(&pv),
        cad_representation::layout::ViewportState::Unsupported(_)
    ));
}

#[test]
fn circle_clip_boundary_is_refused_because_chording_would_invent_geometry() {
    let mut circle = acadrust::entities::Circle::new();
    circle.center = acadrust::types::Vector3::new(50.0, 25.0, 0.0);
    circle.radius = 25.0;
    let mut viewport = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    circle.common.handle = acadrust::types::Handle::NULL;
    let mut doc = acadrust::CadDocument::new();
    let handle = doc
        .add_entity(EntityType::Circle(circle))
        .expect("the circle is accepted");
    viewport.clip_boundary_handle = handle;
    let pv = paper_viewport(&viewport, &doc).expect("still a content viewport");
    assert_eq!(refusal_code(&pv), viewport_clip_reason::UNSUPPORTED_ENTITY);
    // The message names the class and says why it is not approximated.
    let Completeness::Partial(reasons) = &pv.completeness else {
        panic!("expected Partial, got {:?}", pv.completeness);
    };
    assert!(reasons[0].contains("AcDbCircle"), "{reasons:?}");
    assert!(reasons[0].contains("chorded"), "{reasons:?}");
    assert_eq!(pv.clip, rectangle(0.0, 0.0, 100.0, 50.0));
}

#[test]
fn self_intersecting_clip_boundary_is_refused_by_name() {
    // A bow tie that uses the four corners of its envelope: it must not be
    // squared off into a rectangle and must not be stored as a polygon.
    let doc = document_with_viewport(
        Some(EntityType::LwPolyline(closed_polyline(&[
            (0.0, 0.0),
            (100.0, 50.0),
            (100.0, 0.0),
            (0.0, 50.0),
        ]))),
        top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0)),
    );
    let pv = paper_viewport(
        match doc
            .entities()
            .find(|e| matches!(e, EntityType::Viewport(_)))
        {
            Some(EntityType::Viewport(viewport)) => viewport,
            other => panic!("the document holds one viewport, got {other:?}"),
        },
        &doc,
    )
    .expect("still a content viewport");
    assert_eq!(refusal_code(&pv), viewport_clip_reason::SELF_INTERSECTING);
    assert_eq!(
        pv.clip,
        rectangle(0.0, 0.0, 100.0, 50.0),
        "the refused viewport keeps its own rectangle as data, never a guess"
    );
    assert!(
        matches!(
            cad_representation::layout::viewport_transform(&pv),
            cad_representation::layout::ViewportState::Unsupported(_)
        ),
        "a refused boundary is never drawn as a rectangle"
    );
}

#[test]
fn arc_clip_boundary_is_refused_by_name() {
    let mut polyline = closed_polyline(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)]);
    polyline.vertices[0].bulge = 0.5;
    let doc = document_with_viewport(
        Some(EntityType::LwPolyline(polyline)),
        top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0)),
    );
    let drawing = import_document(&doc);
    let viewport = only_viewport(&drawing);
    assert_eq!(refusal_code(viewport), viewport_clip_reason::CURVED);
    assert_eq!(viewport.clip, rectangle(0.0, 0.0, 100.0, 50.0));
}

#[test]
fn curve_fitted_clip_boundary_polyline_is_refused_by_name() {
    // A 2D POLYLINE flagged curve-fit draws a fitted curve; its vertices are
    // control points, so they are not the boundary.
    let mut polyline = acadrust::entities::Polyline2D::new();
    polyline.vertices.extend(
        [(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)].map(|(x, y)| {
            acadrust::entities::Vertex2D::new(acadrust::types::Vector3::new(x, y, 0.0))
        }),
    );
    polyline.flags =
        acadrust::entities::PolylineFlags::CLOSED | acadrust::entities::PolylineFlags::CURVE_FIT;
    let doc = document_with_viewport(
        Some(EntityType::Polyline2D(polyline)),
        top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0)),
    );
    let drawing = import_document(&doc);
    assert_eq!(
        refusal_code(only_viewport(&drawing)),
        viewport_clip_reason::CURVED
    );
}

#[test]
fn heavy_polyline_clip_boundary_imports_as_a_polygon() {
    // R2000-era writers store the boundary as a 2D POLYLINE (OCS vertices, no
    // bulges); it carries the same polygon as the LWPOLYLINE form.
    let mut polyline = acadrust::entities::Polyline2D::new();
    polyline.vertices.extend(
        [
            (0.0, 0.0),
            (100.0, 0.0),
            (100.0, 25.0),
            (50.0, 60.0),
            (0.0, 50.0),
        ]
        .map(|(x, y)| acadrust::entities::Vertex2D::new(acadrust::types::Vector3::new(x, y, 0.0))),
    );
    polyline.flags = acadrust::entities::PolylineFlags::CLOSED;
    let doc = document_with_viewport(
        Some(EntityType::Polyline2D(polyline)),
        top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0)),
    );
    let drawing = import_document(&doc);
    let viewport = only_viewport(&drawing);
    assert_eq!(viewport.completeness, Completeness::Complete);
    assert_eq!(viewport.clip, l_shape());
}

#[test]
fn open_clip_boundary_polyline_is_refused_by_name() {
    // A viewport clip must be a closed region; an open polyline is not one.
    let mut polyline = closed_polyline(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)]);
    polyline.is_closed = false;
    let doc = document_with_viewport(
        Some(EntityType::LwPolyline(polyline)),
        top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0)),
    );
    let drawing = import_document(&doc);
    assert_eq!(
        refusal_code(only_viewport(&drawing)),
        viewport_clip_reason::NOT_CLOSED
    );
}

#[test]
fn degenerate_clip_boundary_is_refused_by_name() {
    // Two distinct vertices and a repeated close: no polygon exists.
    let doc = document_with_viewport(
        Some(EntityType::LwPolyline(closed_polyline(&[
            (0.0, 0.0),
            (100.0, 0.0),
            (0.0, 0.0),
        ]))),
        top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0)),
    );
    let drawing = import_document(&doc);
    assert_eq!(
        refusal_code(only_viewport(&drawing)),
        viewport_clip_reason::DEGENERATE
    );
}

#[test]
fn tilted_clip_boundary_plane_is_refused_by_name() {
    // A tilted OCS is not the paper plane; its projection is not the stored
    // polygon, so it must not be accepted as one.
    let mut polyline = closed_polyline(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)]);
    polyline.normal = acadrust::types::Vector3::new(0.0, 0.5, 0.866_025_403_784_438_6);
    let doc = document_with_viewport(
        Some(EntityType::LwPolyline(polyline)),
        top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0)),
    );
    let drawing = import_document(&doc);
    assert_eq!(
        refusal_code(only_viewport(&drawing)),
        viewport_clip_reason::NON_PLANAR
    );
}

#[test]
fn the_import_report_names_the_clip_refusal_and_the_drawing_is_partial() {
    // A refusal must be visible in the report with its own code, and the drawing
    // must not claim `Complete`.
    let mut polyline = closed_polyline(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)]);
    polyline.vertices[0].bulge = 0.5;
    let doc = document_with_viewport(
        Some(EntityType::LwPolyline(polyline)),
        top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0)),
    );
    let drawing = import_document(&doc);
    let diagnostic = drawing
        .report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == viewport_clip_reason::CURVED)
        .unwrap_or_else(|| {
            panic!(
                "the report names the refusal: {:?}",
                drawing
                    .report
                    .diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.code.as_str())
                    .collect::<Vec<_>>()
            )
        });
    assert!(diagnostic.message.contains("viewport 2"), "{diagnostic:?}");
    assert!(
        !matches!(drawing.report.completeness, Completeness::Complete),
        "a refused boundary is a declared loss"
    );
    // The AcDbViewport capability row is Partial too: its frame is not the
    // file's clipped frame.
    let capability = drawing
        .report
        .capabilities
        .iter()
        .find(|capability| capability.type_key == "AcDbViewport")
        .expect("the viewport capability row exists");
    assert_ne!(
        capability.semantic,
        SupportStatus::Verified,
        "{capability:?}"
    );
}

#[test]
fn the_boundary_handle_survives_a_real_dxf_write_and_reimport() {
    // acadrust writes the boundary handle as DXF group 340 and reads it back;
    // this pins that the polygon reaches the database through real bytes, not
    // only through an in-memory document.
    let doc = document_with_viewport(
        Some(EntityType::LwPolyline(closed_polyline(&[
            (0.0, 0.0),
            (100.0, 0.0),
            (100.0, 25.0),
            (50.0, 60.0),
            (0.0, 50.0),
        ]))),
        top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0)),
    );
    let bytes = acadrust::DxfWriter::new(&doc)
        .write_to_vec()
        .expect("the source document writes as DXF text");
    assert!(
        String::from_utf8_lossy(&bytes).contains("VIEWPORT"),
        "the source bytes carry the viewport"
    );
    let drawing = AcadrustImporter::new()
        .import(&request(bytes), &|| false)
        .expect("the written bytes import");
    let viewport = only_viewport(&drawing);
    assert_eq!(viewport.clip, l_shape());
    assert_eq!(viewport.completeness, Completeness::Complete);
}

#[test]
fn the_exporter_writes_no_viewport_so_a_polygon_clip_does_not_survive_save_as() {
    // `export` is a v1 lossy Save As of **model space only**: it writes no
    // LAYOUT and no VIEWPORT record, so a paper-space clip boundary cannot
    // round-trip through it (and this test documents that instead of pretending
    // otherwise). The imported polygon itself is checked above.
    let doc = document_with_viewport(
        Some(EntityType::LwPolyline(closed_polyline(&[
            (0.0, 0.0),
            (100.0, 0.0),
            (100.0, 25.0),
            (50.0, 60.0),
            (0.0, 50.0),
        ]))),
        top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0)),
    );
    let imported = import_document(&doc);
    assert_eq!(only_viewport(&imported).clip, l_shape());

    let (bytes, _report) = crate::export::export(&crate::export::ExportRequest {
        database: &imported.database,
        document_id: DocumentId(1),
        space: crate::export::ExportSpace::Model,
        format: crate::export::ExportFormat::DxfText,
        acis_version: AcisVersion::default(),
        dxf_target: DxfTarget::default(),
        limits: crate::export::ExportLimits::default(),
        units: None,
    })
    .expect("the imported database exports");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        !text.contains("VIEWPORT"),
        "v1 exports model space only, so a paper viewport is not written"
    );

    let reimported = AcadrustImporter::new()
        .import(&request(bytes), &|| false)
        .expect("the exported bytes import");
    assert!(
        reimported
            .database
            .layouts()
            .all(|layout| layout.viewports.is_empty()),
        "no viewport (and therefore no clip) survives a Save As"
    );
}
