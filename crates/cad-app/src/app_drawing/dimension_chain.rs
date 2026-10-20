//! DIMBASELINE / DIMCONTINUE create commands (ADR 0017 decision C, P5f — the
//! 2026-10-21 un-gating).
//!
//! Both commands take **one explicit base pick** plus N picked points, so neither
//! carries any cross-command state: the base is a pick inside this invocation
//! (`DrawToolKind::picked_step_count() == 1`), and a base that was deleted,
//! replaced or lives in another document is refused by name at apply time. The
//! created entities store **no** base identity and **no** row index, so undo,
//! erase and document switching cannot leave a dangling reference (the ADR's
//! reason for un-gating P5f).
//!
//! * **DIMBASELINE** stacks rows off the base: row `i` (`i = 1..=N` in pick
//!   order) keeps the base's own `first` origin, takes `second = P_i` and the
//!   base's group-50 rotation, and places its dimension line at
//!   `base.location + n̂ · (i · DIMDLI)`. `n̂` is the normal **away from that row's
//!   measured geometry**: `±rot90(direction)` with the sign that makes
//!   `dot(base.location − midpoint(first, second), n̂) > 0`. A zero dot product
//!   means the base's dimension line lies on the measured points, so the side is
//!   **undecidable** and the command refuses instead of guessing.
//! * **DIMCONTINUE** chains segments: segment 1 is `base.second → P_1`, segment
//!   `i > 1` is `P_{i−1} → P_i`, and every segment sits on `base.location` (no
//!   stacking). The pick order is the chain direction; a segment whose
//!   projection along the base's direction is `<= 0` folds back and is refused.
//!
//! Every row goes through the **existing** synthesis
//! ([`DimensionGeometry::synthesized`]), which recomputes the measurement, the
//! formatted text and the display — no new entity field and no second geometry
//! path. The whole batch lands in **one transaction / one undo step** on the
//! active layer through the shared insert funnel, and every refusal is named and
//! opens no transaction.
//!
//! Declared v1 scope (ADR 0017 decision C): only a **linear** base is supported
//! (an aligned/angular/radial/ordinate base is a named `Unsupported`),
//! `DIMCONTINUEMODE` is not written (acadrust cannot express it), there is no
//! associativity and no text override, there is no repeat-until-Esc loop and no
//! per-point undo (the capture is all-or-nothing).

use super::*;

/// The minimum world extent for a row to measure something along the base
/// direction (the same order as the other create commands' degeneracy guards).
const MIN_CHAIN_EXTENT: f64 = 1e-9;

/// Which chain construction the command builds.
#[derive(Clone, Copy)]
enum ChainKind {
    /// DIMBASELINE: rows stacked at `i · DIMDLI` from the base dimension line.
    Baseline,
    /// DIMCONTINUE: chained segments on the base dimension line itself.
    Continue,
}

impl ChainKind {
    /// The command's stable ASCII name for refusal messages.
    fn name(self) -> &'static str {
        match self {
            Self::Baseline => "DIMBASELINE",
            Self::Continue => "DIMCONTINUE",
        }
    }

    /// The undo-record label / outcome label of the commit.
    fn label(self) -> &'static str {
        match self {
            Self::Baseline => "create dimbaseline",
            Self::Continue => "create dimcontinue",
        }
    }
}

/// The resolved **linear base dimension** a chain is built from: the DXF groups
/// `13`/`14`/`10` plus group `50`'s rotation.
///
/// It is a local value, reconstructed from the picked entity on every command —
/// never stored on the created dimensions.
struct ChainBase {
    /// Group 13: the base's first extension-line origin.
    first: Point3,
    /// Group 14: the base's second extension-line origin.
    second: Point3,
    /// Group 10: the base's dimension-line location.
    location: Point3,
    /// Group 50: the base's rotation (radians, about `+Z`).
    rotation: f64,
}

impl Application {
    /// DIMBASELINE: stack N linear dimensions off one picked linear base
    /// dimension (ADR 0017 decision C, P5f).
    pub(crate) fn dim_baseline(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        self.create_dimension_chain(session, command, ChainKind::Baseline)
    }

