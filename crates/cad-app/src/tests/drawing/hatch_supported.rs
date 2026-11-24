//! `HATCH` / `-HATCH` supported-domain completeness contract.
//!
//! The command was partial only because hatch **associativity** and the incident
//! real-time preview/dialog have no model in this build — both are build-wide
//! scope declarations (see the coverage document's global-scope list), not a
//! refusal path inside the supported domain. These tests are the evidence for
//! that reading: every supported boundary kind, every supported fill mode and
//! both command spellings (`HATCH` and `-HATCH`) are exercised to a real write,
//! so there is no remaining `Unsupported`/`InvalidInput` branch on the supported
//! surface. The boundary-defect refusals (open chain, branch, degenerate,
//! non-coplanar, self-intersecting) stay refusal-only by design and are covered
//! by the sibling `hatch*` tests.

use super::*;

use super::hatch::{circle, entity_count, hatch_app, set_flag, set_number, square, undo_depth};
use crate::command_id::hatch::HatchPatternRequest;
use crate::{PatternFileText, HATCH_PATTERN_TYPE_GRADIENT, HATCH_PATTERN_TYPE_PREDEFINED};
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer, Layout};

const TAU: f64 = std::f64::consts::TAU;

/// The two spellings of the same handler.
fn entries() -> [CommandId; 2] {
    [
        CommandId::Hatch,
        CommandId::Extended(ExtendedCommandId::DashHatch),
    ]
}

