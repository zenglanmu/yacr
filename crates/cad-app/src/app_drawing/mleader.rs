//! The `MLEADER` creation command (ADR 0019 decision F.2, P7c).
//!
//! Split out of `drawing_create.rs` to keep that module inside the code-size
//! limit. This is an inherent method on [`Application`], so the dispatch in
//! `drawing_create.rs` reads exactly like the other create commands.
//!
//! The command is the single construction point: it resolves the drawing's
//! `MLeaderStyle` table (decision D), runs the shared creation **option plan**
//! ([`crate::app_drawing::annot_batch::leader::plan_leader_options`] — the same
//! one `QLEADER`/`LEADER` use), asks [`MultiLeaderGeometry::created`] for the
//! definition (one root, one line, two vertices) and inserts one
//! `AcDbMultiLeader` through the shared create funnel — one transaction, one undo
//! step, the active layer. A refusal opens no transaction, so the drawing and the
//! history are untouched.
//!
//! ## Options (the modelled subset of AutoCAD's `Options` sub-prompt)
//!
//! * **leader type** — the resolved style's path type is honoured: a
//!   straight-segments style creates the derived two-vertex line, a **spline**
//!   style stores the spline path type (the definition is derived by the one
//!   [`MultiLeaderGeometry::created`] rule and only its stored path type is
//!   re-stamped, so there is no second derivation). With two vertices a spline and
//!   a straight run are geometrically identical, so the outcome says so
//!   (`mleader.spline_two_vertices`) instead of claiming a bend;
//! * **content / text** — MText content only, non-empty, bounded (the shared
//!   `tool.text_*` rules);
//! * **maximum points** — creation derives exactly
//!   [`MAX_CREATED_LEADER_POINTS`](crate::app_drawing::annot_batch::leader::MAX_CREATED_LEADER_POINTS)
//!   vertices, and a higher request is the named `mleader.option_max_points`
//!   refusal (the payload carries no vertex list);
//! * **arrowhead** — the style's arrowhead size draws it; a style that disables the
//!   arrowhead (size `0`) is refused by name (`mleader.option_arrowhead_off`);
//! * **dogleg length** — `dogleg_length` overrides the style's landing distance
//!   for the created root; it lands on the root's `LEADER 40` and the dogleg
//!   start vertex is derived from it, so it round-trips
//!   (`mleader.options_applied`);
//! * **dogleg direction** — `dogleg_angle` (radians about `+Z`, the direction the
//!   dogleg travels toward the annotation) replaces the arrow/text side
//!   derivation; it lands on the root's `LEADER 11` and round-trips;
//! * **attachment pair** — `attachment` sets the horizontal (`179` /
//!   `CONTEXT_DATA 171`) and vertical (`173` / `CONTEXT_DATA 174`) text
//!   attachment codes; both are written verbatim, so they round-trip.
//!
//! Still deliberate subsets (see [`crate::CommandId::MLeader`]): **no style
//! selector** and **no content-less creation** — a MULTILEADER without text is
//! import/display/export only (decision F.2) — and the AutoCAD **angle
//! constraint** (text angle type, DXF entity `174` / style `175`) has **no field
//! in this domain**: the `MULTILEADER` writer declares it as an unmodelled group,
//! so creation refuses it by name (`mleader.option_text_angle`) instead of adding
//! a field the exporter would silently drop.

use super::*;
use crate::app_drawing::annot_batch::leader::{
    options_report, plan_leader_options, MAX_CREATED_LEADER_POINTS,
};
use cad_domain::{MLeaderCreateOptions, MLeaderPathType, MultiLeaderGeometry, MultiLeaderParts};

// `MLEADERSTYLE SET <name>` writes `SessionState::current_mleader_style`, so
// creation reads the session name (default `Standard`, which the builder pins).
// A missing row is a named refusal rather than a silent fallback.