    /// DIMCONTINUE: chain N linear dimensions from one picked linear base
    /// dimension (ADR 0017 decision C, P5f).
    pub(crate) fn dim_continue(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        self.create_dimension_chain(session, command, ChainKind::Continue)
    }

    /// The shared DIMBASELINE/DIMCONTINUE body: resolve the base and the style,
    /// validate every input, build one definition per picked point, synthesize
    /// each into a `DimensionGeometry` and insert the whole batch in one
    /// transaction.
    ///
    /// Payload is `DimChain { base, points }` — the ref resolved at pick time plus
    /// the picked points in pick order. Every input is resolved to plain data
    /// before any write, so a refusal changes nothing and records no history.
    fn create_dimension_chain(
        &mut self,
        session: &SessionState,
        command: &Command,
        kind: ChainKind,
    ) -> CadResult<CommandOutcome> {
        let name = kind.name();
        let CommandPayload::DimChain(input) = &command.payload else {
            return Err(CadError::InvalidInput(format!(
                "{name} needs a DimChain {{ base, points }} payload"
            )));
        };
        if input.base.document != command.document {
            return Err(CadError::InvalidInput(format!(
                "{name} base belongs to another document"
            )));
        }
        // Zero picked points: an explicit refusal for a programmatic caller (the
        // interactive capture's `AtLeast(2)` arity already requires one measured
        // point beyond the base pick). Never a silent empty success.
        if input.points.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "{name} needs at least one picked point after the base pick"
            )));
        }
        if input.points.iter().any(|point| !is_finite_point(*point)) {
            return Err(CadError::InvalidInput(format!(
                "{name} picked points must all be finite"
            )));
        }
        let document_id = command.document;
        let (base, style) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let base = resolve_chain_base(document.drawing.as_ref(), &input.base, name)?;
            let style = document
                .drawing
                .resolve_dim_style("Standard")
                .ok_or_else(|| {
                    CadError::InvalidInput(
                        "the drawing has no 'Standard' dimension style to create with".into(),
                    )
                })?;
            (base, style)
        };
        check_chain_plane(&base, &input.points, name)?;
        let definitions = match kind {
            ChainKind::Baseline => {
                baseline_definitions(&base, &input.points, style.dim_line_increment, name)?
            }
            ChainKind::Continue => continue_definitions(&base, &input.points, name)?,
        };
        // Each row is an ordinary synthesized dimension: `synthesized` recomputes
        // the measurement, the formatted text and the display from the definition
        // and the resolved style snapshot, so there is no second geometry path.
        let mut dimensions = Vec::with_capacity(definitions.len());
        for definition in definitions {
            dimensions.push(SemanticGeometry::Dimension(Box::new(
                DimensionGeometry::synthesized(
                    definition,
                    "Standard".to_string(),
                    style.clone(),
                    None,
                )?,
            )));
        }
        self.commit_new_entities(session, command, dimensions, kind.label())
    }
}

/// Capture-time validation for a DIMBASELINE/DIMCONTINUE base pick.
///
/// The host calls this before it consumes the pick step, so a wrong object is
/// refused by name at capture time. It is the **same** resolution the command
/// runs at apply time (one predicate, one message); only the resolved value is
/// dropped, so the base struct stays private to this module.
pub(crate) fn ensure_chain_base(
    drawing: &DrawingDatabase,
    reference: &SelectionRef,
    command: &str,
) -> CadResult<()> {
    resolve_chain_base(drawing, reference, command).map(|_| ())
}

