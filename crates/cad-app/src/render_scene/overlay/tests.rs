use super::axes_grid::grid_positions;
use super::selection::same_entity_instance;
use super::*;
use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_domain::{
    BlockId, DatabaseId, EntityId, InstancePath, LayerId, Mesh, ObjectId, Precision, Revision,
    SemanticGeometry, SpaceId, SubElementId, Transform3,
};

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn entity(id: u128, geometry: SemanticGeometry, draw_order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: cad_domain::SpaceId::Model,
        geometry,
        draw_order,
    }
}

/// A drawing with two INSERT placements of one single-line block at x = 10
/// and x = 20, plus a top-level line at x = 30.
fn drawing() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(entity(
        100,
        SemanticGeometry::Line {
            start: p(0.0, -1.0),
            end: p(0.0, 1.0),
        },
        0,
    ))
    .unwrap();
    b.insert_entity(entity(
        10,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(p(10.0, 0.0)),
        },
        1,
    ))
    .unwrap();
    b.insert_entity(entity(
        20,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(p(20.0, 0.0)),
        },
        2,
    ))
    .unwrap();
    b.insert_entity(entity(
        30,
        SemanticGeometry::Line {
            start: p(30.0, -1.0),
            end: p(30.0, 1.0),
        },
        3,
    ))
    .unwrap();
    b.finish().unwrap()
}

fn block_child_ref(instance: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(100),
        instance: InstancePath(vec![EntityId(instance)]),
        sub_element: None,
    }
}

