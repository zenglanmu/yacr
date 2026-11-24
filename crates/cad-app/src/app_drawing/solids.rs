//! ADR-0036 phase C1/C2: real exact bodies and real background booleans.
//!
//! `BOX`, `WEDGE`, `PYRAMID`, `CYLINDER`, `SPHERE`, `CONE` and `TORUS` stop
//! being named refusals. Each handler turns an explicit point payload into one
//! [`cad_kernel_adapter::PrimitiveSpec`], asks the injected
//! [`cad_kernel_adapter::SolidModeler`] for the exact body, tessellates it with
//! the existing `cad-kernel-adapter` tessellator (the modeler never builds a
//! display mesh) and commits **one** `Body3d { kind: Solid, exact, display }` row
//! with the ACIS class key `AcDb3dSolid` — one transaction, one undo step.
//!
//! `UNION`/`SUBTRACT`/`INTERSECT` (phase C2) validate at least two selected exact
//! solid bodies and start a background job; the worker folds them through the
//! modeler and the host commits the result in one transaction (see
//! [`Application::boolean_solids`] and `crate::tasks::SolidBooleanJob`).
//!
//! # Construction contract (typed payloads; no interactive prompt yet)
//!
//! AutoCAD prompts for the dimensions interactively. This carrier has no such
//! prompt in C1 (the prompt capture is a declared ADR global-scope carve-out),
//! so every construction is one typed payload and the arity/option rules are
//! enforced here. Two payload shapes are accepted:
//!
//! * [`CommandPayload::Points`] — the **explicit 3D** form below. Every point
//!   keeps its own z, so the session `ELEV` token never translates it (AutoCAD
//!   behaves the same when all three coordinates are typed).
//! * [`CommandPayload::Solid`] — the AutoCAD option constructions
//!   (`Cube`/`Center`/`Length`, `Diameter`, `2P`, `Top radius`, …). Their base
//!   points are **plan** `[x, y]` coordinates and the base plane's z is the
//!   session `ELEV` token, exactly like a plan pick — except `Center` with a
//!   corner and `SPHERE 2P`, whose AutoCAD prompts are 3D and which keep their
//!   own z. This is the consumer of `ELEV` for explicit 3D solids; see
//!   [`super::solid_spec`] for the full per-kind grammar and the declared limits
//!   (positive extents only, `+Z` axes, no `3P`/`Ttr`/`Elliptical` base).
//!
//! The explicit 3D form (unchanged from C1):
//!
//! | command | points | meaning |
//! |---|---|---|
//! | `BOX` | 2 | opposite corners; `dx, dy, dz > 0`; `origin` = min corner |
//! | `WEDGE` | 3 | base corner `p0`, base opposite corner `p1`, height point `p2`; the two base points must be coplanar in z (`wedge.base_not_coplanar` otherwise) and `dx, dy > 0`; `height = p2.z - base.z > 0`; **the tall face is at the minimum-x corner** (`x = origin.x`, the `polyhedra::wedge_brep` convention) — reversing `p0`/`p1` never moves it |
//! | `PYRAMID` | 3 | base corner `p0`, base opposite corner `p1`, apex `p2`; the base plane is `z = p0.z = p1.z` (a tilted base is refused: `pyramid.base_not_coplanar`); square base (`dx == dy`) and the apex xy over the base centre; `height = p2.z - base.z > 0` |
//! | `CYLINDER` | 3 | base centre `c`, radius point `r`, top point `t`; `radius = |r - c|` in xy `> 0`; `height = t.z - c.z > 0` |
//! | `CONE` | 3 | base centre `c`, base radius point `r`, top point `t`; `radius_bottom = |r - c|` xy `> 0`; `radius_top = |t - c|` xy (`0` is a full cone); `height = t.z - c.z > 0` |
//! | `SPHERE` | 2 | centre, radius point; `radius = |p1 - p0|` (3D) `> 0` |
//! | `TORUS` | 3 | centre `c`, major point `m`, minor point `n`; `major = |m - c|` xy `> 0`; `minor = |n - m|` (3D) `> 0`; `minor < major` |
//!
//! AutoCAD's option constructions (the [`CommandPayload::Solid`] form):
//!
//! | command | construction | payload |
//! |---|---|---|
//! | `BOX`/`WEDGE` | `Cube` | `Cube { corner, edge }` |
//! | `BOX`/`WEDGE` | `Center` + corner | `CenterPoints { center, corner }` (centred on `center`; the corner's mirror image is the opposite corner) |
//! | `BOX`/`WEDGE` | `Center` + `Cube` | `CenterCube { center, edge }` (centred on all axes) |
//! | `BOX`/`WEDGE` | `Center` + `Length` | `CenterLengths { center, length, width, height }` |
//! | `BOX`/`WEDGE` | `Length` | `Length { corner, length, width, height }` |
//! | `CYLINDER` | any base + height | `CylinderPlan { base, height }` |
//! | `CONE` | any base + `Top radius` + height | `ConePlan { base, top_radius, height }` |
//! | `SPHERE` | `Diameter` | `SphereDiameter { center, diameter }` |
//! | `SPHERE` | `2P` (3D) | `SphereTwoPoint { first, second }` |
//! | `TORUS` | any ring + tube | `TorusPlan { ring, tube }` |
//! | `PYRAMID` | `Points` only | `Edge`/`Sides`/`Top radius` need an n-gon/truncated seam primitive that does not exist — refused by name, PYRAMID stays partial |
//!
//! Every point must be finite (`<kind>.non_finite_point`). Non-positive or
//! non-finite extents are `<kind>.degenerate_extent`; a non-square pyramid base
//! or an off-centre apex is `<kind>.invalid_base`; a cone whose top radius equals
//! its base radius is `cone.equal_radii` (that is a `CYLINDER`); a construction
//! the kind does not take is `solid.option_not_supported`. Every refusal happens
//! before the modeler call and before any staging, so it writes nothing.
//!
//! # Carrier capability
//!
//! A carrier that links no modeler passes `None` and every one of the seven
//! refuses with `unsupported-runtime: solid-modeler`, writing nothing (no
//! revision bump, no history entry).
//!
//! **Code fact (what is wired today)**: only the Linux `native-below` session
//! injects `cad_kernel_solidcore::SolidCoreModeler` **and**, beside it, the
//! display mesher `cad_kernel_solidcore::SolidCoreTessellator` (P2.1c part 1).
//! Linux `dom-canvas`
//! (X11/XWayland — its CAD session is the wasm one in the WebView), Web
//! (wasm), Windows, macOS and Android do **not** inject either, so the seven
//! commands refuse by name there and a NURBS face stays a missing face. Whether
//! those carriers can ever host the
//! modeler is a separate, unproven question: their runtime injection/behaviour
//! is **NOT VERIFIED** (only compile/contract evidence exists), never a claim
//! that the feature works there.
//!
//! # Cancellation
//!
//! The **primitive** seam is called synchronously on the command thread with an
//! always-`false` cancellation probe: a deliberate, bounded carve-out (one
//! primitive, shape-relative tessellation, so the facet count stays ~constant
//! regardless of the dimensions). Mid-command cancellation is **not** wired for
//! the primitives (C1).
//!
//! The C2 booleans (`boolean_solids`) deliberately do **not** reuse it: they only
//! validate and snapshot here, then start a real worker job
//! (`crate::tasks::SolidBooleanJob`) with a live cancellation token
//! (ADR-0036 P1/C1). The host commits when it polls a current, non-stale result.
//!
//! # What is *not* here
//!
//! `face_sources`/`VSCURRENT` stay C3, `MASSPROP` for exact bodies is C3a
//! (`crate::app_massprop_body`), curved-operand booleans are P2 (the modeler
//! refuses them), and `3DSOLID`/`BODY` remain **entity type keys**, not AutoCAD
//! commands: the modeler advertises them in its registration, and no command
//! variant is added for them.

