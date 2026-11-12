//! `OVERKILL` (Delete Duplicate Objects) contract tests.
//!
//! Pins the documented model end to end against a real database: the tolerance
//! (0 = exact) and its effect on comparison, the modelled ignore toggles, the
//! collinear combines (partial overlap and end-to-end), arc-over-circle removal,
//! polyline segment optimisation / cross-object duplicate segments, the
//! `Do Not Break Polylines` switch, the named unmodelled-property refusals and the
//! one-transaction / one-undo rule.

use super::*;
use cad_db::{
    DbEntity, DbObject, DrawingDatabaseBuilder, EntityColor, EntityRenderAttributes, Layer,
};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn ent(id: u128, layer: LayerId, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer,
        space: SpaceId::Model,
        geometry,
        draw_order: order,
    }
}

fn build(entities: Vec<DbEntity>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "WALLS")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

fn ref_of(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection =
        SelectionSet::from_refs(ids.iter().copied().map(ref_of).collect::<Vec<_>>());
}

fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: p3(x0, y0, 0.0),
        end: p3(x1, y1, 0.0),
    }
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

fn geometry(app: &Application, id: u128) -> Option<SemanticGeometry> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .map(|entity| entity.geometry.clone())
}

fn present(app: &Application, id: u128) -> bool {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .is_some()
}

fn payload(settings: OverkillSettings) -> CommandPayload {
    CommandPayload::SelectionTool(Box::new(SelectionToolPayload::Overkill(Box::new(settings))))
}

fn run(
    app: &mut Application,
    session: &mut SessionState,
    settings: OverkillSettings,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(CommandId::Overkill, payload(settings)))
}

fn codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

#[test]
fn overkill_deletes_duplicates_and_keeps_the_lowest_draw_order() {
    let mut reversed = line(0.0, 0.0, 10.0, 0.0);
    if let SemanticGeometry::Line { start, end } = &mut reversed {
        std::mem::swap(start, end);
    }
    let (mut app, mut session) = build(vec![
        ent(1, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 5),
        ent(2, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 1),
        // Same unordered endpoint pair: a duplicate.
        ent(3, LayerId(0), reversed, 9),
        // Same geometry on another layer: the default compares layers.
        ent(5, LayerId(7), line(0.0, 0.0, 10.0, 0.0), 0),
        // A different line is never merged (not collinear).
        ent(4, LayerId(0), line(0.0, 0.0, 10.0, 5.0), 2),
    ]);
    select(&mut session, &[1, 2, 3, 4, 5]);
    let before = revision(&app);
    let outcome = run(&mut app, &mut session, OverkillSettings::default()).unwrap();

    assert!(
        codes(&outcome).contains(&"overkill.applied"),
        "{:?}",
        outcome.diagnostics
    );
    assert!(!present(&app, 1), "higher draw_order duplicate deleted");
    assert!(present(&app, 2), "lowest draw_order is the keeper");
    assert!(!present(&app, 3), "reversed duplicate deleted");
    assert!(present(&app, 4), "a different line stays");
    assert!(present(&app, 5), "a cross-layer duplicate is not compared");
    assert_eq!(entity_count(&app), 3);
    assert_ne!(revision(&app), before);
    assert_eq!(undo_depth(&app), 1);

    // One undo restores both deleted rows.
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
    assert_eq!(entity_count(&app), 5);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn overkill_tolerance_zero_is_exact_and_a_positive_tolerance_compares() {
    let near = || {
        vec![
            ent(1, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 0),
            // Every coordinate is off by 0.001.
            ent(2, LayerId(0), line(0.001, 0.001, 10.001, 0.001), 1),
        ]
    };
    let (mut app, mut session) = build(near());
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let exact = OverkillSettings {
        tolerance: 0.0,
        ..OverkillSettings::default()
    };
    let outcome = run(&mut app, &mut session, exact).unwrap();
    assert!(codes(&outcome).contains(&"overkill.unchanged"));
    assert_eq!(entity_count(&app), 2);
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);

    let (mut app, mut session) = build(near());
    select(&mut session, &[1, 2]);
    let tolerant = OverkillSettings {
        tolerance: 0.01,
        ..OverkillSettings::default()
    };
    run(&mut app, &mut session, tolerant).unwrap();
    assert_eq!(entity_count(&app), 1);
    assert!(present(&app, 1), "the lower draw_order survives");
}

