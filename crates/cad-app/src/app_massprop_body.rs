//! ADR-0036 phase C3a: `MASSPROP` for exact `Body3d` solids.
//!
//! A selected `Body3d { kind: Solid, exact: Exact(non-empty) }` is measured by
//! the **injected** [`cad_kernel_adapter::SolidModeler`] through the same
//! `MassProperties` seam every other solid operation uses. The display
//! tessellation is never integrated: a mesh volume is not the solid's volume,
//! so a body without exact geometry is refused by name rather than approximated.
//!
//! # Carrier capability
//!
//! `cad-app` links no kernel. A carrier without a modeler answers
//! [`REASON_BODY_MODELER_UNAVAILABLE`] ("no exact B-rep kernel in this
//! carrier"); the modeler's own refusals stay structured
//! ([`REASON_BODY_NOT_LIFTABLE`] / [`REASON_BODY_KERNEL_FAILED`]).
//!
//! # Synchrony and the kernel mutex
//!
//! This is a **read**, so it adds no worker thread and no cancellation token:
//! the modeler call runs synchronously on the command thread with an
//! always-`false` probe. The seam shares one process-global kernel mutex with
//! the primitive/boolean paths, so the body path refuses by name while a
//! background boolean owns the kernel ([`REASON_KERNEL_BUSY`],
//! `massprop.kernel_busy`) instead of silently blocking the command thread on
//! that mutex — the same fail-fast rule the primitive creates use.
//!
//! The report is a single `massprop.object` diagnostic per body. Its accuracy is
//! the kernel's **requested** relative accuracy, printed as an estimate, never
//! as a proved bound and never as analytic.

use super::*;
use cad_kernel_adapter::{
    execute_solid, MassAccuracy, SolidMassProperties, SolidOperation, SolidOutcome, SolidProduct,
    SolidRequest,
};
use std::collections::BTreeSet;

/// The density every body is integrated at: AutoCAD's unitless default.
pub(super) const MASSPROP_DENSITY: f64 = 1.0;

/// Stable refusal: this carrier injected no exact B-rep modeler.
pub(super) const REASON_BODY_MODELER_UNAVAILABLE: &str = "massprop.body_modeler_unavailable";

/// Stable refusal: the modeler cannot lift this body's B-rep (for example a
/// curved body before phase P2 answers `solid.lift_requires_planar`).
pub(super) const REASON_BODY_NOT_LIFTABLE: &str = "massprop.body_not_liftable";

/// Stable refusal: the modeler ran but produced no usable mass properties.
pub(super) const REASON_BODY_KERNEL_FAILED: &str = "massprop.body_kernel_failed";

/// Stable refusal: the body carries no exact B-rep (missing or empty).
pub(super) const REASON_BODY_NOT_EXACT: &str = "massprop.body_not_exact";

/// Stable refusal: the body is a wire/sheet/unknown topology, not a solid.
pub(super) const REASON_BODY_NOT_SOLID: &str = "massprop.body_not_solid";

/// Stable refusal: the kernel reported no positive enclosed volume.
pub(super) const REASON_BODY_NOT_POSITIVE_VOLUME: &str = "massprop.body_not_positive_volume";

/// Stable refusal: a background solid boolean currently owns the kernel.
pub(super) const REASON_KERNEL_BUSY: &str = "massprop.kernel_busy";

/// Stable refusal: the selection mixes bodies and planar regions.
pub(super) const REASON_MIXED_SELECTION: &str = "massprop.mixed_selection";

/// One selection, routed by report shape.
pub(super) struct Routed<'a> {
    /// Distinct `Body3d` rows, in selection order (a body and its face refs
    /// collapse to one entry).
    pub(super) bodies: Vec<&'a cad_db::DbEntity>,
    /// Non-body rows, in selection order.
    pub(super) regions: Vec<&'a cad_db::DbEntity>,
}

/// Resolve a selection into borrowed rows without cloning them.
///
/// A sub-element ref is measurable only on a body: it names one face of a body
/// whose mass is the body's mass, so the ref resolves to the owning body and all
/// refs to that body count once. A sub-element on any other geometry, and a
/// block-instance member, keep the existing `massprop.not_addressable` refusal.
pub(super) fn route<'a>(
    app: &'a Application,
    document: DocumentId,
    references: &[SelectionRef],
) -> CadResult<Routed<'a>> {
    let mut bodies: Vec<&cad_db::DbEntity> = Vec::with_capacity(references.len());
    let mut regions: Vec<&cad_db::DbEntity> = Vec::with_capacity(references.len());
    for reference in references {
        if reference.document != document {
            return Err(CadError::InvalidInput(format!(
                "massprop.target_missing: the selected ref for entity {} belongs to another \
                 document",
                reference.entity.0
            )));
        }
        if !reference.instance.0.is_empty() {
            return Err(not_addressable());
        }
        let drawing = &app
            .workspace
            .documents
            .get(&document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing;
        let entity = drawing.entity(reference.entity).ok_or_else(|| {
            CadError::InvalidInput(format!(
                "massprop.target_missing: entity {} does not exist in this drawing",
                reference.entity.0
            ))
        })?;
        if matches!(entity.geometry, SemanticGeometry::Body3d(_)) {
            if !bodies.iter().any(|body| body.id == entity.id) {
                bodies.push(entity);
            }
        } else if reference.sub_element.is_some() {
            return Err(not_addressable());
        } else {
            regions.push(entity);
        }
    }
    Ok(Routed { bodies, regions })
}

