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
//! * **content kind** — `content` selects the annotation: `CONTENT MTEXT` (the
//!   default) stores MText, an empty text creates a content-less leader, and the
//!   two non-text kinds are `CONTENT BLOCK <name>` (the annotation is an inserted
//!   block, resolved against the drawing's block table — unknown name
//!   `mleader.block_missing`, and the text field must be empty or
//!   `mleader.block_text_unexpected`) and `CONTENT TOLERANCE` (the text field is
//!   the `|`-separated feature-control frame string, drawn by the same frame rule
//!   the `TOLERANCE` command uses). All four kinds are stored, drawn and written
//!   to entity `172` / style `170`; a text-bearing kind is bounded by the shared
//!   `tool.text_too_long` rule;
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
//! * **style selector** — `style_name` resolves that `MLEADERSTYLE` row instead
//!   of the session's current style; an unknown name is the named
//!   `mleader.style_missing` refusal and writes nothing;
//! * **text angle type** — `text_angle` is the DXF **mode** (`PARALLEL` `0`,
//!   `HORIZONTAL` `1`, `OPTIMIZED` `2`), stored on the entity's `174` with the
//!   `0x2000` override bit when it differs from the style's `175`;
//! * **content-less creation** — an empty text stores `MLeaderContent::None`
//!   with no annotation (arrow, leader line and dogleg only), the real AutoCAD
//!   case; the entity content kind `172` is written verbatim and the `0x400`
//!   override bit is set only when it differs from the style's `170`.
//!
//! ## The four content kinds (they are all creatable)
//!
//! | `CONTENT` token | entity `172` | stored payload | drawn |
//! |---|---|---|---|
//! | `MTEXT` (default) | `2` | [`cad_domain::MLeaderText`] | the annotation string |
//! | (empty text) | `0` | none | arrow, leader line, dogleg |
//! | `BLOCK <name>` | `1` | [`cad_domain::MLeaderBlockContent`] | the block instance, its **base point** on the content point (`T·R·S·T(−base)`, the `INSERT` rule) |
//! | `TOLERANCE` | `3` | [`cad_domain::MLeaderToleranceContent`] | the feature-control frame |
//!
//! Block content needs a real `BLOCK_RECORD` in the output, so `SAVE` writes the
//! definition too and refuses (flattening the leader with a named reason) when
//! the definition cannot be written. Tolerance content is `172 = 3` plus the
//! `CONTEXT_DATA` text fields; acadrust 0.6.8's DXF **reader** reports `172 = 3`
//! as "no content", so a reload resolves the kind from the `MLEADERSTYLE 170`
//! record the same writer sets to `3` (see `cad-import-acadrust`'s multileader
//! module docs).
//!
//! Deliberate subset (see [`crate::CommandId::MLeader`]): the typed line has
//! exactly two coordinates, so a created leader has two vertices
//! ([`MAX_CREATED_LEADER_POINTS`]); more leader *lines* come from
//! `MLEADEREDIT ADD`, which is the AutoCAD path too — the DXF `LEADER_LINE` and
//! the domain `MLeaderLine` both carry a real vertex list, so the ceiling is an
//! **input** limit, not a model limit.

