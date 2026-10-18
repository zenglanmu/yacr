use super::*;
use cad_db::DrawingDatabaseBuilder;
use cad_domain::{Completeness, DatabaseId, LayoutId, Point3, Transform3};

// ---- scale direction (B22) ---------------------------------------------

#[test]
fn paper_per_model_is_paper_height_over_view_height() {
    // 1:100: a 1000-unit-tall model region fits a 10-unit-tall paper window.
    assert_eq!(paper_per_model_from_view(1000.0, 10.0), Some(0.01));
    // The stored legacy scalar is the reciprocal.
    assert_eq!(
        paper_per_model_from_view(1000.0, 10.0).map(|s| 1.0 / s),
        Some(100.0)
    );
    assert_eq!(paper_per_model_from_view(0.0, 10.0), None);
    assert_eq!(paper_per_model_from_view(1000.0, f64::NAN), None);
}

// ---- supported forms ---------------------------------------------------

#[test]
fn four_corner_clip_yields_the_four_paper_corners() {
    let ViewportState::Supported(t) = viewport_transform(&four_corner_viewport()) else {
        panic!("expected a supported viewport");
    };
    assert_eq!(
        t.paper_corners,
        [p(0.0, 0.0), p(100.0, 0.0), p(100.0, 50.0), p(0.0, 50.0)]
    );
    assert_eq!(t.paper_center, [50.0, 25.0]);
    assert_eq!(t.paper_half, [50.0, 25.0]);
    assert_eq!(t.paper_bounds(), ([0.0, 0.0], [100.0, 50.0]));
    // 1:100 → paper 0.01 per model unit.
    assert!((t.paper_per_model - 0.01).abs() < 1e-12);
    // The view centre is the model point shown at the rectangle centre.
    assert_eq!(t.model_anchor, p(10.0, 20.0));
    assert_eq!(t.model_to_paper(p(10.0, 20.0)), p(50.0, 25.0));
}

#[test]
fn known_one_to_one_hundred_viewport_distances() {
    let ViewportState::Supported(t) = viewport_transform(&four_corner_viewport()) else {
        panic!("expected a supported viewport");
    };
    // Paper width 100 ↔ model width 10000; paper height 50 ↔ model 5000.
    let ll = t.paper_to_model(p(0.0, 0.0));
    let lr = t.paper_to_model(p(100.0, 0.0));
    let ul = t.paper_to_model(p(0.0, 50.0));
    assert!((ll.x - lr.x).abs() - 10000.0 < 1e-6, "got {ll:?} {lr:?}");
    assert!((ll.y - ul.y).abs() - 5000.0 < 1e-6, "got {ll:?} {ul:?}");
    // The four paper corners round-trip through the inverse exactly.
    for corner in t.paper_corners {
        let model = t.paper_to_model(corner);
        let back = t.model_to_paper(model);
        assert!((back.x - corner.x).abs() < 1e-9);
        assert!((back.y - corner.y).abs() < 1e-9);
    }
    // One paper unit is exactly 100 model units.
    let a = t.paper_to_model(p(0.0, 0.0));
    let b = t.paper_to_model(p(1.0, 0.0));
    assert!((b.x - a.x - 100.0).abs() < 1e-9);
}

#[test]
fn legacy_three_point_clip_still_derives_the_rectangle() {
    let ViewportState::Supported(t) = viewport_transform(&simple_viewport()) else {
        panic!("expected a supported viewport");
    };
    assert_eq!(t.paper_center, [50.0, 25.0]);
    assert_eq!(t.paper_half, [50.0, 25.0]);
    assert_eq!(
        t.paper_corners,
        [p(0.0, 0.0), p(100.0, 0.0), p(100.0, 50.0), p(0.0, 50.0)]
    );
    assert_eq!(t.model_anchor, p(10.0, 20.0));
    // The stored 100 is model-units-per-paper-unit; the mapping uses the
    // inverse (audit B22 "方向需纠正").
    assert_eq!(t.paper_per_model, 0.01);
}

