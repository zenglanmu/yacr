//! DIMLINEAR / DIMALIGNED create commands (ADR 0017 decision C, P5c).
//!
//! Both commands capture three free points (two extension-line origins, then the
//! dimension-line location), build a [`DimensionGeometry`] through the **shared**
//! synthesis ([`DimensionGeometry::synthesized`], ADR 0017 A.4 invariant 1) from
//! the resolved **current** style (`Standard`, ADR 0017 B.3), and insert it
//! through the single create funnel ([`Application::commit_new_entity`]) on the
//! active layer in **one** transaction / **one** undo step (ADR 0017 decision C).
//! Creation is **non-associative**: no `ACAD_DIMASSOC` is written and editing the
//! measured geometry never updates the dimension (ADR 0017 decision D.1).
//!
//! The two commands differ only in the dimension kind:
//!
//! * **DIMLINEAR** is horizontal or vertical, decided by the **location pick**
//!   (AutoCAD's rule — the location determines the direction): a location offset
//!   more vertically than horizontally from the origins' midpoint gives a
//!   horizontal dimension (measures X), otherwise a vertical one (measures Y).
//!   The kind's group-50 `rotation` is `0` (horizontal) or `π/2` (vertical).
//! * **DIMALIGNED** runs parallel to the line joining the two origins and measures
//!   the true origin-to-origin distance.
//!
//! Every refusal is explicit and changes nothing (no transaction is opened, no
//! history is recorded): a mismatched payload, a non-finite point, a non-plan
//! (off-one-`+Z`-plane) triple, coincident origins, an ambiguous DIMLINEAR
//! direction, a zero measurement, and a missing active layer (refused by the
//! shared funnel). The measurement is the raw geometric distance; the drawn text
//! is produced by the same `DIMLFAC`/`DIMDEC`/`DIMZIN`/`DIMPOST`/`DIMRND`
//! formatting path, so `measurement()` and the drawn text agree.

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