fn model_ref(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn stamp() -> TaskStamp {
    TaskStamp::new(DocumentId(1), 1)
}

#[test]
fn empty_selection_yields_no_batches() {
    let db = drawing();
    let overlay = selection_highlight(&db, &SelectionSet::new(), DocumentId(1), stamp(), None);
    assert!(overlay.is_empty());
    assert_eq!(overlay.completeness, Completeness::Complete);
    assert!(overlay.diagnostics.is_empty());
}

fn flags_inputs(selection: SelectionSet) -> OverlayInputs {
    OverlayInputs {
        selection,
        ..OverlayInputs::default()
    }
}

#[test]
fn entity_level_selection_is_flags_not_an_overlay_batch() {
    let db = drawing();
    let selection = SelectionSet::from_refs([model_ref(30)]);
    // No overlay batch: a whole entity is the renderer flags path.
    let overlay = selection_highlight(&db, &selection, DocumentId(1), stamp(), None);
    assert!(overlay.is_empty());
    assert!(overlay.diagnostics.is_empty());

    // The flags resolver produces one SELECTED flag for the entity.
    let scene = selection_entity_flags(&db, &flags_inputs(selection), DocumentId(1));
    assert_eq!(scene.flags, vec![(model_ref(30), cad_scene::FLAG_SELECTED)]);
    assert!(scene.diagnostics.is_empty());
    assert_eq!(scene.completeness, Completeness::Complete);
}

#[test]
fn two_insert_instances_flag_independently() {
    let db = drawing();
    // Only the first placement (instance [10]) is selected.
    let scene = selection_entity_flags(
        &db,
        &flags_inputs(SelectionSet::from_refs([block_child_ref(10)])),
        DocumentId(1),
    );
    assert_eq!(
        scene.flags,
        vec![(block_child_ref(10), cad_scene::FLAG_SELECTED)]
    );

    // Both placements are distinct entity-level refs, flagged independently
    // (the renderer resolves each to its own placement's pick key).
    let scene = selection_entity_flags(
        &db,
        &flags_inputs(SelectionSet::from_refs([
            block_child_ref(10),
            block_child_ref(20),
        ])),
        DocumentId(1),
    );
    assert_eq!(scene.flags.len(), 2);
    assert_eq!(scene.flags[0].0, block_child_ref(10));
    assert_eq!(scene.flags[1].0, block_child_ref(20));
}

#[test]
fn a_sub_element_selection_still_gets_an_overlay_batch() {
    let db = mesh_insert_drawing();
    let selection = SelectionSet::from_refs([mesh_child_ref(Some(face("face-a")))]);
    let overlay = selection_highlight(&db, &selection, DocumentId(2), stamp(), None);
    assert_eq!(overlay.drawn(), 1);
    assert_eq!(overlay.batches[0].topology, RenderTopology::Mesh);
    assert_eq!(
        overlay.batches[0].sources[0]
            .sub_element
            .as_ref()
            .map(|s| s.source_key.as_str()),
        Some("face-a")
    );
}

#[test]
fn unresolved_entity_level_ref_is_reported_by_the_flags_resolver() {
    let db = drawing();
    let scene = selection_entity_flags(
        &db,
        &flags_inputs(SelectionSet::from_refs([model_ref(999)])),
        DocumentId(1),
    );
    assert!(
        scene.flags.is_empty(),
        "a deleted entity must not produce a fabricated flag"
    );
    assert!(scene
        .diagnostics
        .iter()
        .any(|d| d.code == "highlight.unresolved"));
    assert!(matches!(scene.completeness, Completeness::Missing(_)));
}

// ---- resolve-only-selected-refs contract (perf fix) -------------------

/// The pre-fix behaviour, kept test-only as the correctness oracle: scan
/// **every** pick item, keep the selected ones, build their representations.
fn legacy_selection_highlight(
    database: &DrawingDatabase,
    selection: &SelectionSet,
    document: DocumentId,
    stamp: TaskStamp,
) -> VisualOverlay {
    let mut out = VisualOverlay::default();
    if selection.is_empty() {
        return out;
    }
    let registry = ProviderRegistry::with_default_provider();
    let context =
        RepresentationContext::new(document, cad_domain::TolerancePolicy::default(), stamp);
    let items = crate::picking::drawing_pick_items(database, document);
    let mut representations: Vec<DisplayRepresentation> = Vec::new();
    let mut build_reasons: Vec<Diagnostic> = Vec::new();
    for item in &items {
        let selected = selection
            .refs()
            .iter()
            .any(|reference| same_entity_instance(&item.source, reference));
        if !selected {
            continue;
        }
        match legacy_representation_for_item(&registry, database, item, &context) {
            Some(representation) => representations.push(representation),
            None => build_reasons.push(Diagnostic {
                object: None,
                code: "highlight.entity-missing".into(),
                message: format!(
                    "selection entity {} is present in the pick index but not in the database",
                    item.source.entity.0
                ),
            }),
        }
    }
    let HighlightScene {
        batches,
        completeness,
        diagnostics,
        ..
    } = cad_scene::highlight_batches(
        selection.refs(),
        representations.iter(),
        |_| true,
        &HighlightOptions::default(),
    );
    out.batches = batches;
    out.diagnostics = diagnostics;
    out.completeness = completeness;
    for reason in build_reasons {
        out.diagnostics.push(reason);
    }
    out
}

fn legacy_representation_for_item(
    registry: &ProviderRegistry,
    database: &DrawingDatabase,
    item: &cad_spatial::PickItem,
    context: &RepresentationContext,
) -> Option<DisplayRepresentation> {
    let entity = database.entity(item.source.entity)?;
    let mut representation = match registry.build(entity, context) {
        Ok(representation) => representation,
        Err(error) => DisplayRepresentation {
            fragments: Vec::new(),
            completeness: Completeness::Missing(vec![error.to_string()]),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "highlight.representation".into(),
                message: error.to_string(),
            }],
        },
    };
    for fragment in &mut representation.fragments {
        fragment.primitive = fragment.primitive.transformed(&item.transform);
        fragment.source = item.source.clone();
    }
    Some(representation)
}

fn face(key: &str) -> SubElementId {
    SubElementId {
        source_key: key.into(),
        topology_revision: Revision(0),
    }
}

/// A two-triangle mesh with two distinct stable face sources.
fn two_face_mesh() -> Mesh {
    let p3 = |x: f64, y: f64| Point3 { x, y, z: 0.0 };
    Mesh {
        vertices: vec![
            p3(-2.0, -1.0),
            p3(-1.0, 1.0),
            p3(0.0, -1.0),
            p3(0.0, -1.0),
            p3(1.0, 1.0),
            p3(2.0, -1.0),
        ],
        triangles: vec![[0, 1, 2], [3, 4, 5]],
        normals: Vec::new(),
        face_sources: vec![Some(face("face-a")), Some(face("face-b"))],
        colors: Vec::new(),
    }
}

/// A block whose only member is a two-face mesh, placed once at x = 10.
fn mesh_insert_drawing() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(2));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(300)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(entity(300, SemanticGeometry::Mesh(two_face_mesh()), 0))
        .unwrap();
    b.insert_entity(entity(
        10,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(p(10.0, 0.0)),
        },
        1,
    ))
    .unwrap();
    b.finish().unwrap()
}

