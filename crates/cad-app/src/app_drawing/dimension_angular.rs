//! DIMANGULAR create command (ADR 0017 decision C, P5e).
//!
//! Two constructions share one command:
//!
//! * **two-line** (the interactive tool): two straight-line entity picks; the
//!   vertex is the intersection of the two infinite lines and the dimension-arc
//!   location selects which of the four angles is measured. ADR 0017 U-8 is
//!   resolved in favour of the acadrust struct/reader reading — line 1 is groups
//!   13->14, line 2 is 15->10, the vertex is their intersection, group 16 selects
//!   the sector — pinned against the committed QCAD fixtures (see
//!   `angular_lines_synthesis` in `cad-domain`).
//! * **three-point** (command payload only): vertex, two side points and the
//!   dimension-arc location. There is **no interactive capture** for it yet: the
//!   fixed-step capture state machine cannot express AutoCAD's "select a line
//!   *or* specify a vertex" first step. This is a declared gap, not a silent one.
//!
//! The location pick's sector rule (ADR 0017 U-1, unverified in Autodesk's own
//! documentation) is **our declared choice**: the measured angle is the sector
//! that contains the location pick (for the three-point form: the sector, reflex
//! or not, that contains it). A location exactly on a bounding ray selects the
//! counter-clockwise sector from that ray.
//!
//! Every input is resolved to plain data before any write; a refusal opens no
//! transaction and records no history. The created dimension is non-associative
//! and lands on the active layer in one transaction / one undo step.

use super::*;

/// The world extent below which a line, a side ray or a location offset is
/// degenerate (the same order as the other create commands' guards).
const MIN_ANGULAR_EXTENT: f64 = 1e-9;

/// A straight line resolved from a picked entity.
struct PlanLine {
    start: Point3,
    end: Point3,
}

impl Application {
    /// DIMANGULAR: an angular dimension from two picked lines (interactive) or
    /// three points (command payload), ADR 0017 decision C.
    pub(crate) fn dim_angular(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::DimAngular(input) = &command.payload else {
            return Err(CadError::InvalidInput(
                "DIMANGULAR needs a DimAngular(input) payload".into(),
            ));
        };
        let (definition, style) = match input.as_ref() {
            AngularDimensionInput::ThreePoint {
                vertex,
                first,
                second,
                location,
            } => {
                let definition = angular_three_point_definition(
                    *vertex,
                    *first,
                    *second,
                    *location,
                    "DIMANGULAR",
                )?;
                let style = self.current_dim_style(command.document)?;
                (definition, style)
            }
            AngularDimensionInput::TwoLine {
                first,
                second,
                location,
            } => {
                if first.document != command.document || second.document != command.document {
                    return Err(CadError::InvalidInput(
                        "DIMANGULAR line picks belong to another document".into(),
                    ));
                }
                let document = self
                    .workspace
                    .documents
                    .get(&command.document)
                    .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
                let line1 = resolve_plan_line(document, first, "DIMANGULAR")?;
                let line2 = resolve_plan_line(document, second, "DIMANGULAR")?;
                let definition = angular_two_line_definition(&line1, &line2, *location)?;
                let style = document
                    .drawing
                    .resolve_dim_style("Standard")
                    .ok_or_else(|| {
                        CadError::InvalidInput(
                            "the drawing has no 'Standard' dimension style to create with".into(),
                        )
                    })?;
                (definition, style)
            }
        };
        let dimension =
            DimensionGeometry::synthesized(definition, "Standard".to_string(), style, None)?;
        if dimension.measurement() <= MIN_ANGULAR_EXTENT {
            return Err(CadError::InvalidInput(
                "DIMANGULAR is degenerate: the selected angle measures zero".into(),
            ));
        }
        self.commit_new_entity(
            session,
            command,
            SemanticGeometry::Dimension(Box::new(dimension)),
            "create dimangular",
        )
    }

    /// The resolved `Standard` style snapshot for a create command (ADR 0017 B.3).
    fn current_dim_style(&self, document_id: DocumentId) -> CadResult<DimensionStyleValues> {
        self.workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .resolve_dim_style("Standard")
            .ok_or_else(|| {
                CadError::InvalidInput(
                    "the drawing has no 'Standard' dimension style to create with".into(),
                )
            })
    }
}

/// Resolve a picked entity to a straight line, refusing every other geometry kind
/// explicitly (ADR 0017 decision C: the two-line form picks lines).
fn resolve_plan_line(
    document: &Document,
    reference: &SelectionRef,
    command: &str,
) -> CadResult<PlanLine> {
    let entity = document
        .drawing
        .entity(reference.entity)
        .cloned()
        .ok_or_else(|| {
            CadError::InvalidInput(format!(
                "{command} line pick {} does not exist",
                reference.entity.0
            ))
        })?;
    match &entity.geometry {
        SemanticGeometry::Line { start, end } => Ok(PlanLine {
            start: *start,
            end: *end,
        }),
        other => Err(CadError::Unsupported(format!(
            "{command} needs two straight-line picks; {} is not supported",
            geometry_kind(other)
        ))),
    }
}

