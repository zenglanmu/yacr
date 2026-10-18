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

#[test]
fn one_to_one_hundred_viewport_has_four_corners_and_the_real_transform() {
    // 1:100: a 10000×5000 model region fits the 100×50 paper window, view
    // centre (10, 20).
    let vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    let pv = paper_viewport(&vp).expect("id 2 is a content viewport");
    assert_eq!(pv.completeness, Completeness::Complete);
    assert_eq!(
        pv.clip,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0
            },
            Point3 {
                x: 100.0,
                y: 0.0,
                z: 0.0
            },
            Point3 {
                x: 100.0,
                y: 50.0,
                z: 0.0
            },
            Point3 {
                x: 0.0,
                y: 50.0,
                z: 0.0
            },
        ]
    );
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
    let mut sheet = top_viewport((0.0, 0.0), 100.0, 100.0, 100.0, (0.0, 0.0));
    sheet.id = 1;
    assert!(paper_viewport(&sheet).is_none(), "id 1 is the sheet");
    let mut off = top_viewport((0.0, 0.0), 100.0, 100.0, 100.0, (0.0, 0.0));
    off.status.is_on = false;
    assert!(
        paper_viewport(&off).is_none(),
        "an off viewport draws nothing"
    );
}

#[test]
fn rotated_viewport_is_partial_with_a_twist_reason() {
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.twist_angle = std::f64::consts::FRAC_PI_4;
    let pv = paper_viewport(&vp).expect("still a content viewport");
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
    let pv = paper_viewport(&vp).unwrap();
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
fn complex_clip_viewport_is_partial() {
    let mut vp = top_viewport((50.0, 25.0), 100.0, 50.0, 5000.0, (10.0, 20.0));
    vp.clip_boundary_handle = acadrust::types::Handle::new(0x1A);
    let pv = paper_viewport(&vp).unwrap();
    match &pv.completeness {
        Completeness::Partial(reasons) => {
            assert!(reasons.iter().any(|r| r.contains("clip")), "{reasons:?}");
        }
        other => panic!("expected Partial, got {other:?}"),
    }
}