/// Resolve a picked entity to the **linear** chain base, refusing every other
/// shape by name (ADR 0017 decision C, P5f).
///
/// This is the single resolution both the capture check and the command use, so a
/// wrong pick is refused with the same reason at both points. A ref that carries
/// an instance path (a block-instance member) or a sub-element is refused instead
/// of being reached into, and so is every non-`Linear` dimension kind and every
/// non-dimension geometry.
fn resolve_chain_base(
    drawing: &DrawingDatabase,
    reference: &SelectionRef,
    command: &str,
) -> CadResult<ChainBase> {
    if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
        return Err(CadError::Unsupported(format!(
            "{command} base cannot be a block-instance member or a sub-element; pick the \
             dimension entity itself"
        )));
    }
    let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
        CadError::InvalidInput(format!(
            "{command} base {} does not exist",
            reference.entity.0
        ))
    })?;
    let SemanticGeometry::Dimension(dimension) = &entity.geometry else {
        return Err(CadError::Unsupported(format!(
            "{command} base must be a linear dimension; {} is not a dimension",
            geometry_kind(&entity.geometry)
        )));
    };
    match dimension.kind() {
        DimensionKind::Linear { rotation } => match &dimension.definition().points {
            DimensionPoints::Linear {
                first,
                second,
                dimension_line,
            } => Ok(ChainBase {
                first: *first,
                second: *second,
                location: *dimension_line,
                rotation,
            }),
            // A `Linear` kind always carries linear points (`is_consistent` is
            // enforced on construction); keep an explicit invariant instead of an
            // unreachable panic.
            other => Err(CadError::Invariant(format!(
                "{command} base is a linear dimension with {} points",
                describe_points(other)
            ))),
        },
        other => Err(CadError::Unsupported(format!(
            "{command} base must be a linear dimension; {} is not supported",
            describe_kind(other)
        ))),
    }
}

/// A stable description of a dimension kind for refusal messages. Every kind is
/// named, so a caller never has to guess which one was refused.
fn describe_kind(kind: DimensionKind) -> &'static str {
    match kind {
        DimensionKind::Linear { .. } => "a linear dimension",
        DimensionKind::Aligned => "an aligned dimension",
        DimensionKind::Angular2Ln | DimensionKind::Angular3Pt => "an angular dimension",
        DimensionKind::Radius => "a radius dimension",
        DimensionKind::Diameter => "a diameter dimension",
        DimensionKind::Ordinate { .. } => "an ordinate dimension",
    }
}

/// A stable description of a definition's point slot (only reachable through the
/// defensive invariant above).
fn describe_points(points: &DimensionPoints) -> &'static str {
    match points {
        DimensionPoints::Linear { .. } => "linear",
        DimensionPoints::Radial { .. } => "radial",
        DimensionPoints::Angular2Ln { .. } => "two-line angular",
        DimensionPoints::Angular3Pt { .. } => "three-point angular",
        DimensionPoints::Ordinate { .. } => "ordinate",
    }
}

/// Refuse an off-`+Z`-plane input (ADR 0017 D.6): the base's definition points
/// and every picked point must share one `+Z` plane. No work plane is guessed.
fn check_chain_plane(base: &ChainBase, points: &[Point3], command: &str) -> CadResult<()> {
    let reference = base.first;
    let z_tol = 1e-9
        * (1.0
            + base.first.z.abs()
            + base.second.z.abs()
            + base.location.z.abs()
            + points.iter().map(|point| point.z.abs()).sum::<f64>());
    for (index, point) in points.iter().copied().enumerate() {
        if (point.z - reference.z).abs() > z_tol {
            return Err(CadError::InvalidInput(format!(
                "{command} picked point {} must lie in the base dimension's +Z plane; \
                 off-plane dimension creation is NOT IMPLEMENTED",
                index + 1
            )));
        }
    }
    if (base.second.z - reference.z).abs() > z_tol || (base.location.z - reference.z).abs() > z_tol
    {
        return Err(CadError::InvalidInput(format!(
            "{command} base dimension must lie in one +Z plane; off-plane dimension creation \
             is NOT IMPLEMENTED"
        )));
    }
    Ok(())
}

