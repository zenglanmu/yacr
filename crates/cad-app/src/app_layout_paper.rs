//! The 2026-11-11 paper-space write batch: `VPLAYER`, `MVSETUP`, `PSBSCALE`,
//! `PSTSCALE`.
//!
//! These four names used to be Wave 2 **named refusals** in
//! [`crate::app_extended_refusals`]. They are now real 2D paper-space features,
//! built only from APIs that already existed:
//!
//! * `VPLAYER FREEZE|THAW <layer,...> [<index>]` writes one viewport's
//!   [`cad_db::PaperViewport::layer_overrides`] (the same field `LAYVPI` writes,
//!   whose handler lives in `app_layout_layvpi`). `FREEZE` stores `true`,
//!   `THAW` removes the entry, and the global layer table is never touched. The
//!   bare form is the read-only report. The index may also be written `[n]`,
//!   which is always the index (see [`split_viewport_index`]).
//! * `MVSETUP <rows> <cols> <scale>` builds a rows × cols viewport grid on the
//!   **active paper layout** with [`grid_viewports`] and writes it through
//!   [`cad_db::DrawingDatabase::set_layout_viewports`] — one validated layout
//!   patch, one revision and one undo step. The requested scale (a `VPSCALE`
//!   name, ratio or number, resolved by [`resolve_viewport_scale`]) is applied
//!   to every cell by setting the stored model-per-paper diagonal and keeping
//!   the cell centre as the view centre. This is the same **declared simple
//!   layout** `VPORTS` produces, now with a scale; it is not an AutoCAD Standard
//!   viewport configuration and the title-block/align/options prompts of the
//!   AutoCAD wizard are NOT IMPLEMENTED.
//! * `PSBSCALE` / `PSTSCALE` scale the **selection** by the active layout
//!   viewport's paper-space factor: block references get their placement
//!   transform post-multiplied by a uniform scale about their insertion point,
//!   text gets its stored glyph height (and wrap width) multiplied. Both are one
//!   transaction / one undo step and write nothing else. The factor is **paper
//!   units per model unit** of that viewport (`0.01` at `1:100`, read from the
//!   same [`cad_representation::viewport_transform`] the display uses), so a
//!   model-sized object placed on the sheet becomes its paper-space size. A
//!   viewport that cannot be resolved exactly is refused by name rather than
//!   scaled by a guessed ratio.
//!
//! ## The "active viewport" of a paper layout
//!
//! This build has no viewport selection model. The active viewport of a layout
//! is therefore, in order: an explicit index when the command carries one, the
//! session's `VPMAX`-maximized viewport when it names this layout, else viewport
//! `0`. One rule, [`target_viewport_index`], shared by every command here, so
//! the four can never disagree about which window they act on.
//!
//! ## Typed arguments without a payload variant
//!
//! [`CommandPayload`] is closed outside this lane, so the typed argument forms
//! of `VPLAYER`/`MVSETUP` travel in [`CommandPayload::LayerWalk`] — the existing
//! `Vec<String>` token carrier — and each command parses and validates its own
//! grammar. A malformed shape is that command's `*.invalid_argument` refusal,
//! never a silent default.
//!
//! The command-line parser in `cad-ui-core` does not build those token lists
//! yet (it maps both selectors to a bare request), so a typed `VPLAYER FREEZE …`
//! line currently reaches the report form. That is a documented residual of the
//! out-of-scope parser lane, not a behaviour of this one.
//!
//! ## Work mode
//!
//! Every command here writes the drawing, so each write form refuses a Viewer
//! session with [`CadError::PermissionDenied`]. The extended-command capability
//! registry still lists these four selectors as Viewer-legal (it lives outside
//! this lane's files); the in-handler gate is the authority until that registry
//! is updated.

use super::*;

use cad_db::{DbEntity, DrawingDatabase};
use cad_history::{drawing_patch, DrawingUndoRecord};
use cad_representation::{viewport_transform, ViewportState};