/// Stable refusal for a ref that names no measurable entity of its own.
///
/// A block-instance member has no addressable identity, and a sub-element of
/// non-body geometry has no mass property (a body face is resolved to its
/// owning body earlier, so this never fires for one).
fn not_addressable() -> CadError {
    CadError::Unsupported(
        "massprop.not_addressable: block-instance members and sub-elements have no addressable \
         entity identity, so they have no mass properties"
            .into(),
    )
}

/// Refuse a selection that mixes the two report shapes.
///
/// A body reports `volume`/inertia and a planar region reports
/// `area`/perimeter, so the two are measured by separate runs; combining them
/// would need a summary whose `massprop.solid_unsupported` line contradicts the
/// body rows beside it. The message names the distinct non-body kinds so the
/// caller sees which objects are the offending ones.
pub(super) fn require_single_kind(bodies: usize, non_body: &[&cad_db::DbEntity]) -> CadResult<()> {
    if non_body.is_empty() {
        return Ok(());
    }
    let mut ids = BTreeSet::new();
    let mut kinds = BTreeSet::new();
    for entity in non_body {
        ids.insert(entity.id.0);
        kinds.insert(geometry_kind(&entity.geometry));
    }
    let kinds: Vec<&str> = kinds.into_iter().collect();
    Err(CadError::Unsupported(format!(
        "{}: MASSPROP measures either exact 3D solid bodies or closed planar regions, not both; \
         the selection has {bodies} distinct 3D body entity/entities and {} distinct other \
         object(s) ({})",
        REASON_MIXED_SELECTION,
        ids.len(),
        kinds.join(", ")
    )))
}

