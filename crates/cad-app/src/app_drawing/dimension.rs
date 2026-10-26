//! Dimension create commands (ADR 0017 decision C, P5c/P5d).
//!
//! * **P5c**: DIMLINEAR / DIMALIGNED — three free points (two extension-line
//!   origins, then the dimension-line location).
//! * **P5d**: DIMRADIUS / DIMDIAMETER — an entity pick of an arc/circle (their
//!   **only** documented entry point) followed by the dimension-line location;
//!   and DIMCENTER — an entity pick of an arc/circle producing **plain geometry**
//!   (the non-associative centre mark/centrelines), **not** a `DIMENSION` entity.
//!
//! Every command builds a [`DimensionGeometry`] through the **shared** synthesis
//! ([`DimensionGeometry::synthesized`], ADR 0017 A.4 invariant 1) from the
//! resolved **current** style (`Standard`, ADR 0017 B.3) and inserts through the
//! single create funnel ([`Application::commit_new_entity`] /
//! [`Application::commit_new_entities`]) on the active layer in **one** transaction
//! / **one** undo step (ADR 0017 decision C). Creation is **non-associative**: no
//! `ACAD_DIMASSOC` is written and editing the measured geometry never updates the
//! dimension (ADR 0017 decision D.1).
//!
//! The linear/aligned commands capture three free points; the radial pair and
//! DIMCENTER start with an **entity pick** (the TRIM/EXTEND capture shape, via
//! `DrawToolKind::picked_step_count`), because Autodesk documents "Select arc or
//! circle" as their only entry point — a fillet (which commits an `Arc`) counts.
//!
//! Every refusal is explicit and changes nothing (no transaction is opened, no
//! history is recorded). DIMCEN = 0 for DIMCENTER is the one "nothing to do" case:
//! it produces no entity and reports it with a diagnostic rather than a silent
//! empty success.

use super::*;

/// Which linear dimension [`Application::create_linear_dimension`] builds.
#[derive(Clone, Copy)]
enum LinearKind {
    /// DIMLINEAR: horizontal or vertical, decided by the location pick.
    Linear,
    /// DIMALIGNED: parallel to the origin-to-origin line.
    Aligned,
}

impl LinearKind {
    /// The command's stable ASCII name for refusal messages.
    fn name(self) -> &'static str {
        match self {
            Self::Linear => "DIMLINEAR",
            Self::Aligned => "DIMALIGNED",
        }
    }
}

/// The minimum world extent for two origins to define a direction, for the
/// location pick to define a direction, and for a created dimension to have a
/// non-zero measurement. A tiny relative-free world epsilon, like the other
/// create commands' degeneracy guards.
const MIN_DIMENSION_EXTENT: f64 = 1e-9;

impl Application {
    /// DIMLINEAR: a horizontal/vertical linear dimension (ADR 0017 decision C).
    pub(crate) fn dim_linear(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        self.create_linear_dimension(session, command, LinearKind::Linear)
    }

    /// DIMALIGNED: a dimension parallel to the origin-to-origin line (ADR 0017
    /// decision C).
    pub(crate) fn dim_aligned(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        self.create_linear_dimension(session, command, LinearKind::Aligned)
    }

