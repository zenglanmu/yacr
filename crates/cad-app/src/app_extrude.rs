//! `EXTRUDE`: one selected closed planar profile extruded into a new exact solid.
//!
//! This is the "draw a shape, pull it up" half of SketchUp's modelling surface,
//! kept as an atomic command: the **profile** is the current selection (LINEs, a
//! closed straight LWPOLYLINE, or one CIRCLE) and the signed distance is the
//! session typed channel. The result is a **new** `AcDb3dSolid` row on the active
//! layer; the profile entities stay exactly as they were (this build has no
//! first-class face entity, so the 2D loop is the durable input).
//!
//! The host also reaches this command from the **Push/Pull** tool when the
//! selection is such a profile (there is no solid face to pick yet), through
//! [`Application::closed_profile_selection`]; the command, its validation and
//! its single transaction are the same either way.
//!
//! # Supported profiles (first cut)
//!
//! * LINEs forming exactly one closed chain (every endpoint used twice);
//! * one closed LWPOLYLINE whose segments are all straight;
//! * one CIRCLE, alone.
//!
//! Every entity must be a top-level model-space row and the loop must lie in a
//! plane parallel to XY (`z` constant): the canonical drafting case. Anything
//! else is refused by name, never approximated:
//!
//! * `extrude.profile_not_closed` — an open or branching chain;
//! * `extrude.profile_unsupported` — an entity family this cut cannot use
//!   (ARC/ELLIPSE/SPLINE/bulged LWPOLYLINE), or a circle mixed with other rows;
//! * `extrude.not_planar` — a chain whose points leave one XY-parallel plane;
//! * `extrude.distance_invalid` — unset/zero/non-finite/oversized distance.
//!
//! Partial-arc walls are deliberately absent: the pinned kernel builds them but
//! its exact B-rep comes back as a rational-spline edge, which this seam's
//! `record → BrepData` reader does not map (`solid.lift_unsupported`). Refusing
//! the profile up front is honest, and the named refusal says why.
//!
//! # Evidence boundary
//!
//! `cad-app` links no kernel: the command contracts use a fake modeler, and the
//! real extrusion is exercised by `cad-kernel-solidcore/tests/extrude_profile.rs`.
//! Nothing here is GPU, pixel, carrier or performance evidence.

use super::*;
use cad_kernel_adapter::{
    execute_solid, SolidOperation, SolidOutcome, SolidProduct, SolidRequest, MAX_EXTRUDE_DISTANCE,
};

/// One straight profile edge in the order it is traversed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Edge {
    start: Point3,
    end: Point3,
}

impl Application {
    /// `EXTRUDE`: extrude the selected closed profile into a new exact solid.
    pub(crate) fn extrude_selected_profile(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let refs =
            match &command.payload {
                CommandPayload::ExtrudeProfile(refs) => refs.as_slice(),
                _ => return Err(invalid(
                    "extrude.requires_profile",
                    "EXTRUDE takes the host-built ExtrudeProfile { refs } payload; it is started \
                     by its pre-selection tool or an explicit payload, never a bare typed line",
                )),
            };
        let distance = validate_extrude_distance(
            session
                .tool_parameter(ToolParameter::ExtrudeDistance)
                .number()
                .unwrap_or(0.0),
        )?;
        let profile = self.profile_from_refs(command.document, refs)?;

        let Some(modeler) = self.solid_modeler.clone() else {
            return Err(CadError::Unsupported(
                crate::app_drawing::solids::SOLID_MODELER_UNSUPPORTED.into(),
            ));
        };
        if self.solid_boolean_running() {
            return Err(CadError::Unsupported(
                "extrude.kernel_busy: a solid boolean is running; wait for it to settle before \
                 extruding"
                    .into(),
            ));
        }
        let direction = if distance > 0.0 {
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            }
        } else {
            Point3 {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            }
        };
        let request = SolidRequest {
            stamp: TaskStamp::new(command.document, session.generation),
            operation: SolidOperation::ExtrudeProfile {
                profile: &profile.curves,
                direction,
                distance: distance.abs(),
            },
        };
        let result = execute_solid(modeler.as_ref(), &request, &|| false)?;
        let (brep, kernel_diagnostics) = match result.outcome {
            SolidOutcome::Success(SolidProduct::Body { brep, diagnostics }) => (brep, diagnostics),
            SolidOutcome::Success(SolidProduct::Mass(_)) => {
                return Err(CadError::Invariant(
                    "extrude.unexpected_mass: the modeler answered an extrusion with mass \
                     properties"
                        .into(),
                ))
            }
            SolidOutcome::Unsupported { reason } => {
                return Err(CadError::Unsupported(format!(
                    "{}: {}",
                    reason.code, reason.detail
                )))
            }
            SolidOutcome::Failed { diagnostics } => {
                return Err(CadError::Unsupported(format!(
                    "solid.kernel_failed: {}",
                    crate::app_drawing::solids::kernel_failure_message(&diagnostics)
                )))
            }
        };