use super::*;
use crate::app_drawing::annot_batch::leader::{
    options_report, plan_leader_options, MAX_CREATED_LEADER_POINTS,
};
use crate::command_payload::MLeaderContentSpec;
use cad_domain::{
    MLeaderContentKind, MLeaderCreateContent, MLeaderCreateOptions, MLeaderPathType,
    MultiLeaderGeometry, MultiLeaderParts, MASK_CONTENT_TYPE, MASK_TEXT_ANGLE,
};
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
    /// * `tool.text_too_long` — more than [`MAX_TOOL_TEXT_CHARS`] characters (an
    ///   empty text is *not* a refusal: it creates a content-less leader);
    /// * `mleader.style_missing` — the drawing has no `MLEADERSTYLE` with the
    ///   requested name (the typed `STYLE` selector) or the session's current one
    ///   (the builder pins `Standard`, so the latter is a corrupt-database case);
    /// * the option plan's refusals: `mleader.content_unsupported` (a style row
    ///   whose `170` is outside the four known kinds), `mleader.path_type`
    ///   (invisible path), `mleader.option_arrowhead_off`,
    ///   `mleader.option_max_points`, `mleader.style_invalid`;
    /// * the content refusals: `mleader.block_missing` (unknown block name),
    ///   `mleader.block_text_unexpected` (block content plus annotation text),
    ///   `mleader.tolerance_text_required` (an empty frame) and
    ///   `mleader.tolerance_height`;
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
            style_name,
            dogleg_length,
            dogleg_angle,
            attachment,
            text_angle,
            content,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "mleader.payload: CreateMLeader needs a MLeader { arrow, landing, text, \
                 style_name, dogleg_length, dogleg_angle, attachment, text_angle, content } \
                 payload"
                    .into(),
            ));
        };
        // A block-content leader carries no annotation string: the text field is
        // only the MText value (or the tolerance frame). Refusing here keeps the
        // two from being silently dropped.
        if matches!(content, Some(MLeaderContentSpec::Block(_))) && !text.trim().is_empty() {
            return Err(CadError::InvalidInput(format!(
                "mleader.block_text_unexpected: CONTENT BLOCK creates a leader whose annotation \
                 is the inserted block, so it takes no annotation text (got {} character(s)); \
                 nothing was written",
                text.chars().count()
            )));
        }
        let characters = text.chars().count();
        if characters > MAX_TOOL_TEXT_CHARS {
            return Err(CadError::InvalidInput(format!(
                "tool.text_too_long: MLEADER allows at most {MAX_TOOL_TEXT_CHARS} characters, \
                 got {characters}"
            )));
        }
        let (row, resolution, resolved_style_name, style_requested) = {
            let document = self
                .workspace
                .documents
                .get(&command.document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = document.drawing.as_ref();
            // The typed `STYLE` selector wins over the session's current style;
            // an empty selector is treated as absent (the grammar cannot produce
            // one, a programmatic caller might).
            let style_requested = style_name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty());
            let style_name_query =
                style_requested.unwrap_or_else(|| session.current_mleader_style_name().trim());
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
            (
                row.clone(),
                resolution,
                style_name,
                style_requested.is_some(),
            )
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
            text_angle: *text_angle,
        };
        let create_content = self.resolve_mleader_content(content, text, &command.document)?;
        let geometry = MultiLeaderGeometry::created_with_content(
            values,
            resolved_style_name.clone(),
            resolution.left_attachment,
            *arrow,
            *landing,
            create_content,
            create_options,
        )?;
        // The entity content kind (`172`) overrides the style kind (`170`) only
        // when it actually differs, so a content-less or block/tolerance leader
        // under an MText style (and the reverse) reloads as what was created.
        let content_mask = if created_content_code(geometry.content().kind()) != row.content_type {
            MASK_CONTENT_TYPE
        } else {
            0
        };
        let geometry = with_created_adjustments(geometry, options.path, content_mask)?;
        let content = geometry.content().kind();
        let options_row = create_options_report(create_options, &geometry);
        let mut outcome = self.commit_new_entity(
            session,
            command,
            SemanticGeometry::MultiLeader(Box::new(geometry)),
            "create mleader",
        )?;
        outcome.diagnostics.push(options_report(options, content));
        if let Some(row) = options_row {
            outcome.diagnostics.push(row);
        }
        if style_requested {
            outcome.diagnostics.push(Diagnostic {
                object: outcome.objects.first().copied(),
                code: "mleader.style_selected".into(),
                message: format!(
                    "MLEADER: the typed STYLE selector resolved the MLEADERSTYLE \
                     '{resolved_style_name}'; every style-derived value (dogleg length, landing \
                     gap, arrowhead and text height, text style, path type, text angle type) is \
                     that row's resolved snapshot"
                ),
            });
        }
        if content == MLeaderContentKind::Block {
            outcome.diagnostics.push(Diagnostic {
                object: outcome.objects.first().copied(),
                code: "mleader.content_block".into(),
                message: format!(
                    "MLEADER: the annotation is an inserted block (content kind Block, entity \
                     172 = 1, override mask 0x{content_mask:X} against the style's kind {}) — \
                     the block is stored in CONTEXT_DATA 341/15/16/46 and entity 344/10/43/176, \
                     so a reload resolves the same block and placement",
                    row.content_type
                ),
            });
        }
        if content == MLeaderContentKind::Tolerance {
            outcome.diagnostics.push(Diagnostic {
                object: outcome.objects.first().copied(),
                code: "mleader.content_tolerance".into(),
                message: format!(
                    "MLEADER: the annotation is a feature-control frame (content kind Tolerance, \
                     entity 172 = 3, override mask 0x{content_mask:X} against the style's kind \
                     {}); the frame string and cell height are stored in the CONTEXT_DATA text \
                     fields because DXF has no separate tolerance payload",
                    row.content_type
                ),
            });
        }
        if content == MLeaderContentKind::None {
            outcome.diagnostics.push(Diagnostic {
                object: outcome.objects.first().copied(),
                code: "mleader.content_none".into(),
                message: format!(
                    "MLEADER: created without annotation text (a real content-less leader): \
                     content kind None (entity 172 = 0, override mask 0x{content_mask:X} against \
                     the style's kind {}), so the entity carries the arrow, the leader line and \
                     the dogleg and no text",
                    row.content_type
                ),
            });
        }
        if options.path == MLeaderPathType::Spline {
            outcome.diagnostics.push(Diagnostic {
                object: outcome.objects.first().copied(),
                code: "mleader.spline_two_vertices".into(),
                message: format!(
                    "MLEADER: the style's leader type is spline and it is stored as such, but a \
                     created leader has only {MAX_CREATED_LEADER_POINTS} vertices (the captured \
                     arrow and the style dogleg start), so the drawn path is the straight run \
                     between them; a bending spline needs three or more vertices, and the typed \
                     line has exactly two coordinates (more leader lines come from MLEADEREDIT \
                     ADD, the AutoCAD path)"
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
    if options.text_angle.is_some() {
        let mode = geometry.style().text_angle_type;
        rows.push(format!(
            "text angle type {} (entity 174 = {}, style 175 mirror, mask 0x{:X})",
            mode.token(),
            mode.dxf(),
            geometry.override_mask() & MASK_TEXT_ANGLE
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

/// Re-stamp the derived definition's leader path type and add the entity's
/// content-type override bit.
///
/// Both adjustments are properties `created_with_options` cannot decide alone:
/// the path type comes from the style plan (a spline style is stored, but the
/// derived definition is straight) and the content bit needs the **stored** style
/// row's content kind, which the command closes over. Everything else is the one
/// derivation, and the rebuild goes through [`MultiLeaderGeometry::new`] — the
/// same single display builder every other writer uses.
///
/// The derived display is rebuilt even when only the mask changes, so a stored
/// display can never disagree with the definition.
fn with_created_adjustments(
    geometry: MultiLeaderGeometry,
    path: MLeaderPathType,
    extra_mask: u32,
) -> CadResult<MultiLeaderGeometry> {
    let mask = geometry.override_mask() | extra_mask;
    if path == MLeaderPathType::StraightLineSegments && mask == geometry.override_mask() {
        return Ok(geometry);
    }
    let mut parts = MultiLeaderParts {
        style: geometry.style().clone(),
        style_name: geometry.style_name().to_string(),
        override_mask: mask,
        content: geometry.content().clone(),
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

/// The DXF `172` code of a created content kind, to compare with the style's
/// `170` value.
fn created_content_code(content: MLeaderContentKind) -> i16 {
    content.dxf()
}

impl Application {
    /// Resolve the typed `CONTENT` selection into the domain's creation content.
    ///
    /// * no selection / `CONTENT MTEXT` — the text argument is the annotation
    ///   (the pre-existing rule: empty creates a content-less leader);
    /// * `CONTENT BLOCK <name>` — the name is resolved against the drawing's
    ///   block table and the resolved identity, the table's own name and the
    ///   definition's **base point** travel into the definition, so the placement
    ///   is `T(content point) · R · S · T(−base point)` (the `INSERT` rule) and the
    ///   exporter can write the `BLOCK_RECORD` the content handle refers to. An
    ///   unknown name is `mleader.block_missing`, and a definition with no stored
    ///   base point is `mleader.block_metadata_missing` (never a guessed origin);
    ///   nothing is written by either;
    /// * `CONTENT TOLERANCE` — the text argument is the `|`-separated frame
    ///   string; an empty one is the domain's `mleader.tolerance_text_required`.
    pub(crate) fn resolve_mleader_content(
        &self,
        spec: &Option<MLeaderContentSpec>,
        text: &str,
        document: &crate::DocumentId,
    ) -> CadResult<MLeaderCreateContent> {
        match spec {
            None | Some(MLeaderContentSpec::MText) => {
                Ok(MLeaderCreateContent::Text(text.to_string()))
            }
            Some(MLeaderContentSpec::Tolerance) => Ok(MLeaderCreateContent::Tolerance {
                text: text.to_string(),
            }),
            Some(MLeaderContentSpec::Block(name)) => {
                let requested = name.trim();
                let document = self
                    .workspace
                    .documents
                    .get(document)
                    .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
                let drawing = document.drawing.as_ref();
                let definition = drawing.block_by_name(requested).ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "mleader.block_missing: the drawing has no block named '{requested}', so \
                         the leader has no content to place; nothing was written"
                    ))
                })?;
                let block = definition.id;
                // The placement rule is `INSERT`'s: the content point receives the
                // block's **base point**, so the metadata has to supply it. A
                // definition without one is never given an invented origin (the
                // same refusal `INSERT` makes, `insert.block_metadata_missing`).
                let metadata = drawing.block_metadata(block).ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "mleader.block_metadata_missing: block {block:?} has no stored name/base                          point, so its content cannot be placed at the landing point; nothing was                          written"
                    ))
                })?;
                Ok(MLeaderCreateContent::Block {
                    block,
                    name: metadata.name.clone(),
                    base_point: Some(metadata.base_point),
                })
            }
        }
    }
}
