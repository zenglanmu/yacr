use super::*;
use crate::viewer_config::OverlayVisibility;
use cad_db::DrawingDatabaseBuilder;
use cad_domain::{DatabaseId, LayoutId};

/// Overlay inputs with the reference axes/grid off, so a test isolates the
/// selection/preview overlay from the world-space reference overlays.
fn overlays_without_reference() -> OverlayInputs {
    OverlayInputs {
        visibility: OverlayVisibility {
            axes: false,
            grid: false,
            ..OverlayVisibility::default()
        },
        ..OverlayInputs::default()
    }
}

fn prepare(
    controller: &mut CadSceneController,
    db: Option<&DrawingDatabase>,
    fonts: Option<Arc<FontEngine>>,
) -> CadResult<()> {
    controller.prepare(
        db,
        DocumentId(73),
        fonts,
        &LayerOverrideSet::new(),
        SpaceSelection::Model,
    )
}

#[test]
fn fonts_change_rebuilds_the_base_and_a_repeat_is_a_no_op() {
    let db = DrawingDatabaseBuilder::new(DatabaseId(9)).finish().unwrap();
    let mut controller = CadSceneController::default();
    let first_fonts = Arc::new(FontEngine::new());
    controller.fonts_changed();
    prepare(&mut controller, Some(&db), Some(first_fonts.clone())).unwrap();
    let first = controller.ready.as_ref().unwrap();
    assert_eq!(first.base.stamp.document, DocumentId(73));
    assert_eq!(first.base.stamp.object_revision, db.revision());
    let base = first.base.clone();
    let base_revision = first.base_revision;
    let revision = first.revision;
    prepare(&mut controller, Some(&db), Some(first_fonts)).unwrap();
    assert_eq!(controller.ready.as_ref().unwrap().revision, revision);
    assert!(Arc::ptr_eq(&base, &controller.ready.as_ref().unwrap().base));
    controller.fonts_changed();
    prepare(
        &mut controller,
        Some(&db),
        Some(Arc::new(FontEngine::new())),
    )
    .unwrap();
    assert!(controller.ready.as_ref().unwrap().base_revision > base_revision);
}

