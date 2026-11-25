//! ADR-0036 P2.0: `CHAMFEREDGE` command contracts.
//!
//! The contracts use a **fake** [`SolidModeler`] that answers the `Chamfer`
//! operation with a hand-built planar box plus one extra face (the cut), so the
//! command layer is pinned independently of the kernel. The real kernel is
//! exercised by `cad-cli-tools/tests/solid_command.rs`.
//!
//! Pinned here: one in-place replacement (same id, one revision, one undo), the
//! face-ref → body collapse, the asymmetric/symmetric distance mapping, the
//! `chamferedge.applied` diagnostic, and every named refusal with **zero**
//! revision/undo change. `cad-app` never pre-judges body curvature: the kernel's
//! `solid.lift_requires_planar` / `solid.edge_not_found` answers pass through.

use super::*;
use crate::tasks::{SolidBooleanOperand, SolidBooleanPlan};
use crate::tests::inquiry::app_from_builder;
use cad_db::{DrawingDatabaseBuilder, Layer, Layout};
use cad_domain::LayoutId;
use cad_kernel_adapter::{
    BooleanOp, ModelerCapabilities, ModelerPrimitives, SolidModeler, SolidOperation, SolidOutcome,
    SolidProduct, SolidRequest, SolidResult, SolidUnsupportedReason,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::solids::{p, unit_box};

const BODY_A: u128 = 910;
const BODY_B: u128 = 911;
const LINE_A: u128 = 912;

// ---- fixtures -------------------------------------------------------------

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

/// A minimal closed display cache (the command never reads it; the row must be
/// a valid body for the database).
fn display_mesh() -> Mesh {
    Mesh {
        vertices: vec![
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
        ],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    }
}

fn exact_box(id: u128) -> DbEntity {
    body_entity(id, BodyKind::Solid, BodyExact::Exact(Box::new(unit_box())))
}

fn line_entity(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: p(0.0, 0.0, 0.0),
            end: p(1.0, 0.0, 0.0),
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

fn body_ref(id: u128) -> SelectionRef {
    ref_of(id)
}

fn face_ref(id: u128, key: &str) -> SelectionRef {
    SelectionRef {
        sub_element: Some(face(key)),
        ..ref_of(id)
    }
}

/// The chamfered product: a planar box with its top-front edge cut by a 0.2
/// set-back, i.e. the same six faces re-wound around the cut plus one new
/// chamfer face (seven closed, consistently wound faces). Built by re-winding
/// the unit-box fixture so the real tessellator still produces a closed mesh.
fn chamfered_box() -> BrepData {
    let point = |x: f64, y: f64, z: f64| Point3 { x, y, z };
    let a = point(0.0, 0.2, 1.0);
    let b = point(1.0, 0.2, 1.0);
    let c = point(0.0, 0.0, 0.8);
    let d = point(1.0, 0.0, 0.8);
    let mut brep = unit_box();
    let faces = &mut brep.shells[0].faces;
    // Top (+Z), front (-Y), right (+X) and left (-X) are cut back; bottom and
    // back keep their original loops.
    faces[1].loops = vec![line_loop(&[
        a,
        b,
        point(1.0, 1.0, 1.0),
        point(0.0, 1.0, 1.0),
    ])];
    faces[2].loops = vec![line_loop(&[
        point(0.0, 0.0, 0.0),
        point(1.0, 0.0, 0.0),
        d,
        c,
    ])];
    faces[3].loops = vec![line_loop(&[
        point(1.0, 0.0, 0.0),
        point(1.0, 1.0, 0.0),
        point(1.0, 1.0, 1.0),
        b,
        d,
    ])];
    faces[5].loops = vec![line_loop(&[
        point(0.0, 1.0, 0.0),
        point(0.0, 0.0, 0.0),
        c,
        a,
        point(0.0, 1.0, 1.0),
    ])];
    let mut chamfer = faces[5].clone();
    chamfer.id = 6;
    chamfer.surface = BrepSurface::Plane {
        origin: a,
        normal: point(
            0.0,
            -std::f64::consts::FRAC_1_SQRT_2,
            std::f64::consts::FRAC_1_SQRT_2,
        ),
        u_dir: point(1.0, 0.0, 0.0),
    };
    // Opposite traversal of the top (A→B), front (D→C), left (C→A) and right
    // (B→D) neighbours, which is what keeps every shared edge manifold.
    chamfer.loops = vec![line_loop(&[b, a, c, d])];
    faces.push(chamfer);
    brep
}

/// One closed polyline loop over the given cycle (the cycle is stored once; the
/// closing edge is implicit).
fn line_loop(points: &[Point3]) -> BrepLoop {
    let edges = (0..points.len())
        .map(|index| BrepCurve::Line {
            start: points[index],
            end: points[(index + 1) % points.len()],
        })
        .collect();
    BrepLoop { edges }
}

fn unit_box_faces() -> usize {
    unit_box().shells[0].faces.len()
}

fn stored_brep(app: &Application, id: u128) -> BrepData {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .expect("the body exists");
    let SemanticGeometry::Body3d(body) = &entity.geometry else {
        panic!("entity {id} is not a body");
    };
    body.exact_brep().expect("the body is exact").clone()
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
    // A paper layout exists so the paper-space refusal fixture is a valid row.
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    let (app, session, _) = app_from_builder(builder);
    (app, session)
}

// ---- the fake modeler -----------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
struct ChamferCall {
    selector: Point3,
    distance: f64,
    second: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Chamfered,
    /// The kernel's own capability answer, passed through unchanged.
    Unsupported(&'static str),
    Failed,
}

struct FakeChamferModeler {
    answer: Answer,
    calls: Arc<Mutex<Vec<ChamferCall>>>,
}

impl SolidModeler for FakeChamferModeler {
    fn registration(&self) -> Registration {
        Registration {
            type_key: "test.fake-chamfer".into(),
            version: 1,
            priority: 1,
            entity_types: vec!["3DSOLID".into()],
            capabilities: vec!["chamfer".into()],
        }
    }

    fn capabilities(&self) -> ModelerCapabilities {
        ModelerCapabilities {
            primitives: ModelerPrimitives::default(),
            booleans: false,
            mass_properties: false,
            chamfer: true,
            offset_face: false,
            extrude_profile: false,
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
        let SolidOperation::Chamfer {
            selector,
            distance,
            second,
            ..
        } = request.operation
        else {
            panic!("CHAMFEREDGE must send a Chamfer operation");
        };
        self.calls.lock().expect("call log").push(ChamferCall {
            selector: selector.point(),
            distance,
            second,
        });
        let outcome = match self.answer {
            Answer::Chamfered => SolidOutcome::Success(SolidProduct::Body {
                brep: chamfered_box(),
                diagnostics: Vec::new(),
            }),
            Answer::Unsupported(code) => SolidOutcome::Unsupported {
                reason: SolidUnsupportedReason {
                    code,
                    operation: request.operation.family(),
                    detail: "the fake kernel declines".into(),
                },
            },
            Answer::Failed => SolidOutcome::Failed {
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "solid.kernel_failed".into(),
                    message: "the fake kernel produced nothing".into(),
                }],
            },
        };
        Ok(SolidResult {
            stamp: request.stamp.clone(),
            outcome,
        })
    }
}

fn with_modeler(app: &mut Application, answer: Answer) -> Arc<Mutex<Vec<ChamferCall>>> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    app.set_solid_modeler(Some(Arc::new(FakeChamferModeler {
        answer,
        calls: calls.clone(),
    })));
    calls
}

fn set_distances(session: &mut SessionState, a: f64, b: f64) {
    session
        .set_tool_parameter(ToolParameter::ChamferDistanceA, ToolValue::Number(a))
        .expect("A is a legal parameter value");
    session
        .set_tool_parameter(ToolParameter::ChamferDistanceB, ToolValue::Number(b))
        .expect("B is a legal parameter value");
}

fn chamfer_command(body: SelectionRef, seed: Point3) -> (CommandId, CommandPayload) {
    (
        CommandId::Extended(ExtendedCommandId::ChamferEdge),
        CommandPayload::ChamferSolidEdge { body, seed },
    )
}

fn refusal_text(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) | CadError::Unsupported(text) => text,
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

fn assert_wrote_nothing(app: &Application, before: Revision) {
    assert_eq!(revision(app), before, "a refusal writes nothing");
    assert!(!app.can_undo(&DocumentId(1)), "a refusal records no undo");
}

// ---- apply ----------------------------------------------------------------

#[test]
fn chamfer_edge_applies_in_place_with_one_transaction_and_one_undo() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
    let calls = with_modeler(&mut app, Answer::Chamfered);
    set_distances(&mut session, 0.5, 0.5);
    let before_revision = revision(&app);
    let before_brep = stored_brep(&app, BODY_A);
    let seed = p(0.5, 0.0, 1.0);

    let (id, payload) = chamfer_command(body_ref(BODY_A), seed);
    let outcome = execute(&mut app, &mut session, id, payload).expect("the edge is chamfered");

    assert_eq!(outcome.objects, vec![ObjectId(BODY_A)]);
    assert_eq!(
        revision(&app),
        Revision(before_revision.0 + 1),
        "exactly one revision bump"
    );
    assert_eq!(
        app.history[&DocumentId(1)].undo_depth(),
        1,
        "exactly one undo entry"
    );

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].selector, seed,
        "the kernel gets the pick hit point"
    );
    assert_eq!(calls[0].distance, 0.5);
    assert_eq!(
        calls[0].second, None,
        "equal distances are the symmetric form"
    );
    drop(calls);

    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(BODY_A))
        .expect("the row is still there");
    assert_eq!(
        entity.object.type_key, "AcDb3dSolid",
        "the type key is kept"
    );
    let SemanticGeometry::Body3d(body) = &entity.geometry else {
        panic!("the row is still a body");
    };
    assert_eq!(
        body.exact_brep().unwrap().shells[0].faces.len(),
        unit_box_faces() + 1,
        "one extra face"
    );

    let applied = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "chamferedge.applied")
        .expect("the success names the two distances and the seed");
    assert!(
        applied.message.contains("A=0.5, B=0.5"),
        "{}",
        applied.message
    );
    assert!(
        applied.message.contains("(0.5, 0, 1)"),
        "{}",
        applied.message
    );
    assert!(
        !applied.message.contains("edge 3") && !applied.message.contains("face"),
        "the seam returns only the product body, so no edge identity may be claimed: {}",
        applied.message
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert_eq!(
        stored_brep(&app, BODY_A),
        before_brep,
        "one undo restores the exact original B-rep"
    );
}