#[test]
fn imported_one_to_one_hundred_viewport_is_supported_and_correct() {
    let ViewportState::Supported(t) = viewport_transform(&imported_one_to_one_hundred()) else {
        panic!("the importer's four-corner 1:100 viewport must be supported");
    };
    // Four paper corners and the correct 1:100 scale direction.
    assert_eq!(
        t.paper_corners,
        [p(0.0, 0.0), p(100.0, 0.0), p(100.0, 50.0), p(0.0, 50.0)]
    );
    assert!((t.paper_per_model - 0.01).abs() < 1e-12);
    // The view target lands at the paper centre.
    assert_eq!(t.model_anchor, p(10.0, 20.0));
    assert_eq!(t.model_to_paper(p(10.0, 20.0)), p(50.0, 25.0));
    // One paper unit is 100 model units through the exact inverse.
    let a = t.paper_to_model(p(0.0, 0.0));
    let b = t.paper_to_model(p(1.0, 0.0));
    assert!((b.x - a.x - 100.0).abs() < 1e-9);
}

#[test]
fn imported_rotated_viewport_is_partial_with_the_twist_code() {
    // The importer writes a Partial record (with the same clip/transform)
    // for a twisted view; the representation layer must surface the stable
    // twist code, not square the rectangle off.
    let mut vp = imported_one_to_one_hundred();
    vp.completeness = Completeness::Partial(vec!["viewport has a view twist".into()]);
    assert_eq!(unsupported_code(&vp), viewport_reason::TWISTED_TRANSFORM);
}

#[test]
fn imported_off_plane_viewport_is_partial_with_the_off_plane_code() {
    let mut vp = imported_one_to_one_hundred();
    vp.completeness = Completeness::Partial(vec![
        "viewport view direction is not perpendicular to the paper plane".into(),
    ]);
    assert_eq!(unsupported_code(&vp), viewport_reason::OFF_PLANE_VIEW);
}

#[test]
fn imported_perspective_viewport_is_partial_with_the_perspective_code() {
    let mut vp = imported_one_to_one_hundred();
    vp.completeness = Completeness::Partial(vec!["perspective viewport is not supported".into()]);
    assert_eq!(unsupported_code(&vp), viewport_reason::PERSPECTIVE);
}

#[test]
fn imported_complex_clip_is_partial_with_the_complex_clip_code() {
    let mut vp = imported_one_to_one_hundred();
    vp.completeness =
        Completeness::Partial(vec!["non-rectangular viewport clip is not applied".into()]);
    assert_eq!(unsupported_code(&vp), viewport_reason::COMPLEX_CLIP);
}

#[test]
fn model_to_paper_places_the_anchor_at_the_centre() {
    let ViewportState::Supported(t) = viewport_transform(&simple_viewport()) else {
        panic!("expected a supported viewport");
    };
    assert_eq!(t.model_to_paper(p(10.0, 20.0)), p(50.0, 25.0));
    // One paper unit right of centre is model x = 110.
    let right = t.model_to_paper(p(110.0, 20.0));
    assert!((right.x - 51.0).abs() < 1e-9);
    assert!((right.y - 25.0).abs() < 1e-9);
}

#[test]
fn paper_to_model_is_the_exact_inverse() {
    let ViewportState::Supported(t) = viewport_transform(&simple_viewport()) else {
        panic!("expected a supported viewport");
    };
    let model = p(123.5, -77.25);
    let paper = t.model_to_paper(model);
    let back = t.paper_to_model(paper);
    assert!((back.x - model.x).abs() < 1e-9);
    assert!((back.y - model.y).abs() < 1e-9);
}

