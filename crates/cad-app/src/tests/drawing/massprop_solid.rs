//! ADR-0036 phase C3a: `MASSPROP` for exact `Body3d` solids.
//!
//! The contracts use a **fake** [`SolidModeler`] that answers the
//! `MassProperties` operation with a hand-written unit-box mass, so the command
//! layer is pinned independently of the kernel. The real kernel is exercised by
//! the `cad-cli-tools` integration contract (`tests/solid_command.rs`).
//!
//! Pinned here: one `massprop.object` row per selected body carrying volume,
//! surface area, centroid, moments, products and the **requested** accuracy
//! wording; body-face refs resolving to the owning body (once); every named
//! refusal (`body_modeler_unavailable`, `body_not_liftable`, `body_kernel_failed`,
//! `body_not_exact`, `body_not_solid`, `body_not_positive_volume`,
//! `mixed_selection`, `kernel_busy`) with zero revision/undo change; per-body
//! all-or-nothing failure; and that a display mesh is never integrated as the
//! solid.

use super::*;
use crate::tasks::{SolidBooleanOperand, SolidBooleanPlan};
use crate::tests::inquiry::app_from_builder;
use cad_db::{DrawingDatabaseBuilder, Layer};
use cad_kernel_adapter::{
    BooleanOp, MassAccuracy, ModelerCapabilities, ModelerPrimitives, SolidMassProperties,
    SolidModeler, SolidOperation, SolidOutcome, SolidProduct, SolidRequest, SolidResult,
    SolidUnsupportedReason,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::solids::{p, unit_box};

const BODY_A: u128 = 900;
const BODY_B: u128 = 901;
const CIRCLE_A: u128 = 902;

// ---- the fake modeler -----------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Mass,
    Unsupported,
    Failed,
    /// The kernel integrated a reflected/inside-out body: negative volume.
    NegativeVolume,
}

struct FakeMassModeler {
    /// Answers by call index; the last one repeats.
    answers: Vec<Answer>,
    calls: Arc<Mutex<usize>>,
    density: Arc<Mutex<Option<f64>>>,
}

impl SolidModeler for FakeMassModeler {
    fn registration(&self) -> Registration {
        Registration {
            type_key: "test.fake-mass".into(),
            version: 1,
            priority: 1,
            entity_types: vec!["3DSOLID".into()],
            capabilities: vec!["mass_properties".into()],
        }
    }

    fn capabilities(&self) -> ModelerCapabilities {
        ModelerCapabilities {
            primitives: ModelerPrimitives::default(),
            booleans: false,
            mass_properties: true,
            chamfer: false,
            fillet: false,
            shell: false,
            nurbs: false,
        }
    }

    fn execute(
        &self,
        request: &SolidRequest<'_>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> CadResult<SolidResult> {
        let SolidOperation::MassProperties { density, .. } = request.operation else {
            panic!("MASSPROP must send a MassProperties operation");
        };
        let index = {
            let mut calls = self.calls.lock().expect("call log");
            let index = *calls;
            *calls += 1;
            index
        };
        *self.density.lock().expect("density log") = Some(density);
        let answer = *self
            .answers
            .get(index)
            .unwrap_or_else(|| self.answers.last().expect("at least one answer"));
        let outcome = match answer {
            Answer::Mass => SolidOutcome::Success(SolidProduct::Mass(unit_box_mass())),
            Answer::NegativeVolume => {
                SolidOutcome::Success(SolidProduct::Mass(negative_volume_mass()))
            }
            Answer::Unsupported => SolidOutcome::Unsupported {
                reason: SolidUnsupportedReason {
                    code: "solid.lift_requires_planar",
                    operation: request.operation.family(),
                    detail: "the fake kernel cannot lift a curved face".into(),
                },
            },
            Answer::Failed => SolidOutcome::Failed {
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "solid.kernel_failed".into(),
                    message: "the fake kernel produced no estimate".into(),
                }],
            },
        };
        Ok(SolidResult {
            stamp: request.stamp.clone(),
            outcome,
        })
    }
}

/// A reflected/inside-out body: the kernel reports a negative enclosed volume.
fn negative_volume_mass() -> SolidMassProperties {
    SolidMassProperties {
        volume: -1.0,
        ..unit_box_mass()
    }
}

