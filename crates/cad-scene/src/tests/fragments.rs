//! Unsupported and conic fragment batching.

use super::*;
use cad_representation::ConicSegment;

#[test]
fn text_and_instance_fragments_still_produce_no_batches() {
    let mut cache = SceneCache::default();
    let mut rep = line_representation(1, Vec::new());
    rep.fragments[0].primitive = DisplayPrimitive::Text {
        text: "A".into(),
        origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        font: ResourceKey("style:Standard".into()),
        height: 2.5,
    };
    let delta = cache.build(&rep, stamp()).unwrap();
    assert!(delta.added.is_empty());
    assert!(delta.images.is_empty());

    rep.fragments[0].primitive = DisplayPrimitive::Instance {
        block: BlockId(1),
        transform: Transform3::identity(),
    };
    let delta = cache.build(&rep, stamp()).unwrap();
    assert!(delta.added.is_empty());
    assert!(delta.images.is_empty());
}

/// An **ineligible** conic fragment (unequal semi-axes: an ellipse, slice 2)
/// must still batch byte-for-byte like the equivalent chord polyline. Slice B
/// only routes a slice-1 circle/arc to the SDF path.

#[test]
fn ineligible_conic_fragment_batches_like_its_chord_polyline() {
    let conic = ConicSegment {
        center: Point3 {
            x: 100.0,
            y: -50.0,
            z: 0.0,
        },
        major: Point3 {
            x: 8.0,
            y: 0.0,
            z: 0.0,
        },
        minor: Point3 {
            x: 0.0,
            y: 4.0,
            z: 0.0,
        },
        start: 0.0,
        sweep: std::f64::consts::TAU,
        full: true,
    };
    assert!(
        !conic.is_sdf_circle_arc(),
        "unequal semi-axes are slice 2 and must stay chords"
    );
    let chord = conic.chord_points();
    assert!(chord.len() >= 3);

    let mut conic_rep = line_representation(7, Vec::new());
    conic_rep.fragments[0].primitive = DisplayPrimitive::Conic(Arc::new(conic));
    let lines_rep = line_representation(7, chord);

    for compact in [false, true] {
        let mut conic_cache = SceneCache::default();
        let mut lines_cache = SceneCache::default();
        let conic_delta = if compact {
            conic_cache.build_compact(&conic_rep, stamp()).unwrap()
        } else {
            conic_cache.build(&conic_rep, stamp()).unwrap()
        };
        let lines_delta = if compact {
            lines_cache.build_compact(&lines_rep, stamp()).unwrap()
        } else {
            lines_cache.build(&lines_rep, stamp()).unwrap()
        };
        assert_eq!(conic_delta.added.len(), 1, "compact={compact}");
        assert_eq!(lines_delta.added.len(), 1, "compact={compact}");
        let conic_batch = &conic_delta.added[0];
        let lines_batch = &lines_delta.added[0];
        assert_eq!(
            conic_batch.topology, lines_batch.topology,
            "compact={compact}"
        );
        assert_eq!(
            conic_batch.local_origin, lines_batch.local_origin,
            "compact={compact}"
        );
        assert_eq!(
            conic_batch.vertices, lines_batch.vertices,
            "compact={compact}"
        );
    }
}

/// A slice-1 **circle** fragment becomes a `Conics` batch carrying one f64
/// `RenderConic`, and `build_compact` merges adjacent same-style circles into
/// one batch. It never produces chord vertices.