    /// The shared DIMLINEAR/DIMALIGNED body: validate, build the definition and
    /// the resolved style snapshot, synthesize, then insert one entity.
    ///
    /// Payload is `Points([first, second, location])` — the two extension-line
    /// origins and the dimension-line location in click order (the construction
    /// input, exactly like ELLIPSE/RECTANG/POLYGON; a pre-built geometry arm
    /// would be a second source of truth). Every input is resolved to plain data
    /// before any write, so a refusal changes nothing.
    fn create_linear_dimension(
        &mut self,
        session: &SessionState,
        command: &Command,
        kind: LinearKind,
    ) -> CadResult<CommandOutcome> {
        let name = kind.name();
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(format!(
                "{name} needs a Points([first, second, location]) payload"
            )));
        };
        if points.len() != 3 {
            return Err(CadError::InvalidInput(format!(
                "{name} needs exactly three points (two origins and the dimension-line \
                 location), got {}",
                points.len()
            )));
        }
        let first = points[0];
        let second = points[1];
        let location = points[2];
        if !is_finite_point(first) || !is_finite_point(second) || !is_finite_point(location) {
            return Err(CadError::InvalidInput(format!(
                "{name} points must all be finite"
            )));
        }
        // Flat `+Z` subset (ADR 0017 D.6): the definition points must lie in one
        // `+Z` plane; an off-plane triple is refused rather than guessed onto a
        // work plane.
        let z_tol = 1e-9 * (1.0 + first.z.abs() + second.z.abs() + location.z.abs());
        if (second.z - first.z).abs() > z_tol || (location.z - first.z).abs() > z_tol {
            return Err(CadError::InvalidInput(format!(
                "{name} points must lie in one +Z plane; off-plane dimension creation is \
                 NOT IMPLEMENTED"
            )));
        }
        // The two origins must define a segment: coincident origins have no
        // direction and no measurement.
        if distance(first, second) <= MIN_DIMENSION_EXTENT {
            return Err(CadError::InvalidInput(format!(
                "{name} needs two distinct extension-line origins"
            )));
        }
        let dimension_kind = match kind {
            // DIMLINEAR: the location pick decides horizontal (rotation 0, measures
            // X) or vertical (rotation π/2, measures Y). A tie (including a pick at
            // the origins' midpoint) leaves the direction undefined, so it is an
            // explicit refusal rather than an arbitrary tie-break.
            LinearKind::Linear => {
                let mid = midpoint(first, second);
                let dx = (location.x - mid.x).abs();
                let dy = (location.y - mid.y).abs();
                if (dx - dy).abs() <= MIN_DIMENSION_EXTENT {
                    return Err(CadError::InvalidInput(format!(
                        "{name} location pick does not define a horizontal or vertical \
                         direction"
                    )));
                }
                let rotation = if dy > dx {
                    0.0
                } else {
                    std::f64::consts::FRAC_PI_2
                };
                DimensionKind::Linear { rotation }
            }
            LinearKind::Aligned => DimensionKind::Aligned,
        };
        let definition = DimensionDefinition {
            kind: dimension_kind,
            points: DimensionPoints::Linear {
                first,
                second,
                dimension_line: location,
            },
            // Group 11 (text middle) and group 53 (text rotation) are left at the
            // zero "derive from the dimension line" sentinel, so the synthesis
            // places the text itself (ADR 0017 A.3/decision C).
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
        };
        // The current style is `Standard` (ADR 0017 B.3: P5 has no interactive
        // style manager; creation resolves the one stored style). The resolved
        // snapshot bakes `DIMSCALE` into the sizes and resolves the text style.
        let document_id = command.document;
        let style = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            document
                .drawing
                .resolve_dim_style("Standard")
                .ok_or_else(|| {
                    CadError::InvalidInput(
                        "the drawing has no 'Standard' dimension style to create with".into(),
                    )
                })?
        };
        let dimension =
            DimensionGeometry::synthesized(definition, "Standard".to_string(), style, None)?;
        // A zero measurement is a degenerate dimension: the chosen direction
        // projects the two distinct origins onto the same point. Refuse rather
        // than store a dimension that measures nothing.
        if dimension.measurement() <= MIN_DIMENSION_EXTENT {
            return Err(CadError::InvalidInput(format!(
                "{name} is degenerate: the chosen direction measures zero"
            )));
        }
        self.commit_new_entity(
            session,
            command,
            SemanticGeometry::Dimension(Box::new(dimension)),
            match kind {
                LinearKind::Linear => "create dimlinear",
                LinearKind::Aligned => "create dimaligned",
            },
        )
    }
}

/// The midpoint of two points.
fn midpoint(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: (a.x + b.x) * 0.5,
        y: (a.y + b.y) * 0.5,
        z: (a.z + b.z) * 0.5,
    }
}

// ---------------------------------------------------------------------------
// DIMRADIUS / DIMDIAMETER (ADR 0017 decision C, P5d).
// ---------------------------------------------------------------------------

/// Which radial dimension [`Application::create_radial_dimension`] builds.
#[derive(Clone, Copy)]
enum RadialKind {
    /// DIMRADIUS: group 10 is the circle centre; measures the radius.
    Radius,
    /// DIMDIAMETER: group 10 is the opposite chord point; measures the diameter.
    Diameter,
}

impl RadialKind {
    /// The command's stable ASCII name for refusal messages.
    fn name(self) -> &'static str {
        match self {
            Self::Radius => "DIMRADIUS",
            Self::Diameter => "DIMDIAMETER",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Radius => "create dimradius",
            Self::Diameter => "create dimdiameter",
        }
    }
}

/// The circle/arc frame a radial dimension or centre mark consumes.
struct CircleFrame {
    center: Point3,
    radius: f64,
    normal: Point3,
}