fn mesh_child_ref(sub_element: Option<SubElementId>) -> SelectionRef {
    SelectionRef {
        document: DocumentId(2),
        entity: EntityId(300),
        instance: InstancePath(vec![EntityId(10)]),
        sub_element,
    }
}

/// The resolve-only sub-element path must be byte-for-byte identical to the
/// old full-scan path for every sub-element selection (mesh face, duplicate
/// `(entity, instance)`, missing entity, broken instance path). Entity-level
/// refs are the flags path and never reach `highlight_batches`, so they are
/// exercised by `selection_entity_flags` tests instead.
#[test]
fn resolve_only_selected_refs_matches_the_full_scan_for_sub_elements() {
    let mesh_db = mesh_insert_drawing();
    let mesh_selections = [
        SelectionSet::new(),
        SelectionSet::from_refs([mesh_child_ref(None)]),
        SelectionSet::from_refs([mesh_child_ref(Some(face("face-a")))]),
        SelectionSet::from_refs([mesh_child_ref(Some(face("face-b")))]),
        // The same object selected both whole and by face: the scan matched
        // one item for both refs; the resolve path must resolve it once.
        SelectionSet::from_refs([mesh_child_ref(None), mesh_child_ref(Some(face("face-a")))]),
    ];
    for selection in mesh_selections {
        let fast = selection_highlight(&mesh_db, &selection, DocumentId(2), stamp(), None);
        let legacy = legacy_selection_highlight(&mesh_db, &selection, DocumentId(2), stamp());
        assert_eq!(fast, legacy, "mesh selection {selection:?} diverged");
    }
}

/// A camera settle re-prepares the transient overlay with a new `TaskStamp`
/// (the next scene revision) over the same selection and database. The
/// sub-element highlight is derived **only** from the selection and the
/// database, so it must be byte-identical and add no diagnostic: a camera-only
/// operation cannot degrade it. A `highlight.unresolved` requires a ref the
/// database no longer resolves, i.e. a scene mutation.
#[test]
fn sub_element_highlight_is_identical_across_a_camera_settle() {
    let db = mesh_insert_drawing();
    let selection = SelectionSet::from_refs([mesh_child_ref(Some(face("face-a")))]);
    let before = selection_highlight(&db, &selection, DocumentId(2), stamp(), None);
    assert_eq!(
        before.drawn(),
        1,
        "the face selection must produce one batch"
    );
    assert!(before.diagnostics.is_empty());

    // The settle commit re-prepares the same inputs with the next revision.
    let settled = TaskStamp::new(DocumentId(2), 2);
    let after = selection_highlight(&db, &selection, DocumentId(2), settled, None);
    assert_eq!(
        after, before,
        "a camera settle must not change the sub-element highlight"
    );
    assert!(
        after.diagnostics.is_empty(),
        "a camera settle must not add a highlight diagnostic"
    );
}

/// An `INSERT` root only ever exists as an expansion, so selecting it is
/// unresolved — never a fabricated flag of the block's contents.
#[test]
fn insert_root_is_unresolved_for_the_flags_resolver() {
    let db = drawing();
    let scene = selection_entity_flags(
        &db,
        &flags_inputs(SelectionSet::from_refs([model_ref(10)])),
        DocumentId(1),
    );
    assert!(scene.flags.is_empty());
    assert!(scene
        .diagnostics
        .iter()
        .any(|d| d.code == "highlight.unresolved"));
}

/// A whole-entity and a face selection of the same object: the entity is a
/// flag and the face is the single overlay batch, so neither duplicates.
#[test]
fn a_face_and_its_entity_produce_one_overlay_batch_and_one_flag() {
    let db = mesh_insert_drawing();
    let selection =
        SelectionSet::from_refs([mesh_child_ref(None), mesh_child_ref(Some(face("face-a")))]);
    let overlay = selection_highlight(&db, &selection, DocumentId(2), stamp(), None);
    // Only the face-a batch; the whole entity is a flag, not a batch.
    assert_eq!(overlay.drawn(), 1);
    let scene = selection_entity_flags(&db, &flags_inputs(selection), DocumentId(2));
    assert_eq!(
        scene.flags,
        vec![(mesh_child_ref(None), cad_scene::FLAG_SELECTED)]
    );
}

