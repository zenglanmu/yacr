//! Wave 9 **2D draw/modify** handlers for the extended command batch.
//!
//! Two names are real 2D edits that reuse the shared create funnel:
//!
//! * `BLEND` builds one cubic `Spline` between the nearest ends of two open
//!   `LINE`/`LWPOLYLINE`/`ARC` entities. The end tangents come from the source
//!   curves; the two inner control points sit one third of the gap along those
//!   tangents (a declared rule, not an AutoCAD-verified construction).
//! * `BREAKLINE` builds one open `LWPOLYLINE` whose middle carries a zigzag
//!   break symbol between the two picked points (also a declared symbol rule).
//!
//! `ARRAY`, `-ARRAY` and `ARRAYCLASSIC` are **routers**: a typed
//! `ArrayRect`/`ArrayPolar` payload re-dispatches to the existing
//! `ARRAYRECT`/`ARRAYPOLAR` planner. With no payload they report the available
//! planners instead of inventing an array.
//!
//! `ARCTEXT` is a real 2D edit: the current selection supplies exactly one `ARC`
//! and at least one `TEXT`/`MTEXT`, and each character of the text is placed as
//! its own `TEXT` at an equal angular step along the arc, rotated to the local
//! tangent. **Representation divergence** (deliberate, documented): this build has
//! no `ARCTEXT` object, so the result is *one TEXT entity per character* rather
//! than a single arc-attached annotation; the arc itself is kept, the text sources
//! are consumed, and the whole edit is one transaction / one undo step.
//!
//! Everything else is a stable named refusal: this build has no multileader
//! alignment/edit collection, tolerance-object or arc-length dimension model, and
//! `CAL` has no expression evaluator. A refusal writes nothing, bumps no revision
//! and records no history.

use super::*;
use crate::app_drawing::class_name;
use crate::app_extended_w9_reports::text_explode::{replace_entities, ProducedEntity};
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::require_none_payload;
use cad_db::DbEntity;
use cad_domain::{EntityId, TextAlignH, TextAlignV};
use cad_geometry::{add, cross, length, normalize, scale, sub};
use cad_representation::sanitize_text;

const EPS: f64 = 1e-9;

/// The character bound for one `ARCTEXT` line.
///
/// Every character becomes its own entity, so an unbounded string would mean an
/// unbounded write. Well above any hand-authored annotation, this only catches a
/// hostile value.
const MAX_ARCTEXT_CHARS: usize = 512;