/// Resolve a picked entity's geometry to the circle/arc frame, refusing every
/// other kind explicitly (ADR 0017 decision C: the pick must be an arc or circle;
/// a FILLET commits an `Arc`, so a filleted corner is a legal pick).
fn resolve_arc_or_circle(geometry: &SemanticGeometry, command: &str) -> CadResult<CircleFrame> {
    match geometry {
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => Ok(CircleFrame {
            center: *center,
            radius: *radius,
            normal: *normal,
        }),
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            ..
        } => Ok(CircleFrame {
            center: *center,
            radius: *radius,
            normal: *normal,
        }),
        other => Err(CadError::Unsupported(format!(
            "{command} needs an arc or circle pick; {} is not supported",
            geometry_kind(other)
        ))),
    }
}

/// The off-plane / degeneracy refusals shared by the radial pair and DIMCENTER.
///
/// The arc/circle must lie in an XY-parallel plane (ADR 0017 D.6): a normal with
/// any X/Y component is refused rather than guessed onto a work plane. A `-Z`
/// normal is accepted — the entity is still planar-parallel to XY, and the radial
/// synthesis does not read the normal. The radius must be positive and finite.
fn check_circle_frame(frame: &CircleFrame, command: &str) -> CadResult<()> {
    if frame.normal.x.abs() > 1e-9 || frame.normal.y.abs() > 1e-9 {
        return Err(CadError::InvalidInput(format!(
            "{command} needs an arc/circle in the XY plane; off-plane dimension creation is \
             NOT IMPLEMENTED"
        )));
    }
    if !frame.radius.is_finite() || frame.radius <= MIN_DIMENSION_EXTENT {
        return Err(CadError::InvalidInput(format!(
            "{command} needs an arc/circle with a positive finite radius"
        )));
    }
    Ok(())
}

/// Build the radius/diameter definition from the resolved frame and the location
/// pick (ADR 0017 A.3).
///
/// The location pick sets the **direction** from the centre toward which the
/// leader meets the circle; `point_on_arc` (group 15) is the centre plus the
/// radius along that direction. Group 10 is the **centre** for a radius and the
/// **opposite chord point** for a diameter. Group 40 (leader length) is written
/// as `0.0`: our synthesis draws the leader from group 10 to group 15 and does not
/// model a leader extension (declared), and `0.0` is AutoCAD's neutral value.
///
/// Refuses a location on the entity's plane violation, a location at the centre
/// (no direction) and a non-finite location.
fn radial_definition(
    kind: RadialKind,
    frame: &CircleFrame,
    location: Point3,
    command: &str,
) -> CadResult<DimensionDefinition> {
    if !is_finite_point(location) {
        return Err(CadError::InvalidInput(format!(
            "{command} location must be finite"
        )));
    }
    check_circle_frame(frame, command)?;
    let z_tol = 1e-9 * (1.0 + frame.center.z.abs() + location.z.abs());
    if (location.z - frame.center.z).abs() > z_tol {
        return Err(CadError::InvalidInput(format!(
            "{command} location must lie in the arc/circle's plane"
        )));
    }
    let dx = location.x - frame.center.x;
    let dy = location.y - frame.center.y;
    let len = dx.hypot(dy);
    if !len.is_finite() || len <= MIN_DIMENSION_EXTENT {
        return Err(CadError::InvalidInput(format!(
            "{command} location pick must differ from the arc/circle centre to set the direction"
        )));
    }
    let ux = dx / len;
    let uy = dy / len;
    let on_arc = Point3 {
        x: frame.center.x + ux * frame.radius,
        y: frame.center.y + uy * frame.radius,
        z: frame.center.z,
    };
    let (dimension_kind, center_slot) = match kind {
        RadialKind::Radius => (DimensionKind::Radius, frame.center),
        RadialKind::Diameter => (
            DimensionKind::Diameter,
            Point3 {
                x: frame.center.x - ux * frame.radius,
                y: frame.center.y - uy * frame.radius,
                z: frame.center.z,
            },
        ),
    };
    Ok(DimensionDefinition {
        kind: dimension_kind,
        points: DimensionPoints::Radial {
            center: center_slot,
            point_on_arc: on_arc,
            leader_length: 0.0,
        },
        // Group 11/53 stay at the zero "derive from the geometry" sentinel, so the
        // synthesis places the text at the leader midpoint (ADR 0017 A.3).
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

impl Application {
    /// DIMRADIUS: a radius dimension for a picked arc/circle (ADR 0017 decision C).
    pub(crate) fn dim_radius(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        self.create_radial_dimension(session, command, RadialKind::Radius)
    }

    /// DIMDIAMETER: a diameter dimension for a picked arc/circle (ADR 0017
    /// decision C).
    pub(crate) fn dim_diameter(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        self.create_radial_dimension(session, command, RadialKind::Diameter)
    }

    /// The shared DIMRADIUS/DIMDIAMETER body: validate the payload and target,
    /// resolve the arc/circle, build the definition, synthesize, insert one entity.
    ///
    /// Payload is `DimRadial { target, location }` — the ref resolved at capture
    /// time plus the dimension-line location. The kind (radius vs diameter) comes
    /// from the command id. Every input is resolved to plain data before any write,
    /// so a refusal changes nothing.
    fn create_radial_dimension(
        &mut self,
        session: &SessionState,
        command: &Command,
        kind: RadialKind,
    ) -> CadResult<CommandOutcome> {
        let name = kind.name();
        let CommandPayload::DimRadial(input) = &command.payload else {
            return Err(CadError::InvalidInput(format!(
                "{name} needs a DimRadial {{ target, location }} payload"
            )));
        };
        if input.target.document != command.document {
            return Err(CadError::InvalidInput(format!(
                "{name} target belongs to another document"
            )));
        }
        let location = input.location;
        let document_id = command.document;
        let (definition, style) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let entity = document
                .drawing
                .entity(input.target.entity)
                .cloned()
                .ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "{name} target {} does not exist",
                        input.target.entity.0
                    ))
                })?;
            let frame = resolve_arc_or_circle(&entity.geometry, name)?;
            let definition = radial_definition(kind, &frame, location, name)?;
            let style = document
                .drawing
                .resolve_dim_style("Standard")
                .ok_or_else(|| {
                    CadError::InvalidInput(
                        "the drawing has no 'Standard' dimension style to create with".into(),
                    )
                })?;
            (definition, style)
        };
        let dimension =
            DimensionGeometry::synthesized(definition, "Standard".to_string(), style, None)?;
        if dimension.measurement() <= MIN_DIMENSION_EXTENT {
            return Err(CadError::InvalidInput(format!(
                "{name} is degenerate: the measured radius is zero"
            )));
        }
        self.commit_new_entity(
            session,
            command,
            SemanticGeometry::Dimension(Box::new(dimension)),
            kind.label(),
        )
    }
}