        let body = crate::app_drawing::solids::solid_body(brep, self.solid_tessellator.as_deref())?;
        let mut outcome = self.commit_new_entity_typed(
            session,
            command,
            SemanticGeometry::Body3d(Box::new(body)),
            crate::app_drawing::solids::SOLID_TYPE_KEY,
            "extrude profile",
        )?;
        outcome.diagnostics.extend(kernel_diagnostics);
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: "extrude.applied".into(),
            message: format!(
                "EXTRUDE: extruded the selected {} profile by {distance} along the work plane \
                 normal",
                profile.description
            ),
        });
        Ok(outcome)
    }

    /// Whether the selected top-level rows are one closed XY-parallel profile.
    ///
    /// This is [`Self::profile_from_refs`]'s predicate, exposed read-only so the
    /// host can start **Push/Pull** on a drawn rectangle or circle the way
    /// SketchUp does (there is no face to pick before the first extrusion) and
    /// route it to `EXTRUDE`. It builds nothing, opens no transaction and bumps
    /// no revision; the command validates the same rows again at execute time,
    /// so a document that changed between start and Enter still refuses by name.
    ///
    /// A sub-element or block-instance reference is never a profile: those name
    /// existing geometry, not the loop the user drew.
    pub fn closed_profile_selection(&self, document: DocumentId, refs: &[SelectionRef]) -> bool {
        !refs.is_empty() && self.profile_from_refs(document, refs).is_ok()
    }

    /// Build the one closed XY-parallel profile the selected rows describe.
    fn profile_from_refs(
        &self,
        document: DocumentId,
        refs: &[SelectionRef],
    ) -> CadResult<ExtrudeProfile> {
        let drawing = self
            .workspace
            .documents
            .get(&document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref();
        let mut seen: Vec<EntityId> = Vec::new();
        let mut edges: Vec<Edge> = Vec::new();
        let mut circle: Option<(Point3, Point3, f64)> = None;
        for reference in refs {
            if reference.document != document {
                return Err(invalid(
                    "extrude.selection_changed",
                    "the selected row belongs to another document",
                ));
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(invalid(
                    "extrude.not_addressable",
                    "EXTRUDE reads whole top-level entities, never a block member or a \
                     sub-element",
                ));
            }
            if seen.contains(&reference.entity) {
                continue;
            }
            seen.push(reference.entity);
            let entity = drawing.entity(reference.entity).ok_or_else(|| {
                invalid(
                    "extrude.selection_changed",
                    format!("entity {} does not exist any more", reference.entity.0),
                )
            })?;
            if entity.space != SpaceId::Model {
                return Err(invalid(
                    "extrude.not_model_space",
                    format!("entity {} is not in model space", entity.id.0),
                ));
            }
            match &entity.geometry {
                SemanticGeometry::Line { start, end } => edges.push(Edge {
                    start: *start,
                    end: *end,
                }),
                SemanticGeometry::Circle {
                    center,
                    normal,
                    radius,
                } => {
                    if circle.is_some() {
                        return Err(invalid(
                            "extrude.profile_unsupported",
                            "EXTRUDE accepts one circle at a time",
                        ));
                    }
                    circle = Some((*center, *normal, *radius));
                }
                SemanticGeometry::Polyline {
                    points,
                    bulges,
                    closed,
                    ..
                } => {
                    if !closed {
                        return Err(invalid(
                            "extrude.profile_not_closed",
                            format!("polyline {} is open", entity.id.0),
                        ));
                    }
                    if points.len() < 3 {
                        return Err(invalid(
                            "extrude.profile_unsupported",
                            format!(
                                "polyline {} has {} vertices; a closed profile needs at least 3",
                                entity.id.0,
                                points.len()
                            ),
                        ));
                    }
                    if bulges.iter().any(|bulge| bulge.abs() > 1e-12) {
                        return Err(invalid(
                            "extrude.profile_unsupported",
                            format!(
                                "polyline {} has arc segments; this build extrudes straight \
                                 LWPOLYLINE profiles only",
                                entity.id.0
                            ),
                        ));
                    }
                    for index in 0..points.len() {
                        edges.push(Edge {
                            start: points[index],
                            end: points[(index + 1) % points.len()],
                        });
                    }
                }
                other => {
                    return Err(invalid(
                        "extrude.profile_unsupported",
                        format!(
                            "entity {} is {} ({}); EXTRUDE accepts LINE, a closed straight \
                             LWPOLYLINE or one CIRCLE",
                            entity.id.0,
                            entity.object.type_key,
                            geometry_name(other)
                        ),
                    ))
                }
            }
        }
        if let Some((center, normal, radius)) = circle {
            if !edges.is_empty() {
                return Err(invalid(
                    "extrude.profile_unsupported",
                    "a circle profile must be the only selected row",
                ));
            }
            let normal = if normal.z.abs() >= 1.0 - 1e-9 {
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: normal.z.signum(),
                }
            } else {
                return Err(invalid(
                    "extrude.not_planar",
                    "the selected circle is not parallel to the XY work plane",
                ));
            };
            return Ok(ExtrudeProfile {
                curves: vec![BrepCurve::Circle {
                    center,
                    normal,
                    u_dir: Point3 {
                        x: 1.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    radius,
                }],
                description: "circle".into(),
            });
        }
        let ordered = close_chain(edges)?;
        let z = ordered[0].start.z;
        if ordered
            .iter()
            .any(|edge| (edge.start.z - z).abs() > 1e-7 || (edge.end.z - z).abs() > 1e-7)
        {
            return Err(invalid(
                "extrude.not_planar",
                "the selected profile leaves one plane parallel to the XY work plane",
            ));
        }
        let count = ordered.len();
        Ok(ExtrudeProfile {
            curves: ordered
                .into_iter()
                .map(|edge| BrepCurve::Line {
                    start: edge.start,
                    end: edge.end,
                })
                .collect(),
            description: format!("{count}-segment"),
        })
    }
}