use crate::app_drawing::class_name;
use crate::app_layout::{layout_viewports, require_paper_space};
use crate::app_layout_geometry::{clip_center, grid_viewports, resolve_viewport_scale};
use crate::app_utility::require_none_payload;

/// The most viewports one `MVSETUP` grid may create.
///
/// AutoCAD's `MAXACTVP` default is 64 active viewports per layout; a larger grid
/// is refused by name instead of allocating an unbounded vector.
const MVSETUP_MAX_VIEWPORTS: u32 = 64;

/// The typed argument tokens of one command (see the module docs).
pub(crate) fn argument_tokens<'a>(command: &'a Command, name: &str) -> CadResult<&'a [String]> {
    match &command.payload {
        CommandPayload::LayerWalk(tokens) => Ok(tokens.as_slice()),
        _ => Err(CadError::InvalidInput(format!(
            "{}.invalid_argument: {name} needs its typed argument tokens, and the request \
             carried a different payload",
            name.to_ascii_lowercase()
        ))),
    }
}

/// Refuse a Viewer session before a drawing write.
pub(crate) fn require_work(session: &SessionState) -> CadResult<()> {
    if session.app_mode() == AppMode::Viewer {
        Err(CadError::PermissionDenied)
    } else {
        Ok(())
    }
}

/// The viewport a paper-space write applies to (see the module docs).
///
/// Refuses `{prefix}.no_viewport` when the layout has no viewport at all and
/// `{prefix}.invalid_viewport` for an index the layout does not contain.
pub(crate) fn target_viewport_index(
    session: &SessionState,
    layout_id: LayoutId,
    explicit: Option<usize>,
    viewports: &[PaperViewport],
    prefix: &str,
) -> CadResult<usize> {
    if viewports.is_empty() {
        return Err(CadError::Unsupported(format!(
            "{prefix}.no_viewport: layout {} has no viewport to operate on",
            layout_id.0
        )));
    }
    let index = explicit.unwrap_or_else(|| {
        session
            .maximized_viewport()
            .filter(|viewport| viewport.layout == layout_id)
            .map(|viewport| viewport.index)
            .unwrap_or(0)
    });
    if index >= viewports.len() {
        return Err(CadError::InvalidInput(format!(
            "{prefix}.invalid_viewport: layout {} has {} viewport(s); index {index} is out of \
             range",
            layout_id.0,
            viewports.len()
        )));
    }
    Ok(index)
}