impl Application {
    /// Dispatch one Wave 9 draw/modify command by its selector.
    pub(crate) fn extended_w9_draw(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::Array
            | ExtendedCommandId::DashArray
            | ExtendedCommandId::ArrayClassic => self.array_router(session, command, ext),
            ExtendedCommandId::ArcText => self.arc_text(session, command),
            ExtendedCommandId::Blend => self.blend(session, command),
            ExtendedCommandId::BreakLine => self.break_line(session, command),
            ExtendedCommandId::DimRouter => self.dim_router(command),
            other => Err(CadError::Unsupported(reason(other))),
        }
    }

    /// `ARRAY` / `-ARRAY` / `ARRAYCLASSIC`: route a typed array payload to the
    /// one existing planner; with no payload, report the two planners.
    fn array_router(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match &command.payload {
            CommandPayload::ArrayRect { .. } => self.array_rect(session, command),
            CommandPayload::ArrayPolar { .. } => self.array_polar(session, command),
            CommandPayload::None => Ok(report_outcome(vec![diagnostic(
                "array.needs_payload",
                format!(
                    "{}: no array construction was supplied; use ARRAYRECT \
                     (rows/columns/spacing) or ARRAYPOLAR (centre/items/fill angle) with the \
                     typed payload, or the ARRAYRECT/ARRAYPOLAR selectors. No array was \
                     created and nothing was written",
                    ext.selector()
                ),
            )])),
            _ => Err(CadError::InvalidInput(format!(
                "{} needs an ArrayRect or ArrayPolar payload",
                ext.selector()
            ))),
        }
    }

    /// `ARCTEXT`: place one `TEXT` per character along the selected arc.
    ///
    /// Selection-based entry (no payload): exactly one `ARC` plus at least one
    /// `TEXT`/`MTEXT`. The characters are spaced at equal angular steps
    /// (`sweep / characters`), the first character centred in the first step;
    /// each glyph is rotated to the arc's tangent at its own angle and flipped by
    /// 180° when that rotation would read upside down. Height, style, font, layer
    /// and space come from the **first** selected text; several text sources are
    /// concatenated in selection order.
    ///
    /// **Representation divergence:** this build has no arc-attached text object,
    /// so the result is one `TEXT` entity per character (a single `ARCTEXT`
    /// annotation is NOT created) and the text sources are deleted. The arc is
    /// kept. One transaction / one undo step.
    ///
    /// Refusals (named, nothing written): `arctext.payload`,
    /// `arctext.selection`, `arctext.not_addressable`, `arctext.no_arc`,
    /// `arctext.multiple_arcs`, `arctext.no_text`, `arctext.unsupported_selection`,
    /// `arctext.plane`, `arctext.degenerate`, `arctext.multiline`,
    /// `arctext.text_empty`, `arctext.too_many`, `arctext.height`; Viewer mode is
    /// `PermissionDenied` (the command writes entity rows).
    fn arc_text(&mut self, session: &SessionState, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "ARCTEXT")?;
        if session.app_mode() == AppMode::Viewer {
            return Err(CadError::PermissionDenied);
        }
        let drawing = document_drawing(self, command)?.clone();
        let entities = resolve_selected(session, &drawing, command, "arctext")?;
        let mut arcs: Vec<&DbEntity> = Vec::new();
        let mut texts: Vec<&DbEntity> = Vec::new();
        for entity in &entities {
            match &entity.geometry {
                SemanticGeometry::Arc { .. } => arcs.push(entity),
                SemanticGeometry::Text { .. } => texts.push(entity),
                other => {
                    return Err(invalid(
                        "arctext",
                        "unsupported_selection",
                        format!(
                            "ARCTEXT takes one ARC plus TEXT/MTEXT; entity {} is {}",
                            entity.id.0,
                            class_name(other)
                        ),
                    ))
                }
            }
        }
        let arc = match arcs.len() {
            0 => {
                return Err(invalid(
                    "arctext",
                    "no_arc",
                    "the selection holds no ARC; ARCTEXT places text along a selected arc",
                ))
            }
            1 => arcs[0],
            count => {
                return Err(invalid(
                    "arctext",
                    "multiple_arcs",
                    format!("ARCTEXT takes exactly one ARC, got {count}"),
                ))
            }
        };
        if texts.is_empty() {
            return Err(invalid(
                "arctext",
                "no_text",
                "the selection holds no TEXT/MTEXT to place along the arc",
            ));
        }
        let SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } = &arc.geometry
        else {
            return Err(CadError::Invariant(
                "arctext: the resolved arc is not an arc".into(),
            ));
        };
        let axis = normalize(*normal);
        if !axis.z.is_finite() || (axis.x).abs() > 1e-9 || (axis.y).abs() > 1e-9 || axis.z <= 0.0 {
            return Err(unsupported(
                "arctext",
                "plane",
                "ARCTEXT places TEXT entities, whose rotation is the +Z plane's; the selected arc \
                 is not on the +Z plane",
            ));
        }
        if !radius.is_finite()
            || *radius <= 0.0
            || !start.is_finite()
            || !sweep.is_finite()
            || sweep.abs() <= EPS
        {
            return Err(invalid(
                "arctext",
                "degenerate",
                format!(
                    "the arc needs a finite positive radius and a non-zero sweep, got radius \
                     {radius} and sweep {sweep}"
                ),
            ));
        }
        let mut string = String::new();
        for source in &texts {
            if let SemanticGeometry::Text { text, .. } = &source.geometry {
                string.push_str(&sanitize_text(text));
            }
        }
        if string.contains('\n') {
            return Err(invalid(
                "arctext",
                "multiline",
                "the selected text is multi-line; ARCTEXT places one single-line arc of characters",
            ));
        }
        let characters: Vec<char> = string.chars().collect();
        if characters.len() > MAX_ARCTEXT_CHARS {
            return Err(invalid(
                "arctext",
                "too_many",
                format!(
                    "ARCTEXT places one entity per character and allows at most \
                     {MAX_ARCTEXT_CHARS}, got {}",
                    characters.len()
                ),
            ));
        }
        let first = texts[0];
        let SemanticGeometry::Text {
            style,
            height,
            font,
            ..
        } = &first.geometry
        else {
            return Err(CadError::Invariant(
                "arctext: the formatting source is not a text".into(),
            ));
        };
        if !height.is_finite() || *height <= 0.0 {
            return Err(invalid(
                "arctext",
                "height",
                format!("the text height must be finite and positive, got {height}"),
            ));
        }
        let count = characters.len();
        let step = sweep / count as f64;
        let mut produced: Vec<ProducedEntity> = Vec::with_capacity(count);
        let mut flipped = 0usize;
        for (index, character) in characters.iter().enumerate() {
            let angle = start + step * (index as f64 + 0.5);
            let position = arc_point(*center, *normal, *radius, angle);
            // The tangent at `angle`: the direction of travel keeps the reading
            // order along the arc (CCW: +90°, CW: -90° from the radius).
            let mut rotation = if *sweep > 0.0 {
                angle + std::f64::consts::FRAC_PI_2
            } else {
                angle - std::f64::consts::FRAC_PI_2
            };
            // An upside-down run is unreadable: flip a glyph whose reading
            // direction points into the -X half. A 180° turn keeps it tangential.
            if rotation.cos() < -EPS {
                rotation += std::f64::consts::PI;
                flipped += 1;
            }
            if character.is_whitespace() {
                // Whitespace still consumes its angular step, so the visual gaps
                // match the string, but no empty TEXT row is written.
                continue;
            }
            produced.push(ProducedEntity {
                geometry: SemanticGeometry::Text {
                    text: character.to_string(),
                    position,
                    style: *style,
                    height: *height,
                    rotation: normalize_angle(rotation),
                    font: font.clone(),
                    // Each glyph is centred on its own arc position, with its
                    // baseline on the arc: the declared placement rule.
                    h_align: TextAlignH::Center,
                    v_align: TextAlignV::Baseline,
                    wrap_width: None,
                    width_factor: 1.0,
                },
                layer: first.layer,
                space: first.space.clone(),
            });
        }
        if produced.is_empty() {
            return Err(invalid(
                "arctext",
                "text_empty",
                "the selected text has no non-whitespace character to place along the arc",
            ));
        }
        let placed = produced.len();
        let source_count = texts.len();
        let mut outcome = replace_entities(
            self,
            command,
            texts.into_iter().cloned().collect(),
            produced,
            "arc text",
            "drawing.arctext",
        )?;
        let mut diagnostics = outcome.diagnostics.clone();
        diagnostics.push(diagnostic(
            "arctext.partial",
            format!(
                "ARCTEXT: {placed} TEXT entities placed at equal angular steps along the arc \
                 ({count} character(s), {source_count} text source(s) consumed); this build has \
                 no arc-attached ARCTEXT object, so one entity per character is the modeled \
                 representation and the arc itself is kept"
            ),
        ));
        diagnostics.push(diagnostic(
            "arctext.spacing_rule",
            "ARCTEXT: the angular step is the arc sweep divided by the character count and the \
             glyph height/style/font come from the first selected text (a declared rule; AutoCAD \
             spaces by advance width)"
                .into(),
        ));
        if flipped > 0 {
            diagnostics.push(diagnostic(
                "arctext.upright_flip",
                format!(
                    "ARCTEXT: {flipped} glyph(s) were turned 180 degrees to stay upright; the turn \
                     keeps them tangential (the arm of the arc that would read upside down)"
                ),
            ));
        }
        outcome.diagnostics = diagnostics;
        Ok(outcome)
    }

    /// `BLEND`: one cubic spline bridging the nearest ends of two open curves.
    fn blend(&mut self, session: &SessionState, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "BLEND")?;
        let drawing = document_drawing(self, command)?.clone();
        let entities = resolve_selected(session, &drawing, command, "blend")?;
        if entities.len() != 2 {
            return Err(invalid(
                "blend",
                "selection",
                format!(
                    "BLEND needs exactly two selected open curves, got {}",
                    entities.len()
                ),
            ));
        }
        let a = open_endpoints(&entities[0])?;
        let b = open_endpoints(&entities[1])?;
        let (a_end, b_end, a_tangent, b_tangent) = nearest_ends(a, b)?;
        let span = length(sub(b_end, a_end));
        if !span.is_finite() || span <= EPS {
            return Err(invalid(
                "blend",
                "degenerate",
                "the two nearest ends coincide; BLEND has no gap to bridge",
            ));
        }
        let third = span / 3.0;
        let control = vec![
            a_end,
            add(a_end, scale(a_tangent, third)),
            add(b_end, scale(b_tangent, third)),
            b_end,
        ];
        let spline = SemanticGeometry::Spline {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control_points: control,
            weights: vec![1.0, 1.0, 1.0, 1.0],
        };
        let mut outcome = self.commit_new_entities(session, command, vec![spline], "blend")?;
        outcome.diagnostics.push(diagnostic(
            "blend.partial",
            "BLEND: one cubic spline built from the nearest source ends; the inner control \
             points sit one third of the gap along the source end tangents (declared rule), \
             and no curve-curve continuity option is offered"
                .into(),
        ));
        Ok(outcome)
    }

    /// `BREAKLINE`: one open polyline with a zigzag break symbol.
    fn break_line(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "BREAKLINE needs two points (start and end)".into(),
            ));
        };
        if points.len() != 2 {
            return Err(invalid(
                "breakline",
                "points",
                format!("BREAKLINE needs exactly two points, got {}", points.len()),
            ));
        }
        let (start, end) = (points[0], points[1]);
        let span = sub(end, start);
        let len = length(span);
        if !len.is_finite() || len <= EPS {
            return Err(invalid(
                "breakline",
                "degenerate",
                "the two BREAKLINE points coincide",
            ));
        }
        let dir = scale(span, 1.0 / len);
        let mut normal = cross(
            dir,
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        );
        if length(normal) <= EPS {
            normal = Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            };
        } else {
            normal = normalize(normal);
        }
        let mid = add(start, scale(span, 0.5));
        let half = len * 0.15;
        let depth = len * 0.05;
        let zig = vec![
            start,
            sub(mid, scale(dir, half)),
            add(sub(mid, scale(dir, half * 0.55)), scale(normal, depth)),
            sub(mid, scale(dir, half * 0.15)),
            add(add(mid, scale(dir, half * 0.15)), scale(normal, -depth)),
            sub(end, scale(dir, half)),
            end,
        ];
        let polyline = SemanticGeometry::Polyline {
            points: zig,
            bulges: Vec::new(),
            closed: false,
        };
        let mut outcome =
            self.commit_new_entities(session, command, vec![polyline], "breakline")?;
        outcome.diagnostics.push(diagnostic(
            "breakline.partial",
            "BREAKLINE: one open polyline with a declared zigzag break symbol; the symbol \
             width (15% of the span) and depth (5%) are our rule, and no block-based \
             breakline symbol is produced"
                .into(),
        ));
        Ok(outcome)
    }

    /// `DIM`: read-only router report over the implemented dimension creators.
    fn dim_router(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "DIM")?;
        let mut rows = Vec::new();
        for name in [
            "DIMLINEAR",
            "DIMALIGNED",
            "DIMANGULAR",
            "DIMDIAMETER",
            "DIMRADIUS",
            "DIMORDINATE",
            "DIMBASELINE",
            "DIMCONTINUE",
            "DIMROTATED",
            "DIMCENTER",
            "DIMSPACE",
            "DIMTEDIT",
            "DIMEDIT",
            "DIMOVERRIDE",
            "DIMSTYLE",
            "QDIM",
        ] {
            rows.push(diagnostic(
                "dim.available",
                format!("DIM: {name} is an implemented dimension selector; type it directly"),
            ));
        }
        rows.push(diagnostic(
            "dim.router_only",
            "DIM: this build has no interactive dimension-type prompt; it lists the supported \
             DIM* selectors and creates nothing"
                .into(),
        ));
        Ok(report_outcome(rows))
    }
}