#[test]
fn complex_clip_is_refused_with_its_reason() {
    let mut vp = simple_viewport();
    vp.completeness = Completeness::Partial(vec!["complex viewport clip".into()]);
    let code = unsupported_code(&vp);
    assert_eq!(code, viewport_reason::COMPLEX_CLIP);
    let ViewportState::Unsupported(why) = viewport_transform(&vp) else {
        unreachable!()
    };
    assert!(why.reason().contains("complex viewport clip"));
}

#[test]
fn rotated_four_corner_clip_is_refused() {
    // A 45°-rotated rectangle: all four corners have x strictly inside the
    // bounding box, so it cannot be represented as an axis-aligned rect.
    let mut vp = four_corner_viewport();
    vp.clip = vec![p(50.0, 0.0), p(100.0, 50.0), p(50.0, 100.0), p(0.0, 50.0)];
    assert_eq!(unsupported_code(&vp), viewport_reason::ROTATED_CLIP);
}

#[test]
fn twisted_view_transform_is_refused() {
    let mut vp = four_corner_viewport();
    let c = std::f64::consts::FRAC_1_SQRT_2;
    vp.model_to_paper = Transform3 {
        matrix: [
            [100.0 * c, -100.0 * c, 0.0, 0.0],
            [100.0 * c, 100.0 * c, 0.0, 0.0],
            [0.0, 0.0, 100.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    assert_eq!(unsupported_code(&vp), viewport_reason::TWISTED_TRANSFORM);
}

#[test]
fn mirrored_view_transform_is_refused() {
    let mut vp = four_corner_viewport();
    vp.model_to_paper = stored_view(-100.0, Point3::default());
    assert_eq!(unsupported_code(&vp), viewport_reason::MIRROR);
}

#[test]
fn non_uniform_view_transform_is_refused() {
    let mut vp = four_corner_viewport();
    let mut m = Transform3::scale(100.0).matrix;
    m[1][1] = 200.0;
    vp.model_to_paper = Transform3 { matrix: m };
    assert_eq!(unsupported_code(&vp), viewport_reason::NON_UNIFORM);
}

#[test]
fn off_plane_view_direction_is_refused() {
    // A shear of model z into paper x: a tilted 3D view we cannot flatten.
    let mut vp = four_corner_viewport();
    let mut m = Transform3::scale(100.0).matrix;
    m[0][2] = 5.0;
    vp.model_to_paper = Transform3 { matrix: m };
    assert_eq!(unsupported_code(&vp), viewport_reason::OFF_PLANE_VIEW);
}

#[test]
fn legacy_clip_with_transform_translation_is_ambiguous_and_refused() {
    let mut vp = simple_viewport();
    vp.model_to_paper = stored_view(100.0, p(1.0, 2.0));
    assert_eq!(unsupported_code(&vp), viewport_reason::TRANSLATION_MISMATCH);
}

#[test]
fn missing_scale_is_refused() {
    let vp = viewport(vec![p(0.0, 0.0), p(10.0, 10.0), p(0.0, 0.0)], 0.0);
    assert_eq!(unsupported_code(&vp), viewport_reason::SCALE);
}

#[test]
fn adjacent_clip_corners_are_refused_as_not_opposite() {
    // The importer currently stores the two bottom corners (same y); the
    // paper height is not stored, so this must be refused, not guessed.
    let vp = viewport(vec![p(0.0, 0.0), p(10.0, 0.0), p(0.0, 0.0)], 100.0);
    assert_eq!(unsupported_code(&vp), viewport_reason::ADJACENT_CORNERS);
}

#[test]
fn zero_size_rectangle_is_refused() {
    let vp = viewport(vec![p(0.0, 0.0), p(0.0, 0.0), p(0.0, 0.0)], 1.0);
    assert_eq!(unsupported_code(&vp), viewport_reason::DEGENERATE_RECT);
}

#[test]
fn non_finite_clip_is_refused() {
    let vp = viewport(vec![p(0.0, 0.0), p(f64::NAN, 10.0), p(0.0, 0.0)], 100.0);
    assert_eq!(unsupported_code(&vp), viewport_reason::NON_FINITE);
}

#[test]
fn wrong_arity_clip_is_refused() {
    let vp = viewport(vec![p(0.0, 0.0), p(10.0, 10.0)], 1.0);
    assert_eq!(unsupported_code(&vp), viewport_reason::CLIP_ARITY);
}

// ---- rectangular clipping ----------------------------------------------

#[test]
fn clipping_keeps_the_inside_part_of_a_crossing_segment() {
    // Rectangle [-1,1]²; a line from (-5,0) to (5,0) survives as (-1,0)-(1,0).
    let runs = clip_polyline_to_rect(&[p(-5.0, 0.0), p(5.0, 0.0)], [0.0, 0.0], [1.0, 1.0]);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].len(), 2);
    assert!((runs[0][0].x + 1.0).abs() < 1e-9);
    assert!((runs[0][1].x - 1.0).abs() < 1e-9);
}

#[test]
fn clipping_drops_a_fully_outside_polyline() {
    let runs = clip_polyline_to_rect(
        &[p(2.0, 5.0), p(3.0, 6.0), p(4.0, 5.0)],
        [0.0, 0.0],
        [1.0, 1.0],
    );
    assert!(runs.is_empty());
}

#[test]
fn clipping_keeps_a_fully_inside_polyline() {
    let inside = [p(-0.5, -0.5), p(0.0, 0.0), p(0.5, 0.5)];
    let runs = clip_polyline_to_rect(&inside, [0.0, 0.0], [1.0, 1.0]);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0], inside.to_vec());
}