// ---------------------------------------------------------------------------
// DIMCENTER (ADR 0017 decision C/D.7, P5d): plain geometry, not a DIMENSION.
// ---------------------------------------------------------------------------

/// The centre-mark/centreline geometry `DIMCENTER` inserts (plain `Line`s).
///
/// Pinned `DIMCEN` sign reading (the same one the radial synthesis uses): `> 0` a
/// **centre mark** — a crossed pair of LINEs with arms `|DIMCEN|`; `< 0`
/// **centrelines** — a crossed pair reaching `radius + |DIMCEN|` past the circle.
/// The cross is aligned with the world X/Y axes.
///
/// **Declared approximation**: AutoCAD's negative-`DIMCEN` centrelines are a centre
/// mark plus **gapped** centrelines (which is why ADR 0017 decision C calls the
/// output "LINE/ARC"); we draw a plain cross, the **same** declared approximation
/// as the radial synthesis (ADR 0017 P5b deviation 3). The gap/arc variant is not
/// modelled.
///
/// `dimcen` is the already-resolved (`DIMSCALE`-baked) value; a non-finite or zero
/// value is handled by the caller (zero produces nothing, never this).
fn center_mark_geometry(frame: &CircleFrame, dimcen: f64) -> CadResult<Vec<SemanticGeometry>> {
    let arm = dimcen.abs();
    let reach = if dimcen > 0.0 {
        arm
    } else {
        frame.radius + arm
    };
    if !reach.is_finite() || reach <= MIN_DIMENSION_EXTENT {
        return Err(CadError::InvalidInput(
            "DIMCENTER needs a finite, non-zero DIMCEN size".into(),
        ));
    }
    let center = frame.center;
    let mut out = Vec::with_capacity(2);
    for axis in [(1.0_f64, 0.0_f64), (0.0, 1.0)] {
        out.push(SemanticGeometry::Line {
            start: Point3 {
                x: center.x - axis.0 * reach,
                y: center.y - axis.1 * reach,
                z: center.z,
            },
            end: Point3 {
                x: center.x + axis.0 * reach,
                y: center.y + axis.1 * reach,
                z: center.z,
            },
        });
    }
    Ok(out)
}