#[test]
fn an_asymmetric_distance_rides_the_second_set_back() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
    let calls = with_modeler(&mut app, Answer::Chamfered);
    set_distances(&mut session, 1.0, 2.0);
    let (id, payload) = chamfer_command(body_ref(BODY_A), p(0.0, 0.0, 1.0));
    execute(&mut app, &mut session, id, payload).expect("the edge is chamfered");
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].distance, 1.0);
    assert_eq!(calls[0].second, Some(2.0));
}

#[test]
fn a_face_ref_selection_collapses_to_the_owning_body() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
    with_modeler(&mut app, Answer::Chamfered);
    set_distances(&mut session, 0.5, 0.5);
    let before_revision = revision(&app);
    let (id, payload) = chamfer_command(face_ref(BODY_A, "face-3"), p(0.5, 0.0, 1.0));
    let outcome = execute(&mut app, &mut session, id, payload)
        .expect("a face ref names its owning body, which is what is chamfered");
    assert_eq!(outcome.objects, vec![ObjectId(BODY_A)]);
    assert_eq!(revision(&app), Revision(before_revision.0 + 1));
    assert!(app.can_undo(&DocumentId(1)));
}

#[test]
fn a_body_and_its_own_face_ref_are_one_body_not_two() {
    let (app, _session) = app_with_bodies(vec![exact_box(BODY_A)]);
    let refs = [body_ref(BODY_A), face_ref(BODY_A, "face-0")];
    let target = crate::chamfer_edge_target(&app, DocumentId(1), &refs)
        .expect("several refs to one body collapse to one");
    assert_eq!(target.body.entity, EntityId(BODY_A));
    assert_eq!(target.body.sub_element, None);
    assert_eq!(target.body.instance, InstancePath::default());
}

