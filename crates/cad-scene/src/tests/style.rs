//! Alpha, color, lineweight, and linetype materialization.

use super::*;

fn line_representation_alpha(
    entity: u128,
    points: Vec<Point3>,
    alpha: f32,
) -> DisplayRepresentation {
    let mut rep = line_representation(entity, points);
    rep.fragments[0].alpha = alpha;
    rep
}

#[test]
fn batch_alpha_comes_from_the_fragment() {
    let mut cache = SceneCache::default();
    let rep = line_representation_alpha(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
        0.25,
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    assert_eq!(delta.added.len(), 1);
    assert_eq!(delta.added[0].alpha, 0.25);
}

#[test]
fn sanitize_alpha_matches_the_renderer_policy() {
    assert_eq!(sanitize_alpha(0.5), 0.5);
    assert_eq!(sanitize_alpha(-1.0), 0.0);
    assert_eq!(sanitize_alpha(2.0), 1.0);
    assert_eq!(sanitize_alpha(0.0), 0.0);
    // An unreadable opacity must not delete geometry.
    assert_eq!(sanitize_alpha(f32::NAN), 1.0);
}

fn line_representation_styled(
    entity: u128,
    points: Vec<Point3>,
    color: [f32; 3],
    color_unresolved: bool,
    lineweight: f32,
    lineweight_unresolved: bool,
) -> DisplayRepresentation {
    let mut rep = line_representation(entity, points);
    let fragment = &mut rep.fragments[0];
    fragment.color = color;
    fragment.color_unresolved = color_unresolved;
    fragment.lineweight = lineweight;
    fragment.lineweight_unresolved = lineweight_unresolved;
    rep
}

#[test]
fn batch_carries_the_linetype_pattern_for_diagnostics() {
    let mut cache = SceneCache::default();
    let mut rep = line_representation(1, unit_points());
    rep.fragments[0].linetype = cad_db::LinetypePattern::from_elements([0.5, -0.25]);
    rep.fragments[0].linetype_unresolved = false;
    let delta = cache.build(&rep, stamp()).unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.linetype.elements, vec![0.5, -0.25]);
    assert!(!batch.linetype_unresolved);
}

#[test]
fn batch_color_and_lineweight_come_from_the_fragment() {
    let mut cache = SceneCache::default();
    let rep = line_representation_styled(1, unit_points(), [0.25, 0.5, 0.75], false, 0.35, false);
    let delta = cache.build(&rep, stamp()).unwrap();
    assert_eq!(delta.added.len(), 1);
    let batch = &delta.added[0];
    assert_eq!(batch.color, [0.25, 0.5, 0.75]);
    assert!(!batch.color_unresolved);
    assert_eq!(batch.lineweight, 0.35);
    assert!(!batch.lineweight_unresolved);
}

#[test]
fn unresolved_color_and_lineweight_are_carried_as_explicit_defaults() {
    let mut cache = SceneCache::default();
    // ByLayer/ByBlock with no reachable value: the fragment already carries
    // the documented fallback and marks it unresolved.
    let rep = line_representation_styled(
        1,
        unit_points(),
        [1.0, 1.0, 1.0],
        true,
        cad_representation::DEFAULT_LINEWEIGHT_MM,
        true,
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.color, DEFAULT_BATCH_COLOR);
    assert!(batch.color_unresolved);
    assert_eq!(batch.lineweight, DEFAULT_BATCH_LINEWEIGHT_MM);
    assert!(batch.lineweight_unresolved);
}

#[test]
fn non_finite_color_and_lineweight_are_replaced_by_defaults() {
    // A hostile fragment must never inject NaN/inf into the GPU uniform.
    assert_eq!(
        sanitize_color([f32::NAN, f32::INFINITY, -1.0]),
        [1.0, 1.0, 0.0]
    );
    assert_eq!(
        sanitize_color([f32::NEG_INFINITY, 2.0, 0.5]),
        [1.0, 1.0, 0.5]
    );
    assert_eq!(sanitize_lineweight(f32::NAN), DEFAULT_BATCH_LINEWEIGHT_MM);
    assert_eq!(
        sanitize_lineweight(f32::NEG_INFINITY),
        DEFAULT_BATCH_LINEWEIGHT_MM
    );
    assert_eq!(sanitize_lineweight(-1.0), 0.0);

    let mut cache = SceneCache::default();
    let rep = line_representation_styled(
        1,
        unit_points(),
        [f32::NAN, f32::NAN, f32::NAN],
        false,
        f32::NAN,
        false,
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    assert_eq!(delta.added[0].color, DEFAULT_BATCH_COLOR);
    assert_eq!(delta.added[0].lineweight, DEFAULT_BATCH_LINEWEIGHT_MM);
}

#[test]
fn fully_transparent_fragment_is_carried_not_forced_opaque() {
    let mut cache = SceneCache::default();
    let rep = line_representation_alpha(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
        0.0,
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    // The batch keeps the real value; the renderer classifies it invisible.
    assert_eq!(delta.added[0].alpha, 0.0);
}

#[test]
fn out_of_range_fragment_alpha_is_clamped_for_the_batch() {
    let mut cache = SceneCache::default();
    let rep = line_representation_alpha(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
        4.0,
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    assert_eq!(delta.added[0].alpha, 1.0);
}

// ---- Dynamic-block visibility delta (spec §3.2 / incremental update) ----
