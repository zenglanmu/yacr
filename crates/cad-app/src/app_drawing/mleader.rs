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
//!   arrowhead (size `0`) is refused by name (`mleader.option_arrowhead_off`).
//!
//! Still deliberate subsets (see [`crate::CommandId::MLeader`]): **no style
//! selector**, no dogleg or attachment override, **no angular constraint**, and **no
//! content-less creation** — a MULTILEADER without text is import/display/export
//! only (decision F.2).

use super::*;
use crate::app_drawing::annot_batch::leader::{
    options_report, plan_leader_options, MAX_CREATED_LEADER_POINTS,
};
use cad_domain::{MLeaderPathType, MultiLeaderGeometry, MultiLeaderParts};

/// The multileader style `MLEADER` creation resolves by name (ADR 0019 D).
///
/// There is no style selector in the capture, so `Standard` is the one name the
/// command reads; the table always carries it (the builder pins it).
const MLEADER_STYLE_NAME: &str = "Standard";

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
    /// * every refusal [`MultiLeaderGeometry::created`] raises: non-finite or
    ///   non-`+Z`-coplanar points, a landing point no farther than the style's
    ///   dogleg length, or an unusable style size;
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
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "mleader.payload: CreateMLeader needs a MLeader { arrow, landing, text } payload"
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
            let row = drawing
                .mleader_style_by_name(MLEADER_STYLE_NAME)
                .ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "mleader.style_missing: the drawing has no MLEADERSTYLE named \
                         '{MLEADER_STYLE_NAME}'"
                    ))
                })?;
            // Store the row's own name, not the constant, so a source row with a
            // different case is not silently renamed on the entity (and in the
            // written MLEADERSTYLE record).
            let style_name = row.name.clone();
            let resolution = drawing
                .resolve_mleader_style(MLEADER_STYLE_NAME)
                .ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "mleader.style_missing: the MLEADERSTYLE '{MLEADER_STYLE_NAME}' did not \
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
        let geometry = MultiLeaderGeometry::created(
            values,
            style_name,
            resolution.left_attachment,
            *arrow,
            *landing,
            text.clone(),
        )?;
        let geometry = with_leader_path_type(geometry, options.path)?;
        let mut outcome = self.commit_new_entity(
            session,
            command,
            SemanticGeometry::MultiLeader(Box::new(geometry)),
            "create mleader",
        )?;
        outcome.diagnostics.push(options_report(options));
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