impl Application {
    /// Insert one multileader into model space on the active layer.
    ///
    /// Everything but the two points and the text comes from the drawing's
    /// `MLeaderStyle` row: dogleg length, landing gap, arrowhead size, text
    /// height, text style, vertical attachment, path type and content kind. The
    /// resolved snapshot is stored on the entity (P7b-1) — there is no second
    /// source, because the same resolution feeds both the definition and the
    /// export's style record.
    ///
    /// Refusals (named, nothing written, no transaction):
    ///
    /// * `mleader.payload` — a payload that is not `MLeader { .. }`;
    /// * `tool.text_empty` / `tool.text_too_long` — the text rules shared with
    ///   `TEXT` (empty/whitespace-only or more than
    ///   [`MAX_TOOL_TEXT_CHARS`] characters);
    /// * `mleader.style_missing` — the drawing has no `Standard` MLEADERSTYLE
    ///   (the builder pins one, so this is a corrupt-database case);
    /// * the option plan's refusals: `mleader.text_required` /
    ///   `mleader.content_unsupported` (annotation type), `mleader.path_type`
    ///   (invisible path), `mleader.option_arrowhead_off`,
    ///   `mleader.option_max_points`, `mleader.style_invalid`;
    /// * the creation options' refusals: `mleader.option_dogleg` (a non-finite or
    ///   non-positive `dogleg_length`), `mleader.option_angle` (a non-finite
    ///   `dogleg_angle`), `mleader.option_attachment` (a horizontal code outside
    ///   `1..=3` or a vertical code outside `0..=10`);
    /// * every refusal [`MultiLeaderGeometry::created`] raises: non-finite or
    ///   non-`+Z`-coplanar points, a landing point no farther than the dogleg
    ///   length, or an unusable style size;
    /// * a missing active layer (the shared create funnel).
    pub(crate) fn create_mleader(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::MLeader {
            arrow,
            landing,
            text,
            dogleg_length,
            dogleg_angle,
            attachment,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "mleader.payload: CreateMLeader needs a MLeader { arrow, landing, text, \
                 dogleg_length, dogleg_angle, attachment } payload"
                    .into(),
            ));
        };
        if text.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "tool.text_empty: a multileader needs a non-empty, non-whitespace text; \
                 content-less leaders are not created"
                    .into(),
            ));
        }
        let characters = text.chars().count();
        if characters > MAX_TOOL_TEXT_CHARS {
            return Err(CadError::InvalidInput(format!(
                "tool.text_too_long: MLEADER allows at most {MAX_TOOL_TEXT_CHARS} characters, \
                 got {characters}"
            )));
        }
        let (row, resolution, style_name) = {
            let document = self
                .workspace
                .documents
                .get(&command.document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = document.drawing.as_ref();
            let style_name_query = session.current_mleader_style_name();
            let row = drawing
                .mleader_style_by_name(style_name_query)
                .ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "mleader.style_missing: the drawing has no MLEADERSTYLE named \
                         '{style_name_query}'"
                    ))
                })?;
            // Store the row's own name, not the constant, so a source row with a
            // different case is not silently renamed on the entity (and in the
            // written MLEADERSTYLE record).
            let style_name = row.name.clone();
            let resolution = drawing
                .resolve_mleader_style(style_name_query)
                .ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "mleader.style_missing: the MLEADERSTYLE '{style_name_query}' did not \
                         resolve"
                    ))
                })?;
            (row.clone(), resolution, style_name)
        };
        let options = plan_leader_options(&row, &resolution)?;
        // The definition is derived by the one `created` rule, which only accepts
        // a straight path type; the plan has already refused the invisible one, and
        // a spline style is re-stamped onto the derived definition below rather
        // than re-derived here (one derivation rule, not two).
        let mut values = resolution.values.clone();
        values.path_type = MLeaderPathType::StraightLineSegments;
        let create_options = MLeaderCreateOptions {
            dogleg_length: *dogleg_length,
            dogleg_angle: *dogleg_angle,
            attachment_point: attachment.map(|pair| pair.point),
            left_attachment: attachment.map(|pair| pair.left),
        };
        let geometry = MultiLeaderGeometry::created_with_options(
            values,
            style_name,
            resolution.left_attachment,
            *arrow,
            *landing,
            text.clone(),
            create_options,
        )?;
        let geometry = with_leader_path_type(geometry, options.path)?;
        let options_row = create_options_report(create_options, &geometry);
        let mut outcome = self.commit_new_entity(
            session,
            command,
            SemanticGeometry::MultiLeader(Box::new(geometry)),
            "create mleader",
        )?;
        outcome.diagnostics.push(options_report(options));
        if let Some(row) = options_row {
            outcome.diagnostics.push(row);
        }
        if options.path == MLeaderPathType::Spline {
            outcome.diagnostics.push(Diagnostic {
                object: outcome.objects.first().copied(),
                code: "mleader.spline_two_vertices".into(),
                message: format!(
                    "MLEADER: the style's leader type is spline and it is stored as such, but a \
                     created leader has only {MAX_CREATED_LEADER_POINTS} vertices (the captured \
                     arrow and the style dogleg start), so the drawn path is the straight run \
                     between them; a bending spline needs three or more vertices, which the payload \
                     does not carry"
                ),
            });
        }
        Ok(outcome)
    }
}