#[test]
fn overkill_ignore_layer_and_ignore_color_are_honoured() {
    let (mut app, mut session) = build(vec![
        ent(1, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 0),
        ent(2, LayerId(7), line(0.0, 0.0, 10.0, 0.0), 1),
    ]);
    select(&mut session, &[1, 2]);
    run(&mut app, &mut session, OverkillSettings::default()).unwrap();
    assert_eq!(entity_count(&app), 2, "different layers are compared");

    let (mut app, mut session) = build(vec![
        ent(1, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 0),
        ent(2, LayerId(7), line(0.0, 0.0, 10.0, 0.0), 1),
    ]);
    select(&mut session, &[1, 2]);
    let settings = OverkillSettings {
        ignore_layer: true,
        ..OverkillSettings::default()
    };
    run(&mut app, &mut session, settings).unwrap();
    assert_eq!(entity_count(&app), 1, "ignore-layer makes them duplicates");
    assert!(present(&app, 1));

    // Colour: same geometry and layer, one explicitly red.
    let colored = || {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
        builder
            .insert_layer(Layer {
                id: LayerId(0),
                name: "0".into(),
                visible: true,
            })
            .unwrap();
        builder
            .insert_entity(ent(1, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 0))
            .unwrap();
        builder
            .insert_entity(ent(2, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 1))
            .unwrap();
        builder
            .set_entity_render_attributes(
                EntityId(2),
                EntityRenderAttributes {
                    clip: None,
                    color: EntityColor::Explicit([255, 0, 0]),
                    ..EntityRenderAttributes::default()
                },
            )
            .unwrap();
        let drawing = builder.finish().unwrap();
        let mut app = Application::new();
        app.workspace.documents.insert(
            DocumentId(1),
            Document {
                id: DocumentId(1),
                drawing: Arc::new(drawing),
                identity: DocumentIdentity::Sha256([9u8; 32]),
                units: UnitContext::drawing_units(),
                resource_keys: Vec::new(),
            },
        );
        app.workspace.viewports.insert(
            ViewportId(1),
            Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
        );
        (app, SessionState::new(DocumentId(1), AppMode::Work))
    };

    let (mut app, mut session) = colored();
    select(&mut session, &[1, 2]);
    run(&mut app, &mut session, OverkillSettings::default()).unwrap();
    assert_eq!(entity_count(&app), 2, "different colours are compared");

    let (mut app, mut session) = colored();
    select(&mut session, &[1, 2]);
    let settings = OverkillSettings {
        ignore_color: true,
        ..OverkillSettings::default()
    };
    run(&mut app, &mut session, settings).unwrap();
    assert_eq!(entity_count(&app), 1, "ignore-colour makes them duplicates");
    assert!(present(&app, 1));
}