use super::*;
use crate::app_utility::require_none_payload;
use crate::command_payload::SolidPrimitiveCommand;
use crate::tasks::{SolidBooleanOperand, SolidBooleanPlan};
use cad_kernel_adapter::{
    execute_solid, BooleanOp, PrimitiveKind, SolidExchange, SolidOperation, SolidOutcome,
    SolidProduct, SolidRequest, TessellationBudget, TessellationOutcome,
};

/// The stable refusal a carrier without a modeler reports.
pub const SOLID_MODELER_UNSUPPORTED: &str = "unsupported-runtime: solid-modeler";

/// The ACIS class key every created/combined body is stored under.
pub(crate) const SOLID_TYPE_KEY: &str = "AcDb3dSolid";

impl Application {
    /// Create one primitive solid body and commit it as an `AcDb3dSolid` row.
    ///
    /// The sequence is: payload/point validation (no modeler call, no state
    /// change) → the modeler → tessellation of the returned exact B-rep →
    /// one validated insert. Every refusal happens before the transaction, so a
    /// failed create never bumps the revision or records an undo step.
    pub(crate) fn create_solid_primitive(
        &mut self,
        session: &SessionState,
        command: &Command,
        kind: PrimitiveKind,
    ) -> CadResult<CommandOutcome> {
        // Both accepted shapes describe the same input type; the explicit points
        // form is the pre-existing contract and is mapped onto the same typed
        // value, so there is exactly one spec path and one set of refusals.
        let input = match &command.payload {
            CommandPayload::Points(points) => SolidPrimitiveCommand::Points(points.clone()),
            CommandPayload::Solid(input) => input.clone(),
            _ => {
                return Err(CadError::InvalidInput(format!(
                    "{}.requires_points: {} takes an explicit Points([..]) or Solid(..) payload \
                     (no interactive prompt in this build)",
                    kind.as_str(),
                    kind.as_str().to_ascii_uppercase()
                )))
            }
        };
        // Construction mapping and validation run before any modeler call: a bad
        // payload is an input error, never an `Unsupported`/`Failed` kernel answer.
        // Plan-form constructions take their base z from the session `ELEV`
        // token here; the create funnel's `Body3d` exemption then leaves the
        // already-placed body alone (no double translation).
        let spec = solid_spec::solid_primitive_spec(kind, &input, session.drafting_extras.elev())?;

        // Both sides of the seam share one kernel mutex: a synchronous create
        // issued while a boolean worker owns it would block the command thread.
        // Refuse by name, before the modeler call and before any staging.
        if self.solid_boolean_running() {
            return Err(CadError::Unsupported(
                "solid.boolean_busy: a boolean is running; wait for it to settle before creating another solid"
                    .into(),
            ));
        }
        let Some(modeler) = self.solid_modeler.clone() else {
            return Err(CadError::Unsupported(SOLID_MODELER_UNSUPPORTED.into()));
        };
        let request = SolidRequest {
            stamp: TaskStamp::new(command.document, session.generation),
            operation: SolidOperation::Primitive { spec },
        };
        // Synchronous by contract; mid-command cancellation is not wired in C1
        // (see the module docs).
        let result = execute_solid(modeler.as_ref(), &request, &|| false)?;

        let (brep, kernel_diagnostics) = match result.outcome {
            SolidOutcome::Success(SolidProduct::Body { brep, diagnostics }) => (brep, diagnostics),
            SolidOutcome::Success(SolidProduct::Mass(_)) => {
                return Err(CadError::Invariant(format!(
                    "{}.unexpected_mass: the modeler answered a primitive create with mass \
                     properties",
                    kind.as_str()
                )));
            }
            SolidOutcome::Unsupported { reason } => {
                return Err(CadError::Unsupported(format!(
                    "{}: {}",
                    reason.code, reason.detail
                )));
            }
            SolidOutcome::Failed { diagnostics } => {
                return Err(CadError::Unsupported(format!(
                    "solid.kernel_failed: {}",
                    kernel_failure_message(&diagnostics)
                )));
            }
        };

        let body = solid_body(brep, self.solid_tessellator.as_deref())?;
        let mut outcome = self.commit_new_entity_typed(
            session,
            command,
            SemanticGeometry::Body3d(Box::new(body)),
            SOLID_TYPE_KEY,
            match kind {
                PrimitiveKind::Box => "create box",
                PrimitiveKind::Wedge => "create wedge",
                PrimitiveKind::Pyramid => "create pyramid",
                PrimitiveKind::Cylinder => "create cylinder",
                PrimitiveKind::Sphere => "create sphere",
                PrimitiveKind::Cone => "create cone",
                PrimitiveKind::Torus => "create torus",
            },
        )?;
        // Non-fatal kernel notes are carried through, never dropped: they are the
        // only place a "created, but the kernel said X" answer is visible.
        outcome.diagnostics.extend(kernel_diagnostics);
        Ok(outcome)
    }