fn document_drawing<'a>(
    app: &'a Application,
    command: &Command,
) -> CadResult<&'a std::sync::Arc<cad_db::DrawingDatabase>> {
    app.workspace
        .documents
        .get(&command.document)
        .map(|document| &document.drawing)
        .ok_or_else(|| invalid("extended9", "not_addressable", "document not open"))
}

fn invalid(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{prefix}.{code}: {detail}"))
}

fn unsupported(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{prefix}.{code}: {detail}"))
}

fn resolve_selected(
    session: &SessionState,
    drawing: &cad_db::DrawingDatabase,
    command: &Command,
    prefix: &str,
) -> CadResult<Vec<DbEntity>> {
    let refs = session.selection.refs();
    if refs.is_empty() {
        return Err(invalid(prefix, "selection", "the selection is empty"));
    }
    let mut seen: Vec<EntityId> = Vec::new();
    let mut entities = Vec::new();
    for reference in refs {
        if reference.document != command.document {
            return Err(invalid(
                prefix,
                "not_addressable",
                "the selection contains an entity from another document",
            ));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(unsupported(
                prefix,
                "not_addressable",
                "block-instance and sub-element references are not supported",
            ));
        }
        if seen.contains(&reference.entity) {
            continue;
        }
        seen.push(reference.entity);
        let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
            invalid(
                prefix,
                "not_addressable",
                format!("entity {} is not in the drawing", reference.entity.0),
            )
        })?;
        entities.push(entity);
    }
    Ok(entities)
}