#[test]
fn eligible_circle_fragment_becomes_a_conic_batch_and_merges() {
    let circle = |cx: f64| ConicSegment {
        center: Point3 {
            x: cx,
            y: 0.0,
            z: 0.0,
        },
        major: Point3 {
            x: 5.0,
            y: 0.0,
            z: 0.0,
        },
        minor: Point3 {
            x: 0.0,
            y: 5.0,
            z: 0.0,
        },
        start: 0.0,
        sweep: std::f64::consts::TAU,
        full: true,
    };
    let mut rep = line_representation(7, Vec::new());
    rep.fragments[0].primitive = DisplayPrimitive::Conic(Arc::new(circle(0.0)));
    let mut second = line_representation(8, Vec::new());
    second.fragments[0].primitive = DisplayPrimitive::Conic(Arc::new(circle(20.0)));
    rep.fragments
        .push(second.fragments.pop().expect("one fragment"));

    let mut cache = SceneCache::default();
    let delta = cache.build_compact(&rep, stamp()).unwrap();
    assert_eq!(delta.added.len(), 1, "adjacent same-style circles merge");
    let batch = &delta.added[0];
    assert_eq!(batch.topology, RenderTopology::Conics);
    assert_eq!(batch.conics.len(), 2);
    assert!(
        batch.vertices.is_empty(),
        "a conic batch has no chord vertices"
    );
    assert!(batch.conics[0].full);
    assert_eq!(batch.local_origin.x, 0.0);
}

/// A WIPEOUT mask is carried as its own draw item with the **world** boundary
/// polygon, not as a geometry batch (ADR 0022): the frame plan must never order
/// it with the geometry, and `cad-scene` must not fan-triangulate a concave
/// boundary here.
#[test]
fn mask_fragment_becomes_a_mask_draw_item_with_its_polygon() {
    let polygon: Vec<Point3> = vec![
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 10.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 10.0,
            y: 4.0,
            z: 0.0,
        },
        Point3 {
            x: 4.0,
            y: 4.0,
            z: 0.0,
        },
        Point3 {
            x: 4.0,
            y: 8.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 8.0,
            z: 0.0,
        },
    ];
    let mut rep = line_representation(5, Vec::new());
    rep.fragments[0].primitive = DisplayPrimitive::Mask {
        boundary: polygon.clone().into(),
        inverted: false,
    };

    let mut cache = SceneCache::default();
    let delta = cache.build(&rep, stamp()).unwrap();
    assert!(delta.added.is_empty(), "a mask is not a geometry batch");
    assert!(delta.images.is_empty());
    assert_eq!(delta.masks.len(), 1);
    assert_eq!(
        delta.masks[0].polygon.as_ref(),
        polygon.as_slice(),
        "the world boundary polygon is preserved verbatim"
    );
    assert_eq!(delta.masks[0].draw_order, SCENE_DRAW_ORDER);
    assert_eq!(delta.masks[0].sources.len(), 1);
    assert!(
        !delta.masks[0].inverted,
        "clip mode Outside stays non-inverted"
    );

    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.mask_count(), 1);
    assert_eq!(cache.mask_batches()[0].polygon.len(), polygon.len());
}

#[test]
fn mask_without_three_vertices_is_an_explicit_error() {
    let mut rep = line_representation(6, Vec::new());
    rep.fragments[0].primitive = DisplayPrimitive::Mask {
        boundary: vec![
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
        ]
        .into(),
        inverted: false,
    };
    let mut cache = SceneCache::default();
    assert!(cache.build(&rep, stamp()).is_err());
}

/// A change to the mask's entity invalidates the cached mask, exactly like an
/// image or a chunk.
#[test]
fn mask_is_invalidated_by_its_entity_change() {
    let mut rep = line_representation(9, Vec::new());
    rep.fragments[0].primitive = DisplayPrimitive::Mask {
        boundary: vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 2.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 2.0,
                y: 2.0,
                z: 0.0,
            },
        ]
        .into(),
        inverted: true,
    };
    let mut cache = SceneCache::default();
    let delta = cache.build(&rep, stamp()).unwrap();
    assert!(
        delta.masks[0].inverted,
        "clip mode Inside must reach the renderer through the mask batch"
    );
    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.mask_count(), 1);
    assert!(cache.mask_batches()[0].inverted);
    let changes = ChangeSet {
        database: DatabaseId(1),
        before: Revision(0),
        after: Revision(1),
        transaction: TransactionId(1),
        reason: "mask edit".into(),
        changes: vec![ObjectChange::Update(
            ObjectId(9),
            cad_db::ChangeMask::GEOMETRY,
        )],
    };
    cache.apply_changes(&changes).unwrap();
    assert_eq!(cache.mask_count(), 0);
}
