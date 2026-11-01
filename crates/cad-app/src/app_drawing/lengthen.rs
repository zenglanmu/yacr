//! `LENGTHEN`: change the length of one selected LINE or ARC.
//!
//! The capture is the zero-point pre-selection shape
//! ([`crate::draw_tool::DrawToolKind::Lengthen`]): the one selected curve is the
//! whole input, and the mode + amount are the session typed channel
//! ([`crate::draw_tool::ToolParameter::LengthenMode`] —
//! `0` Delta / `1` Percent / `2` Total — and
//! [`crate::draw_tool::ToolParameter::LengthenValue`], finite and non-zero).
//!
//! ## New length
//!
//! | mode | formula |
//! | --- | --- |
//! | `0` Delta | `new = old + value` (a negative value shortens) |
//! | `1` Percent | `new = old · value / 100` (`value > 0`) |
//! | `2` Total | `new = value` (`value > 0`) |
//!
//! ## Declared v1 deviation: the **end** endpoint always moves
//!
//! AutoCAD lengthens the end nearest the pick (`LENGTHEN` then "select object to
//! lengthen"); the fixed-step capture cannot express that per-end pick, so this
//! build always moves the **end**: a LINE's `end` (its `start` is kept) and an
//! ARC's far end (`start + sweep`, with centre, normal, radius and `start` all
//! kept). A **negative** Delta shortens toward the kept end; it never flips the
//! curve through it (a non-positive new length is a refusal).
//!
//! An ARC's new sweep is `sign(old_sweep) · new_length / radius`. A sweep beyond
//! one full turn is `lengthen.sweep_overflow` (never wrapped or clamped), while a
//! sweep of **exactly** one turn is accepted and stays an `Arc` (this build does
//! not silently convert it into a `Circle`).
//!
//! ## Transaction shape
//!
//! Every refusal happens **before** `begin_drawing_transaction`, so a refused
//! LENGTHEN writes nothing: no revision bump, no `ChangeSet`, no undo entry. On
//! success the **same entity id** is updated with `staged.update_entity` in one
//! transaction / one undo step (layer, space, draw order and `type_key` are
//! preserved); the committed row is read back so the undo patch carries the exact
//! stored value. Work-only through [`crate::CommandId::Lengthen`].

use super::trim_curves::cut_tolerance;
use super::*;

/// LENGTHEN mode `Delta`: a signed length change.
const MODE_DELTA: f64 = 0.0;
/// LENGTHEN mode `Percent`: a percentage of the current length.
const MODE_PERCENT: f64 = 1.0;
/// LENGTHEN mode `Total`: the absolute new length.
const MODE_TOTAL: f64 = 2.0;

const TAU: f64 = std::f64::consts::TAU;

/// A LENGTHEN refusal: `InvalidInput` carrying its stable code.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// The name a refused target kind is reported under (ASCII, no class prefix).
fn kind_name(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Circle { .. } => "CIRCLE",
        SemanticGeometry::Polyline { .. } => "LWPOLYLINE",
        SemanticGeometry::Spline { .. } => "SPLINE",
        SemanticGeometry::Ellipse { .. } => "ELLIPSE",
        SemanticGeometry::Line { .. } => "LINE",
        SemanticGeometry::Arc { .. } => "ARC",
        _ => "that entity",
    }
}

/// The LENGTHEN mode and amount read from the session typed channel.
struct LengthenSettings {
    mode: f64,
    value: f64,
}

impl Application {
    /// `LENGTHEN` the one selected LINE/ARC (see the module docs).
    ///
    /// The same entity id is updated in one transaction / one undo step; every
    /// refusal leaves the drawing untouched.
    pub(crate) fn lengthen_curve(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Lengthen { refs } = &command.payload else {
            return Err(CadError::InvalidInput(
                "LENGTHEN needs a Lengthen { refs } payload".into(),
            ));
        };
        // 1. Exactly one selected curve.
        let [reference] = refs.as_slice() else {
            return Err(refusal(
                "lengthen.needs_one",
                format!(
                    "LENGTHEN needs exactly one selected curve, got {}",
                    refs.len()
                ),
            ));
        };
        // 2. Directly addressable model-space entities only.
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(refusal(
                "lengthen.not_addressable",
                format!(
                    "LENGTHEN target {} is a block-instance member or sub-element; only \
                     directly addressable model-space entities can be lengthened",
                    reference.entity.0
                ),
            ));
        }
        let settings = lengthen_settings(session)?;