/// Mass of the fixture unit cube at density 1: `I = m/12 (b² + c²) = 1/6`.
fn unit_box_mass() -> SolidMassProperties {
    SolidMassProperties {
        volume: 1.0,
        surface_area: 6.0,
        centroid: p(0.5, 0.5, 0.5),
        inertia: Some([
            [1.0 / 6.0, 0.0, 0.0],
            [0.0, 1.0 / 6.0, 0.0],
            [0.0, 0.0, 1.0 / 6.0],
        ]),
        density: 1.0,
        accuracy: MassAccuracy::Requested { relative: 1e-6 },
    }
}

// ---- fixtures -------------------------------------------------------------

/// A display mesh the command must **never** integrate: a unit tetrahedron.
fn display_mesh() -> Mesh {
    Mesh {
        vertices: vec![
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(0.0, 0.0, 1.0),
        ],
        triangles: vec![[0, 1, 2], [0, 1, 3], [1, 2, 3], [2, 0, 3]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    }
}

fn body_entity(id: u128, kind: BodyKind, exact: BodyExact) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDb3dSolid".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Body3d(Box::new(Body3d {
            kind,
            exact,
            display: BodyDisplay {
                mesh: display_mesh(),
                edges: Vec::new(),
                tolerance: Some(0.01),
                precision: Precision::Approximate {
                    error_bound: Some(0.01),
                },
                completeness: Completeness::Partial(vec!["test display cache".into()]),
            },
        })),
        draw_order: id as i64,
    }
}

fn exact_box(id: u128) -> DbEntity {
    body_entity(id, BodyKind::Solid, BodyExact::Exact(Box::new(unit_box())))
}

fn circle_entity(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbCircle".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Circle {
            center: p(0.0, 0.0, 0.0),
            normal: p(0.0, 0.0, 1.0),
            radius: 2.0,
        },
        draw_order: id as i64,
    }
}

fn face(key: &str) -> SubElementId {
    SubElementId {
        source_key: key.into(),
        topology_revision: Revision(0),
    }
}

fn body_ref(id: u128, sub_element: Option<SubElementId>) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(id),
        instance: InstancePath::default(),
        sub_element,
    }
}

fn app_with_bodies(bodies: Vec<DbEntity>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for body in bodies {
        builder.insert_entity(body).unwrap();
    }
    let (app, session, _) = app_from_builder(builder);
    (app, session)
}

fn with_modeler(
    app: &mut Application,
    answer: Answer,
) -> (Arc<Mutex<usize>>, Arc<Mutex<Option<f64>>>) {
    with_modeler_sequence(app, vec![answer])
}

fn with_modeler_sequence(
    app: &mut Application,
    answers: Vec<Answer>,
) -> (Arc<Mutex<usize>>, Arc<Mutex<Option<f64>>>) {
    let calls = Arc::new(Mutex::new(0));
    let density = Arc::new(Mutex::new(None));
    app.set_solid_modeler(Some(Arc::new(FakeMassModeler {
        answers,
        calls: calls.clone(),
        density: density.clone(),
    })));
    (calls, density)
}

/// A modeler whose `Boolean` operation blocks until `release` is set. It lets a
/// test hold the kernel slot (a real background boolean) without a real kernel.
struct BlockingBooleanModeler {
    release: Arc<AtomicBool>,
}

impl SolidModeler for BlockingBooleanModeler {
    fn registration(&self) -> Registration {
        Registration {
            type_key: "test.blocking-boolean".into(),
            version: 1,
            priority: 1,
            entity_types: vec!["3DSOLID".into()],
            capabilities: vec!["booleans".into()],
        }
    }

    fn capabilities(&self) -> ModelerCapabilities {
        ModelerCapabilities {
            primitives: ModelerPrimitives::default(),
            booleans: true,
            mass_properties: false,
            chamfer: false,
            fillet: false,
            shell: false,
            nurbs: false,
        }
    }

    fn execute(
        &self,
        request: &SolidRequest<'_>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> CadResult<SolidResult> {
        assert!(
            matches!(request.operation, SolidOperation::Boolean { .. }),
            "the busy fixture only answers a boolean"
        );
        while !self.release.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(SolidResult {
            stamp: request.stamp.clone(),
            outcome: SolidOutcome::Success(SolidProduct::Body {
                brep: unit_box(),
                diagnostics: Vec::new(),
            }),
        })
    }
}