/// Both ends of one supported open curve, with the tangent pointing **out** of
/// the curve at each end.
struct OpenEnds {
    start: Point3,
    start_tangent: Point3,
    end: Point3,
    end_tangent: Point3,
}

fn open_endpoints(entity: &DbEntity) -> CadResult<OpenEnds> {
    match &entity.geometry {
        SemanticGeometry::Line { start, end } => {
            let dir = normalize(sub(*end, *start));
            Ok(OpenEnds {
                start: *start,
                start_tangent: scale(dir, -1.0),
                end: *end,
                end_tangent: dir,
            })
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let end_angle = start + sweep;
            let start_point = arc_point(*center, *normal, *radius, *start);
            let end_point = arc_point(*center, *normal, *radius, end_angle);
            let dir_start = arc_tangent(*center, *normal, *radius, *start, *sweep);
            let dir_end = arc_tangent(*center, *normal, *radius, end_angle, *sweep);
            Ok(OpenEnds {
                start: start_point,
                start_tangent: scale(dir_start, -1.0),
                end: end_point,
                end_tangent: dir_end,
            })
        }
        SemanticGeometry::Polyline {
            points,
            closed: false,
            ..
        } => {
            if points.len() < 2 {
                return Err(invalid(
                    "blend",
                    "degenerate",
                    "the polyline has fewer than two points",
                ));
            }
            let start = points[0];
            let start_dir = normalize(sub(points[1], start));
            let end = points[points.len() - 1];
            let end_dir = normalize(sub(end, points[points.len() - 2]));
            Ok(OpenEnds {
                start,
                start_tangent: scale(start_dir, -1.0),
                end,
                end_tangent: end_dir,
            })
        }
        other => Err(unsupported(
            "blend",
            "unsupported",
            format!(
                "BLEND supports open LINE, ARC and LWPOLYLINE only; got {}",
                class_name(other)
            ),
        )),
    }
}