/// The report row for the creation options that were actually applied.
///
/// Every explicit option lands in a stored field the exporter writes verbatim, so
/// this row names the value and the group it went to; no option is applied
/// silently. `None` when the payload asked for none.
fn create_options_report(
    options: MLeaderCreateOptions,
    geometry: &MultiLeaderGeometry,
) -> Option<Diagnostic> {
    let mut rows: Vec<String> = Vec::new();
    if options.dogleg_length.is_some() {
        let root = geometry.roots().first();
        rows.push(format!(
            "dogleg length {} (LEADER 40)",
            root.map(|root| root.dogleg_length).unwrap_or(f64::NAN)
        ));
    }
    if options.dogleg_angle.is_some() {
        let vector = geometry
            .roots()
            .first()
            .map(|root| root.dogleg_vector)
            .unwrap_or(cad_domain::Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            });
        rows.push(format!(
            "dogleg direction {}° (LEADER 11: {}, {})",
            vector.y.atan2(vector.x).to_degrees(),
            vector.x,
            vector.y
        ));
    }
    if options.left_attachment.is_some() || options.attachment_point.is_some() {
        let text = geometry.text();
        rows.push(format!(
            "text attachment pair {} / {} (entity 179/173, CONTEXT_DATA 171/174)",
            text.attachment_point, text.left_attachment
        ));
    }
    if rows.is_empty() {
        return None;
    }
    Some(Diagnostic {
        object: None,
        code: "mleader.options_applied".into(),
        message: format!(
            "MLEADER: the typed creation option(s) replaced the resolved style values: {}. Every \
             one of them is stored on the entity in the group named above and written verbatim, so \
             it round-trips",
            rows.join("; ")
        ),
    })
}

/// Re-stamp the derived definition's leader path type.
///
/// The straight case is the derivation as-is. The spline case rewrites the stored
/// style snapshot **and** every leader line's own path type (the per-line override
/// bit stays clear, so the style value is what the display and a reload read), then
/// rebuilds the derived display through [`MultiLeaderGeometry::new`] — the same
/// single display builder every other writer uses.
fn with_leader_path_type(
    geometry: MultiLeaderGeometry,
    path: MLeaderPathType,
) -> CadResult<MultiLeaderGeometry> {
    if path == MLeaderPathType::StraightLineSegments {
        return Ok(geometry);
    }
    let mut parts = MultiLeaderParts {
        style: geometry.style().clone(),
        style_name: geometry.style_name().to_string(),
        override_mask: geometry.override_mask(),
        content: geometry.content(),
        text: geometry.text().clone(),
        roots: geometry.roots().to_vec(),
        lines: geometry.lines().to_vec(),
        reserved: geometry.reserved().clone(),
        plane: geometry.plane(),
    };
    parts.style.path_type = path;
    for line in &mut parts.lines {
        line.path_type = path;
    }
    MultiLeaderGeometry::new(parts)
}