fn select_refs(app: &mut Application, session: &mut SessionState, refs: Vec<SelectionRef>) {
    app.execute(
        session,
        command(CommandId::Select, CommandPayload::Selection(refs)),
    )
    .expect("select");
}

fn select_bodies(app: &mut Application, session: &mut SessionState, ids: &[u128]) {
    select_refs(
        app,
        session,
        ids.iter().map(|id| body_ref(*id, None)).collect(),
    );
}

fn massprop(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    app.execute(session, command(CommandId::MassProp, CommandPayload::None))
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn unsupported_text(error: CadError) -> String {
    match error {
        CadError::Unsupported(text) => text,
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

// ---- contracts ------------------------------------------------------------

#[test]
fn massprop_measures_an_exact_unit_box_body_through_the_modeler() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
    let (calls, density) = with_modeler(&mut app, Answer::Mass);
    let before = revision(&app);
    select_bodies(&mut app, &mut session, &[BODY_A]);
    let outcome = massprop(&mut app, &mut session).expect("an exact solid is measurable");

    assert_eq!(*calls.lock().unwrap(), 1, "one integration per body");
    assert_eq!(
        *density.lock().unwrap(),
        Some(1.0),
        "AutoCAD's unitless default density"
    );
    let row = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "massprop.object")
        .expect("one massprop.object row");
    assert_eq!(row.object, Some(ObjectId(BODY_A)));
    let text = &row.message;
    assert!(text.contains("volume=1.000000"), "{text}");
    assert!(text.contains("surface_area=6.000000"), "{text}");
    assert!(
        text.contains("centroid=(0.500000, 0.500000, 0.500000)"),
        "{text}"
    );
    assert!(
        text.contains("Ixx=0.166667 Iyy=0.166667 Izz=0.166667"),
        "{text}"
    );
    assert!(
        text.contains("Iyz=0.000000 Izx=0.000000 Ixy=0.000000"),
        "{text}"
    );
    assert!(
        text.contains("estimated at requested relative accuracy 1e-6, not a proved bound"),
        "{text}"
    );
    assert!(
        !text.contains("analytic"),
        "the estimate is never called analytic: {text}"
    );
    // The row names the stored class and layer, like the 2D object lines.
    assert!(text.contains("3D solid AcDb3dSolid, layer 0"), "{text}");
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app), before, "a read never bumps the revision");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn a_carrier_without_a_modeler_refuses_a_body_by_name() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
    let before = revision(&app);
    select_bodies(&mut app, &mut session, &[BODY_A]);
    let text = unsupported_text(massprop(&mut app, &mut session).expect_err("no kernel"));
    assert!(
        text.starts_with("massprop.body_modeler_unavailable:"),
        "{text}"
    );
    assert!(
        text.contains("no exact B-rep kernel in this carrier"),
        "{text}"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn an_unsupported_lift_reports_the_modeler_reason() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
    let (calls, _) = with_modeler(&mut app, Answer::Unsupported);
    select_bodies(&mut app, &mut session, &[BODY_A]);
    let text = unsupported_text(massprop(&mut app, &mut session).expect_err("curved body"));
    assert!(text.starts_with("massprop.body_not_liftable:"), "{text}");
    assert!(text.contains("solid.lift_requires_planar"), "{text}");
    assert!(
        text.contains("the fake kernel cannot lift a curved face"),
        "{text}"
    );
    assert_eq!(*calls.lock().unwrap(), 1);
}

#[test]
fn a_kernel_failure_reports_the_first_diagnostic() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
    with_modeler(&mut app, Answer::Failed);
    select_bodies(&mut app, &mut session, &[BODY_A]);
    let text = unsupported_text(massprop(&mut app, &mut session).expect_err("kernel failed"));
    assert!(text.starts_with("massprop.body_kernel_failed:"), "{text}");
    assert!(
        text.contains("the fake kernel produced no estimate"),
        "{text}"
    );
}