#[test]
fn empty_preview_yields_no_batches() {
    let overlay = preview_overlay(None, DocumentId(1), true, &PreviewOptions::default());
    assert!(overlay.is_empty());
}

#[test]
fn measurement_preview_produces_n_minus_one_segments_and_markers() {
    let preview = MeasurementPreview {
        kind: crate::measure_tool::MeasurementToolKind::PolylineLength,
        points: vec![p(0.0, 0.0), p(1.0, 0.0), p(2.0, 1.0)],
        cursor: None,
        remaining: 0,
        ready: true,
    };
    let overlay = preview_overlay(
        Some(&preview),
        DocumentId(1),
        true,
        &PreviewOptions::default(),
    );
    // One polyline batch (3 vertices -> 2 segments) + one marker batch.
    assert_eq!(overlay.drawn(), 2);
    assert_eq!(overlay.batches[0].vertices.len(), 3);
    // 3 points -> 3 crosses -> 6 segments -> 12 vertices.
    assert_eq!(overlay.batches[1].vertices.len(), 12);
    assert_eq!(overlay.batches[0].color, MEASUREMENT_PREVIEW_COLOR);
    assert!(overlay.batches[0].draw_order >= PREVIEW_DRAW_ORDER);
    assert!(overlay.batches.iter().all(|b| !b.color_unresolved));
}

#[test]
fn measurement_cursor_extends_the_chain_to_the_last_point() {
    let preview = MeasurementPreview {
        kind: crate::measure_tool::MeasurementToolKind::Distance,
        points: vec![p(0.0, 0.0)],
        cursor: Some(p(5.0, 0.0)),
        remaining: 1,
        ready: false,
    };
    let overlay = preview_overlay(
        Some(&preview),
        DocumentId(1),
        true,
        &PreviewOptions::default(),
    );
    // Chain is [captured, cursor] -> 2 vertices; plus the captured marker.
    assert_eq!(overlay.drawn(), 2);
    assert_eq!(overlay.batches[0].vertices.len(), 2);
    assert_eq!(overlay.batches[0].vertices[1], [5.0, 0.0, 0.0]);
}

#[test]
fn cancelling_a_preview_makes_it_disappear() {
    let preview = MeasurementPreview {
        kind: crate::measure_tool::MeasurementToolKind::Distance,
        points: vec![p(0.0, 0.0), p(1.0, 0.0)],
        cursor: None,
        remaining: 0,
        ready: true,
    };
    let active = preview_overlay(
        Some(&preview),
        DocumentId(1),
        true,
        &PreviewOptions::default(),
    );
    assert!(!active.is_empty());
    let cancelled = preview_overlay(None, DocumentId(1), true, &PreviewOptions::default());
    assert!(cancelled.is_empty());
}

#[test]
fn overlay_fingerprint_tracks_sub_element_selection_and_cursor() {
    let mut a = OverlayInputs::default();
    let base = overlay_fingerprint(&a);
    a.selection = SelectionSet::from_refs([mesh_child_ref(Some(face("face-a")))]);
    let selected = overlay_fingerprint(&a);
    assert_ne!(base, selected);
    a.selection.replace([mesh_child_ref(Some(face("face-b")))]);
    assert_ne!(selected, overlay_fingerprint(&a));

    // An entity-level selection is the flags path: it must NOT change the
    // overlay fingerprint (so no overlay geometry is rebuilt/re-uploaded),
    // only the flags fingerprint.
    let mut entity_only = OverlayInputs::default();
    let overlay_before = overlay_fingerprint(&entity_only);
    let flags_before = entity_flags_fingerprint(&entity_only);
    entity_only.selection = SelectionSet::from_refs([block_child_ref(10)]);
    assert_eq!(overlay_fingerprint(&entity_only), overlay_before);
    assert_ne!(entity_flags_fingerprint(&entity_only), flags_before);

    let mut b = OverlayInputs {
        measurement: Some(MeasurementPreview {
            kind: crate::measure_tool::MeasurementToolKind::Distance,
            points: vec![p(0.0, 0.0)],
            cursor: None,
            remaining: 1,
            ready: false,
        }),
        ..OverlayInputs::default()
    };
    let no_cursor = overlay_fingerprint(&b);
    b.measurement.as_mut().unwrap().cursor = Some(p(1.0, 0.0));
    assert_ne!(no_cursor, overlay_fingerprint(&b));
    // Rebuilding with identical inputs is stable.
    assert_eq!(overlay_fingerprint(&b), overlay_fingerprint(&b));
}