// ---- refusals (zero revision, zero undo) ----------------------------------

#[test]
fn an_empty_selection_is_refused() {
    let (app, _session) = app_with_bodies(vec![exact_box(BODY_A)]);
    let error = crate::chamfer_edge_target(&app, DocumentId(1), &[]).expect_err("empty selection");
    assert!(
        refusal_text(error).starts_with("chamferedge.selection_required:"),
        "got the wrong refusal"
    );
}

#[test]
fn two_distinct_bodies_are_refused() {
    let (app, _session) = app_with_bodies(vec![exact_box(BODY_A), exact_box(BODY_B)]);
    let refs = [body_ref(BODY_A), body_ref(BODY_B)];
    let error = crate::chamfer_edge_target(&app, DocumentId(1), &refs).expect_err("two bodies");
    assert!(refusal_text(error).starts_with("chamferedge.selection_multiple:"));
}

#[test]
fn a_line_is_not_a_solid() {
    let (app, _session) = app_with_bodies(vec![line_entity(LINE_A)]);
    let error =
        crate::chamfer_edge_target(&app, DocumentId(1), &[body_ref(LINE_A)]).expect_err("a LINE");
    assert!(refusal_text(error).starts_with("chamferedge.not_a_solid:"));
}

#[test]
fn a_block_instance_member_is_not_addressable() {
    let (app, _session) = app_with_bodies(vec![exact_box(BODY_A)]);
    let reference = SelectionRef {
        instance: InstancePath(vec![EntityId(900)]),
        ..body_ref(BODY_A)
    };
    let error = crate::chamfer_edge_target(&app, DocumentId(1), &[reference])
        .expect_err("an instance path");
    assert!(refusal_text(error).starts_with("chamferedge.not_addressable:"));
}