fn full_ellipse(id: u128) -> (u128, SemanticGeometry) {
    (
        id,
        SemanticGeometry::Ellipse {
            center: p3(0.0, 0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(5.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: TAU,
        },
    )
}

fn line(id: u128, start: (f64, f64), end: (f64, f64)) -> (u128, SemanticGeometry) {
    (
        id,
        SemanticGeometry::Line {
            start: p3(start.0, start.1, 0.0),
            end: p3(end.0, end.1, 0.0),
        },
    )
}

fn hatch_payload(ids: &[u128]) -> CommandPayload {
    CommandPayload::Hatch {
        refs: ids.iter().copied().map(ref_of).collect(),
    }
}

fn run(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    execute(app, session, id, payload)
}

#[test]
fn hatch_and_dash_hatch_accept_every_supported_boundary_kind() {
    let boundaries: Vec<Vec<(u128, SemanticGeometry)>> = vec![
        vec![circle(1, 0.0, 0.0, 0.0, 5.0)],
        vec![full_ellipse(1)],
        vec![square(1, 0.0, 10.0, 0.0)],
        // A closed LINE chain (the stitching path).
        vec![
            line(1, (0.0, 0.0), (10.0, 0.0)),
            line(2, (10.0, 0.0), (10.0, 10.0)),
            line(3, (10.0, 10.0), (0.0, 10.0)),
            line(4, (0.0, 10.0), (0.0, 0.0)),
        ],
    ];
    for entry in entries() {
        for entities in &boundaries {
            let (mut app, mut session) = hatch_app(entities.clone());
            let before = revision(&app);
            let refs: Vec<u128> = entities.iter().map(|(id, _)| *id).collect();
            let outcome = run(&mut app, &mut session, entry, hatch_payload(&refs))
                .unwrap_or_else(|error| panic!("{entry:?} on {refs:?}: {error:?}"));
            assert_eq!(outcome.objects.len(), 1, "{entry:?} creates one hatch");
            assert_eq!(revision(&app).0, before.0 + 1, "{entry:?}: one revision");
            assert_eq!(undo_depth(&app), 1, "{entry:?}: one undo step");
            assert_eq!(
                entity_count(&app),
                entities.len() + 1,
                "{entry:?}: the boundary entities are retained"
            );
        }
    }
}

type FillMode = (&'static str, fn(&mut SessionState));

/// A drawing with layer `0`, the empty layout `Sheet1` (id 1) and the given
/// entities in their own spaces (model or that layout).
fn space_app(entities: Vec<(u128, SemanticGeometry, SpaceId)>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    for (index, (id, geometry, space)) in entities.into_iter().enumerate() {
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(id),
                    type_key: "AcDbEntity".into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: EntityId(id),
                layer: LayerId(0),
                space,
                geometry,
                draw_order: index as i64,
            })
            .unwrap();
    }
    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([79u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
            compat: Vec::new(),
            sources: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

#[test]
fn hatch_and_dash_hatch_write_into_the_boundary_space() {
    for entry in entries() {
        let (square_id, square_geometry) = square(1, 0.0, 10.0, 0.0);
        let (mut app, mut session) = space_app(vec![(
            square_id,
            square_geometry,
            SpaceId::Paper(LayoutId(1)),
        )]);
        let before = revision(&app);
        let outcome = run(&mut app, &mut session, entry, hatch_payload(&[square_id]))
            .unwrap_or_else(|error| panic!("{entry:?} on a paper boundary: {error:?}"));
        let object = EntityId(outcome.objects[0].0);
        let stored = app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(object)
            .expect("the created hatch");
        assert_eq!(
            stored.space,
            SpaceId::Paper(LayoutId(1)),
            "{entry:?}: the hatch follows its boundary into the layout"
        );
        assert_eq!(revision(&app), Revision(before.0 + 1), "{entry:?}");
        assert_eq!(undo_depth(&app), 1, "{entry:?}: one undo step");
        assert_eq!(
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .layout_entities(LayoutId(1))
                .len(),
            2,
            "{entry:?}: the boundary is retained beside the hatch"
        );
    }
}

#[test]
fn hatch_refuses_a_mixed_space_boundary_without_writing() {
    let (model_id, model_geometry) = square(1, 0.0, 10.0, 0.0);
    let (paper_id, paper_geometry) = square(2, 20.0, 30.0, 0.0);
    let (mut app, mut session) = space_app(vec![
        (model_id, model_geometry, SpaceId::Model),
        (paper_id, paper_geometry, SpaceId::Paper(LayoutId(1))),
    ]);
    let before = revision(&app);
    let error = run(
        &mut app,
        &mut session,
        CommandId::Hatch,
        hatch_payload(&[model_id, paper_id]),
    )
    .expect_err("a mixed-space boundary is refused");
    assert!(
        format!("{error:?}").contains("hatch.mixed_space"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before, "nothing was written");
    assert!(!app.can_undo(&DocumentId(1)), "no transaction was opened");
}

#[test]
fn hatch_and_dash_hatch_accept_every_supported_fill_mode() {
    // Each closure configures one supported fill mode on the session channel.
    let fill_modes: Vec<FillMode> = vec![
        // Solid is the session default; make it explicit.
        ("solid", |_| {}),
        ("user-defined pattern", |session| {
            set_number(session, ToolParameter::HatchPatternType, 1.0);
            set_number(session, ToolParameter::HatchSpacing, 2.0);
            set_number(session, ToolParameter::HatchAngle, 15.0);
            set_flag(session, ToolParameter::HatchDouble, true);
        }),
        ("predefined pattern", |session| {
            set_number(
                session,
                ToolParameter::HatchPatternType,
                HATCH_PATTERN_TYPE_PREDEFINED,
            );
            set_number(session, ToolParameter::HatchPattern, 1.0);
            set_number(session, ToolParameter::HatchSpacing, 3.0);
        }),
        ("gradient", |session| {
            set_number(
                session,
                ToolParameter::HatchPatternType,
                HATCH_PATTERN_TYPE_GRADIENT,
            );
            set_number(session, ToolParameter::HatchGradientAngle, 30.0);
        }),
    ];
    for entry in entries() {
        for (name, configure) in &fill_modes {
            let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
            configure(&mut session);
            let before = revision(&app);
            let outcome = run(&mut app, &mut session, entry, hatch_payload(&[1]))
                .unwrap_or_else(|error| panic!("{entry:?} {name}: {error:?}"));
            assert_eq!(outcome.objects.len(), 1, "{entry:?} {name}");
            assert_eq!(revision(&app).0, before.0 + 1, "{entry:?} {name}");
            assert_eq!(undo_depth(&app), 1, "{entry:?} {name}");
        }
    }
}

#[test]
fn hatch_and_dash_hatch_accept_both_typed_pattern_forms() {
    const LADDER: &str = "\
; a test library
*LADDER, rungs
0, 0,0, 0,.5
90, 0,0, 1,0, .25,-.25
";
    let requests = [
        HatchPatternRequest::Named("ansi31".into()),
        HatchPatternRequest::File {
            path: "/tmp/acad.pat".into(),
            name: None,
            text: PatternFileText::new(LADDER),
        },
    ];
    for entry in entries() {
        for request in &requests {
            let (mut app, mut session) = hatch_app(vec![square(1, 0.0, 10.0, 0.0)]);
            let before = revision(&app);
            let payload = CommandPayload::HatchPattern {
                refs: vec![ref_of(1)],
                request: request.clone(),
            };
            let outcome = run(&mut app, &mut session, entry, payload)
                .unwrap_or_else(|error| panic!("{entry:?} {request:?}: {error:?}"));
            assert_eq!(outcome.objects.len(), 1, "{entry:?} {request:?}");
            assert_eq!(revision(&app).0, before.0 + 1, "{entry:?} {request:?}");
            assert_eq!(undo_depth(&app), 1, "{entry:?}: one undo step");
        }
    }
}