impl Application {
    /// `MVSETUP <rows> <cols> <scale>`: create a scaled viewport grid.
    pub(crate) fn mv_setup(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_work(session)?;
        let tokens = argument_tokens(command, "MVSETUP")?;
        let [rows, cols, scale] = tokens else {
            return Err(CadError::InvalidInput(
                "mvsetup.invalid_argument: MVSETUP needs `<rows> <cols> <scale>` (for example \
                 `MVSETUP 2 2 1:100`); the align/create/scale-viewports/options/title-block \
                 prompts of the AutoCAD wizard are NOT IMPLEMENTED"
                    .into(),
            ));
        };
        let rows = parse_grid_count(rows, "rows")?;
        let cols = parse_grid_count(cols, "columns")?;
        let total = rows.checked_mul(cols).ok_or_else(|| {
            CadError::InvalidInput("mvsetup.invalid_grid: MVSETUP rows x columns overflows".into())
        })?;
        if total > MVSETUP_MAX_VIEWPORTS {
            return Err(CadError::InvalidInput(format!(
                "mvsetup.invalid_grid: MVSETUP would create {total} viewports; at most \
                 {MVSETUP_MAX_VIEWPORTS} (AutoCAD's MAXACTVP default) are supported"
            )));
        }
        let layout_id = require_paper_space(session, "mvsetup")?;
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        let (scale_name, factor) = resolve_viewport_scale(&drawing, scale).map_err(|error| {
            CadError::InvalidInput(format!(
                "mvsetup.invalid_scale: MVSETUP could not resolve the scale {:?}: {}",
                scale.trim(),
                error_detail(error)
            ))
        })?;
        if !factor.is_finite() || factor <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "mvsetup.invalid_scale: resolved scale {:?} has the non-finite or non-positive \
                 factor {factor}",
                scale_name
            )));
        }
        // `factor` is paper/drawing, so the stored paper→model diagonal is its
        // reciprocal (model units per paper unit) — the same convention
        // `VPSCALE` writes.
        let model_per_paper = 1.0 / factor;
        if !model_per_paper.is_finite() || model_per_paper <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "mvsetup.invalid_scale: scale {:?} gives the unusable model-per-paper factor \
                 {model_per_paper}",
                scale_name
            )));
        }
        let mut viewports = grid_viewports(&drawing, layout_id, rows, cols, total);
        for viewport in &mut viewports {
            let centre = clip_center(&viewport.clip).ok_or_else(|| {
                CadError::Invariant(format!(
                    "MVSETUP: the generated grid cell of layout {} has no finite paper rectangle",
                    layout_id.0
                ))
            })?;
            // Keep the model origin at the cell centre: translation =
            // -scale * paper_centre (the `grid_viewports` convention with a
            // non-unit scale).
            viewport.model_to_paper.matrix[0][0] = model_per_paper;
            viewport.model_to_paper.matrix[1][1] = model_per_paper;
            viewport.model_to_paper.matrix[0][3] = -model_per_paper * centre.x;
            viewport.model_to_paper.matrix[1][3] = -model_per_paper * centre.y;
        }
        let before = layout_row(&drawing, layout_id);
        let transaction = next_transaction_id();
        let mut candidate = drawing;
        let change_set =
            candidate.set_layout_viewports(layout_id, viewports, transaction, "mvsetup")?;
        let after = layout_row(&candidate, layout_id);
        self.publish_layout_record(
            document_id,
            candidate,
            transaction,
            "mvsetup",
            vec![layout_patch(layout_id, before, after)],
        )?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "mvsetup.applied".into(),
                message: format!(
                    "MVSETUP: created a {rows} x {cols} grid of {total} viewport(s) in layout {} \
                     at scale {:?} (stored model units per paper unit = {model_per_paper}); the \
                     cells are equal and cover the sheet from its origin, and this is not an \
                     AutoCAD Standard viewport configuration",
                    layout_id.0, scale_name
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `PSBSCALE`: scale the selected paper-space block references.
    pub(crate) fn psb_scale(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_work(session)?;
        require_none_payload(command, "PSBSCALE")?;
        let layout_id = require_paper_space(session, "psbscale")?;
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        let (index, factor) = paper_factor(&drawing, session, layout_id, "psbscale")?;
        let sources = paper_selection(&drawing, session, command, layout_id, "psbscale")?;
        let mut planned = Vec::with_capacity(sources.len());
        for before in sources {
            let SemanticGeometry::Insert { block, transform } = &before.geometry else {
                return Err(CadError::Unsupported(format!(
                    "psbscale.not_block: PSBSCALE scales block references only; entity {} is {}",
                    before.id.0,
                    class_name(&before.geometry)
                )));
            };
            // Post-multiplying by a uniform scale keeps the insertion point
            // (the transform's translation) fixed and scales the block's local
            // extents — "scale the reference about its insertion point".
            let mut after = before.clone();
            after.geometry = SemanticGeometry::Insert {
                block: *block,
                transform: transform.matrix_mul(&Transform3::scale(factor)),
            };
            if after.geometry != before.geometry {
                planned.push((before, after));
            }
        }
        if planned.is_empty() {
            return Ok(unchanged_outcome(
                "psbscale.unchanged",
                format!(
                    "PSBSCALE: the paper-space factor of layout {} viewport {index} is {factor}; \
                     no selected block reference would change and nothing was written",
                    layout_id.0
                ),
            ));
        }
        let count = planned.len();
        let (objects, change_set) =
            commit_entity_updates(self, document_id, drawing, &planned, "psbscale")?;
        Ok(CommandOutcome {
            objects,
            changes: Some(change_set),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "psbscale.applied".into(),
                message: format!(
                    "PSBSCALE: scaled {count} paper-space block reference(s) by the paper/model \
                     factor {factor} of layout {} viewport {index} (insertion points unchanged)",
                    layout_id.0
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `PSTSCALE`: scale the selected paper-space text heights.
    pub(crate) fn pst_scale(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_work(session)?;
        require_none_payload(command, "PSTSCALE")?;
        let layout_id = require_paper_space(session, "pstscale")?;
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        let (index, factor) = paper_factor(&drawing, session, layout_id, "pstscale")?;
        let sources = paper_selection(&drawing, session, command, layout_id, "pstscale")?;
        let mut planned = Vec::with_capacity(sources.len());
        for before in sources {
            let SemanticGeometry::Text {
                text,
                position,
                style,
                height,
                rotation,
                font,
                h_align,
                v_align,
                wrap_width,
            } = before.geometry.clone()
            else {
                return Err(CadError::Unsupported(format!(
                    "pstscale.not_text: PSTSCALE scales Text and MText only; entity {} is {}",
                    before.id.0,
                    class_name(&before.geometry)
                )));
            };
            if !height.is_finite() || height <= 0.0 {
                return Err(CadError::InvalidInput(format!(
                    "pstscale.degenerate: the selected text entity {} has the degenerate stored \
                     height {height}",
                    before.id.0
                )));
            }
            if let Some(width) = wrap_width {
                if !width.is_finite() || width <= 0.0 {
                    return Err(CadError::InvalidInput(format!(
                        "pstscale.degenerate: the selected text entity {} has the degenerate \
                         stored wrap width {width}",
                        before.id.0
                    )));
                }
            }
            let new_height = height * factor;
            if !new_height.is_finite() || new_height <= 0.0 {
                return Err(CadError::InvalidInput(format!(
                    "pstscale.invalid_result: the height {height} x {factor} = {new_height} is \
                     not a usable dimension"
                )));
            }
            let new_width = match wrap_width {
                Some(width) => {
                    let scaled = width * factor;
                    if !scaled.is_finite() || scaled <= 0.0 {
                        return Err(CadError::InvalidInput(format!(
                            "pstscale.invalid_result: the wrap width {width} x {factor} = \
                             {scaled} is not a usable dimension"
                        )));
                    }
                    Some(scaled)
                }
                None => None,
            };
            let mut after = before.clone();
            after.geometry = SemanticGeometry::Text {
                text,
                position,
                style,
                height: new_height,
                rotation,
                font,
                h_align,
                v_align,
                wrap_width: new_width,
            };
            if after.geometry != before.geometry {
                planned.push((before, after));
            }
        }
        if planned.is_empty() {
            return Ok(unchanged_outcome(
                "pstscale.unchanged",
                format!(
                    "PSTSCALE: the paper-space factor of layout {} viewport {index} is {factor}; \
                     no selected text would change and nothing was written",
                    layout_id.0
                ),
            ));
        }
        let count = planned.len();
        let (objects, change_set) =
            commit_entity_updates(self, document_id, drawing, &planned, "pstscale")?;
        Ok(CommandOutcome {
            objects,
            changes: Some(change_set),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "pstscale.applied".into(),
                message: format!(
                    "PSTSCALE: scaled the height of {count} paper-space text entity/entities by \
                     the paper/model factor {factor} of layout {} viewport {index}",
                    layout_id.0
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Parse one `MVSETUP` grid count: a positive integer, never a silent clamp.
fn parse_grid_count(token: &str, label: &str) -> CadResult<u32> {
    let value = token.trim().parse::<u32>().map_err(|_| {
        CadError::InvalidInput(format!(
            "mvsetup.invalid_grid: {:?} is not a whole {label} count",
            token.trim()
        ))
    })?;
    if value == 0 {
        return Err(CadError::InvalidInput(format!(
            "mvsetup.invalid_grid: MVSETUP needs {label} >= 1"
        )));
    }
    Ok(value)
}

/// The active viewport index and its **paper units per model unit** factor.
///
/// `paper_per_model` is the direction the two paper-space scale commands need:
/// the drawing scale `1:100` means one paper unit is 100 model units, so a
/// model-space-sized quantity becomes `0.01` of it on the sheet. It is read from
/// the same [`cad_representation::viewport_transform`] the display uses, so a
/// viewport this build cannot resolve exactly is refused by name rather than
/// guessed at.
fn paper_factor(
    drawing: &DrawingDatabase,
    session: &SessionState,
    layout_id: LayoutId,
    prefix: &str,
) -> CadResult<(usize, f64)> {
    let viewports = layout_viewports(drawing, layout_id)?;
    let index = target_viewport_index(session, layout_id, None, &viewports, prefix)?;
    match viewport_transform(&viewports[index]) {
        ViewportState::Supported(transform) => {
            let factor = transform.paper_per_model;
            if !factor.is_finite() || factor <= 0.0 {
                return Err(CadError::InvalidInput(format!(
                    "{prefix}.invalid_viewport: layout {} viewport {index} has the unusable \
                     paper/model factor {factor}",
                    layout_id.0
                )));
            }
            Ok((index, factor))
        }
        ViewportState::Unsupported(reason) => Err(CadError::Unsupported(format!(
            "{prefix}.unsupported_viewport: layout {} viewport {index} cannot be resolved: \
             {reason}",
            layout_id.0
        ))),
    }
}

/// Resolve the current selection to directly addressable entities of `layout_id`.
///
/// Model-space targets are refused by name (`{prefix}.model_space`): these two
/// commands scale **paper-space** objects relative to the sheet, so an entity on
/// the sheet is the only meaningful target.
fn paper_selection(
    drawing: &DrawingDatabase,
    session: &SessionState,
    command: &Command,
    layout_id: LayoutId,
    prefix: &str,
) -> CadResult<Vec<DbEntity>> {
    let references = session.selection.refs().to_vec();
    if references.is_empty() {
        return Err(CadError::InvalidInput(format!(
            "{prefix}.needs_selection: {prefix} needs at least one selected paper-space entity"
        )));
    }
    let mut entities = Vec::with_capacity(references.len());
    let mut seen = std::collections::BTreeSet::new();
    for reference in &references {
        if reference.document != command.document {
            return Err(CadError::InvalidInput(format!(
                "{prefix}.not_addressable: target {} belongs to another document",
                reference.entity.0
            )));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(CadError::Unsupported(format!(
                "{prefix}.not_addressable: target {} is a block-instance member or sub-element; \
                 only directly addressable entities are supported",
                reference.entity.0
            )));
        }
        let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
            CadError::InvalidInput(format!(
                "{prefix}.target_missing: target {} does not exist",
                reference.entity.0
            ))
        })?;
        match &entity.space {
            SpaceId::Paper(id) if *id == layout_id => {}
            SpaceId::Model => {
                return Err(CadError::Unsupported(format!(
                    "{prefix}.model_space: target {} is in model space; {prefix} scales \
                     paper-space objects of layout {} only",
                    entity.id.0, layout_id.0
                )))
            }
            other => {
                return Err(CadError::Unsupported(format!(
                    "{prefix}.not_addressable: target {} is not in layout {} ({other:?})",
                    entity.id.0, layout_id.0
                )))
            }
        }
        if seen.insert(entity.id) {
            entities.push(entity);
        }
    }
    Ok(entities)
}

/// Commit every planned entity rewrite in one transaction / one undo step.
fn commit_entity_updates(
    app: &mut Application,
    document_id: DocumentId,
    mut candidate: DrawingDatabase,
    planned: &[(DbEntity, DbEntity)],
    label: &str,
) -> CadResult<(Vec<ObjectId>, Arc<ChangeSet>)> {
    let transaction = next_transaction_id();
    let change_set = {
        let mut staged = candidate.begin_drawing_transaction(label, transaction)?;
        for (_, after) in planned {
            staged.update_entity(after.clone())?;
        }
        staged.commit()?
    };
    let mut patches = Vec::with_capacity(planned.len());
    let mut objects = Vec::with_capacity(planned.len());
    for (before, _) in planned {
        let after = candidate.entity(before.id).cloned().ok_or_else(|| {
            CadError::Invariant(format!(
                "{label}: an updated entity vanished after the commit"
            ))
        })?;
        patches.push(drawing_patch(before.id, Some(before.clone()), Some(after)));
        objects.push(ObjectId(before.id.0));
    }
    app.publish_candidate(
        document_id,
        candidate,
        DrawingUndoRecord {
            transaction: change_set.transaction,
            label: label.into(),
            patches,
            block_patches: Vec::new(),
            layer_patches: Vec::new(),
            layout_patches: Vec::new(),
            merge_key: None,
        },
    )?;
    Ok((objects, Arc::new(change_set)))
}

/// The named no-op outcome: no transaction, no `ChangeSet`, no undo entry.
pub(crate) fn unchanged_outcome(code: &str, message: String) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: code.into(),
            message,
        }],
        measurement: None,
        point_report: None,
    }
}

/// The message of one refusal, without its `CadError` wrapper.
fn error_detail(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
        other => format!("{other:?}"),
    }
}

/// The two `VPLAYER` operations this build models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VPlayerAction {
    Freeze,
    Thaw,
}

impl VPlayerAction {
    /// Parse the typed operation word; anything else is a named refusal.
    fn parse(token: &str) -> CadResult<Self> {
        let token = token.trim();
        if token.eq_ignore_ascii_case("FREEZE") || token.eq_ignore_ascii_case("F") {
            Ok(Self::Freeze)
        } else if token.eq_ignore_ascii_case("THAW") || token.eq_ignore_ascii_case("T") {
            Ok(Self::Thaw)
        } else {
            Err(CadError::InvalidInput(format!(
                "vplayer.invalid_argument: {:?} is not a VPLAYER operation; this build supports \
                 `VPLAYER FREEZE <layer,...> [<viewport-index>]` and `VPLAYER THAW <layer,...> \
                 [<viewport-index>]`",
                token
            )))
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Freeze => "FREEZE",
            Self::Thaw => "THAW",
        }
    }
}

impl Application {
    /// `VPLAYER FREEZE|THAW <layer,...> [<viewport-index>]`, or the bare report.
    ///
    /// The write lands in [`cad_db::PaperViewport::layer_overrides`] of **one**
    /// viewport (the explicit index, else the active viewport — see
    /// [`target_viewport_index`]) as one validated `set_layout_viewports` call:
    /// one revision, one `LayoutPatch`, one undo step. The global layer table is
    /// never touched, so this can never turn a layer off for the whole drawing.
    ///
    /// `FREEZE` stores `true`; `THAW` removes the override (the documented
    /// "not overridden in this viewport" state, never `false`, so a report can
    /// list exactly the frozen layers).
    ///
    /// The bare form (`CommandPayload::None`) is the read-only report of the
    /// stored overrides, which is Viewer-legal.
    pub(crate) fn vplayer(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let layout_id = require_paper_space(session, "vplayer")?;
        if matches!(command.payload, CommandPayload::None) {
            return self.vplayer_report(session, command, layout_id);
        }
        require_work(session)?;
        let tokens = argument_tokens(command, "VPLAYER")?;
        let Some((operation, rest)) = tokens.split_first() else {
            return Err(CadError::InvalidInput(
                "vplayer.invalid_argument: VPLAYER needs `FREEZE|THAW <layer,...> [<index>]` (or \
                 no argument at all for the read-only report)"
                    .into(),
            ));
        };
        let operation = VPlayerAction::parse(operation)?;
        if rest.is_empty() {
            return Err(CadError::InvalidInput(format!(
                "vplayer.invalid_argument: VPLAYER {} needs at least one layer name",
                operation.word()
            )));
        }
        let document_id = command.document;
        let drawing = self.drawing(document_id)?.clone();
        // The optional viewport index is either an explicit `[n]` token (always
        // an index, so a numeric *layer* name stays addressable) or a trailing
        // bare whole number that names no layer. Layer names are resolved
        // against the drawing first, so `FREEZE 2` freezes the layer named "2".
        let (layer_tokens, explicit) = split_viewport_index(&drawing, rest)?;
        if layer_tokens.is_empty() {
            return Err(CadError::InvalidInput(
                "vplayer.invalid_argument: VPLAYER needs at least one layer name before the \
                 viewport index"
                    .into(),
            ));
        }
        // Resolve every name first: an unknown or ambiguous name refuses before
        // anything mutates. Repeated names collapse to one override.
        let mut layers: Vec<(LayerId, String)> = Vec::with_capacity(layer_tokens.len());
        for name in layer_tokens {
            let id = resolve_vplayer_layer(&drawing, name)?;
            if layers.iter().any(|(known, _)| *known == id) {
                continue;
            }
            let canonical = drawing
                .layers()
                .find(|layer| layer.id == id)
                .map(|layer| layer.name.clone())
                .unwrap_or_else(|| name.clone());
            layers.push((id, canonical));
        }
        let mut viewports = layout_viewports(&drawing, layout_id)?;
        let index = target_viewport_index(session, layout_id, explicit, &viewports, "vplayer")?;
        let mut changed = false;
        {
            let viewport = &mut viewports[index];
            for (id, _) in &layers {
                match operation {
                    VPlayerAction::Freeze => {
                        if viewport.layer_overrides.get(id) != Some(&true) {
                            viewport.layer_overrides.insert(*id, true);
                            changed = true;
                        }
                    }
                    VPlayerAction::Thaw => {
                        if viewport.layer_overrides.remove(id).is_some() {
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            return Ok(unchanged_outcome(
                "vplayer.unchanged",
                format!(
                    "VPLAYER {}: viewport {index} of layout {} already stores the requested \
                     override(s); no transaction was opened",
                    operation.word(),
                    layout_id.0
                ),
            ));
        }
        let names = layers
            .iter()
            .map(|(_, name)| format!("{name:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let before = layout_row(&drawing, layout_id);
        let transaction = next_transaction_id();
        let mut candidate = drawing;
        let change_set =
            candidate.set_layout_viewports(layout_id, viewports, transaction, "vplayer")?;
        let after = layout_row(&candidate, layout_id);
        self.publish_layout_record(
            document_id,
            candidate,
            transaction,
            "vplayer",
            vec![layout_patch(layout_id, before, after)],
        )?;
        let (code, verb) = match operation {
            VPlayerAction::Freeze => ("vplayer.frozen", "froze"),
            VPlayerAction::Thaw => ("vplayer.thawed", "thawed"),
        };
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: code.into(),
                message: format!(
                    "VPLAYER: {verb} layer(s) {names} in viewport {index} of layout {} \
                     (per-viewport state; the global layer table is unchanged)",
                    layout_id.0
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// The bare `VPLAYER` report: every viewport's stored layer overrides.
    fn vplayer_report(
        &self,
        session: &SessionState,
        command: &Command,
        layout_id: LayoutId,
    ) -> CadResult<CommandOutcome> {
        let drawing = self.drawing(command.document)?;
        let viewports = layout_viewports(drawing, layout_id)?;
        if viewports.is_empty() {
            return Ok(unchanged_outcome(
                "vplayer.report.empty",
                format!(
                    "VPLAYER: layout {} has no viewport, so there is no per-viewport layer state \
                     to report",
                    layout_id.0
                ),
            ));
        }
        let active = target_viewport_index(session, layout_id, None, &viewports, "vplayer")?;
        let mut diagnostics = Vec::new();
        for (index, viewport) in viewports.iter().enumerate() {
            let frozen: Vec<String> = drawing
                .layers()
                .filter(|layer| viewport.layer_overrides.get(&layer.id) == Some(&true))
                .map(|layer| layer.name.clone())
                .collect();
            diagnostics.push(Diagnostic {
                object: None,
                code: "vplayer.report".into(),
                message: if frozen.is_empty() {
                    format!(
                        "VPLAYER: viewport {index} of layout {} has no layer overrides",
                        layout_id.0
                    )
                } else {
                    format!(
                        "VPLAYER: viewport {index} of layout {} freezes: {}",
                        layout_id.0,
                        frozen.join(", ")
                    )
                },
            });
        }
        diagnostics.push(Diagnostic {
            object: None,
            code: "vplayer.report.active".into(),
            message: format!(
                "VPLAYER: the active viewport of layout {} is index {active} (explicit index, \
                 else the VPMAX-maximized viewport, else 0)",
                layout_id.0
            ),
        });
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

/// Split the optional `VPLAYER` viewport index off the layer tokens.
///
/// Two unambiguous spellings are accepted, and nothing else:
///
/// * a trailing `[n]` token — **always** the viewport index, so a layer whose
///   name is a number stays addressable (`FREEZE WALLS [2]`);
/// * a trailing bare whole number that resolves to **no** layer — the
///   convenience form (`FREEZE WALLS 2`). Because layer names are resolved
///   first, `FREEZE 2` freezes the layer named "2" and never viewport 2.
fn split_viewport_index<'a>(
    drawing: &DrawingDatabase,
    rest: &'a [String],
) -> CadResult<(&'a [String], Option<usize>)> {
    let Some((last, head)) = rest.split_last() else {
        return Ok((rest, None));
    };
    let trimmed = last.trim();
    if let Some(inner) = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
        if head.is_empty() {
            return Err(CadError::InvalidInput(
                "vplayer.invalid_argument: the viewport index needs at least one layer name \
                 before it"
                    .into(),
            ));
        }
        let index = inner.trim().parse::<usize>().map_err(|_| {
            CadError::InvalidInput(format!(
                "vplayer.invalid_viewport: {:?} is not a viewport index",
                inner.trim()
            ))
        })?;
        return Ok((head, Some(index)));
    }
    if head.is_empty() || name_is_a_layer(drawing, trimmed) {
        return Ok((rest, None));
    }
    match trimmed.parse::<usize>() {
        Ok(index) => Ok((head, Some(index))),
        Err(_) => Ok((rest, None)),
    }
}

/// Whether `name` resolves to a layer of the drawing (exact, else any
/// case-insensitive match).
fn name_is_a_layer(drawing: &DrawingDatabase, name: &str) -> bool {
    drawing.layer_id_by_name(name).is_some()
        || drawing
            .layers()
            .any(|layer| layer.name.eq_ignore_ascii_case(name))
}

/// Resolve one `VPLAYER` layer name with the `vplayer.*` refusal namespace.
///
/// The lookup rule is `LAYVPI`'s (`app_layout_names::resolve_layer_name`): exact
/// name first, then a unique ASCII case-insensitive match. It is restated here
/// only because that helper carries `layvpi.*` in its messages; the rule itself
/// is not duplicated by intent.
fn resolve_vplayer_layer(drawing: &DrawingDatabase, name: &str) -> CadResult<LayerId> {
    if let Some(id) = drawing.layer_id_by_name(name) {
        return Ok(id);
    }
    let matches: Vec<LayerId> = drawing
        .layers()
        .filter(|layer| layer.name.eq_ignore_ascii_case(name))
        .map(|layer| layer.id)
        .collect();
    match matches.as_slice() {
        [id] => Ok(*id),
        [] => Err(CadError::InvalidInput(format!(
            "vplayer.unknown_layer: no layer named {:?}",
            name
        ))),
        _ => Err(CadError::InvalidInput(format!(
            "vplayer.ambiguous_layer: more than one layer matches {:?} case-insensitively",
            name
        ))),
    }
}