/// The validated profile and the word the success diagnostic uses for it.
struct ExtrudeProfile {
    curves: Vec<BrepCurve>,
    description: String,
}

/// Order `edges` into one closed traversal, reversing individual edges as needed.
///
/// Every endpoint must be used by exactly two edges (no branch, no open end) and
/// the walk must consume every edge. A zero-length edge, a gap or a branch is a
/// named `extrude.profile_not_closed` refusal.
fn close_chain(mut edges: Vec<Edge>) -> CadResult<Vec<Edge>> {
    const TOLERANCE: f64 = 1e-7;
    fn near(a: Point3, b: Point3) -> bool {
        (a.x - b.x).abs() <= TOLERANCE
            && (a.y - b.y).abs() <= TOLERANCE
            && (a.z - b.z).abs() <= TOLERANCE
    }
    if edges.is_empty() {
        return Err(invalid(
            "extrude.profile_not_closed",
            "no profile edge was selected",
        ));
    }
    if edges.iter().any(|edge| near(edge.start, edge.end)) {
        return Err(invalid(
            "extrude.profile_degenerate",
            "a zero-length profile edge cannot bound an area",
        ));
    }
    // Each endpoint must have exactly two incident edges.
    let mut endpoints: Vec<Point3> = Vec::with_capacity(edges.len() * 2);
    for edge in &edges {
        for point in [edge.start, edge.end] {
            if !endpoints.iter().any(|candidate| near(*candidate, point)) {
                endpoints.push(point);
            }
        }
    }
    for point in &endpoints {
        let degree = edges
            .iter()
            .filter(|edge| near(edge.start, *point) || near(edge.end, *point))
            .count();
        if degree != 2 {
            return Err(invalid(
                "extrude.profile_not_closed",
                format!(
                    "profile endpoint ({}, {}, {}) is used by {degree} edges; a closed profile \
                     needs exactly 2",
                    point.x, point.y, point.z
                ),
            ));
        }
    }
    // Walk the cycle from the first edge.
    let first = edges.remove(0);
    let start = first.start;
    let mut ordered = vec![first];
    let mut current = first.end;
    while !edges.is_empty() {
        let Some(index) = edges
            .iter()
            .position(|edge| near(edge.start, current) || near(edge.end, current))
        else {
            return Err(invalid(
                "extrude.profile_not_closed",
                "the selected edges do not form one closed chain",
            ));
        };
        let mut edge = edges.remove(index);
        if near(edge.end, current) {
            std::mem::swap(&mut edge.start, &mut edge.end);
        }
        current = edge.end;
        ordered.push(edge);
    }
    if !near(current, start) {
        return Err(invalid(
            "extrude.profile_not_closed",
            "the selected edges do not close back onto the first endpoint",
        ));
    }
    Ok(ordered)
}

/// The command's distance rule, mirroring the seam's bound.
fn validate_extrude_distance(value: f64) -> CadResult<f64> {
    if value.is_finite() && value != 0.0 && value.abs() <= MAX_EXTRUDE_DISTANCE {
        return Ok(value);
    }
    Err(invalid(
        "extrude.distance_invalid",
        format!(
            "the extrusion distance must be finite, non-zero and at most \
             {MAX_EXTRUDE_DISTANCE} in magnitude, got {value}"
        ),
    ))
}

/// A short locale-neutral name of a geometry family, for the refusal sentence.
fn geometry_name(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Line { .. } => "line",
        SemanticGeometry::Arc { .. } => "arc",
        SemanticGeometry::Ellipse { .. } => "ellipse",
        SemanticGeometry::Spline { .. } => "spline",
        SemanticGeometry::Circle { .. } => "circle",
        SemanticGeometry::Polyline { .. } => "polyline",
        SemanticGeometry::Point(_) => "point",
        SemanticGeometry::Mesh(_) => "mesh",
        SemanticGeometry::Body3d(_) => "body",
        SemanticGeometry::Insert { .. } => "insert",
        SemanticGeometry::Text { .. } => "text",
        _ => "unsupported geometry",
    }
}

/// A plan-level `EXTRUDE` refusal: `InvalidInput` carrying its stable code.
fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}