        let document_id = command.document;
        // 3./4./5./6. Resolve the target and build the new geometry as plain data,
        //    before any transaction exists.
        let (before, after_geometry, old_length, new_length) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = document.drawing.as_ref();
            let (sources, _diagnostics) = Self::resolve_selection(
                drawing,
                document_id,
                refs,
                "lengthen",
                "LENGTHEN",
                "lengthen",
                false,
            )
            .map_err(|error| {
                refusal(
                    "lengthen.target_missing",
                    format!(
                        "LENGTHEN target {} does not resolve: {}",
                        reference.entity.0,
                        error_detail(error)
                    ),
                )
            })?;
            let before = sources.into_iter().next().ok_or_else(|| {
                refusal(
                    "lengthen.target_missing",
                    format!("LENGTHEN target {} does not resolve", reference.entity.0),
                )
            })?;
            if before.space != SpaceId::Model {
                return Err(refusal(
                    "lengthen.not_addressable",
                    format!(
                        "LENGTHEN target {} is not a model-space entity; only model space is \
                         supported",
                        before.id.0
                    ),
                ));
            }
            let (after_geometry, old_length, new_length) =
                lengthen_geometry(&before.geometry, &settings)?;
            (before, after_geometry, old_length, new_length)
        };

        let source_id = before.id;
        let mut after = before.clone();
        after.geometry = after_geometry;

        // 7. One transaction on the **same** entity (identity, layer, space, draw
        //    order and type key preserved; only the geometry changes).
        let mut candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("lengthen curve", transaction)?;
            staged.update_entity(after)?;
            staged.commit()?
        };
        // Read the committed row back so the undo patch's `after` is the exact
        // stored value, never a re-derivation.
        let after = candidate
            .entity(source_id)
            .cloned()
            .ok_or_else(|| CadError::Invariant("lengthened entity vanished after commit".into()))?;
        let object = ObjectId(source_id.0);
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "lengthen curve".into(),
                patches: vec![drawing_patch(source_id, Some(before), Some(after))],
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects: vec![object],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(object),
                code: "drawing.lengthen".into(),
                message: format!(
                    "LENGTHEN: source {} mode {} ({}), old length {old_length}, new length \
                     {new_length}; the end endpoint moved",
                    source_id.0,
                    mode_name(settings.mode),
                    settings.mode as i64
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// The mode's stable name (`Delta` / `Percent` / `Total`).
fn mode_name(mode: f64) -> &'static str {
    if mode == MODE_DELTA {
        "Delta"
    } else if mode == MODE_PERCENT {
        "Percent"
    } else {
        "Total"
    }
}

/// The channel error's inner message (a named refusal keeps its own text).
fn error_detail(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
        other => format!("{other:?}"),
    }
}

/// Read and defensively re-validate the LENGTHEN mode and amount.
///
/// The channel refuses a zero amount and a mode outside `{0, 1, 2}` at set time;
/// this is the defense in depth for a value injected past the channel. Percent
/// and Total additionally require a **positive** amount here (Delta is signed).
fn lengthen_settings(session: &SessionState) -> CadResult<LengthenSettings> {
    let mode_parameter = ToolParameter::LengthenMode;
    let mode_value = session.tool_parameter(mode_parameter);
    mode_parameter.validate(mode_value).map_err(|_| {
        refusal(
            "lengthen.invalid_value",
            format!(
                "LENGTHEN mode must be 0 (Delta), 1 (Percent) or 2 (Total), got {mode_value:?}"
            ),
        )
    })?;
    let value_parameter = ToolParameter::LengthenValue;
    let value = session.tool_parameter(value_parameter);
    value_parameter.validate(value).map_err(|_| {
        refusal(
            "lengthen.invalid_value",
            format!("LENGTHEN amount must be finite and non-zero, got {value:?}"),
        )
    })?;
    let (mode, value) = match (mode_value, value) {
        (ToolValue::Number(mode), ToolValue::Number(value)) => (mode, value),
        // `validate` accepted both values, so this arm is unreachable; refuse
        // instead of inventing a mode or an amount.
        other => {
            return Err(refusal(
                "lengthen.invalid_value",
                format!("LENGTHEN needs a numeric mode and amount, got {other:?}"),
            ))
        }
    };
    if (mode == MODE_PERCENT || mode == MODE_TOTAL) && value <= 0.0 {
        return Err(refusal(
            "lengthen.invalid_value",
            format!(
                "LENGTHEN {} needs a positive amount, got {value}",
                mode_name(mode)
            ),
        ));
    }
    Ok(LengthenSettings { mode, value })
}

