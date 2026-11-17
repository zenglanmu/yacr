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
//! # Point contract (this build; no interactive prompt yet)
//!
//! AutoCAD prompts for the dimensions interactively. This carrier has no such
//! prompt in C1, so every command takes its points explicitly and the arity is
//! enforced (a wrong count is an `InvalidInput` with a stable
//! `<kind>.requires_{two,three}_points` code, never a silent truncation):
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
//! Every point must be finite (`<kind>.non_finite_point`). Non-positive or
//! non-finite extents are `<kind>.degenerate_extent`; a non-square pyramid base
//! or an off-centre apex is `<kind>.invalid_base`; a cone whose top radius equals
//! its base radius is `cone.equal_radii` (that is a `CYLINDER`).
//!
//! # Carrier capability
//!
//! A carrier that links no modeler passes `None` and every one of the seven
//! refuses with `unsupported-runtime: solid-modeler`, writing nothing (no
//! revision bump, no history entry).
//!
//! **Code fact (what is wired today)**: only the Linux `native-below` session
//! injects `cad_kernel_solidcore::SolidCoreModeler`. Linux `dom-canvas`
//! (X11/XWayland — its CAD session is the wasm one in the WebView), Web
//! (wasm), Windows, macOS and Android do **not** inject a modeler, so the seven
//! commands refuse by name there. Whether those carriers can ever host the
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
use crate::tasks::{SolidBooleanOperand, SolidBooleanPlan};
use cad_kernel_adapter::{
    execute_solid, tessellate_exchange, BooleanOp, PrimitiveKind, PrimitiveSpec, SolidExchange,
    SolidOperation, SolidOutcome, SolidProduct, SolidRequest, TessellationBudget,
    TessellationOutcome,
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
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(format!(
                "{}.requires_points: {} takes an explicit Points([..]) payload (no interactive \
                 prompt in this build)",
                kind.as_str(),
                kind.as_str().to_ascii_uppercase()
            )));
        };
        // Point mapping and validation run before any modeler call: a bad payload
        // is an input error, never an `Unsupported`/`Failed` kernel answer.
        let spec = primitive_spec(kind, points)?;

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

        let body = solid_body(brep)?;
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
pub(crate) fn solid_body(brep: BrepData) -> CadResult<Body3d> {
    // The **shared** shape-relative rule (`1e-3 x bbox diagonal`): the same
    // policy the importer uses, so a created body's facet density does not
    // depend on the drawing unit, and a huge primitive asks for a chord budget
    // in proportion to its own size instead of an absolute 0.01 constant (which
    // used to turn a radius-500 sphere into a budget refusal and could make the
    // tessellator materialise a 100M-triangle mesh before it was rejected).
    let (tolerance, _diagonal) = cad_kernel_adapter::shape_relative_tolerance(&brep);
    let outcome = tessellate_exchange(
        &SolidExchange::Brep(brep.clone()),
        tolerance,
        TessellationBudget::default(),
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

/// Map the explicit point payload onto the seam's [`PrimitiveSpec`].
///
/// All arity, finiteness and extent rules of the module table are enforced here,
/// so `PrimitiveSpec::validate` (and the kernel) never sees a bad payload.
fn primitive_spec(kind: PrimitiveKind, points: &[Point3]) -> CadResult<PrimitiveSpec> {
    match kind {
        PrimitiveKind::Box => {
            require_arity(kind, points, 2)?;
            let (p0, p1) = (points[0], points[1]);
            require_finite(kind, &[p0, p1])?;
            let (low, high) = (min_point(p0, p1), max_point(p0, p1));
            Ok(PrimitiveSpec::Box {
                origin: low,
                length: require_extent(kind, "dx (|p1.x - p0.x|)", high.x - low.x)?,
                width: require_extent(kind, "dy (|p1.y - p0.y|)", high.y - low.y)?,
                height: require_extent(kind, "dz (|p1.z - p0.z|)", high.z - low.z)?,
            })
        }
        PrimitiveKind::Wedge => {
            require_arity(kind, points, 3)?;
            let (p0, p1, p2) = (points[0], points[1], points[2]);
            require_finite(kind, &[p0, p1, p2])?;
            let dx = (p1.x - p0.x).abs();
            let dy = (p1.y - p0.y).abs();
            let length = require_extent(kind, "dx (|p1.x - p0.x|)", dx)?;
            let width = require_extent(kind, "dy (|p1.y - p0.y|)", dy)?;
            // The base must be one plane: a base built from two different z
            // values has no single `height` and no kernel wedge to match it.
            let scale = length.max(width).max(1.0);
            if (p0.z - p1.z).abs() > 1e-9 * scale {
                return Err(CadError::InvalidInput(format!(
                    "wedge.base_not_coplanar: the two base corners must share z, got {} and {}",
                    p0.z, p1.z
                )));
            }
            // The tall face is at the **minimum-x** corner (`x = origin.x`), the
            // `polyhedra::wedge_brep` convention; reversing p0/p1 cannot move it.
            let origin = Point3 {
                x: p0.x.min(p1.x),
                y: p0.y.min(p1.y),
                z: p0.z.min(p1.z),
            };
            Ok(PrimitiveSpec::Wedge {
                origin,
                length,
                width,
                height: require_extent(kind, "height (p2.z - base z)", p2.z - origin.z)?,
            })
        }
        PrimitiveKind::Pyramid => {
            require_arity(kind, points, 3)?;
            let (p0, p1, apex) = (points[0], points[1], points[2]);
            require_finite(kind, &[p0, p1, apex])?;
            let dx = (p1.x - p0.x).abs();
            let dy = (p1.y - p0.y).abs();
            let length = require_extent(kind, "dx (|p1.x - p0.x|)", dx)?;
            let width = require_extent(kind, "dy (|p1.y - p0.y|)", dy)?;
            // A rectangular base would need a different apex rule than the seam's
            // square-base `Pyramid`; refuse rather than silently square it.
            let scale = length.max(width).max(1.0);
            if (dx - dy).abs() > 1e-9 * scale {
                return Err(CadError::InvalidInput(format!(
                    "pyramid.invalid_base: the base must be square, got {dx} x {dy}"
                )));
            }
            // The base is one horizontal plane (`z = p0.z = p1.z`), exactly like
            // WEDGE: a tilted base has no single height, and silently taking the
            // lower z would drop the other corner's z.
            if (p0.z - p1.z).abs() > 1e-9 * scale {
                return Err(CadError::InvalidInput(format!(
                    "pyramid.base_not_coplanar: the two base corners must share z, got {} and {}",
                    p0.z, p1.z
                )));
            }
            let base_z = p0.z.min(p1.z);
            let centre = Point3 {
                x: (p0.x + p1.x) / 2.0,
                y: (p0.y + p1.y) / 2.0,
                z: base_z,
            };
            let off_centre = ((apex.x - centre.x).powi(2) + (apex.y - centre.y).powi(2)).sqrt();
            if off_centre > 1e-9 * scale {
                return Err(CadError::InvalidInput(format!(
                    "pyramid.invalid_base: the apex must be over the base centre, it is \
                     {off_centre} away in xy"
                )));
            }
            Ok(PrimitiveSpec::Pyramid {
                base_center: centre,
                length,
                height: require_extent(kind, "height (apex.z - base z)", apex.z - base_z)?,
            })
        }
        PrimitiveKind::Cylinder => {
            require_arity(kind, points, 3)?;
            let (centre, radius_point, top) = (points[0], points[1], points[2]);
            require_finite(kind, &[centre, radius_point, top])?;
            Ok(PrimitiveSpec::Cylinder {
                origin: centre,
                radius: require_extent(
                    kind,
                    "radius (|r - c| in xy)",
                    planar_distance(centre, radius_point),
                )?,
                height: require_extent(kind, "height (t.z - c.z)", top.z - centre.z)?,
            })
        }
        PrimitiveKind::Sphere => {
            require_arity(kind, points, 2)?;
            let (centre, radius_point) = (points[0], points[1]);
            require_finite(kind, &[centre, radius_point])?;
            Ok(PrimitiveSpec::Sphere {
                origin: centre,
                radius: require_extent(kind, "radius (|p1 - p0|)", distance(centre, radius_point))?,
            })
        }
        PrimitiveKind::Cone => {
            require_arity(kind, points, 3)?;
            let (centre, radius_point, top) = (points[0], points[1], points[2]);
            require_finite(kind, &[centre, radius_point, top])?;
            let radius_bottom = require_extent(
                kind,
                "radius_bottom (|r - c| in xy)",
                planar_distance(centre, radius_point),
            )?;
            // `0` is the seam's full-cone apex; any other value is a frustum top.
            let radius_top = planar_distance(centre, top);
            if !radius_top.is_finite() {
                return Err(CadError::InvalidInput(
                    "cone.degenerate_extent: radius_top must be finite".into(),
                ));
            }
            // The upstream cone constructor refuses a zero slope: equal radii
            // are a cylinder, and this build says so instead of surfacing a
            // kernel failure.
            if (radius_top - radius_bottom).abs() <= 1e-12 * radius_bottom.max(1.0) {
                return Err(CadError::InvalidInput(format!(
                    "cone.equal_radii: radius_top {radius_top} equals radius_bottom \
                     {radius_bottom}; use CYLINDER for a straight tube"
                )));
            }
            Ok(PrimitiveSpec::Cone {
                origin: centre,
                radius_bottom,
                radius_top,
                height: require_extent(kind, "height (t.z - c.z)", top.z - centre.z)?,
            })
        }
        PrimitiveKind::Torus => {
            require_arity(kind, points, 3)?;
            let (centre, major_point, minor_point) = (points[0], points[1], points[2]);
            require_finite(kind, &[centre, major_point, minor_point])?;
            let major = require_extent(
                kind,
                "major_radius (|m - c| in xy)",
                planar_distance(centre, major_point),
            )?;
            let minor = require_extent(
                kind,
                "minor_radius (|n - m|)",
                distance(major_point, minor_point),
            )?;
            if minor >= major {
                return Err(CadError::InvalidInput(format!(
                    "torus.degenerate_extent: the minor radius {minor} must be smaller than the \
                     major radius {major}"
                )));
            }
            Ok(PrimitiveSpec::Torus {
                origin: centre,
                major_radius: major,
                minor_radius: minor,
            })
        }
    }
}

/// Enforce the explicit point count of one primitive family.
fn require_arity(kind: PrimitiveKind, points: &[Point3], expected: usize) -> CadResult<()> {
    if points.len() == expected {
        return Ok(());
    }
    let word = if expected == 2 { "two" } else { "three" };
    Err(CadError::InvalidInput(format!(
        "{}.requires_{word}_points: {} takes exactly {expected} point(s), got {}",
        kind.as_str(),
        kind.as_str().to_ascii_uppercase(),
        points.len()
    )))
}

fn require_finite(kind: PrimitiveKind, points: &[Point3]) -> CadResult<()> {
    if points.iter().all(|point| is_finite_point(*point)) {
        return Ok(());
    }
    Err(CadError::InvalidInput(format!(
        "{}.non_finite_point: every point must have finite x, y and z",
        kind.as_str()
    )))
}

fn require_extent(kind: PrimitiveKind, name: &str, value: f64) -> CadResult<f64> {
    if value.is_finite() && value > 0.0 {
        return Ok(value);
    }
    Err(CadError::InvalidInput(format!(
        "{}.degenerate_extent: {name} must be finite and positive, got {value}",
        kind.as_str()
    )))
}

fn planar_distance(a: Point3, b: Point3) -> f64 {
    ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt()
}

fn min_point(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x.min(b.x),
        y: a.y.min(b.y),
        z: a.z.min(b.z),
    }
}

fn max_point(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x.max(b.x),
        y: a.y.max(b.y),
        z: a.z.max(b.z),
    }
}