#[test]
fn overkill_combines_partially_overlapping_collinear_lines() {
    let pieces = || {
        vec![
            ent(1, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 0),
            ent(2, LayerId(0), line(5.0, 0.0, 20.0, 0.0), 1),
        ]
    };
    let (mut app, mut session) = build(pieces());
    select(&mut session, &[1, 2]);
    let before = revision(&app);
    let outcome = run(&mut app, &mut session, OverkillSettings::default()).unwrap();
    assert!(codes(&outcome).contains(&"overkill.applied"));
    assert_eq!(entity_count(&app), 1, "the pair becomes one LINE");
    assert!(present(&app, 1), "the lower draw_order is the keeper");
    assert_eq!(
        geometry(&app, 1).unwrap(),
        line(0.0, 0.0, 20.0, 0.0),
        "the keeper stores the union"
    );
    assert_ne!(revision(&app), before);
    assert_eq!(undo_depth(&app), 1);

    // One undo restores both rows and the keeper's original geometry.
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .unwrap();
    assert_eq!(entity_count(&app), 2);
    assert_eq!(geometry(&app, 1).unwrap(), line(0.0, 0.0, 10.0, 0.0));

    // The option gates the partial-overlap combine.
    let (mut app, mut session) = build(pieces());
    select(&mut session, &[1, 2]);
    let settings = OverkillSettings {
        combine_partial_overlap: false,
        ..OverkillSettings::default()
    };
    let outcome = run(&mut app, &mut session, settings).unwrap();
    assert!(codes(&outcome).contains(&"overkill.unchanged"));
    assert_eq!(entity_count(&app), 2);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn overkill_combines_end_to_end_collinear_lines() {
    let pieces = || {
        vec![
            ent(1, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 0),
            ent(2, LayerId(0), line(10.0, 0.0, 20.0, 0.0), 1),
        ]
    };
    let (mut app, mut session) = build(pieces());
    select(&mut session, &[1, 2]);
    run(&mut app, &mut session, OverkillSettings::default()).unwrap();
    assert_eq!(entity_count(&app), 1);
    assert_eq!(geometry(&app, 1).unwrap(), line(0.0, 0.0, 20.0, 0.0));

    let (mut app, mut session) = build(pieces());
    select(&mut session, &[1, 2]);
    let settings = OverkillSettings {
        combine_end_to_end: false,
        ..OverkillSettings::default()
    };
    let outcome = run(&mut app, &mut session, settings).unwrap();
    assert!(codes(&outcome).contains(&"overkill.unchanged"));
    assert_eq!(entity_count(&app), 2);
}

#[test]
fn overkill_deletes_an_arc_drawn_over_a_circle() {
    let normal = p3(0.0, 0.0, 1.0);
    let (mut app, mut session) = build(vec![
        ent(
            1,
            LayerId(0),
            SemanticGeometry::Circle {
                center: p3(0.0, 0.0, 0.0),
                normal,
                radius: 5.0,
            },
            0,
        ),
        ent(
            2,
            LayerId(0),
            SemanticGeometry::Arc {
                center: p3(0.0, 0.0, 0.0),
                normal,
                radius: 5.0,
                start: 0.0,
                sweep: std::f64::consts::FRAC_PI_2,
            },
            1,
        ),
    ]);
    select(&mut session, &[1, 2]);
    run(&mut app, &mut session, OverkillSettings::default()).unwrap();
    assert_eq!(entity_count(&app), 1);
    assert!(present(&app, 1), "the circle stays");
    assert!(!present(&app, 2), "the redundant arc is deleted");
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn overkill_optimizes_polyline_segments_and_duplicate_standalone_segments() {
    let fixture = || {
        vec![
            ent(
                1,
                LayerId(0),
                SemanticGeometry::Polyline {
                    // One zero-length segment: the middle point is repeated.
                    points: vec![
                        p3(0.0, 0.0, 0.0),
                        p3(5.0, 0.0, 0.0),
                        p3(5.0, 0.0, 0.0),
                        p3(10.0, 0.0, 0.0),
                    ],
                    bulges: vec![0.0; 4],
                    closed: false,
                    width: 0.0,
                },
                0,
            ),
            // Exactly the polyline's first segment.
            ent(2, LayerId(0), line(0.0, 0.0, 5.0, 0.0), 1),
        ]
    };
    let polyline_points = |app: &Application| match geometry(app, 1).unwrap() {
        SemanticGeometry::Polyline { points, .. } => points.len(),
        other => panic!("expected a polyline, got {other:?}"),
    };

    let (mut app, mut session) = build(fixture());
    select(&mut session, &[1, 2]);
    run(&mut app, &mut session, OverkillSettings::default()).unwrap();
    assert_eq!(
        entity_count(&app),
        1,
        "the duplicate standalone line is gone"
    );
    assert_eq!(
        polyline_points(&app),
        3,
        "the zero-length segment was removed"
    );
    assert_eq!(undo_depth(&app), 1);

    // `Do Not Break Polylines` leaves the polyline object unchanged, while the
    // cross-object segment comparison still removes the standalone duplicate.
    let (mut app, mut session) = build(fixture());
    select(&mut session, &[1, 2]);
    let settings = OverkillSettings {
        do_not_break_polylines: true,
        ..OverkillSettings::default()
    };
    run(&mut app, &mut session, settings).unwrap();
    assert_eq!(entity_count(&app), 1);
    assert_eq!(polyline_points(&app), 4);

    // Optimisation off: neither pass runs.
    let (mut app, mut session) = build(fixture());
    select(&mut session, &[1, 2]);
    let settings = OverkillSettings {
        optimize_segments: false,
        ..OverkillSettings::default()
    };
    let outcome = run(&mut app, &mut session, settings).unwrap();
    assert!(codes(&outcome).contains(&"overkill.unchanged"));
    assert_eq!(entity_count(&app), 2);
    assert_eq!(polyline_points(&app), 4);
}

#[test]
fn overkill_reports_a_per_segment_bulge_layout_without_guessing() {
    let (mut app, mut session) = build(vec![ent(
        1,
        LayerId(0),
        SemanticGeometry::Polyline {
            points: vec![p3(0.0, 0.0, 0.0), p3(5.0, 0.0, 0.0), p3(5.0, 5.0, 0.0)],
            // Per-segment layout (n - 1 entries): not rewritten, reported.
            bulges: vec![0.5, 0.5],
            closed: false,
            width: 0.0,
        },
        0,
    )]);
    select(&mut session, &[1]);
    let outcome = run(&mut app, &mut session, OverkillSettings::default()).unwrap();
    assert!(codes(&outcome).contains(&"overkill.polyline_bulge_layout"));
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn overkill_refuses_unmodelled_comparisons_and_reports_the_named_noops() {
    for (mutate, code) in [
        (
            (|settings: &mut OverkillSettings| settings.ignore_thickness = false)
                as fn(&mut OverkillSettings),
            "overkill.thickness_not_modelled",
        ),
        (
            (|settings: &mut OverkillSettings| settings.ignore_plot_style = false)
                as fn(&mut OverkillSettings),
            "overkill.plot_style_not_modelled",
        ),
        (
            (|settings: &mut OverkillSettings| settings.ignore_material = false)
                as fn(&mut OverkillSettings),
            "overkill.material_not_modelled",
        ),
    ] {
        let (mut app, mut session) = build(vec![
            ent(1, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 0),
            ent(2, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 1),
        ]);
        select(&mut session, &[1, 2]);
        let before = revision(&app);
        let mut settings = OverkillSettings::default();
        mutate(&mut settings);
        let error = run(&mut app, &mut session, settings).unwrap_err();
        assert!(
            matches!(error, CadError::Unsupported(ref reason) if reason.starts_with(code)),
            "expected {code}, got {error:?}"
        );
        assert_eq!(entity_count(&app), 2);
        assert_eq!(revision(&app), before);
        assert_eq!(undo_depth(&app), 0);
    }

    // `Maintain associative objects` is a named no-op, not a failure.
    let (mut app, mut session) = build(vec![
        ent(1, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 0),
        ent(2, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 1),
    ]);
    select(&mut session, &[1, 2]);
    let settings = OverkillSettings {
        maintain_associative: true,
        ..OverkillSettings::default()
    };
    let outcome = run(&mut app, &mut session, settings).unwrap();
    assert!(codes(&outcome).contains(&"overkill.associative_noop"));
    assert!(codes(&outcome).contains(&"overkill.segment_width_not_modelled"));
    assert_eq!(entity_count(&app), 1);
}

#[test]
fn overkill_refuses_empty_selection_bad_tolerance_and_foreign_payloads() {
    let (mut app, mut session) = build(vec![ent(1, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 0)]);
    let before = revision(&app);
    let error = run(&mut app, &mut session, OverkillSettings::default()).unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref reason) if reason.starts_with("overkill.selection"))
    );

    select(&mut session, &[1]);
    let settings = OverkillSettings {
        tolerance: -1.0,
        ..OverkillSettings::default()
    };
    let error = run(&mut app, &mut session, settings).unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref reason) if reason.starts_with("overkill.tolerance"))
    );

    // A ref that does not resolve is a named refusal, not a silent skip.
    select(&mut session, &[999]);
    let error = run(&mut app, &mut session, OverkillSettings::default()).unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref reason) if reason.starts_with("overkill.target_missing"))
    );

    let error = app
        .execute(
            &mut session,
            command(
                CommandId::Overkill,
                CommandPayload::SelectionTool(Box::new(SelectionToolPayload::SetByLayer(
                    SetByLayerSettings::default(),
                ))),
            ),
        )
        .unwrap_err();
    assert!(
        matches!(error, CadError::InvalidInput(ref reason) if reason.starts_with("overkill.invalid_payload"))
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn overkill_is_refused_in_viewer_mode() {
    let (mut app, mut session) = build(vec![
        ent(1, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 0),
        ent(2, LayerId(0), line(0.0, 0.0, 10.0, 0.0), 1),
    ]);
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    select(&mut viewer, &[1, 2]);
    let before = revision(&app);
    let error = run(&mut app, &mut viewer, OverkillSettings::default()).unwrap_err();
    assert_eq!(error, CadError::PermissionDenied);
    assert_eq!(entity_count(&app), 2);
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    let _ = &mut session;
}