/// Normalise an angle in radians into `(-π, π]`, so a stored text rotation stays
/// in the range the other rotation writers use and two equivalent angles compare
/// equal.
fn normalize_angle(angle: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    let mut value = angle % TAU;
    if value <= -PI {
        value += TAU;
    }
    if value > PI {
        value -= TAU;
    }
    value
}

fn arc_point(center: Point3, normal: Point3, radius: f64, angle: f64) -> Point3 {
    let axis = if length(normal) > EPS {
        normalize(normal)
    } else {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    };
    let mut x_axis = cross(
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        axis,
    );
    if length(x_axis) <= EPS {
        x_axis = cross(
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            axis,
        );
    }
    let x_axis = normalize(x_axis);
    let y_axis = normalize(cross(axis, x_axis));
    add(
        center,
        add(
            scale(x_axis, radius * angle.cos()),
            scale(y_axis, radius * angle.sin()),
        ),
    )
}

fn arc_tangent(center: Point3, normal: Point3, radius: f64, angle: f64, sweep: f64) -> Point3 {
    let point = arc_point(center, normal, radius, angle);
    let radial = normalize(sub(point, center));
    let axis = if length(normal) > EPS {
        normalize(normal)
    } else {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    };
    let dir = normalize(cross(axis, radial));
    if sweep < 0.0 {
        scale(dir, -1.0)
    } else {
        dir
    }
}

/// Pick the two nearest ends and return the blend direction at each: from the
/// chosen end **into** the gap.
fn nearest_ends(a: OpenEnds, b: OpenEnds) -> CadResult<(Point3, Point3, Point3, Point3)> {
    let candidates = [
        (a.start, a.start_tangent, b.start, b.start_tangent),
        (a.start, a.start_tangent, b.end, b.end_tangent),
        (a.end, a.end_tangent, b.start, b.start_tangent),
        (a.end, a.end_tangent, b.end, b.end_tangent),
    ];
    let mut best = None;
    for (pa, ta, pb, tb) in candidates {
        let gap = length(sub(pb, pa));
        if !gap.is_finite() {
            continue;
        }
        if best.is_none_or(|(_, _, _, _, best_gap)| gap < best_gap) {
            best = Some((pa, ta, pb, tb, gap));
        }
    }
    let Some((pa, ta, pb, tb, _)) = best else {
        return Err(invalid("blend", "degenerate", "no finite end pair"));
    };
    // Both stored tangents point **out** of their own curve, i.e. into the gap
    // between the two chosen ends; the cubic control points move along them.
    Ok((pa, pb, ta, tb))
}

fn reason(ext: ExtendedCommandId) -> String {
    let text = match ext {
        ExtendedCommandId::DimArc => {
            "dimarc.unsupported: the dimension domain has no arc-length kind"
        }
        ExtendedCommandId::Calc => "cal.unsupported: no expression evaluator is implemented",
        other => {
            return format!(
                "{}: this command is not a Wave 9 draw/refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