/// Build the two-line angular definition (ADR 0017 A.3/U-8).
///
/// Groups 13/14 keep the first picked line's endpoints in its own direction,
/// groups 15/10 the second line's endpoints, and group 16 is the arc-location
/// pick. The synthesis derives the vertex as the intersection of the two infinite
/// lines; the pick order therefore does not change the measured sector (only the
/// sector containing the pick does).
fn angular_two_line_definition(
    line1: &PlanLine,
    line2: &PlanLine,
    location: Point3,
) -> CadResult<DimensionDefinition> {
    const COMMAND: &str = "DIMANGULAR";
    let points = [line1.start, line1.end, line2.start, line2.end, location];
    if points.iter().any(|point| !is_finite_point(*point)) {
        return Err(CadError::InvalidInput(format!(
            "{COMMAND} points must all be finite"
        )));
    }
    let z_tol = 1e-9 * (1.0 + points.iter().map(|p| p.z.abs()).sum::<f64>());
    if points
        .iter()
        .any(|point| (point.z - line1.start.z).abs() > z_tol)
    {
        return Err(CadError::InvalidInput(format!(
            "{COMMAND} points must lie in one +Z plane; off-plane dimension creation is \
             NOT IMPLEMENTED"
        )));
    }
    let d1 = planar_delta(line1.start, line1.end);
    let d2 = planar_delta(line2.start, line2.end);
    if d1.0.hypot(d1.1) <= MIN_ANGULAR_EXTENT || d2.0.hypot(d2.1) <= MIN_ANGULAR_EXTENT {
        return Err(CadError::InvalidInput(format!(
            "{COMMAND} line picks must have two distinct endpoints"
        )));
    }
    let cross = d1.0 * d2.1 - d1.1 * d2.0;
    let scale = d1.0.hypot(d1.1) * d2.0.hypot(d2.1);
    if !cross.is_finite() || cross.abs() <= scale * 1e-12 {
        return Err(CadError::InvalidInput(format!(
            "{COMMAND} needs two non-parallel, non-collinear lines"
        )));
    }
    let vertex = line_vertex(line1, d1, line2, d2).ok_or_else(|| {
        CadError::InvalidInput(format!(
            "{COMMAND} could not intersect the two picked lines"
        ))
    })?;
    if planar_distance(vertex, location) <= MIN_ANGULAR_EXTENT {
        return Err(CadError::InvalidInput(format!(
            "{COMMAND} location pick must differ from the lines' intersection to select an angle"
        )));
    }
    Ok(DimensionDefinition {
        kind: DimensionKind::Angular2Ln,
        points: DimensionPoints::Angular2Ln {
            line1_start: line1.start,
            line1_end: line1.end,
            line2_start: line2.start,
            line2_end: line2.end,
            arc_point: location,
        },
        // Group 11/53 stay at the zero "derive from the geometry" sentinel.
        text_middle: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        text_rotation: 0.0,
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    })
}

/// Build the three-point angular definition from a vertex, two side points and the
/// arc-location pick.
fn angular_three_point_definition(
    vertex: Point3,
    first: Point3,
    second: Point3,
    location: Point3,
    command: &str,
) -> CadResult<DimensionDefinition> {
    let points = [vertex, first, second, location];
    if points.iter().any(|point| !is_finite_point(*point)) {
        return Err(CadError::InvalidInput(format!(
            "{command} points must all be finite"
        )));
    }
    let z_tol = 1e-9 * (1.0 + points.iter().map(|p| p.z.abs()).sum::<f64>());
    if points
        .iter()
        .any(|point| (point.z - vertex.z).abs() > z_tol)
    {
        return Err(CadError::InvalidInput(format!(
            "{command} points must lie in one +Z plane; off-plane dimension creation is \
             NOT IMPLEMENTED"
        )));
    }
    if planar_distance(vertex, first) <= MIN_ANGULAR_EXTENT
        || planar_distance(vertex, second) <= MIN_ANGULAR_EXTENT
    {
        return Err(CadError::InvalidInput(format!(
            "{command} side points must both differ from the vertex"
        )));
    }
    if planar_distance(vertex, location) <= MIN_ANGULAR_EXTENT {
        return Err(CadError::InvalidInput(format!(
            "{command} location pick must differ from the vertex to select an angle"
        )));
    }
    // The two side rays must not be the same ray: that would measure zero. The
    // opposite ray is a valid straight (180-degree) angle.
    let ray1 = planar_delta(vertex, first);
    let ray2 = planar_delta(vertex, second);
    let sweep = wrap_positive(ray2.1.atan2(ray2.0) - ray1.1.atan2(ray1.0));
    if sweep <= MIN_ANGULAR_EXTENT || std::f64::consts::TAU - sweep <= MIN_ANGULAR_EXTENT {
        return Err(CadError::InvalidInput(format!(
            "{command} side points must not lie on the same ray from the vertex"
        )));
    }
    Ok(DimensionDefinition {
        kind: DimensionKind::Angular3Pt,
        points: DimensionPoints::Angular3Pt {
            vertex,
            first,
            second,
            arc_point: location,
        },
        text_middle: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        text_rotation: 0.0,
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    })
}