/// Build one stacked definition per picked point (ADR 0017 decision C, P5f).
///
/// Row `i` (`i = 1..=N`) uses `first = base.first`, `second = P_i`, the base's
/// rotation, and `dimension_line = base.location + n̂ · (i · increment)`. `n̂` is
/// the base dimension line's normal pointing **away from that row's measured
/// geometry**; a zero dot product leaves the side undecidable, which is a named
/// refusal (this command never picks a side arbitrarily).
fn baseline_definitions(
    base: &ChainBase,
    points: &[Point3],
    increment: f64,
    command: &str,
) -> CadResult<Vec<DimensionDefinition>> {
    if !increment.is_finite() || increment <= MIN_CHAIN_EXTENT {
        return Err(CadError::InvalidInput(format!(
            "{command} needs a positive finite DIMDLI row spacing"
        )));
    }
    let direction = direction_of(base.rotation);
    let normal = perpendicular(direction);
    let mut out = Vec::with_capacity(points.len());
    for (index, point) in points.iter().copied().enumerate() {
        let row = index + 1;
        // A row whose measured point has the base origin's own projection
        // measures zero along the base direction (a coincident point, or one
        // offset only across the dimension line): refuse it by name.
        let measured = dot(sub(point, base.first), direction);
        if !measured.is_finite() || measured.abs() <= MIN_CHAIN_EXTENT {
            return Err(CadError::InvalidInput(format!(
                "{command} row {row} is degenerate: the picked point has the same projection \
                 as the base dimension's first origin and would measure zero"
            )));
        }
        let mid = midpoint(base.first, point);
        let side = dot(sub(base.location, mid), normal);
        if !side.is_finite() || side.abs() <= MIN_CHAIN_EXTENT {
            return Err(CadError::InvalidInput(format!(
                "{command} cannot decide which side of the measured geometry row {row} is \
                 stacked on: the base dimension line lies on the measured points"
            )));
        }
        let away = if side > 0.0 {
            normal
        } else {
            scale_point(normal, -1.0)
        };
        let offset = increment * row as f64;
        out.push(DimensionDefinition {
            kind: DimensionKind::Linear {
                rotation: base.rotation,
            },
            points: DimensionPoints::Linear {
                first: base.first,
                second: point,
                dimension_line: add(base.location, scale_point(away, offset)),
            },
            // Group 11/53 stay at the zero "derive from the geometry" sentinel, so
            // the synthesis places the text itself (ADR 0017 A.3).
            text_middle: origin(),
            text_rotation: 0.0,
            normal: z_up(),
        });
    }
    Ok(out)
}

/// Build one chained definition per picked point (ADR 0017 decision C, P5f).
///
/// Segment 1 is `base.second → P_1` and segment `i > 1` is `P_{i−1} → P_i`; every
/// segment keeps `dimension_line = base.location` (no stacking). The pick order
/// is the chain direction, so a segment whose projection along the base direction
/// is `<= 0` would fold back and is refused by name.
fn continue_definitions(
    base: &ChainBase,
    points: &[Point3],
    command: &str,
) -> CadResult<Vec<DimensionDefinition>> {
    let direction = direction_of(base.rotation);
    let mut out = Vec::with_capacity(points.len());
    let mut previous = base.second;
    for (index, point) in points.iter().copied().enumerate() {
        let segment = index + 1;
        let advance = dot(sub(point, previous), direction);
        if !advance.is_finite() || advance <= MIN_CHAIN_EXTENT {
            return Err(CadError::InvalidInput(format!(
                "{command} segment {segment} must advance along the base dimension's \
                 direction; a zero-length or folding-back segment is not supported"
            )));
        }
        out.push(DimensionDefinition {
            kind: DimensionKind::Linear {
                rotation: base.rotation,
            },
            points: DimensionPoints::Linear {
                first: previous,
                second: point,
                dimension_line: base.location,
            },
            text_middle: origin(),
            text_rotation: 0.0,
            normal: z_up(),
        });
        previous = point;
    }
    Ok(out)
}

/// The XY unit direction of a linear dimension's group-50 rotation.
fn direction_of(rotation: f64) -> Point3 {
    Point3 {
        x: rotation.cos(),
        y: rotation.sin(),
        z: 0.0,
    }
}

/// The zero point, used as the "derive it yourself" sentinel for group 11.
fn origin() -> Point3 {
    Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    }
}

/// The `+Z` extrusion normal every created dimension carries.
fn z_up() -> Point3 {
    Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    }
}

fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn scale_point(v: Point3, factor: f64) -> Point3 {
    Point3 {
        x: v.x * factor,
        y: v.y * factor,
        z: v.z * factor,
    }
}

fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn midpoint(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: (a.x + b.x) * 0.5,
        y: (a.y + b.y) * 0.5,
        z: (a.z + b.z) * 0.5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> ChainBase {
        // A horizontal base dimension (rotation 0) whose dimension line sits
        // 10 units **below** the measured segment y = 0.
        ChainBase {
            first: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            second: Point3 {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            location: Point3 {
                x: 5.0,
                y: -10.0,
                z: 0.0,
            },
            rotation: 0.0,
        }
    }

    fn at(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn baseline_rows_stack_away_from_the_measured_geometry() {
        let definitions = baseline_definitions(
            &base(),
            &[at(10.0, 0.0), at(20.0, 0.0)],
            3.75,
            "DIMBASELINE",
        )
        .unwrap();
        assert_eq!(definitions.len(), 2);
        // The base line is below the geometry, so "away" is -Y: rows step down.
        for (index, definition) in definitions.iter().enumerate() {
            let row = index + 1;
            match &definition.points {
                DimensionPoints::Linear {
                    first,
                    second,
                    dimension_line,
                } => {
                    assert_eq!(*first, base().first);
                    assert_eq!(*second, at(10.0 * row as f64, 0.0));
                    assert_eq!(dimension_line.y, -10.0 - 3.75 * row as f64);
                    assert_eq!(dimension_line.x, 5.0);
                }
                other => panic!("expected Linear points, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_base_on_the_other_side_flips_the_stacking_normal() {
        let mut flipped = base();
        flipped.location = at(5.0, 10.0);
        let definitions =
            baseline_definitions(&flipped, &[at(10.0, 0.0)], 3.75, "DIMBASELINE").unwrap();
        match &definitions[0].points {
            DimensionPoints::Linear { dimension_line, .. } => {
                assert_eq!(dimension_line.y, 10.0 + 3.75);
            }
            other => panic!("expected Linear points, got {other:?}"),
        }
    }

    #[test]
    fn undecidable_baseline_normal_and_degenerate_rows_are_refused_by_name() {
        // The base dimension line lies exactly on the measured segment: no side.
        let mut on_the_geometry = base();
        on_the_geometry.location = at(5.0, 0.0);
        let error = baseline_definitions(&on_the_geometry, &[at(10.0, 0.0)], 3.75, "DIMBASELINE")
            .unwrap_err();
        assert!(format!("{error:?}").contains("cannot decide"), "{error:?}");
        // A point at the base's first origin measures zero.
        let error =
            baseline_definitions(&base(), &[at(0.0, 0.0)], 3.75, "DIMBASELINE").unwrap_err();
        assert!(format!("{error:?}").contains("degenerate"), "{error:?}");
        // A zero row spacing would stack every row on the same line.
        let error =
            baseline_definitions(&base(), &[at(10.0, 0.0)], 0.0, "DIMBASELINE").unwrap_err();
        assert!(
            format!("{error:?}").contains("DIMDLI row spacing"),
            "{error:?}"
        );
    }

    #[test]
    fn continue_segments_chain_from_the_base_second_origin() {
        let definitions = continue_definitions(
            &base(),
            &[at(20.0, 0.0), at(35.0, 0.0), at(40.0, 0.0)],
            "DIMCONTINUE",
        )
        .unwrap();
        let pairs: Vec<(Point3, Point3)> = definitions
            .iter()
            .map(|definition| match &definition.points {
                DimensionPoints::Linear {
                    first,
                    second,
                    dimension_line,
                } => {
                    assert_eq!(*dimension_line, base().location, "no stacking");
                    (*first, *second)
                }
                other => panic!("expected Linear points, got {other:?}"),
            })
            .collect();
        assert_eq!(
            pairs,
            vec![
                (at(10.0, 0.0), at(20.0, 0.0)),
                (at(20.0, 0.0), at(35.0, 0.0)),
                (at(35.0, 0.0), at(40.0, 0.0)),
            ]
        );
        // A segment that does not advance (or goes backwards) folds back.
        for bad in [at(10.0, 0.0), at(5.0, 0.0)] {
            let error = continue_definitions(&base(), &[bad], "DIMCONTINUE").unwrap_err();
            assert!(format!("{error:?}").contains("advance"), "{error:?}");
        }
    }

    #[test]
    fn off_plane_picks_are_refused_by_name() {
        let mut off_plane = at(10.0, 0.0);
        off_plane.z = 5.0;
        let error = check_chain_plane(&base(), &[off_plane], "DIMCONTINUE").unwrap_err();
        assert!(format!("{error:?}").contains("+Z plane"), "{error:?}");
        assert!(check_chain_plane(&base(), &[at(10.0, 0.0)], "DIMCONTINUE").is_ok());
    }
}
