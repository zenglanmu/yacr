//! Viewport content transforms and clipping semantics.

use super::*;

/// Build a top/plan VIEWPORT entity with the given paper rectangle, scale,
/// and model view target.
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

/// A closed LWPOLYLINE clip boundary plus a document that contains it.
fn document_with_polyline(
    polyline: acadrust::entities::LwPolyline,
) -> (acadrust::CadDocument, acadrust::types::Handle) {
    let mut doc = acadrust::CadDocument::new();
    let handle = doc
        .add_entity(acadrust::EntityType::LwPolyline(polyline))
        .expect("clip boundary entity is accepted");
    (doc, handle)
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

#[test]
fn dangling_clip_boundary_handle_is_partial() {
    // The handle references no entity: the clip cannot be proven rectangular,
    // so the honest status is Partial (never a squared-off guess).
    let vp = viewport_with_dangling_clip();
    let doc = acadrust::CadDocument::new();
    let pv = paper_viewport(&vp, &doc).unwrap();
    match &pv.completeness {
        Completeness::Partial(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("clip")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}

#[test]
fn rectangular_clip_boundary_equals_the_viewport_rectangle() {
    // VPCLIP stores a boundary entity even when it matches the viewport
    // rectangle exactly; that must stay Complete and use the boundary corners.
    let (doc, handle) = document_with_polyline(closed_polyline(&[
        (0.0, 0.0),
        (100.0, 0.0),
        (100.0, 50.0),
        (0.0, 50.0),
    ]));
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.clip_boundary_handle = handle;
    let pv = paper_viewport(&vp, &doc).expect("still a content viewport");
    assert_eq!(pv.completeness, Completeness::Complete);
    assert_eq!(pv.clip, rectangle(0.0, 0.0, 100.0, 50.0));
}

#[test]
fn offset_rectangular_clip_boundary_uses_the_boundary_rectangle() {
    // The real VPCLIP semantics: the boundary is what is visible, and it may
    // be smaller than (and offset from) the viewport rectangle. The transform
    // still anchors on the viewport centre, not the clip centre.
    let (doc, handle) = document_with_polyline(closed_polyline(&[
        (20.0, 10.0),
        (70.0, 10.0),
        (70.0, 40.0),
        (20.0, 40.0),
    ]));
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.clip_boundary_handle = handle;
    let pv = paper_viewport(&vp, &doc).expect("still a content viewport");
    assert_eq!(pv.completeness, Completeness::Complete);
    assert_eq!(pv.clip, rectangle(20.0, 10.0, 70.0, 40.0));
    // The viewport centre still lands on the view target at 1:100.
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
fn duplicate_clip_boundary_vertices_are_collapsed() {
    // Real files store consecutive duplicate vertices plus a closing duplicate
    // (7 stored vertices for 4 corners); that is still a rectangle.
    let (doc, handle) = document_with_polyline(closed_polyline(&[
        (10.0, 5.0),
        (10.0, 5.0),
        (90.0, 5.0),
        (90.0, 45.0),
        (90.0, 45.0),
        (10.0, 45.0),
        (10.0, 5.0),
    ]));
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.clip_boundary_handle = handle;
    let pv = paper_viewport(&vp, &doc).expect("still a content viewport");
    assert_eq!(pv.completeness, Completeness::Complete);
    assert_eq!(pv.clip, rectangle(10.0, 5.0, 90.0, 45.0));
}

#[test]
fn five_corner_clip_boundary_is_partial() {
    let (doc, handle) = document_with_polyline(closed_polyline(&[
        (0.0, 0.0),
        (100.0, 0.0),
        (100.0, 50.0),
        (50.0, 60.0),
        (0.0, 50.0),
    ]));
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.clip_boundary_handle = handle;
    let pv = paper_viewport(&vp, &doc).unwrap();
    match &pv.completeness {
        Completeness::Partial(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("clip")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}

#[test]
fn curved_clip_boundary_is_partial() {
    // A nonzero bulge makes at least one edge an arc, so the boundary is not a
    // straight-edged rectangle even though its four vertices line up.
    let mut polyline = closed_polyline(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)]);
    polyline.vertices[0].bulge = 0.5;
    let (doc, handle) = document_with_polyline(polyline);
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.clip_boundary_handle = handle;
    let pv = paper_viewport(&vp, &doc).unwrap();
    match &pv.completeness {
        Completeness::Partial(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("clip")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}

#[test]
fn open_clip_boundary_polyline_is_partial() {
    // A viewport clip must be a closed region; an open polyline is not one.
    let mut polyline = closed_polyline(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)]);
    polyline.is_closed = false;
    let (doc, handle) = document_with_polyline(polyline);
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.clip_boundary_handle = handle;
    let pv = paper_viewport(&vp, &doc).unwrap();
    match &pv.completeness {
        Completeness::Partial(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("clip")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}

#[test]
fn tilted_clip_boundary_plane_is_partial() {
    // A tilted OCS is not the paper plane; its projection is not provably the
    // stored rectangle, so it must not be accepted as one.
    let mut polyline = closed_polyline(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)]);
    polyline.normal = acadrust::types::Vector3::new(0.0, 0.5, 0.866_025_403_784_438_6);
    let (doc, handle) = document_with_polyline(polyline);
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.clip_boundary_handle = handle;
    let pv = paper_viewport(&vp, &doc).unwrap();
    match &pv.completeness {
        Completeness::Partial(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("clip")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}