#[test]
fn clipping_joins_continuous_segments_through_the_middle() {
    let runs = clip_polyline_to_rect(
        &[p(-3.0, 0.0), p(0.0, 0.0), p(3.0, 0.0)],
        [0.0, 0.0],
        [1.0, 1.0],
    );
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].len(), 3);
    assert!((runs[0][0].x + 1.0).abs() < 1e-9);
    assert!((runs[0][2].x - 1.0).abs() < 1e-9);
}

#[test]
fn clipping_splits_a_polyline_that_leaves_and_re_enters() {
    let runs = clip_polyline_to_rect(
        &[
            p(0.0, 0.0),
            p(0.5, 0.0),
            p(0.5, 5.0),
            p(-0.5, 5.0),
            p(-0.5, 0.0),
            p(0.0, 0.0),
        ],
        [0.0, 0.0],
        [1.0, 1.0],
    );
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().all(|run| run.len() >= 2));
}

// ---- enumeration -------------------------------------------------------
#[test]
fn enumeration_reports_ids_names_and_support() {
    let db = db_with_layouts();
    let layouts = enumerate_layouts(&db);
    assert_eq!(layouts.len(), 2);
    assert_eq!(layouts[0].id, LayoutId(1));
    assert_eq!(layouts[0].name, "Layout1");
    assert!(layouts[0].supported);
    assert_eq!(layouts[0].viewport_count, 1);
    assert!(layouts[0].reason.is_empty());

    assert_eq!(layouts[1].id, LayoutId(2));
    assert!(!layouts[1].supported);
    // The reason is machine-readable: it starts with the stable code.
    assert!(layouts[1].reason.starts_with(viewport_reason::SCALE));
}

#[test]
fn empty_database_has_no_layouts() {
    let db = DrawingDatabaseBuilder::new(DatabaseId(1)).finish().unwrap();
    assert!(enumerate_layouts(&db).is_empty());
}

#[test]
fn space_selection_round_trips() {
    assert!(SpaceSelection::Model.is_model());
    assert_eq!(SpaceSelection::Model.layout(), None);
    let paper = SpaceSelection::Paper(LayoutId(3));
    assert!(!paper.is_model());
    assert_eq!(paper.layout(), Some(LayoutId(3)));
}