#[test]
fn a_body_without_exact_geometry_refuses_and_never_measures_the_display_mesh() {
    // The display cache is a valid closed tetrahedron: a dishonest path could
    // report a mesh volume from it, so the refusal must win and the modeler must
    // not even be called.
    let (mut app, mut session) = app_with_bodies(vec![body_entity(
        BODY_A,
        BodyKind::Solid,
        BodyExact::Unavailable {
            reason: "the ACIS payload carried no exact geometry".into(),
        },
    )]);
    let (calls, _) = with_modeler(&mut app, Answer::Mass);
    let before = revision(&app);
    select_bodies(&mut app, &mut session, &[BODY_A]);
    let text = unsupported_text(massprop(&mut app, &mut session).expect_err("no exact B-rep"));
    assert!(text.starts_with("massprop.body_not_exact:"), "{text}");
    assert!(text.contains("no exact B-rep"), "{text}");
    assert!(
        !text.contains("volume="),
        "no mesh volume is invented: {text}"
    );
    assert_eq!(*calls.lock().unwrap(), 0, "nothing is sent to the kernel");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn two_selected_bodies_report_two_object_rows() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A), exact_box(BODY_B)]);
    with_modeler(&mut app, Answer::Mass);
    select_bodies(&mut app, &mut session, &[BODY_A, BODY_B]);
    let outcome = massprop(&mut app, &mut session).expect("two exact bodies");
    let rows: Vec<_> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "massprop.object")
        .collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0].object, Some(ObjectId(BODY_A)));
    assert_eq!(rows[1].object, Some(ObjectId(BODY_B)));
    for row in rows {
        assert!(row.message.contains("volume=1.000000"), "{}", row.message);
    }
}

#[test]
fn a_body_face_ref_alone_measures_the_owning_body() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
    with_modeler(&mut app, Answer::Mass);
    select_refs(
        &mut app,
        &mut session,
        vec![body_ref(BODY_A, Some(face("brep:face:0/0")))],
    );
    let outcome = massprop(&mut app, &mut session).expect("a face ref is a body ref");
    let rows: Vec<_> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "massprop.object")
        .collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].object, Some(ObjectId(BODY_A)));
}

#[test]
fn a_body_face_ref_resolves_to_its_body_and_counts_once() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
    let (calls, _) = with_modeler(&mut app, Answer::Mass);
    // A plain body ref plus two of its face refs: one body has one mass.
    select_refs(
        &mut app,
        &mut session,
        vec![
            body_ref(BODY_A, None),
            body_ref(BODY_A, Some(face("brep:face:0/0"))),
            body_ref(BODY_A, Some(face("brep:face:0/1"))),
        ],
    );
    let outcome = massprop(&mut app, &mut session).expect("a face ref measures its owning body");
    let rows: Vec<_> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "massprop.object")
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "the body and its faces are one object: {rows:?}"
    );
    assert_eq!(rows[0].object, Some(ObjectId(BODY_A)));
    assert_eq!(*calls.lock().unwrap(), 1, "one integration for one body");
}

#[test]
fn a_sub_element_ref_on_non_body_geometry_still_refuses_not_addressable() {
    let (mut app, mut session) = app_with_bodies(vec![circle_entity(CIRCLE_A)]);
    with_modeler(&mut app, Answer::Mass);
    select_refs(
        &mut app,
        &mut session,
        vec![body_ref(CIRCLE_A, Some(face("brep:face:0/0")))],
    );
    let text = unsupported_text(massprop(&mut app, &mut session).expect_err("a circle face"));
    assert!(text.starts_with("massprop.not_addressable:"), "{text}");
    assert!(text.contains("sub-elements"), "{text}");
}

#[test]
fn a_mixed_body_and_region_selection_names_the_kinds_and_counts() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A), circle_entity(CIRCLE_A)]);
    with_modeler(&mut app, Answer::Mass);
    select_bodies(&mut app, &mut session, &[BODY_A, CIRCLE_A]);
    let text = unsupported_text(massprop(&mut app, &mut session).expect_err("mixed"));
    assert!(text.starts_with("massprop.mixed_selection:"), "{text}");
    assert!(
        text.contains("1 distinct 3D body entity/entities and 1 distinct other object(s)"),
        "{text}"
    );
    assert!(
        text.contains("a CIRCLE"),
        "the offending kind is named: {text}"
    );
}