#[test]
fn entity_flags_fingerprint_tracks_hover_and_hidden() {
    let base = entity_flags_fingerprint(&OverlayInputs::default());
    let hovered = OverlayInputs {
        hovered: SelectionSet::from_refs([block_child_ref(10)]),
        ..OverlayInputs::default()
    };
    let hidden = OverlayInputs {
        hidden: SelectionSet::from_refs([block_child_ref(10)]),
        ..OverlayInputs::default()
    };
    assert_ne!(entity_flags_fingerprint(&hovered), base);
    assert_ne!(entity_flags_fingerprint(&hidden), base);
    assert_ne!(
        entity_flags_fingerprint(&hovered),
        entity_flags_fingerprint(&hidden),
        "hover and hidden are distinct flag kinds"
    );
}

#[test]
fn overlay_fingerprint_tracks_each_visibility_flag() {
    let baseline = overlay_fingerprint(&OverlayInputs::default());
    let flips: [fn(&mut OverlayVisibility); 4] = [
        |v| v.axes = false,
        |v| v.grid = false,
        |v| v.selection_highlight = false,
        |v| v.snap_hints = false,
    ];
    for flip in flips {
        let mut inputs = OverlayInputs::default();
        flip(&mut inputs.visibility);
        assert_ne!(
            baseline,
            overlay_fingerprint(&inputs),
            "flipping a visibility flag must invalidate the overlay"
        );
    }
}

fn hint(kind: SnapHintKind, point: Point3) -> SnapHint {
    SnapCandidate {
        kind,
        point,
        space: SpaceId::Model,
        source: model_ref(1),
        secondary: None,
        precision: Precision::Analytic,
        logical_pixel_distance: 0.0,
    }
}

fn snap_hint_kinds() -> [SnapHintKind; 6] {
    [
        SnapKind::Endpoint,
        SnapKind::Midpoint,
        SnapKind::Center,
        SnapKind::Quadrant,
        SnapKind::Perpendicular,
        SnapKind::LocalIntersection,
    ]
}