/// Build the lengthened geometry, returning it with the old and new lengths.
fn lengthen_geometry(
    geometry: &SemanticGeometry,
    settings: &LengthenSettings,
) -> CadResult<(SemanticGeometry, f64, f64)> {
    let eps = cut_tolerance();
    match geometry {
        SemanticGeometry::Line { start, end } => {
            let old = distance(*start, *end);
            if !old.is_finite() || old <= eps {
                return Err(refusal(
                    "lengthen.degenerate",
                    "the LENGTHEN target LINE has no measurable length",
                ));
            }
            let new = new_length(old, settings)?;
            let unit = {
                let dx = (end.x - start.x) / old;
                let dy = (end.y - start.y) / old;
                let dz = (end.z - start.z) / old;
                Point3 {
                    x: dx,
                    y: dy,
                    z: dz,
                }
            };
            // Keep `start`; the END endpoint moves along the original direction.
            let new_end = Point3 {
                x: start.x + unit.x * new,
                y: start.y + unit.y * new,
                z: start.z + unit.z * new,
            };
            if !is_finite_point(new_end) {
                return Err(refusal(
                    "lengthen.non_positive",
                    "the LENGTHEN result is not finite",
                ));
            }
            Ok((
                SemanticGeometry::Line {
                    start: *start,
                    end: new_end,
                },
                old,
                new,
            ))
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let radius = radius.abs();
            if !radius.is_finite() || radius <= eps {
                return Err(refusal(
                    "lengthen.degenerate",
                    "the LENGTHEN target ARC has no measurable length",
                ));
            }
            let old = radius * sweep.abs();
            if !old.is_finite() || old <= eps {
                return Err(refusal(
                    "lengthen.degenerate",
                    "the LENGTHEN target ARC has no measurable length",
                ));
            }
            let new = new_length(old, settings)?;
            let new_sweep = sweep.signum() * new / radius;
            if !new_sweep.is_finite() || new_sweep.abs() <= eps {
                return Err(refusal(
                    "lengthen.non_positive",
                    "the LENGTHEN result would have no sweep",
                ));
            }
            if new_sweep.abs() > TAU + 1e-9 {
                return Err(refusal(
                    "lengthen.sweep_overflow",
                    format!(
                        "the LENGTHEN result sweeps {:.9} rad, more than one full turn ({TAU}); \
                         it is refused rather than wrapped",
                        new_sweep.abs()
                    ),
                ));
            }
            // Keep centre/normal/radius/start; the far end (start + sweep) moves. A
            // sweep of exactly one turn stays an ARC (never silently a CIRCLE).
            Ok((
                SemanticGeometry::Arc {
                    center: *center,
                    normal: *normal,
                    radius,
                    start: *start,
                    sweep: new_sweep,
                },
                old,
                new,
            ))
        }
        other => Err(CadError::Unsupported(format!(
            "LENGTHEN supports LINE and ARC only; {} is not supported",
            kind_name(other)
        ))),
    }
}

/// The new length of a curve from the mode and amount.
///
/// Requires a finite, strictly positive result: a zero/negative length has no
/// direction and is refused (`lengthen.non_positive`), never clamped.
fn new_length(old: f64, settings: &LengthenSettings) -> CadResult<f64> {
    let eps = cut_tolerance();
    let new = if settings.mode == MODE_DELTA {
        old + settings.value
    } else if settings.mode == MODE_PERCENT {
        old * settings.value / 100.0
    } else {
        settings.value
    };
    if !new.is_finite() || new <= eps {
        return Err(refusal(
            "lengthen.non_positive",
            format!(
                "LENGTHEN {} produces the non-positive length {new} (old length {old})",
                mode_name(settings.mode)
            ),
        ));
    }
    Ok(new)
}