#[test]
fn a_non_solid_body_refuses_by_name() {
    let (mut app, mut session) = app_with_bodies(vec![body_entity(
        BODY_A,
        BodyKind::Sheet,
        BodyExact::Exact(Box::new(unit_box())),
    )]);
    with_modeler(&mut app, Answer::Mass);
    select_bodies(&mut app, &mut session, &[BODY_A]);
    let text = unsupported_text(massprop(&mut app, &mut session).expect_err("a sheet"));
    assert!(text.starts_with("massprop.body_not_solid:"), "{text}");
    assert!(text.contains("Sheet"), "{text}");
}

#[test]
fn an_empty_exact_brep_refuses_by_name() {
    let (mut app, mut session) = app_with_bodies(vec![body_entity(
        BODY_A,
        BodyKind::Solid,
        BodyExact::Exact(Box::new(BrepData {
            shells: Vec::new(),
            placement: None,
            tolerance: None,
        })),
    )]);
    with_modeler(&mut app, Answer::Mass);
    select_bodies(&mut app, &mut session, &[BODY_A]);
    let text = unsupported_text(massprop(&mut app, &mut session).expect_err("an empty B-rep"));
    assert!(text.starts_with("massprop.body_not_exact:"), "{text}");
    assert!(text.contains("empty exact B-rep"), "{text}");
}

#[test]
fn a_negative_volume_refuses_as_not_positive() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
    with_modeler(&mut app, Answer::NegativeVolume);
    select_bodies(&mut app, &mut session, &[BODY_A]);
    let text = unsupported_text(massprop(&mut app, &mut session).expect_err("inside-out body"));
    assert!(
        text.starts_with("massprop.body_not_positive_volume:"),
        "{text}"
    );
    assert!(text.contains("not finite and positive"), "{text}");
}

#[test]
fn one_failing_body_makes_the_whole_run_report_nothing() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A), exact_box(BODY_B)]);
    let (calls, _) = with_modeler_sequence(&mut app, vec![Answer::Mass, Answer::Failed]);
    let before = revision(&app);
    select_bodies(&mut app, &mut session, &[BODY_A, BODY_B]);
    let text = unsupported_text(massprop(&mut app, &mut session).expect_err("body #2 failed"));
    assert!(text.starts_with("massprop.body_kernel_failed:"), "{text}");
    assert_eq!(
        *calls.lock().unwrap(),
        2,
        "both bodies were integrated before the refusal"
    );
    assert_eq!(
        revision(&app),
        before,
        "a refusal reports no partial object"
    );
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn massprop_refuses_while_a_solid_boolean_runs() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A), exact_box(BODY_B)]);
    let (calls, _) = with_modeler(&mut app, Answer::Mass);
    let release = Arc::new(AtomicBool::new(false));
    let operands: Vec<SolidBooleanOperand> = {
        let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
        [BODY_A, BODY_B]
            .iter()
            .map(|id| {
                let entity = drawing.entity(EntityId(*id)).expect("body").clone();
                SolidBooleanOperand {
                    id: entity.id,
                    revision: entity.object.revision,
                    brep: unit_box(),
                    snapshot: entity,
                }
            })
            .collect()
    };
    let plan = SolidBooleanPlan {
        document: DocumentId(1),
        op: BooleanOp::Union,
        layer: LayerId(0),
        space: SpaceId::Model,
        draw_order: 0,
        operands,
    };
    app.start_solid_boolean(
        plan,
        Arc::new(BlockingBooleanModeler {
            release: release.clone(),
        }),
    )
    .expect("start a blocking boolean");
    assert!(app.solid_boolean_running(), "the kernel slot is held");

    select_bodies(&mut app, &mut session, &[BODY_A]);
    let text = unsupported_text(massprop(&mut app, &mut session).expect_err("kernel busy"));
    assert!(text.starts_with("massprop.kernel_busy:"), "{text}");
    assert!(
        text.contains("a solid boolean is running; wait for it to settle"),
        "{text}"
    );
    assert_eq!(
        *calls.lock().unwrap(),
        0,
        "MASSPROP must not queue behind the kernel"
    );

    release.store(true, Ordering::SeqCst);
    app.abort_solid_boolean();
}