    /// `UNION` / `SUBTRACT` / `INTERSECT` as a real **background** job (C2).
    ///
    /// Everything that can refuse happens here, before any worker or staging:
    /// payload shape, selection arity, whole-body refs, layer/space agreement and
    /// every operand's `Body3d { Solid, Exact }` (stored as `AcDb3dSolid`). A
    /// curved/unliftable operand is deliberately **not** pre-judged: the modeler
    /// decides, and its `Unsupported` answer becomes the job's `Failed` terminal.
    ///
    /// On success this starts the job and returns [`CommandOutcome::none`] with a
    /// `solid.boolean_started` diagnostic: the drawing, its revision and history
    /// stay untouched until the host polls a current, non-stale result and commits
    /// it in one transaction.
    pub(crate) fn boolean_solids(
        &mut self,
        session: &SessionState,
        command: &Command,
        op: BooleanOp,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, op_name(op))?;
        let Some(modeler) = self.solid_modeler.clone() else {
            return Err(CadError::Unsupported(SOLID_MODELER_UNSUPPORTED.into()));
        };
        let refs = session.selection.refs().to_vec();
        // A boolean combines bodies, never a face/edge of one or a block member.
        if refs
            .iter()
            .any(|reference| reference.sub_element.is_some() || !reference.instance.0.is_empty())
        {
            return Err(invalid(
                "boolean.sub_element_not_supported",
                format!(
                    "{} combines whole bodies; a sub-element or instance-path reference is not a body",
                    op_label(op)
                ),
            ));
        }
        if refs.len() < 2 {
            return Err(invalid(
                "boolean.needs_two",
                format!(
                    "{} needs at least two selected solid bodies, got {}",
                    op_label(op),
                    refs.len()
                ),
            ));
        }
        let (plan, mut diagnostics) = {
            let Some(document) = self.workspace.documents.get(&command.document) else {
                return Err(CadError::InvalidInput("document not open".into()));
            };
            let drawing = document.drawing.as_ref();
            let (operands, diagnostics) = Self::resolve_selection(
                drawing,
                command.document,
                &refs,
                "boolean",
                op_name(op),
                "boolean",
                false,
            )?;
            if operands.len() < 2 {
                return Err(invalid(
                    "boolean.needs_two",
                    format!(
                        "{} needs at least two distinct selected solid bodies",
                        op_label(op)
                    ),
                ));
            }
            let layer = operands[0].layer;
            let space = operands[0].space.clone();
            let mut snapshots = Vec::with_capacity(operands.len());
            for entity in &operands {
                if entity.layer != layer {
                    return Err(invalid(
                        "boolean.layer_mismatch",
                        format!(
                            "entity {} is on layer {} but the first operand is on layer {}",
                            entity.id.0, entity.layer.0, layer.0
                        ),
                    ));
                }
                if entity.space != space {
                    return Err(invalid(
                        "boolean.space_mismatch",
                        format!(
                            "entity {} is in another space than the first operand; a boolean never \
                             crosses model/paper space",
                            entity.id.0
                        ),
                    ));
                }
                let snapshot = entity.clone();
                let brep = boolean_operand_brep(&snapshot).ok_or_else(|| {
                    invalid(
                        "boolean.non_solid_operand",
                        format!(
                            "entity {} is {}; {} needs exact {SOLID_TYPE_KEY} solid bodies",
                            entity.id.0,
                            describe_operand(entity),
                            op_label(op)
                        ),
                    )
                })?;
                snapshots.push(SolidBooleanOperand {
                    id: snapshot.id,
                    revision: snapshot.object.revision,
                    brep,
                    snapshot,
                });
            }
            let plan = SolidBooleanPlan {
                document: command.document,
                op,
                layer,
                space,
                // The result replaces the group in place: inherit the lowest
                // draw order instead of jumping above every entity.
                draw_order: operands
                    .iter()
                    .map(|entity| entity.draw_order)
                    .min()
                    .unwrap_or(0),
                operands: snapshots,
            };
            (plan, diagnostics)
        };
        let operand_count = plan.operands.len();
        self.start_solid_boolean(plan, modeler)?;
        diagnostics.push(Diagnostic {
            object: None,
            code: "solid.boolean_started".into(),
            message: format!(
                "{}: started in the background for {operand_count} bodies; the drawing changes when \
                 the job settles",
                op_label(op)
            ),
        });
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

/// The lower-case stable tag of a boolean (diagnostic/selection stem).
pub(crate) fn op_name(op: BooleanOp) -> &'static str {
    match op {
        BooleanOp::Union => "union",
        BooleanOp::Subtract => "subtract",
        BooleanOp::Intersect => "intersect",
    }
}