#[test]
fn a_sub_element_of_non_body_geometry_is_not_addressable() {
    let (app, _session) = app_with_bodies(vec![line_entity(LINE_A)]);
    let error = crate::chamfer_edge_target(&app, DocumentId(1), &[face_ref(LINE_A, "v0")])
        .expect_err("a LINE sub-element");
    assert!(refusal_text(error).starts_with("chamferedge.not_addressable:"));
}

#[test]
fn a_body_without_exact_geometry_is_not_exact() {
    let body = body_entity(
        BODY_A,
        BodyKind::Solid,
        BodyExact::Unavailable {
            reason: "imported without ACIS".into(),
        },
    );
    let (app, _session) = app_with_bodies(vec![body]);
    let error = crate::chamfer_edge_target(&app, DocumentId(1), &[body_ref(BODY_A)])
        .expect_err("non-exact body");
    assert!(refusal_text(error).starts_with("chamferedge.not_exact:"));
}

#[test]
fn a_sheet_body_is_not_a_solid() {
    let body = body_entity(
        BODY_A,
        BodyKind::Sheet,
        BodyExact::Exact(Box::new(unit_box())),
    );
    let (app, _session) = app_with_bodies(vec![body]);
    let error = crate::chamfer_edge_target(&app, DocumentId(1), &[body_ref(BODY_A)])
        .expect_err("a sheet body");
    assert!(refusal_text(error).starts_with("chamferedge.not_a_solid:"));
}

#[test]
fn a_paper_space_body_is_not_model_space() {
    let mut body = exact_box(BODY_A);
    body.space = SpaceId::Paper(LayoutId(1));
    let (app, _session) = app_with_bodies(vec![body]);
    let error = crate::chamfer_edge_target(&app, DocumentId(1), &[body_ref(BODY_A)])
        .expect_err("a paper-space body");
    assert!(refusal_text(error).starts_with("chamferedge.not_model_space:"));
}

#[test]
fn an_erased_body_is_a_selection_change() {
    let (app, _session) = app_with_bodies(vec![exact_box(BODY_A)]);
    let error = crate::chamfer_edge_target(&app, DocumentId(1), &[body_ref(BODY_B)])
        .expect_err("an entity that does not exist");
    assert!(refusal_text(error).starts_with("chamferedge.selection_changed:"));
}

#[test]
fn a_curved_body_refusal_passes_through_as_solid_lift_requires_planar() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
    with_modeler(&mut app, Answer::Unsupported("solid.lift_requires_planar"));
    set_distances(&mut session, 0.5, 0.5);
    let before = revision(&app);
    let (id, payload) = chamfer_command(body_ref(BODY_A), p(0.5, 0.0, 1.0));
    let error = execute(&mut app, &mut session, id, payload).expect_err("the kernel declines");
    let text = refusal_text(error);
    assert!(
        text.starts_with("solid.lift_requires_planar:"),
        "the kernel reason is never re-prefixed or wrapped: {text}"
    );
    assert_wrote_nothing(&app, before);
}

#[test]
fn a_seed_far_from_every_edge_passes_through_as_solid_edge_not_found() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
    with_modeler(&mut app, Answer::Unsupported("solid.edge_not_found"));
    set_distances(&mut session, 0.5, 0.5);
    let before = revision(&app);
    let (id, payload) = chamfer_command(body_ref(BODY_A), p(0.5, 0.5, 0.0));
    let error = execute(&mut app, &mut session, id, payload).expect_err("no edge near the seed");
    assert!(refusal_text(error).starts_with("solid.edge_not_found:"));
    assert_wrote_nothing(&app, before);
}