#[test]
fn viewport_grid_change_rebuilds_overlay_without_rebuilding_base() {
    let db = Arc::new(db_with_line());
    let layers = LayerOverrideSet::new();
    let mut overlays = OverlayInputs {
        viewport_bounds: Some((
            cad_domain::Point3 {
                x: -10.0,
                y: -5.0,
                z: 0.0,
            },
            cad_domain::Point3 {
                x: 10.0,
                y: 5.0,
                z: 0.0,
            },
        )),
        visibility: OverlayVisibility {
            grid: true,
            axes: false,
            ..OverlayVisibility::default()
        },
        ..OverlayInputs::default()
    };
    let mut controller = CadSceneController::default();
    controller
        .prepare_shared_with_overlays(
            Some(db.clone()),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &overlays,
        )
        .unwrap();
    let base = controller.ready.as_ref().unwrap().base.clone();
    let base_revision = controller.ready.as_ref().unwrap().base_revision;
    let revision = controller.ready.as_ref().unwrap().revision;
    overlays.viewport_bounds = Some((
        cad_domain::Point3 {
            x: 1000.0,
            y: -2.0,
            z: 0.0,
        },
        cad_domain::Point3 {
            x: 1400.0,
            y: 2.0,
            z: 0.0,
        },
    ));
    controller
        .prepare_shared_with_overlays(
            Some(db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &overlays,
        )
        .unwrap();
    let current = controller.ready.as_ref().unwrap();
    assert!(Arc::ptr_eq(&base, &current.base));
    assert_eq!(current.base_revision, base_revision);
    assert!(current.revision > revision);
}

#[test]
fn image_cache_is_carried_into_the_prepared_scene_and_a_repeat_is_a_no_op() {
    let db = Arc::new(db_with_line());
    let mut controller = CadSceneController::default();
    let layers = LayerOverrideSet::new();
    let overlays = overlays_without_reference();
    let cache = Arc::new(DecodedImageCache::new());
    controller
        .prepare_shared_with_overlays_and_images(
            Some(db.clone()),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &overlays,
            Some(cache.clone()),
        )
        .unwrap();
    let first = controller.ready.as_ref().unwrap();
    let carried = first.images.as_ref().expect("cache is carried");
    assert!(Arc::ptr_eq(carried, &cache));
    let base = first.base.clone();
    let base_revision = first.base_revision;
    let revision = first.revision;

    // The same cache pointer is a no-op: the base Arc is reused.
    controller
        .prepare_shared_with_overlays_and_images(
            Some(db.clone()),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &overlays,
            Some(cache.clone()),
        )
        .unwrap();
    assert_eq!(controller.ready.as_ref().unwrap().revision, revision);
    assert!(Arc::ptr_eq(&base, &controller.ready.as_ref().unwrap().base));

    // A new cache pointer rebuilds the base and carries the new cache.
    let replacement = Arc::new(DecodedImageCache::new());
    controller
        .prepare_shared_with_overlays_and_images(
            Some(db.clone()),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &overlays,
            Some(replacement.clone()),
        )
        .unwrap();
    let after = controller.ready.as_ref().unwrap();
    assert!(after.base_revision > base_revision);
    assert!(Arc::ptr_eq(after.images.as_ref().unwrap(), &replacement));

    // `None` is the previous behaviour and clears the carried cache.
    controller
        .prepare_shared_with_overlays_and_images(
            Some(db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &overlays,
            None,
        )
        .unwrap();
    assert!(controller.ready.as_ref().unwrap().images.is_none());
}

#[test]
fn failed_build_keeps_previous_version_and_close_publishes_empty_scene() {
    let db = DrawingDatabaseBuilder::new(DatabaseId(9)).finish().unwrap();
    let mut controller = CadSceneController::default();
    prepare(&mut controller, Some(&db), None).unwrap();
    let previous = controller.ready.as_ref().unwrap().revision;
    assert!(controller
        .prepare(
            Some(&db),
            DocumentId(73),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Paper(LayoutId(999)),
        )
        .is_err());
    assert_eq!(controller.ready.as_ref().unwrap().revision, previous);
    assert!(controller.diagnostic.is_some());
    prepare(&mut controller, None, None).unwrap();
    let closed = controller.ready.as_ref().unwrap();
    assert!(closed.base.added.is_empty() && closed.highlight.added.is_empty());
    assert!(closed.revision > previous);
    assert!(controller.diagnostic.is_none());
}

#[test]
fn shared_adoption_distinguishes_opens_even_with_equal_database_identity() {
    let first = Arc::new(DrawingDatabaseBuilder::new(DatabaseId(8)).finish().unwrap());
    let second = Arc::new((*first).clone());
    let mut controller = CadSceneController::default();
    let layers = LayerOverrideSet::new();
    controller
        .prepare_shared(
            Some(first.clone()),
            DocumentId(1),
            None,
            &layers,
            SpaceSelection::Model,
        )
        .unwrap();
    let revision = controller.ready.as_ref().unwrap().base_revision;
    controller
        .prepare_shared(
            Some(first),
            DocumentId(1),
            None,
            &layers,
            SpaceSelection::Model,
        )
        .unwrap();
    assert_eq!(controller.ready.as_ref().unwrap().base_revision, revision);
    controller
        .prepare_shared(
            Some(second),
            DocumentId(1),
            None,
            &layers,
            SpaceSelection::Model,
        )
        .unwrap();
    assert!(controller.ready.as_ref().unwrap().base_revision > revision);
}

fn db_with_line() -> DrawingDatabase {
    use cad_db::{DbEntity, DbObject, Layer};
    use cad_domain::{EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry, SpaceId};
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(9));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 5.0,
                y: 0.0,
                z: 0.0,
            },
        },
        draw_order: 0,
    })
    .unwrap();
    b.finish().unwrap()
}