/// Local (origin-relative) segment pairs of a `LineList` batch.
fn segment_pairs(batch: &RenderBatch) -> Vec<([f32; 3], [f32; 3])> {
    batch
        .vertices
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

#[test]
fn empty_snap_hints_yield_no_batch() {
    let overlay = snap_hint_overlay(&[], DocumentId(1), &PreviewOptions::default());
    assert!(overlay.is_empty());
    assert_eq!(overlay.completeness, Completeness::Complete);
    assert!(overlay.diagnostics.is_empty());
}

#[test]
fn snap_hint_overlay_is_one_bounded_batch_below_the_preview() {
    let hints = [
        hint(SnapKind::Endpoint, p(0.0, 0.0)),
        hint(SnapKind::Center, p(5.0, 0.0)),
    ];
    let overlay = snap_hint_overlay(&hints, DocumentId(1), &PreviewOptions::default());
    assert_eq!(overlay.drawn(), 1, "all hints share one bounded batch");
    let batch = &overlay.batches[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert_eq!(batch.color, SNAP_HINT_COLOR);
    assert_eq!(batch.draw_order, SNAP_HINT_DRAW_ORDER);
    assert!(batch.draw_order > 900_000 && batch.draw_order < PREVIEW_DRAW_ORDER);
    // LineList: segment pairs are explicit; the count is even.
    assert_eq!(batch.vertices.len() % 2, 0);
    // Endpoint square (4) + center circle (12) segments.
    assert_eq!(segment_pairs(batch).len(), 4 + SNAP_HINT_CIRCLE_SEGMENTS);
    assert!(!batch.color_unresolved);
}

#[test]
fn each_snap_kind_produces_a_distinct_marker_shape() {
    let kinds = snap_hint_kinds();
    let mut shapes: Vec<Vec<[f32; 3]>> = Vec::new();
    for kind in kinds {
        let overlay = snap_hint_overlay(
            &[hint(kind, p(1.0, 2.0))],
            DocumentId(1),
            &PreviewOptions::default(),
        );
        assert_eq!(overlay.drawn(), 1, "{kind:?} must draw exactly one batch");
        let batch = &overlay.batches[0];
        let segments = segment_pairs(batch);
        assert!(!segments.is_empty(), "{kind:?} must emit segments");
        // Every marker is drawn around the hint point, not at the origin.
        for (a, b) in &segments {
            for vertex in [a, b] {
                let x = batch.local_origin.x + vertex[0] as f64;
                let y = batch.local_origin.y + vertex[1] as f64;
                assert!(
                    (x - 1.0).abs() <= 0.5 + 1e-9 && (y - 2.0).abs() <= 0.5 + 1e-9,
                    "{kind:?} vertex {vertex:?} off the marker centre"
                );
            }
        }
        // Local (origin-relative) vertices, directly comparable per kind.
        shapes.push(batch.vertices.clone());
    }
    for i in 0..shapes.len() {
        for j in (i + 1)..shapes.len() {
            assert_ne!(
                shapes[i],
                shapes[j],
                "snap kinds {} and {} must not share a marker shape",
                kinds[i].as_str(),
                kinds[j].as_str()
            );
        }
    }
}

#[test]
fn snap_hint_markers_scale_with_the_marker_size() {
    let small = snap_hint_overlay(
        &[hint(SnapKind::Endpoint, p(0.0, 0.0))],
        DocumentId(1),
        &PreviewOptions {
            marker_size: 0.25,
            ..PreviewOptions::default()
        },
    );
    let large = snap_hint_overlay(
        &[hint(SnapKind::Endpoint, p(0.0, 0.0))],
        DocumentId(1),
        &PreviewOptions {
            marker_size: 1.0,
            ..PreviewOptions::default()
        },
    );
    let extent = |overlay: &VisualOverlay| {
        overlay.batches[0]
            .vertices
            .iter()
            .map(|v| v[0].abs().max(v[1].abs()))
            .fold(0.0f32, f32::max)
    };
    assert!(extent(&large) > extent(&small));
}

#[test]
fn overlay_fingerprint_tracks_snap_hints() {
    let baseline = overlay_fingerprint(&OverlayInputs::default());
    let with_hint = OverlayInputs {
        snap_hints_input: vec![hint(SnapKind::Endpoint, p(0.0, 0.0))],
        ..OverlayInputs::default()
    };
    let hashed = overlay_fingerprint(&with_hint);
    assert_ne!(baseline, hashed);
    // The kind is part of the fingerprint even at the same point.
    let other_kind = OverlayInputs {
        snap_hints_input: vec![hint(SnapKind::Midpoint, p(0.0, 0.0))],
        ..OverlayInputs::default()
    };
    assert_ne!(hashed, overlay_fingerprint(&other_kind));
    // The point is part of the fingerprint even at the same kind.
    let other_point = OverlayInputs {
        snap_hints_input: vec![hint(SnapKind::Endpoint, p(1.0, 0.0))],
        ..OverlayInputs::default()
    };
    assert_ne!(hashed, overlay_fingerprint(&other_point));
    // Rebuilding with identical hints is stable.
    assert_eq!(hashed, overlay_fingerprint(&with_hint));
}

#[test]
fn axes_overlay_draws_a_symmetric_origin_cross_below_the_highlight() {
    let overlay = axes_overlay((p(-2.0, -3.0), p(4.0, 5.0)), DocumentId(1));
    assert_eq!(overlay.drawn(), 1);
    let batch = &overlay.batches[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    // Each axis is symmetric about the origin: X spans |-4|..|4| (the larger
    // absolute x bound), Y spans |-5|..|5|, so both arms of each axis are
    // equal regardless of which side the drawing sits on.
    assert_eq!(batch.vertices.len(), 4);
    assert_eq!(batch.local_origin, p(-4.0, 0.0));
    assert_eq!(batch.vertices[0], [0.0, 0.0, 0.0]);
    assert_eq!(batch.vertices[1], [8.0, 0.0, 0.0]);
    assert_eq!(batch.vertices[2], [4.0, -5.0, 0.0]);
    assert_eq!(batch.vertices[3], [4.0, 5.0, 0.0]);
    assert_eq!(batch.color, AXES_COLOR);
    assert!(batch.draw_order < 900_000);
    assert!(!batch.color_unresolved);
}

#[test]
fn grid_overlay_is_one_bounded_batch_spanning_the_supplied_view_bounds() {
    let overlay = grid_overlay((p(-50.0, -50.0), p(50.0, 50.0)), DocumentId(1));
    assert_eq!(overlay.drawn(), 1);
    let batch = &overlay.batches[0];
    assert!(!batch.vertices.is_empty());
    assert_eq!(batch.vertices.len() % 2, 0);
    assert!(batch.vertices.len() / 2 <= GRID_MAX_LINES);
    assert_eq!(batch.color, GRID_COLOR);
    assert!(batch.draw_order < 900_000);
    // Geometry spans the supplied view rectangle; line batches are clipped
    // to this exact extent by the renderer for every live camera/resize.
    let world_points = batch.vertices.iter().map(|v| {
        (
            v[0] as f64 + batch.local_origin.x,
            v[1] as f64 + batch.local_origin.y,
        )
    });
    let (min_x, max_x, min_y, max_y) = world_points.fold(
        (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ),
        |(min_x, max_x, min_y, max_y), (x, y)| {
            (min_x.min(x), max_x.max(x), min_y.min(y), max_y.max(y))
        },
    );
    assert!((min_x + 50.0).abs() < 1e-4 && (max_x - 50.0).abs() < 1e-4);
    assert!((min_y + 50.0).abs() < 1e-4 && (max_y - 50.0).abs() < 1e-4);
}

#[test]
fn grid_positions_caps_a_dense_axis_without_shrinking_it() {
    let positions = grid_positions(0.0, 1000.0, 0.001, 10);
    assert_eq!(positions.len(), 10);
    assert_eq!(positions[0], 0.0);
    assert!(
        positions.last().copied().unwrap() > 900.0,
        "the cap samples across the whole range, not just the corner"
    );
}

#[test]
fn grid_overlay_remains_bounded_at_extreme_pan_and_zoom() {
    let overlay = grid_overlay(
        (p(1.0e12, -1.0e12), p(1.0e12 + 1.0e-2, -1.0e12 + 1.0e-2)),
        DocumentId(1),
    );
    let batch = &overlay.batches[0];
    assert!(!batch.vertices.is_empty());
    assert!(batch.vertices.len() / 2 <= GRID_MAX_LINES);
}

#[test]
fn snap_hints_false_suppresses_measurement_markers_but_keeps_the_chain() {
    let preview = MeasurementPreview {
        kind: crate::measure_tool::MeasurementToolKind::Distance,
        points: vec![p(0.0, 0.0), p(1.0, 0.0)],
        cursor: None,
        remaining: 0,
        ready: true,
    };
    let with = preview_overlay(
        Some(&preview),
        DocumentId(1),
        true,
        &PreviewOptions::default(),
    );
    // Rubber-band chain + point-marker crosses.
    assert_eq!(with.drawn(), 2);
    let without = preview_overlay(
        Some(&preview),
        DocumentId(1),
        false,
        &PreviewOptions::default(),
    );
    // The chain stays; only the cursor/snap crosses disappear.
    assert_eq!(without.drawn(), 1);
    assert_eq!(without.batches[0].topology, RenderTopology::Lines);
}

/// A four-segment rectangle on the work plane (the shape the host tessellates).
fn selection_box(crossing: bool) -> BoxSelectionPreview {
    BoxSelectionPreview {
        segments: vec![
            [p(0.0, 0.0), p(4.0, 0.0)],
            [p(4.0, 0.0), p(4.0, 3.0)],
            [p(4.0, 3.0), p(0.0, 3.0)],
            [p(0.0, 3.0), p(0.0, 0.0)],
        ],
        crossing,
    }
}

#[test]
fn window_box_is_one_blue_batch_at_the_box_selection_draw_order() {
    let selection = selection_box(false);
    let overlay = box_selection_overlay(&selection, DocumentId(1));
    assert_eq!(overlay.drawn(), 1, "the box shares one bounded batch");
    let batch = &overlay.batches[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert_eq!(batch.draw_order, BOX_SELECT_DRAW_ORDER);
    assert_eq!(batch.color, BOX_SELECT_WINDOW_COLOR);
    assert!(!batch.color_unresolved);
    // Segment count is preserved: two vertices per host segment, no markers.
    assert_eq!(segment_pairs(batch).len(), selection.segments.len());
    assert_eq!(
        batch.vertices.len(),
        selection.segments.len() * 2,
        "no point markers are invented"
    );
    assert!(batch.draw_order > SNAP_HINT_DRAW_ORDER && batch.draw_order < PREVIEW_DRAW_ORDER);
}

#[test]
fn crossing_box_is_green_at_the_same_draw_order() {
    let overlay = box_selection_overlay(&selection_box(true), DocumentId(1));
    assert_eq!(overlay.drawn(), 1);
    let batch = &overlay.batches[0];
    assert_eq!(batch.draw_order, BOX_SELECT_DRAW_ORDER);
    assert_eq!(batch.color, BOX_SELECT_CROSSING_COLOR);
    assert!(!batch.color_unresolved);
    assert_eq!(batch.vertices.len(), 8);
}

#[test]
fn empty_box_segments_yield_no_batch_and_empty_inputs() {
    let empty = BoxSelectionPreview {
        segments: Vec::new(),
        crossing: false,
    };
    let overlay = box_selection_overlay(&empty, DocumentId(1));
    assert!(overlay.is_empty(), "not an empty batch");
    assert!(overlay.diagnostics.is_empty());
    let inputs = OverlayInputs {
        selection_box: Some(empty),
        ..OverlayInputs::default()
    };
    assert!(
        !inputs.is_empty(),
        "a present box input participates in `OverlayInputs::is_empty`"
    );
}

#[test]
fn overlay_fingerprint_tracks_every_box_endpoint() {
    let baseline = overlay_fingerprint(&OverlayInputs::default());
    let with_box = OverlayInputs {
        selection_box: Some(selection_box(false)),
        ..OverlayInputs::default()
    };
    let hashed = overlay_fingerprint(&with_box);
    assert_ne!(baseline, hashed);

    // Moving exactly one endpoint (the live cursor while dragging) must
    // invalidate the transient overlay.
    let mut moved = selection_box(false);
    moved.segments[1][1].x += 0.5;
    let moved_inputs = OverlayInputs {
        selection_box: Some(moved),
        ..OverlayInputs::default()
    };
    assert_ne!(hashed, overlay_fingerprint(&moved_inputs));

    // Crossing vs WINDOW is a colour change, so it is part of the fingerprint.
    let crossing = OverlayInputs {
        selection_box: Some(selection_box(true)),
        ..OverlayInputs::default()
    };
    assert_ne!(hashed, overlay_fingerprint(&crossing));

    // Rebuilding with identical inputs is stable.
    assert_eq!(hashed, overlay_fingerprint(&with_box));
}

#[test]
fn entity_flags_fingerprint_ignores_the_box_selection() {
    let baseline = entity_flags_fingerprint(&OverlayInputs::default());
    let with_box = OverlayInputs {
        selection_box: Some(selection_box(true)),
        ..OverlayInputs::default()
    };
    assert_eq!(
        baseline,
        entity_flags_fingerprint(&with_box),
        "the box is live gesture feedback, not an entity flag"
    );
}

#[test]
fn rectang_preview_draws_the_closed_outline_between_the_two_corners() {
    // RECTANG reuses the existing `DrawPreview` points + cursor plumbing (no new
    // overlay channel): one captured corner plus the live cursor is previewed as
    // the closed four-corner outline, as a single `Lines` batch.
    let preview = DrawPreview {
        kind: DrawToolKind::Rectang,
        points: vec![p(1.0, 1.0)],
        picks: Vec::new(),
        cursor: Some(p(5.0, 4.0)),
        candidate: None,
        remaining: 1,
        selection_count: 0,
        ready: false,
    };
    let overlay = draw_preview_overlay(&preview, DocumentId(1), false, &PreviewOptions::default());
    assert_eq!(overlay.drawn(), 1);
    let batch = &overlay.batches[0];
    assert_eq!(batch.topology, RenderTopology::Lines);
    assert_eq!(
        batch.vertices.len(),
        5,
        "four corners with the last closing back onto the first"
    );
    // A degenerate span (the cursor on the corner's vertical line) has no
    // rectangle, so only the rubber-band chain is previewed.
    let degenerate = DrawPreview {
        cursor: Some(p(1.0, 9.0)),
        ..preview
    };
    let overlay = draw_preview_overlay(
        &degenerate,
        DocumentId(1),
        false,
        &PreviewOptions::default(),
    );
    assert_eq!(
        overlay.batches[0].vertices.len(),
        2,
        "corner + cursor chain, never a fabricated rectangle"
    );
}

mod ellipse_preview;