/// The upper-case command word of a boolean.
fn op_label(op: BooleanOp) -> &'static str {
    match op {
        BooleanOp::Union => "UNION",
        BooleanOp::Subtract => "SUBTRACT",
        BooleanOp::Intersect => "INTERSECT",
    }
}

/// A plan-level boolean refusal: `InvalidInput` carrying its stable code.
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// The exact B-rep of a boolean operand, or `None` when the row is not one.
///
/// The stored class key, the topological kind and the exact geometry are all
/// required: a `Sheet`/`Wire`/`Unknown` body or a `BodyExact::Unavailable` row
/// would give the kernel nothing exact to combine.
fn boolean_operand_brep(entity: &DbEntity) -> Option<BrepData> {
    if entity.object.type_key != SOLID_TYPE_KEY {
        return None;
    }
    let SemanticGeometry::Body3d(body) = &entity.geometry else {
        return None;
    };
    if body.kind != BodyKind::Solid {
        return None;
    }
    match &body.exact {
        BodyExact::Exact(brep) => Some(brep.as_ref().clone()),
        BodyExact::Unavailable { .. } => None,
    }
}

/// A short honest description of a rejected operand row.
///
/// Only an `AcDb3dSolid` row holding an exact `Body3d { Solid, Exact }` is an
/// operand, so the sentence names the stored type **and** what its geometry
/// actually is. An exact solid stored under another key (for example an
/// imported `AcDbBody` row) is described as such instead of being called a
/// malformed `AcDb3dSolid`.
pub(crate) fn describe_operand(entity: &DbEntity) -> String {
    let type_key = &entity.object.type_key;
    match &entity.geometry {
        SemanticGeometry::Body3d(body) => {
            let kind = match body.kind {
                BodyKind::Solid => "solid",
                BodyKind::Sheet => "sheet",
                BodyKind::Wire => "wire",
                BodyKind::Unknown => "unknown-topology",
            };
            let exact = if body.exact_brep().is_some() {
                "exact"
            } else {
                "non-exact"
            };
            if type_key == SOLID_TYPE_KEY {
                format!("a row of type {type_key} holding a {exact} {kind} body")
            } else {
                format!(
                    "a row of type {type_key} holding a {exact} {kind} body (only {SOLID_TYPE_KEY} \
                     rows are operands)"
                )
            }
        }
        _ => format!("a row of type {type_key} that is not a body"),
    }
}