impl Application {
    /// DIMCENTER: the non-associative centre mark/centrelines of a picked
    /// arc/circle as **plain geometry** (ADR 0017 decision C/D.7).
    ///
    /// Autodesk documents DIMCENTER as creating a non-associative centre mark or
    /// centrelines, i.e. ordinary LINE geometry — **not** a `DIMENSION` entity. The
    /// one pick is an entity pick of an arc/circle. With `DIMCEN = 0` the command
    /// produces **nothing** and reports it with an explicit diagnostic, opening **no
    /// transaction** (never a silent empty success). Otherwise the crossed LINEs are
    /// committed through the batch funnel in one transaction / one undo step on the
    /// active layer. The AutoCAD centre-mark "only outside the circle" placement
    /// rule and the gapped-centrelines variant are declared approximations (see
    /// [`center_mark_geometry`]).
    pub(crate) fn dim_center(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::DimCenter(target) = &command.payload else {
            return Err(CadError::InvalidInput(
                "DIMCENTER needs a DimCenter(target) payload".into(),
            ));
        };
        if target.document != command.document {
            return Err(CadError::InvalidInput(
                "DIMCENTER target belongs to another document".into(),
            ));
        }
        let document_id = command.document;
        let (frame, dimcen) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let entity = document
                .drawing
                .entity(target.entity)
                .cloned()
                .ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "DIMCENTER target {} does not exist",
                        target.entity.0
                    ))
                })?;
            let frame = resolve_arc_or_circle(&entity.geometry, "DIMCENTER")?;
            check_circle_frame(&frame, "DIMCENTER")?;
            let dimcen = document
                .drawing
                .resolve_dim_style("Standard")
                .ok_or_else(|| {
                    CadError::InvalidInput(
                        "the drawing has no 'Standard' dimension style to create with".into(),
                    )
                })?
                .center_mark;
            (frame, dimcen)
        };
        // DIMCEN = 0 (or non-finite): nothing is produced. This is an explicit
        // diagnostic with **no transaction**, the same shape as ARRAY's 1x1
        // no-copy case — never a silent empty success.
        if !dimcen.is_finite() || dimcen == 0.0 {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "drawing.dimcenter".into(),
                    message: "DIMCENTER: DIMCEN is 0; no centre mark or centrelines are created \
                              (no transaction)"
                        .into(),
                }],
                measurement: None,
                point_report: None,
            });
        }
        let geometries = center_mark_geometry(&frame, dimcen)?;
        self.commit_new_entities(session, command, geometries, "create dimcenter")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(radius: f64, normal: Point3) -> CircleFrame {
        CircleFrame {
            center: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            radius,
            normal,
        }
    }

    fn up() -> Point3 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    }

    /// The degenerate-radius refusal cannot be reached through a validated
    /// database (its `validate` refuses a non-positive/non-finite radius), so it is
    /// pinned on the pure frame check the command path uses — the same predicate,
    /// by name.
    #[test]
    fn degenerate_radius_and_off_plane_normal_are_refused_by_name() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let error = check_circle_frame(&frame(bad, up()), "DIMRADIUS").unwrap_err();
            assert!(
                format!("{error:?}").contains("positive finite radius"),
                "radius {bad} must be refused by name: {error:?}"
            );
        }
        let error = check_circle_frame(
            &frame(
                5.0,
                Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            ),
            "DIMCENTER",
        )
        .unwrap_err();
        assert!(format!("{error:?}").contains("XY plane"), "{error:?}");
        // A +Z or -Z normal is accepted: the entity is planar-parallel to XY.
        assert!(check_circle_frame(&frame(5.0, up()), "DIMCENTER").is_ok());
        assert!(check_circle_frame(
            &frame(
                5.0,
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: -1.0
                }
            ),
            "DIMCENTER"
        )
        .is_ok());
    }

    #[test]
    fn radial_definition_places_the_point_on_the_circle_in_the_picked_direction() {
        let definition = radial_definition(
            RadialKind::Diameter,
            &frame(4.0, up()),
            Point3 {
                x: 0.0,
                y: 9.0,
                z: 0.0,
            },
            "DIMDIAMETER",
        )
        .unwrap();
        match definition.points {
            DimensionPoints::Radial {
                center,
                point_on_arc,
                leader_length,
            } => {
                assert_eq!(
                    center,
                    Point3 {
                        x: 0.0,
                        y: -4.0,
                        z: 0.0
                    }
                );
                assert_eq!(
                    point_on_arc,
                    Point3 {
                        x: 0.0,
                        y: 4.0,
                        z: 0.0
                    }
                );
                assert_eq!(leader_length, 0.0);
            }
            other => panic!("expected Radial points, got {other:?}"),
        }
        // A location at the centre has no direction.
        let error = radial_definition(
            RadialKind::Radius,
            &frame(4.0, up()),
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            "DIMRADIUS",
        )
        .unwrap_err();
        assert!(
            format!("{error:?}").contains("differ from the arc/circle centre"),
            "{error:?}"
        );
    }
}