impl Application {
    /// `MASSPROP` for a selection made only of `Body3d` rows.
    ///
    /// Every geometry/modeler refusal happens before the first diagnostic is
    /// built, so a refused run reports one stable error and leaves the session,
    /// drawing, revision, history and selection untouched.
    pub(super) fn mass_prop_bodies(
        &self,
        session: &SessionState,
        command: &Command,
        entities: &[&cad_db::DbEntity],
    ) -> CadResult<CommandOutcome> {
        // The exact B-rep is the only measurable input. Check every body first so
        // a missing kernel is never blamed for a body that has nothing to lift.
        let mut breps = Vec::with_capacity(entities.len());
        for entity in entities.iter().copied() {
            let SemanticGeometry::Body3d(body) = &entity.geometry else {
                return Err(CadError::Invariant(format!(
                    "massprop.body_missing_route: entity {} is not a 3D body",
                    entity.id.0
                )));
            };
            if body.kind != BodyKind::Solid {
                return Err(CadError::Unsupported(format!(
                    "{}: entity {} ({}) is a {:?} body, not a closed solid; its display mesh is \
                     never measured as a solid",
                    REASON_BODY_NOT_SOLID, entity.id.0, entity.object.type_key, body.kind
                )));
            }
            let brep = match &body.exact {
                BodyExact::Exact(brep) if !brep.is_empty() => brep.as_ref(),
                BodyExact::Exact(_) => {
                    return Err(CadError::Unsupported(format!(
                        "{}: entity {} ({}) has an empty exact B-rep; its display mesh is never \
                         measured as the solid's volume",
                        REASON_BODY_NOT_EXACT, entity.id.0, entity.object.type_key
                    )))
                }
                BodyExact::Unavailable { reason } => {
                    return Err(CadError::Unsupported(format!(
                        "{}: entity {} ({}) has no exact B-rep ({reason}); its display mesh is \
                         never measured as the solid's volume",
                        REASON_BODY_NOT_EXACT, entity.id.0, entity.object.type_key
                    )))
                }
            };
            breps.push(brep);
        }

        // One process-global kernel mutex is shared with the boolean worker: a
        // synchronous read must not block behind it. Refuse by name, before the
        // modeler call, exactly like the primitive creates (`solid.boolean_busy`).
        if self.solid_boolean_running() {
            return Err(CadError::Unsupported(format!(
                "{}: a solid boolean is running; wait for it to settle",
                REASON_KERNEL_BUSY
            )));
        }

        let Some(modeler) = self.solid_modeler.clone() else {
            return Err(CadError::Unsupported(format!(
                "{}: no exact B-rep kernel in this carrier; the selected 3D solid bodies are not \
                 measured and their display meshes are never substituted",
                REASON_BODY_MODELER_UNAVAILABLE
            )));
        };

        // One read-only integration per body. The stamp is the current
        // document/generation because the seam is synchronous: there is no late
        // result to reject.
        let stamp = TaskStamp::new(command.document, session.generation);
        let mut diagnostics = Vec::with_capacity(entities.len());
        for (entity, brep) in entities.iter().copied().zip(&breps) {
            let request = SolidRequest {
                stamp: stamp.clone(),
                operation: SolidOperation::MassProperties {
                    brep,
                    density: MASSPROP_DENSITY,
                },
            };
            let result = execute_solid(modeler.as_ref(), &request, &|| false)?;
            let mass = match result.outcome {
                SolidOutcome::Success(SolidProduct::Mass(mass)) => mass,
                SolidOutcome::Success(SolidProduct::Body { .. }) => {
                    return Err(CadError::Invariant(format!(
                        "massprop.unexpected_body: the modeler answered entity {}'s \
                         mass-properties request with a body",
                        entity.id.0
                    )))
                }
                SolidOutcome::Unsupported { reason } => {
                    return Err(CadError::Unsupported(format!(
                        "{}: {}: {}",
                        REASON_BODY_NOT_LIFTABLE, reason.code, reason.detail
                    )))
                }
                SolidOutcome::Failed {
                    diagnostics: kernel,
                } => {
                    return Err(CadError::Unsupported(format!(
                        "{}: {}",
                        REASON_BODY_KERNEL_FAILED,
                        kernel
                            .first()
                            .map(|diagnostic| format!(
                                "{}: {}",
                                diagnostic.code, diagnostic.message
                            ))
                            .unwrap_or_else(|| "the kernel produced no mass properties".into())
                    )))
                }
            };
            // A reflected/inside-out B-rep integrates to a negative enclosed
            // volume; no mass property is meaningful, so this is named rather
            // than printed as a negative solid.
            if !mass.volume.is_finite() || mass.volume <= 0.0 {
                return Err(CadError::Unsupported(format!(
                    "{}: entity {} ({}) encloses volume {} (not finite and positive); no mass \
                     properties are reported",
                    REASON_BODY_NOT_POSITIVE_VOLUME,
                    entity.id.0,
                    entity.object.type_key,
                    mass.volume
                )));
            }
            diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "massprop.object".into(),
                message: body_report(entity, &mass),
            });
        }

        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

/// One body's `massprop.object` line.
///
/// The units are the drawing's (volume in units³, area in units²), no conversion
/// happens here. The inertia tensor is reported about the centroid with the
/// **products** as the printed `Iyz`/`Izx`/`Ixy` (the seam stores the standard
/// tensor, whose off-diagonal entries are negated products).
fn body_report(entity: &cad_db::DbEntity, mass: &SolidMassProperties) -> String {
    let accuracy = match mass.accuracy {
        MassAccuracy::Requested { relative } => {
            format!("estimated at requested relative accuracy {relative:e}, not a proved bound")
        }
    };
    format!(
        "MASSPROP: entity {} (3D solid {}, layer {}) volume={:.6} surface_area={:.6} \
         centroid={} density={:.6} mass={:.6} inertia about the centroid: {}; {accuracy}",
        entity.id.0,
        entity.object.type_key,
        entity.layer.0,
        mass.volume,
        mass.surface_area,
        format_point(mass.centroid),
        mass.density,
        mass.volume * mass.density,
        inertia_report(mass.inertia),
    )
}

/// The centroidal moments/products, or the honest absence of a tensor.
fn inertia_report(inertia: Option<[[f64; 3]; 3]>) -> String {
    match inertia {
        Some(tensor) => format!(
            "Ixx={:.6} Iyy={:.6} Izz={:.6} Iyz={:.6} Izx={:.6} Ixy={:.6}",
            tensor[0][0],
            tensor[1][1],
            tensor[2][2],
            positive_zero(-tensor[1][2]),
            positive_zero(-tensor[2][0]),
            positive_zero(-tensor[0][1]),
        ),
        None => "NOT REPORTED (the kernel returned no tensor)".into(),
    }
}

/// Print a zero product as `0`, never `-0` (negating a stored `+0.0` gives
/// `-0.0`, whose six-decimal rendering is `-0.000000`).
fn positive_zero(value: f64) -> f64 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}