#[test]
fn a_kernel_failure_is_reported_with_its_named_reason() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
    with_modeler(&mut app, Answer::Failed);
    set_distances(&mut session, 0.5, 0.5);
    let before = revision(&app);
    let (id, payload) = chamfer_command(body_ref(BODY_A), p(0.5, 0.0, 1.0));
    let error = execute(&mut app, &mut session, id, payload).expect_err("the kernel fails");
    assert!(refusal_text(error).starts_with("solid.kernel_failed:"));
    assert_wrote_nothing(&app, before);
}

#[test]
fn a_carrier_without_a_modeler_refuses_by_name_and_writes_nothing() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
    set_distances(&mut session, 0.5, 0.5);
    let before = revision(&app);
    let (id, payload) = chamfer_command(body_ref(BODY_A), p(0.5, 0.0, 1.0));
    let error = execute(&mut app, &mut session, id, payload).expect_err("no modeler");
    assert_eq!(
        error,
        CadError::Unsupported("unsupported-runtime: solid-modeler".into())
    );
    assert_wrote_nothing(&app, before);
}

#[test]
fn the_distances_are_validated_at_execute_not_at_tool_start() {
    for (a, b) in [(0.0, 0.5), (0.5, 0.0), (0.5, f64::NAN), (0.5, 1.0e6 + 1.0)] {
        let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
        with_modeler(&mut app, Answer::Chamfered);
        // `NAN` cannot be stored through the validated channel; every legal
        // value is stored and the command still refuses the resulting pair.
        let stored_b = if b.is_nan() { 0.0 } else { b };
        set_distances(&mut session, a, stored_b);
        let before = revision(&app);
        let (id, payload) = chamfer_command(body_ref(BODY_A), p(0.5, 0.0, 1.0));
        let error = match execute(&mut app, &mut session, id, payload) {
            Err(error) => error,
            Ok(_) => panic!("A={a} B={b} must be refused"),
        };
        assert!(
            refusal_text(error).starts_with("chamferedge.distance_invalid:"),
            "A={a} B={b}"
        );
        assert_wrote_nothing(&app, before);
    }
}

#[test]
fn the_distance_predicate_refuses_zero_negative_nan_and_over_the_seam_bound() {
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, 1.0e6 + 1.0] {
        let error = crate::app_chamfer_edge::validate_chamfer_distance(bad)
            .expect_err("an out-of-range set-back");
        assert!(
            refusal_text(error).starts_with("chamferedge.distance_invalid:"),
            "{bad}"
        );
    }
    assert_eq!(
        crate::app_chamfer_edge::validate_chamfer_distance(0.5).unwrap(),
        0.5
    );
}

#[test]
fn a_none_payload_is_refused_before_any_kernel_work() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A)]);
    let calls = with_modeler(&mut app, Answer::Chamfered);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::ChamferEdge),
        CommandPayload::None,
    )
    .expect_err("a typed CHAMFEREDGE carries no body or seed");
    assert!(refusal_text(error).starts_with("chamferedge.requires_payload:"));
    assert!(calls.lock().unwrap().is_empty());
    assert_wrote_nothing(&app, before);
}

// ---- the kernel slot ------------------------------------------------------

/// A modeler whose `Boolean` operation blocks until `release` is set, so a real
/// background boolean holds the kernel slot without a real kernel.
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
            offset_face: false,
            extrude_profile: false,
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
        assert!(matches!(request.operation, SolidOperation::Boolean { .. }));
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

#[test]
fn chamfer_refuses_while_a_solid_boolean_runs() {
    let (mut app, mut session) = app_with_bodies(vec![exact_box(BODY_A), exact_box(BODY_B)]);
    let calls = with_modeler(&mut app, Answer::Chamfered);
    set_distances(&mut session, 0.5, 0.5);
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

    let before = revision(&app);
    let (id, payload) = chamfer_command(body_ref(BODY_A), p(0.5, 0.0, 1.0));
    let error = execute(&mut app, &mut session, id, payload).expect_err("kernel busy");
    assert!(
        refusal_text(error).starts_with("chamferedge.kernel_busy:"),
        "the command must not queue behind the kernel"
    );
    assert!(
        calls.lock().unwrap().is_empty(),
        "the chamfer never reached the modeler"
    );
    assert_wrote_nothing(&app, before);

    release.store(true, Ordering::SeqCst);
    app.abort_solid_boolean();
}