/// Build the committed [`Body3d`] from the modeler's exact B-rep.
///
/// The display cache is the shared tessellator's output; a body whose facets are
/// missing or empty is refused (`solid.tessellation_failed`) instead of being
/// committed as a `Solid` with an empty display — `cad-db` would accept that and
/// the representer would then draw nothing.
///
/// `tessellator` is the carrier's injected display mesher (P2.1c part 1), or
/// `None` for the neutral path. The seam is consulted **only** for a body with a
/// NURBS face (see `cad_kernel_adapter::tessellate_exchange_with`), so an
/// analytic-only body's facets are exactly what they were before. Cancellation is
/// checked by the callers before this bounded display step; the neutral path is
/// not mid-cancellable, so the seam is handed a `false` probe rather than a token
/// that could silently stop the fallback.
pub(crate) fn solid_body(
    brep: BrepData,
    tessellator: Option<&dyn cad_kernel_adapter::SolidTessellator>,
) -> CadResult<Body3d> {
    // The **shared** shape-relative rule (`1e-3 x bbox diagonal`): the same
    // policy the importer uses, so a created body's facet density does not
    // depend on the drawing unit, and a huge primitive asks for a chord budget
    // in proportion to its own size instead of an absolute 0.01 constant (which
    // used to turn a radius-500 sphere into a budget refusal and could make the
    // tessellator materialise a 100M-triangle mesh before it was rejected).
    let (tolerance, _diagonal) = cad_kernel_adapter::shape_relative_tolerance(&brep);
    let outcome = cad_kernel_adapter::tessellate_exchange_with(
        &SolidExchange::Brep(brep.clone()),
        tolerance,
        TessellationBudget::default(),
        tessellator,
        &|| false,
    );
    let usable = outcome.is_success() && outcome.mesh().is_some_and(|mesh| !mesh.is_empty());
    if !usable {
        return Err(CadError::Unsupported(format!(
            "solid.tessellation_failed: {}",
            tessellation_failure(&outcome)
        )));
    }
    let geometry = outcome.mesh().ok_or_else(|| {
        CadError::Unsupported("solid.tessellation_failed: no facet set".to_string())
    })?;
    // The completeness follows the achieved precision: a planar body is exact
    // (Analytic); a curved one is drawn as facets at the tolerance above and
    // says so, with the same sentence the import path uses.
    let completeness = match &geometry.precision {
        Precision::Analytic => Completeness::Complete,
        Precision::Approximate { .. } | Precision::Unknown => {
            Completeness::Partial(vec![cad_kernel_adapter::discrete_mesh_reason(
                tolerance.linear_deflection,
            )])
        }
    };
    Ok(Body3d {
        kind: BodyKind::Solid,
        exact: BodyExact::Exact(Box::new(brep)),
        display: BodyDisplay {
            mesh: geometry.mesh.clone(),
            edges: geometry.edges.clone(),
            tolerance: Some(tolerance.linear_deflection),
            precision: geometry.precision.clone(),
            completeness,
        },
    })
}

