//! `HATCHSETORIGIN <x> <y>` contract tests (AutoCAD 2026).
//!
//! The selector takes the pattern's **absolute** origin (a hatch-plane point) and
//! writes the same stored phase displacement `HATCHEDIT ORIGIN` writes. The
//! displacement is derived from the pattern's definition rows through
//! [`HatchGeometry::absolute_pattern_origin`] — the composed base point of the
//! first row — so the tests pin exactly the case where the two editors differ: a
//! hatch whose first row has a non-zero base (what an imported file looks like).
//!
//! Every refusal writes nothing, bumps no revision and records no undo entry.

use super::*;

use cad_domain::hatch::GradientStop;
use cad_domain::{
    HatchFill, HatchLoop, HatchLoopFlags, HatchPattern, HatchPatternLine, HatchPatternType,
    HatchPlane, HatchStyle,
};

/// One default layer plus the given entities, in model space.
fn build_app(entities: Vec<(u128, SemanticGeometry)>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Sheet1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    for (index, (id, geometry)) in entities.into_iter().enumerate() {
        b.insert_entity(ent(id, geometry, index as i64)).unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([21u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

/// A unit-square loop around the origin.
fn square_loop() -> HatchLoop {
    HatchLoop::new(
        vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]],
        vec![0.0; 4],
        HatchLoopFlags(HatchLoopFlags::POLYLINE),
    )
}

/// A pattern whose **first row base is not the plane origin**, plus a second row
/// with its own offset: exactly the shape an imported file gives us (the phase
/// lives in the rows, not in a dedicated origin group).
fn offset_base_pattern() -> HatchFill {
    HatchFill::Pattern(HatchPattern {
        name: "USER".into(),
        angle: 0.0,
        scale: 1.0,
        double: false,
        lines: vec![
            HatchPatternLine {
                angle: 0.0,
                base_x: 0.5,
                base_y: 1.0,
                offset_x: 0.0,
                offset_y: 2.0,
                dashes: Vec::new(),
            },
            HatchPatternLine {
                angle: std::f64::consts::FRAC_PI_2,
                base_x: 1.0,
                base_y: 2.0,
                offset_x: 0.0,
                offset_y: 3.0,
                dashes: Vec::new(),
            },
        ],
    })
}

/// A model-space hatch on `id` with `fill` and an explicit phase displacement.
fn hatch_entity(id: u128, fill: HatchFill, displacement: [f64; 2]) -> (u128, SemanticGeometry) {
    let (hatch, reasons) = HatchGeometry::from_import(
        vec![square_loop()],
        HatchPlane::default(),
        HatchStyle::Normal,
        fill,
        Vec::new(),
        None,
        HatchPatternType::Custom,
    );
    assert!(reasons.is_empty(), "{reasons:?}");
    (
        id,
        SemanticGeometry::Hatch(Box::new(hatch.with_pattern_origin(displacement))),
    )
}

fn set_origin(
    app: &mut Application,
    session: &mut SessionState,
    ids: &[u128],
    x: f64,
    y: f64,
) -> CadResult<CommandOutcome> {
    session.selection = SelectionSet::from_refs(ids.iter().map(|id| ref_of(*id)));
    execute(
        app,
        session,
        CommandId::HatchSetOrigin,
        CommandPayload::Annotation(Box::new(AnnotationBatchPayload::HatchSetOrigin(
            HatchOriginCommand { x, y },
        ))),
    )
}

fn stored_hatch(app: &Application, id: u128) -> HatchGeometry {
    let SemanticGeometry::Hatch(hatch) = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .cloned()
        .expect("the hatch must exist")
        .geometry
    else {
        panic!("entity {id} must stay a hatch");
    };
    *hatch
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// The absolute → displacement resolution
// ---------------------------------------------------------------------------

#[test]
fn the_argument_is_an_absolute_origin_resolved_through_the_stored_rows() {
    // First row base (0.5, 1), no displacement: the pattern's absolute origin is
    // (0.5, 1), so asking for (2, 3) must store the *difference* (1.5, 2), never
    // the verbatim (2, 3) an absolute→relative confusion would produce.
    let (mut app, mut session) =
        build_app(vec![hatch_entity(1, offset_base_pattern(), [0.0, 0.0])]);
    let before = stored_hatch(&app, 1);
    assert_eq!(before.absolute_pattern_origin(), Some([0.5, 1.0]));
    assert_eq!(before.pattern_origin(), [0.0, 0.0]);
    let revision_before = revision(&app);

    let out = set_origin(&mut app, &mut session, &[1], 2.0, 3.0).expect("HATCHSETORIGIN");
    assert_eq!(out.objects, vec![ObjectId(1)]);
    assert!(out
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "hatchsetorigin.applied"));
    assert_eq!(revision(&app).0, revision_before.0 + 1, "one revision");
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");

    let after = stored_hatch(&app, 1);
    assert_eq!(after.pattern_origin(), [1.5, 2.0]);
    assert_eq!(
        after.absolute_pattern_origin(),
        Some([2.0, 3.0]),
        "the pattern origin landed exactly on the requested point"
    );
    // Nothing else changed: the second row keeps its stored offset from the
    // reference row, and every non-fill field is untouched.
    assert_eq!(after.loops(), before.loops());
    assert_eq!(after.plane(), before.plane());
    assert_eq!(after.style(), before.style());
    assert_eq!(after.fill(), before.fill());
    assert_eq!(after.seeds(), before.seeds());
    assert_eq!(after.pixel_size(), before.pixel_size());
    assert_eq!(after.pattern_type(), before.pattern_type());

    // One undo restores the exact previous displacement.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert_eq!(stored_hatch(&app, 1).pattern_origin(), [0.0, 0.0]);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn an_existing_displacement_is_absorbed_not_accumulated() {
    // A hatch already displaced by (2, -1) has its absolute origin at (2.5, 0).
    // Asking for (0, 0) must land there, whatever the previous displacement was —
    // the command is absolute, never a relative nudge.
    let (mut app, mut session) =
        build_app(vec![hatch_entity(1, offset_base_pattern(), [2.0, -1.0])]);
    assert_eq!(
        stored_hatch(&app, 1).absolute_pattern_origin(),
        Some([2.5, 0.0])
    );
    set_origin(&mut app, &mut session, &[1], 0.0, 0.0).expect("absolute origin 0 0");
    assert_eq!(stored_hatch(&app, 1).pattern_origin(), [-0.5, -1.0]);
    assert_eq!(
        stored_hatch(&app, 1).absolute_pattern_origin(),
        Some([0.0, 0.0])
    );

    // And the same command run twice is idempotent (the second run is a no-op).
    let undo_before = undo_depth(&app);
    let out = set_origin(&mut app, &mut session, &[1], 0.0, 0.0).expect("repeat");
    assert_eq!(out.diagnostics[0].code, "hatchsetorigin.unchanged");
    assert!(out.changes.is_none(), "a no-op opens no transaction");
    assert_eq!(undo_depth(&app), undo_before);
}

#[test]
fn requesting_the_current_absolute_origin_is_a_named_no_op() {
    let (mut app, mut session) =
        build_app(vec![hatch_entity(1, offset_base_pattern(), [4.0, 5.0])]);
    let anchor = stored_hatch(&app, 1)
        .absolute_pattern_origin()
        .expect("a pattern has an origin");
    let revision_before = revision(&app);
    let out = set_origin(&mut app, &mut session, &[1], anchor[0], anchor[1]).expect("no-op");
    assert_eq!(out.diagnostics[0].code, "hatchsetorigin.unchanged");
    assert!(out.changes.is_none());
    assert_eq!(revision(&app).0, revision_before.0);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(stored_hatch(&app, 1).pattern_origin(), [4.0, 5.0]);
}

#[test]
fn the_composed_base_follows_the_stored_pattern_angle_and_scale() {
    // The rows are stored in pattern space; the hatch-level angle and scale are
    // composed in by `HatchPattern::families`, so the absolute origin is not the
    // raw `base` pair. Row base (1, 0) at 90 degrees and scale 2 sits at (0, 2).
    let fill = HatchFill::Pattern(HatchPattern {
        name: "USER".into(),
        angle: std::f64::consts::FRAC_PI_2,
        scale: 2.0,
        double: false,
        lines: vec![HatchPatternLine {
            angle: 0.0,
            base_x: 1.0,
            base_y: 0.0,
            offset_x: 0.0,
            offset_y: 1.0,
            dashes: Vec::new(),
        }],
    });
    let (mut app, mut session) = build_app(vec![hatch_entity(1, fill, [0.0, 0.0])]);
    set_origin(&mut app, &mut session, &[1], 0.0, 0.0).expect("absolute origin 0 0");
    // (0, 0) - (0, 2) = (0, -2).
    let stored = stored_hatch(&app, 1);
    assert!((stored.pattern_origin()[0] - 0.0).abs() < 1e-12);
    assert!((stored.pattern_origin()[1] + 2.0).abs() < 1e-12);
    let anchor = stored.absolute_pattern_origin().expect("an origin");
    assert!(anchor[0].abs() < 1e-12, "{anchor:?}");
    assert!(anchor[1].abs() < 1e-12, "{anchor:?}");
}

// ---------------------------------------------------------------------------
// Refusals (nothing written)
// ---------------------------------------------------------------------------

#[test]
fn a_solid_or_gradient_fill_is_refused() {
    for fill in [HatchFill::Solid, {
        // The default gradient HATCHEDIT GRADIENT writes.
        HatchFill::Gradient(HatchGradient::TwoColor {
            name: "LINEAR".into(),
            reserved: 0,
            angle_raw: 0.0,
            shift: 0.0,
            stops: vec![
                GradientStop {
                    value: 0.0,
                    rgb: [255, 0, 0],
                },
                GradientStop {
                    value: 1.0,
                    rgb: [0, 0, 255],
                },
            ],
        })
    }] {
        let kind = match &fill {
            HatchFill::Solid => "a solid fill",
            _ => "a gradient fill",
        };
        let (mut app, mut session) = build_app(vec![hatch_entity(1, fill, [0.0, 0.0])]);
        let revision_before = revision(&app);
        let error = set_origin(&mut app, &mut session, &[1], 1.0, 2.0)
            .expect_err("a fill without rows has no origin");
        let text = format!("{error:?}");
        assert!(text.contains("hatchsetorigin.not_pattern"), "{text}");
        assert!(text.contains(kind), "{text}");
        assert_eq!(revision(&app).0, revision_before.0);
        assert_eq!(undo_depth(&app), 0);
        assert_eq!(stored_hatch(&app, 1).pattern_origin(), [0.0, 0.0]);
    }
}

#[test]
fn a_pattern_without_definition_rows_keeps_a_named_refusal() {
    // A user-defined pattern carrying only a spacing has no base point at all, so
    // there is no origin this model can name: the (narrowed) `missing_field`
    // refusal stays, and it says what to use instead.
    let fill = HatchFill::Pattern(HatchPattern {
        name: "USER".into(),
        angle: 0.0,
        scale: 2.0,
        double: false,
        lines: Vec::new(),
    });
    // `from_import` declares the degradation ("no line families") instead of
    // rejecting the stored hatch, exactly like an imported file would.
    let (hatch, reasons) = HatchGeometry::from_import(
        vec![square_loop()],
        HatchPlane::default(),
        HatchStyle::Normal,
        fill,
        Vec::new(),
        None,
        HatchPatternType::UserDefined,
    );
    assert_eq!(
        reasons,
        vec!["hatch pattern has no line families".to_string()]
    );
    let (mut app, mut session) = build_app(vec![(1, SemanticGeometry::Hatch(Box::new(hatch)))]);
    let revision_before = revision(&app);
    let error = set_origin(&mut app, &mut session, &[1], 1.0, 2.0)
        .expect_err("no rows means no derivable origin");
    let text = format!("{error:?}");
    assert!(text.contains("hatchsetorigin.missing_field"), "{text}");
    assert!(text.contains("HATCHEDIT ORIGIN"), "{text}");
    assert_eq!(revision(&app).0, revision_before.0);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_non_finite_argument_is_refused() {
    let (mut app, mut session) =
        build_app(vec![hatch_entity(1, offset_base_pattern(), [0.0, 0.0])]);
    for (x, y) in [(f64::NAN, 0.0), (0.0, f64::INFINITY)] {
        let error = set_origin(&mut app, &mut session, &[1], x, y)
            .expect_err("a non-finite origin is refused");
        assert!(
            format!("{error:?}").contains("hatchsetorigin.bad_point"),
            "{error:?}"
        );
    }
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(stored_hatch(&app, 1).pattern_origin(), [0.0, 0.0]);
}

#[test]
fn the_selection_must_be_exactly_one_hatch() {
    let (mut app, mut session) = build_app(vec![
        hatch_entity(1, offset_base_pattern(), [0.0, 0.0]),
        hatch_entity(2, offset_base_pattern(), [0.0, 0.0]),
        (
            3,
            SemanticGeometry::Line {
                start: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                end: Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
        ),
    ]);
    let revision_before = revision(&app);

    // Empty selection.
    let error = set_origin(&mut app, &mut session, &[], 1.0, 1.0).expect_err("empty");
    assert!(
        format!("{error:?}").contains("hatchsetorigin.selection"),
        "{error:?}"
    );
    // Two hatches: exactly one target.
    let error = set_origin(&mut app, &mut session, &[1, 2], 1.0, 1.0).expect_err("two hatches");
    assert!(
        format!("{error:?}").contains("hatchsetorigin.selection"),
        "{error:?}"
    );
    // A non-hatch entity.
    let error = set_origin(&mut app, &mut session, &[3], 1.0, 1.0).expect_err("a line");
    assert!(
        format!("{error:?}").contains("hatchsetorigin.not_hatch"),
        "{error:?}"
    );
    // A block-instance / sub-element ref is not directly addressable.
    session.selection = SelectionSet::from_refs([SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath(vec![EntityId(3)]),
        sub_element: None,
    }]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchSetOrigin,
        CommandPayload::Annotation(Box::new(AnnotationBatchPayload::HatchSetOrigin(
            HatchOriginCommand { x: 1.0, y: 1.0 },
        ))),
    )
    .expect_err("an instance-path ref is refused");
    assert!(
        format!("{error:?}").contains("hatchsetorigin.not_addressable"),
        "{error:?}"
    );

    assert_eq!(revision(&app).0, revision_before.0);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(stored_hatch(&app, 1).pattern_origin(), [0.0, 0.0]);
    assert_eq!(stored_hatch(&app, 2).pattern_origin(), [0.0, 0.0]);
}

#[test]
fn a_missing_or_wrong_payload_is_refused() {
    let (mut app, mut session) =
        build_app(vec![hatch_entity(1, offset_base_pattern(), [0.0, 0.0])]);
    session.selection = SelectionSet::from_refs([ref_of(1)]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchSetOrigin,
        CommandPayload::None,
    )
    .expect_err("a bare selector is refused");
    assert!(
        format!("{error:?}").contains("hatchsetorigin.needs_argument"),
        "{error:?}"
    );
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn viewer_mode_refuses_the_write() {
    let (mut app, mut session) =
        build_app(vec![hatch_entity(1, offset_base_pattern(), [0.0, 0.0])]);
    session.switch_mode(AppMode::Viewer).unwrap();
    let revision_before = revision(&app);
    let error = set_origin(&mut app, &mut session, &[1], 1.0, 1.0)
        .expect_err("Viewer mode refuses a write");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(revision(&app).0, revision_before.0);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(stored_hatch(&app, 1).pattern_origin(), [0.0, 0.0]);
}