/// The XY delta between two points.
fn planar_delta(start: Point3, end: Point3) -> (f64, f64) {
    (end.x - start.x, end.y - start.y)
}

/// The XY distance between two points.
fn planar_distance(a: Point3, b: Point3) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// The intersection of two infinite XY lines (the caller has already excluded
/// parallel/collinear directions).
fn line_vertex(
    line1: &PlanLine,
    d1: (f64, f64),
    line2: &PlanLine,
    d2: (f64, f64),
) -> Option<Point3> {
    let cross = d1.0 * d2.1 - d1.1 * d2.0;
    if cross.abs() <= f64::MIN_POSITIVE {
        return None;
    }
    let offset = planar_delta(line1.start, line2.start);
    let t = (offset.0 * d2.1 - offset.1 * d2.0) / cross;
    let vertex = Point3 {
        x: line1.start.x + d1.0 * t,
        y: line1.start.y + d1.1 * t,
        z: line1.start.z,
    };
    vertex.x.is_finite().then_some(vertex)
}

/// Normalize an angle to `[0, 2π)`.
fn wrap_positive(angle: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let mut wrapped = angle % tau;
    if wrapped < 0.0 {
        wrapped += tau;
    }
    if wrapped >= tau {
        wrapped -= tau;
    }
    wrapped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(sx: f64, sy: f64, ex: f64, ey: f64) -> PlanLine {
        PlanLine {
            start: Point3 {
                x: sx,
                y: sy,
                z: 0.0,
            },
            end: Point3 {
                x: ex,
                y: ey,
                z: 0.0,
            },
        }
    }

    fn at(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    /// A horizontal line and a vertical line crossing at (2, 3). Picking the
    /// location in each quadrant selects that quadrant's angle (our declared
    /// U-1 rule).
    #[test]
    fn two_line_definition_keeps_the_picked_endpoints_and_arc_location() {
        let first = line(0.0, 3.0, 5.0, 3.0);
        let second = line(2.0, 0.0, 2.0, 6.0);
        let definition =
            angular_two_line_definition(&first, &second, at(4.0, 5.0)).expect("definition");
        assert_eq!(definition.kind, DimensionKind::Angular2Ln);
        match definition.points {
            DimensionPoints::Angular2Ln {
                line1_start,
                line1_end,
                line2_start,
                line2_end,
                arc_point,
            } => {
                assert_eq!(line1_start, at(0.0, 3.0));
                assert_eq!(line1_end, at(5.0, 3.0));
                assert_eq!(line2_start, at(2.0, 0.0));
                assert_eq!(line2_end, at(2.0, 6.0));
                assert_eq!(arc_point, at(4.0, 5.0));
            }
            other => panic!("expected Angular2Ln points, got {other:?}"),
        }
    }

    #[test]
    fn parallel_and_collinear_lines_are_refused_by_name() {
        let parallel = angular_two_line_definition(
            &line(0.0, 0.0, 10.0, 0.0),
            &line(0.0, 5.0, 10.0, 5.0),
            at(5.0, 2.0),
        )
        .unwrap_err();
        assert!(
            format!("{parallel:?}").contains("non-parallel, non-collinear"),
            "{parallel:?}"
        );
        let collinear = angular_two_line_definition(
            &line(0.0, 0.0, 10.0, 0.0),
            &line(20.0, 0.0, 30.0, 0.0),
            at(5.0, 2.0),
        )
        .unwrap_err();
        assert!(
            format!("{collinear:?}").contains("non-parallel, non-collinear"),
            "{collinear:?}"
        );
    }

    #[test]
    fn coincident_and_off_plane_inputs_are_refused_by_name() {
        let degenerate = angular_two_line_definition(
            &line(1.0, 1.0, 1.0, 1.0),
            &line(0.0, 0.0, 0.0, 5.0),
            at(2.0, 2.0),
        )
        .unwrap_err();
        assert!(
            format!("{degenerate:?}").contains("distinct endpoints"),
            "{degenerate:?}"
        );
        let mut off_plane = line(2.0, 0.0, 2.0, 6.0);
        off_plane.end.z = 3.0;
        let error =
            angular_two_line_definition(&line(0.0, 3.0, 5.0, 3.0), &off_plane, at(4.0, 5.0))
                .unwrap_err();
        assert!(format!("{error:?}").contains("+Z plane"), "{error:?}");
    }

    #[test]
    fn a_location_at_the_vertex_or_the_same_ray_is_refused() {
        let error = angular_two_line_definition(
            &line(0.0, 3.0, 5.0, 3.0),
            &line(2.0, 0.0, 2.0, 6.0),
            at(2.0, 3.0),
        )
        .unwrap_err();
        assert!(
            format!("{error:?}").contains("lines' intersection"),
            "{error:?}"
        );
        let error = angular_three_point_definition(
            at(0.0, 0.0),
            at(1.0, 0.0),
            at(2.0, 0.0),
            at(0.5, 1.0),
            "DIMANGULAR",
        )
        .unwrap_err();
        assert!(format!("{error:?}").contains("same ray"), "{error:?}");
    }
}