/// Why a tessellation attempt produced no usable display mesh.
///
/// `TessellationOutcome::Unsupported` carries **no diagnostics** (it has a
/// structured `reason` instead), so reporting "kernel failed" for it would
/// lose the only actionable information; each arm reports what it actually has.
fn tessellation_failure(outcome: &TessellationOutcome) -> String {
    match outcome {
        TessellationOutcome::Unsupported { reason } => {
            format!("{}: {}", reason.code, reason.detail)
        }
        TessellationOutcome::Success { .. } | TessellationOutcome::Partial { .. } => outcome
            .diagnostics()
            .first()
            .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
            .unwrap_or_else(|| "no facet set".to_string()),
        TessellationOutcome::Failed { diagnostics } => diagnostics
            .first()
            .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
            .unwrap_or_else(|| "no diagnostic".to_string()),
    }
}

/// The kernel failure text: the diagnostic's **message alone**.
///
/// The outer `solid.kernel_failed:` prefix already names the situation, so
/// repeating a diagnostic whose own code is `solid.kernel_failed` would read
/// `solid.kernel_failed: solid.kernel_failed: …`.
pub(crate) fn kernel_failure_message(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .first()
        .map(|diagnostic| diagnostic.message.clone())
        .unwrap_or_else(|| "the kernel produced no body".to_string())
}