#[test]
fn entity_selection_change_publishes_flags_without_rebuilding_the_overlay() {
    use crate::selection::{model_ref, SelectionSet};
    let db = db_with_line();
    let mut controller = CadSceneController::default();
    let layers = LayerOverrideSet::new();
    let empty = overlays_without_reference();
    controller
        .prepare_with_overlays(
            Some(&db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &empty,
        )
        .unwrap();
    let first = controller.ready.as_ref().unwrap();
    let base = first.base.clone();
    let base_revision = first.base_revision;
    let highlight = first.highlight.clone();
    assert!(first.highlight.added.is_empty());
    assert!(first.entity_flags.is_empty());

    // Selecting the line is the flags path: the base and the (empty) overlay
    // delta are both reused; only the entity flags change.
    let overlays = OverlayInputs {
        selection: SelectionSet::from_refs([model_ref(DocumentId(73), cad_domain::EntityId(1))]),
        visibility: OverlayVisibility {
            axes: false,
            grid: false,
            ..OverlayVisibility::default()
        },
        ..OverlayInputs::default()
    };
    controller
        .prepare_with_overlays(
            Some(&db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &overlays,
        )
        .unwrap();
    let after = controller.ready.as_ref().unwrap();
    assert!(
        Arc::ptr_eq(&base, &after.base),
        "the base drawing must be reused across a selection change"
    );
    assert_eq!(after.base_revision, base_revision);
    assert!(
        Arc::ptr_eq(&highlight, &after.highlight),
        "an entity-level selection must not rebuild the overlay delta"
    );
    assert!(after.highlight.added.is_empty());
    assert_eq!(
        after.entity_flags,
        vec![(
            model_ref(DocumentId(73), cad_domain::EntityId(1)),
            cad_scene::FLAG_SELECTED
        )]
    );

    // Rebuilding with the same selection is a no-op (revision unchanged).
    let revision = after.revision;
    controller
        .prepare_with_overlays(
            Some(&db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &overlays,
        )
        .unwrap();
    assert_eq!(controller.ready.as_ref().unwrap().revision, revision);
}

#[test]
fn flags_only_refresh_republishes_flags_without_touching_the_scene() {
    use crate::selection::{model_ref, SelectionSet};
    let db = db_with_line();
    let mut controller = CadSceneController::default();
    let layers = LayerOverrideSet::new();
    let empty = overlays_without_reference();
    controller
        .prepare_with_overlays(
            Some(&db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &empty,
        )
        .unwrap();
    let before = controller.ready.as_ref().unwrap().clone();
    assert!(before.entity_flags.is_empty());

    let overlays = OverlayInputs {
        selection: SelectionSet::from_refs([model_ref(DocumentId(73), cad_domain::EntityId(1))]),
        visibility: OverlayVisibility {
            axes: false,
            grid: false,
            ..OverlayVisibility::default()
        },
        ..OverlayInputs::default()
    };
    assert!(
        controller.refresh_flags_only(Some(&db), DocumentId(73), &overlays),
        "a selection change re-resolves the flags"
    );
    let after = controller.ready.as_ref().unwrap();
    assert_eq!(
        after.entity_flags,
        vec![(
            model_ref(DocumentId(73), cad_domain::EntityId(1)),
            cad_scene::FLAG_SELECTED
        )]
    );
    assert_eq!(
        after.revision, before.revision,
        "a flags-only refresh must not advance the scene revision"
    );
    assert_eq!(after.base_revision, before.base_revision);
    assert!(Arc::ptr_eq(&after.base, &before.base));
    assert!(Arc::ptr_eq(&after.highlight, &before.highlight));
    assert!(
        after.flags_revision > before.flags_revision,
        "the flags revision advances on a flags-only refresh"
    );

    // The same inputs are a no-op (nothing is republished).
    let revision = controller.ready.as_ref().unwrap().flags_revision;
    assert!(!controller.refresh_flags_only(Some(&db), DocumentId(73), &overlays));
    assert_eq!(controller.ready.as_ref().unwrap().flags_revision, revision);
}

#[test]
fn preview_change_rebuilds_only_the_highlight_overlay_and_cancel_clears_it() {
    use crate::measure_tool::{MeasurementPreview, MeasurementToolKind};
    let db = db_with_line();
    let mut controller = CadSceneController::default();
    let layers = LayerOverrideSet::new();
    let overlays = overlays_without_reference();
    controller
        .prepare_with_overlays(
            Some(&db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &overlays,
        )
        .unwrap();
    let base = controller.ready.as_ref().unwrap().base.clone();

    let active = OverlayInputs {
        measurement: Some(MeasurementPreview {
            kind: MeasurementToolKind::Distance,
            points: vec![cad_domain::Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }],
            cursor: Some(cad_domain::Point3 {
                x: 4.0,
                y: 0.0,
                z: 0.0,
            }),
            remaining: 1,
            ready: false,
        }),
        visibility: OverlayVisibility {
            axes: false,
            grid: false,
            ..OverlayVisibility::default()
        },
        ..OverlayInputs::default()
    };
    controller
        .prepare_with_overlays(
            Some(&db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &active,
        )
        .unwrap();
    let with_preview = controller.ready.as_ref().unwrap();
    assert!(Arc::ptr_eq(&base, &with_preview.base));
    // One rubber-band chain (2 vertices) + one marker batch (4 vertices).
    assert_eq!(with_preview.highlight.added.len(), 2);

    // Cancelling (None) makes the preview overlay disappear.
    controller
        .prepare_with_overlays(
            Some(&db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &overlays_without_reference(),
        )
        .unwrap();
    assert!(controller
        .ready
        .as_ref()
        .unwrap()
        .highlight
        .added
        .is_empty());
}

fn snap_hint(kind: cad_measure::SnapKind, point: cad_domain::Point3) -> SnapHint {
    cad_measure::SnapCandidate {
        kind,
        point,
        space: cad_domain::SpaceId::Model,
        source: cad_domain::SelectionRef {
            document: DocumentId(73),
            entity: cad_domain::EntityId(1),
            instance: cad_domain::InstancePath::default(),
            sub_element: None,
        },
        secondary: None,
        precision: cad_domain::Precision::Analytic,
        logical_pixel_distance: 0.0,
    }
}

#[test]
fn snap_hints_draw_only_when_enabled_and_keep_the_base() {
    let db = db_with_line();
    let mut controller = CadSceneController::default();
    let layers = LayerOverrideSet::new();

    let off = OverlayInputs {
        snap_hints_input: vec![snap_hint(
            cad_measure::SnapKind::Endpoint,
            cad_domain::Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        )],
        visibility: OverlayVisibility {
            axes: false,
            grid: false,
            snap_hints: false,
            selection_highlight: false,
        },
        ..OverlayInputs::default()
    };
    controller
        .prepare_with_overlays(
            Some(&db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &off,
        )
        .unwrap();
    let base = controller.ready.as_ref().unwrap().base.clone();
    assert!(
        controller
            .ready
            .as_ref()
            .unwrap()
            .highlight
            .added
            .is_empty(),
        "snap_hints off must draw no snap-hint batch even when hints are fed"
    );

    let on = OverlayInputs {
        snap_hints_input: vec![
            snap_hint(
                cad_measure::SnapKind::Endpoint,
                cad_domain::Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
            ),
            snap_hint(
                cad_measure::SnapKind::Center,
                cad_domain::Point3 {
                    x: 5.0,
                    y: 0.0,
                    z: 0.0,
                },
            ),
        ],
        visibility: OverlayVisibility {
            axes: false,
            grid: false,
            snap_hints: true,
            selection_highlight: false,
        },
        ..OverlayInputs::default()
    };
    controller
        .prepare_with_overlays(
            Some(&db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &on,
        )
        .unwrap();
    let after = controller.ready.as_ref().unwrap();
    assert!(
        Arc::ptr_eq(&base, &after.base),
        "snap hints must not rebuild the base drawing"
    );
    assert_eq!(
        after.highlight.added.len(),
        1,
        "one bounded snap-hint batch"
    );
    assert_eq!(
        after.highlight.added[0].draw_order,
        crate::render_scene::SNAP_HINT_DRAW_ORDER
    );
    assert!(
        after.highlight.added[0].draw_order > crate::render_scene::CURSOR_DRAW_ORDER,
        "the snap marker draws above the pointer cursor so the pickbox cannot hide it"
    );

    // Clearing the hints removes the batch but keeps the base.
    controller
        .prepare_with_overlays(
            Some(&db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &overlays_without_reference(),
        )
        .unwrap();
    let cleared = controller.ready.as_ref().unwrap();
    assert!(cleared.highlight.added.is_empty());
    assert!(Arc::ptr_eq(&base, &cleared.base));
}

#[test]
fn disabling_the_selection_highlight_omits_the_selection_flags() {
    use crate::selection::{model_ref, SelectionSet};
    let db = db_with_line();
    let mut controller = CadSceneController::default();
    let layers = LayerOverrideSet::new();
    let hidden = OverlayInputs {
        selection: SelectionSet::from_refs([model_ref(DocumentId(73), cad_domain::EntityId(1))]),
        visibility: OverlayVisibility {
            axes: false,
            grid: false,
            selection_highlight: false,
            ..OverlayVisibility::default()
        },
        ..OverlayInputs::default()
    };
    controller
        .prepare_with_overlays(
            Some(&db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &hidden,
        )
        .unwrap();
    let ready = controller.ready.as_ref().unwrap();
    assert!(
        ready.highlight.added.is_empty(),
        "a non-empty selection with selection_highlight off must draw nothing"
    );
    assert!(
        ready.entity_flags.is_empty(),
        "selection_highlight off must also clear the selected flags"
    );
}

#[test]
fn axes_and_grid_appear_only_when_enabled_and_keep_the_base() {
    let db = db_with_line();
    let mut controller = CadSceneController::default();
    let layers = LayerOverrideSet::new();
    let off = overlays_without_reference();
    controller
        .prepare_with_overlays(
            Some(&db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &off,
        )
        .unwrap();
    let base = controller.ready.as_ref().unwrap().base.clone();
    assert!(controller
        .ready
        .as_ref()
        .unwrap()
        .highlight
        .added
        .is_empty());

    let on = OverlayInputs {
        visibility: OverlayVisibility {
            selection_highlight: false,
            ..OverlayVisibility::default()
        },
        ..OverlayInputs::default()
    };
    controller
        .prepare_with_overlays(
            Some(&db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &on,
        )
        .unwrap();
    let after = controller.ready.as_ref().unwrap();
    assert!(
        Arc::ptr_eq(&base, &after.base),
        "reference overlays must not rebuild the base drawing"
    );
    assert!(!after.highlight.added.is_empty());
    assert!(after
        .highlight
        .added
        .iter()
        .all(|b| b.draw_order > 0 && b.draw_order < 900_000));
    assert!(after
        .highlight
        .added
        .iter()
        .any(|b| b.draw_order == AXES_GRID_DRAW_ORDER));
}

#[test]
fn axes_without_bounds_report_a_diagnostic_instead_of_pretending() {
    let db = DrawingDatabaseBuilder::new(DatabaseId(9)).finish().unwrap();
    let mut controller = CadSceneController::default();
    let layers = LayerOverrideSet::new();
    let on = OverlayInputs {
        visibility: OverlayVisibility {
            selection_highlight: false,
            ..OverlayVisibility::default()
        },
        ..OverlayInputs::default()
    };
    controller
        .prepare_with_overlays(
            Some(&db),
            DocumentId(73),
            None,
            &layers,
            SpaceSelection::Model,
            &on,
        )
        .unwrap();
    let ready = controller.ready.as_ref().unwrap();
    assert!(ready.highlight.added.is_empty());
    assert!(ready
        .highlight_diagnostics
        .iter()
        .any(|d| d.code == "overlay.bounds-unavailable"));
    assert!(matches!(
        ready.highlight_completeness,
        cad_domain::Completeness::Partial(_)
    ));
}

/// The world-space half-extent of the largest X/Y span of a batch, independent
/// of where it sits (a marker glyph's half-size).
fn batch_half_extent(batch: &cad_scene::RenderBatch) -> f64 {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for vertex in &batch.vertices {
        let x = batch.local_origin.x + f64::from(vertex[0]);
        let y = batch.local_origin.y + f64::from(vertex[1]);
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    (max_x - min_x).max(max_y - min_y) * 0.5
}

/// The object-snap marker's world size is a fixed **screen** size: the same pixel
/// size at a larger view scale must produce a proportionally larger world size,
/// so the glyph stays legible under any zoom (defect #2).
#[test]
fn snap_marker_size_is_a_fixed_screen_size_through_the_scene_assembly() {
    use cad_domain::Point3;
    let db = db_with_line();
    let layers = LayerOverrideSet::new();
    let hint = snap_hint(
        cad_measure::SnapKind::Endpoint,
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    );
    let build = |scale: f64| {
        let overlays = OverlayInputs {
            snap_hints_input: vec![hint.clone()],
            view_scale: Some(scale),
            visibility: OverlayVisibility {
                axes: false,
                grid: false,
                snap_hints: true,
                selection_highlight: false,
            },
            ..OverlayInputs::default()
        };
        let mut controller = CadSceneController::default();
        controller
            .prepare_with_overlays(
                Some(&db),
                DocumentId(73),
                None,
                &layers,
                SpaceSelection::Model,
                &overlays,
            )
            .unwrap();
        let batch = controller
            .ready
            .as_ref()
            .unwrap()
            .highlight
            .added
            .iter()
            .find(|batch| batch.draw_order == crate::render_scene::SNAP_HINT_DRAW_ORDER)
            .expect("a snap-hint batch");
        batch_half_extent(batch)
    };
    let small = build(0.1);
    let large = build(0.4);
    assert!(
        (small - crate::render_scene::SNAP_MARKER_HALF_EXTENT_PIXELS * 0.1).abs() < 1e-6,
        "half-extent = marker pixels × scale: {small}"
    );
    assert!(
        (large / small - 4.0).abs() < 1e-6,
        "the same pixel size at 4× the scale is 4× the world size: {large} vs {small}"
    );
}

/// The pointer cursor crosshair + pickbox is built by the scene assembly from the
/// cursor world point and the view scale, at the highest draw order, and is
/// absent when either is missing (defect #1).
#[test]
fn the_cursor_overlay_is_built_from_the_cursor_and_the_view() {
    use cad_domain::Point3;
    let db = db_with_line();
    let layers = LayerOverrideSet::new();
    let bounds = Some((
        Point3 {
            x: -10.0,
            y: -5.0,
            z: 0.0,
        },
        Point3 {
            x: 10.0,
            y: 5.0,
            z: 0.0,
        },
    ));
    let cursor = Point3 {
        x: 1.0,
        y: 2.0,
        z: 0.0,
    };
    let build = |cursor: Option<Point3>, view_scale: Option<f64>| {
        let overlays = OverlayInputs {
            viewport_bounds: bounds,
            view_scale,
            cursor,
            visibility: OverlayVisibility {
                axes: false,
                grid: false,
                ..OverlayVisibility::default()
            },
            ..OverlayInputs::default()
        };
        let mut controller = CadSceneController::default();
        controller
            .prepare_with_overlays(
                Some(&db),
                DocumentId(73),
                None,
                &layers,
                SpaceSelection::Model,
                &overlays,
            )
            .unwrap();
        let ready = controller.ready.as_ref().unwrap().clone();
        ready
            .highlight
            .added
            .iter()
            .filter(|batch| batch.draw_order == crate::render_scene::CURSOR_DRAW_ORDER)
            .cloned()
            .collect::<Vec<_>>()
    };

    let batches = build(Some(cursor), Some(0.25));
    assert_eq!(batches.len(), 1, "one bounded cursor batch");
    // Crosshair (2 segments = 4 vertices) + pickbox (4 segments = 8 vertices).
    assert_eq!(batches[0].vertices.len(), 12);
    assert_eq!(batches[0].color, crate::render_scene::CURSOR_COLOR);
    assert!(
        batches[0].draw_order > crate::render_scene::PREVIEW_DRAW_ORDER,
        "the cursor draws above every other overlay"
    );
    // The pickbox half-side is the pick tolerance × the view scale.
    let half = {
        let tail = &batches[0].vertices[batches[0].vertices.len() - 8..];
        let xs: Vec<f64> = tail
            .iter()
            .map(|v| batches[0].local_origin.x + f64::from(v[0]))
            .collect();
        (xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
            - xs.iter().cloned().fold(f64::INFINITY, f64::min))
            * 0.5
    };
    let expected = cad_domain::TolerancePolicy::default().pickbox_logical_pixels * 0.25;
    assert!(
        (half - expected).abs() < 1e-6,
        "pickbox half-side must be PICKBOX × scale: {half} vs {expected}"
    );

    // No cursor position produces no cursor batch; a missing view scale still
    // draws the crosshair but omits the pickbox (a screen-relative size has no
    // domain), never a fabricated box.
    assert!(build(None, Some(0.25)).is_empty());
    let no_scale = build(Some(cursor), None);
    assert_eq!(no_scale.len(), 1);
    assert_eq!(
        no_scale[0].vertices.len(),
        4,
        "crosshair only, no pickbox, when the view scale is missing"
    );
}
