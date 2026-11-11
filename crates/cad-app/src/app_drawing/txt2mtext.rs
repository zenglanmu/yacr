//! `TXT2MTXT` — combine selected TEXT/MTEXT into one or more MTEXT objects.
//!
//! ## Settings
//!
//! * **Combine into a single mtext object** (default on): one `AcDbMText` holds
//!   every source, joined with raw `\P` paragraph breaks. The first source in
//!   the chosen order supplies position/style/font/height/rotation/alignment,
//!   layer, draw order and (when word-wrap is on) `wrap_width`. A source whose
//!   stored height differs from that first-source base height keeps its height
//!   through the MTEXT per-run height code (`{\H<factor>x;<text>}`), which
//!   `cad-representation`'s MTEXT parser applies to that run only — the text
//!   string itself is never re-processed. A source whose string has unbalanced
//!   `{`/`}` cannot be wrapped in a group without changing how the markup is
//!   framed, so the override is skipped for that source, the raw string is
//!   emitted and `txt2mtxt.height_override_skipped` names it. Per-source
//!   **font** differences still cannot survive one run and are not claimed.
//! * **Combine off** (`one per source`): every source becomes one new
//!   `AcDbMText` at its own position, carrying its own style/font/height/
//!   rotation/alignment/layer/draw order. This is the declared non-combined
//!   rule; an MTEXT source is re-created as MTEXT, not left with a stale id.
//! * **Text ordering**: `TopDown` sorts descending Y then ascending X, stably;
//!   `Selection` keeps the session selection order exactly.
//! * **Word-wrap**: uses the existing `wrap_width` field. On keeps a stored
//!   boundary instead of dropping it: the combined object takes the **first
//!   stored** `Some(wrap_width)` in the chosen order (a plain `AcDbText` source
//!   has no boundary, so a later MTEXT's declared width is not lost), and
//!   `None` when no source declares one — no width is invented. Off stores
//!   `None`.
//! * **Force uniform line spacing**: AutoCAD stores an MTEXT line-spacing
//!   factor; this build's `SemanticGeometry::Text` has no such field and MTEXT
//!   `\P` is only a paragraph break, so the setting is a named
//!   `txt2mtxt.uniform_line_spacing` `Unsupported` refusal before any write.
//!
//! ## Write discipline
//!
//! Every source is deleted and every new MTEXT inserted in **one** transaction /
//! one undo step. The viewer permission check is unchanged. Refusals happen
//! before the transaction and write nothing; an empty selection is
//! `txt2mtxt.needs_selection`, a non-text source is `txt2mtxt.unsupported`, and a
//! block-instance/sub-element ref is `txt2mtxt.not_addressable`.

use super::*;
use crate::command_payload::{TextOrder, Txt2MTextOptions, UtilityPayload};

/// The `TXT2MTXT` settings carried by this command, or the documented defaults
/// for the bare selection form.
fn options(command: &Command) -> CadResult<Txt2MTextOptions> {
    match utility_payload(command) {
        Some(UtilityPayload::Txt2MText(options)) => Ok(*options),
        Some(_) => Err(CadError::InvalidInput(
            "txt2mtxt.invalid_payload: TXT2MTXT does not accept another utility command's payload"
                .into(),
        )),
        None => match &command.payload {
            CommandPayload::None => Ok(Txt2MTextOptions::default()),
            _ => Err(CadError::InvalidInput(
                "txt2mtxt.invalid_payload: TXT2MTXT takes no payload or a Txt2MText utility \
                 payload; the selection is the input"
                    .into(),
            )),
        },
    }
}

/// One planned replacement: every source is deleted and `replacements` inserted.
struct Plan {
    sources: Vec<DbEntity>,
    replacements: Vec<DbEntity>,
    /// Sources whose stored height differs from the combined base height but
    /// whose string could not be wrapped in a `{\H…x;…}` group (unbalanced
    /// `{`/`}`), so the raw string was emitted and the override is reported.
    height_override_skipped: Vec<EntityId>,
}

impl Application {
    /// `TXT2MTXT`: combine the selected Text/MText entities according to the
    /// settings payload.
    pub(crate) fn txt2mtext(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let settings = options(command)?;
        if settings.force_uniform_line_spacing {
            return Err(CadError::Unsupported(
                "txt2mtxt.uniform_line_spacing: MTEXT line-spacing is not modelled in this build \
                 (SemanticGeometry::Text has no line-spacing field; \\P is a paragraph break, not \
                 a 50%-larger paragraph spacing); the setting was not applied"
                    .to_string(),
            ));
        }
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(CadError::InvalidInput(
                "txt2mtxt.needs_selection: TXT2MTXT needs at least one selected Text/MText entity"
                    .into(),
            ));
        }
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref();
        let mut sources: Vec<DbEntity> = Vec::with_capacity(references.len());
        let mut seen = std::collections::BTreeSet::new();
        for reference in &references {
            if reference.document != document_id {
                return Err(CadError::InvalidInput(format!(
                    "txt2mtxt.not_addressable: TXT2MTXT target {} belongs to another document",
                    reference.entity.0
                )));
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(CadError::InvalidInput(format!(
                    "txt2mtxt.not_addressable: TXT2MTXT target {} is a block-instance member or \
                     sub-element; only directly addressable model-space text entities can be \
                     combined",
                    reference.entity.0
                )));
            }
            let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "txt2mtxt.target_missing: TXT2MTXT target {} does not exist",
                    reference.entity.0
                ))
            })?;
            if entity.space != SpaceId::Model {
                return Err(CadError::Unsupported(format!(
                    "txt2mtxt.unsupported: entity {} is not model space; v1 combines model-space \
                     text only",
                    entity.id.0
                )));
            }
            if !matches!(entity.geometry, SemanticGeometry::Text { .. }) {
                return Err(CadError::Unsupported(format!(
                    "txt2mtxt.unsupported: {} is not a Text/MText entity; a mixed or invalid \
                     selection is refused as a whole and no source is converted",
                    class_name(&entity.geometry)
                )));
            }
            if seen.insert(entity.id) {
                sources.push(entity);
            }
        }
        if settings.order == TextOrder::TopDown {
            // Top-to-bottom (descending Y), then left-to-right (ascending X). The
            // sort is stable, so equal anchors keep the selection order.
            sources.sort_by(|first, second| {
                let (
                    SemanticGeometry::Text { position: a, .. },
                    SemanticGeometry::Text { position: b, .. },
                ) = (&first.geometry, &second.geometry)
                else {
                    return std::cmp::Ordering::Equal;
                };
                b.y.total_cmp(&a.y).then_with(|| a.x.total_cmp(&b.x))
            });
        }
        let mut candidate = drawing.clone();
        let plan = plan_replacements(&mut candidate, &sources, &settings)?;
        let display_states: Vec<_> = plan
            .sources
            .iter()
            .map(|source| candidate.entity_display_state(source.id))
            .collect();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("combine text", transaction)?;
            for source in &plan.sources {
                staged.delete_entity(source.id)?;
            }
            for replacement in &plan.replacements {
                staged.insert_entity(replacement.clone())?;
            }
            if settings.combine {
                // Keep the first ordered source's render attributes (colour,
                // linetype, opacity) on the single replacement.
                if let (Some(state), Some(replacement)) =
                    (display_states.first(), plan.replacements.first())
                {
                    staged.update_entity_display_state(replacement.id, state)?;
                }
            } else {
                // One replacement per source, in the same order.
                for (state, replacement) in display_states.iter().zip(&plan.replacements) {
                    staged.update_entity_display_state(replacement.id, state)?;
                }
            }
            staged.commit()?
        };
        let mut patches: Vec<DrawingPatch> = plan
            .sources
            .iter()
            .zip(display_states.iter())
            .map(|(source, state)| drawing_patch_deleted(source.id, source.clone(), state.clone()))
            .collect();
        let mut objects = Vec::with_capacity(plan.replacements.len());
        for (index, replacement) in plan.replacements.iter().enumerate() {
            let stored = candidate.entity(replacement.id).cloned().ok_or_else(|| {
                CadError::Invariant("TXT2MTXT: a replacement vanished after commit".into())
            })?;
            let state = if settings.combine {
                display_states.first().cloned()
            } else {
                display_states.get(index).cloned()
            };
            patches.push(DrawingPatch {
                id: replacement.id,
                before: None,
                after: Some(stored),
                before_state: None,
                after_state: state,
            });
            objects.push(ObjectId(replacement.id.0));
        }
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "combine text".into(),
                patches,
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        let count = plan.sources.len();
        let made = objects.len();
        let mut diagnostics = vec![Diagnostic {
            object: None,
            code: "drawing.txt2mtxt".into(),
            message: format!(
                "TXT2MTXT: replaced {count} source(s) with {made} AcDbMText object(s); \
                 combine = {}, order = {}, word-wrap = {}",
                settings.combine,
                if settings.order == TextOrder::TopDown {
                    "top-down"
                } else {
                    "selection"
                },
                settings.word_wrap
            ),
        }];
        if !plan.height_override_skipped.is_empty() {
            let named = plan
                .height_override_skipped
                .iter()
                .map(|id| id.0.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            diagnostics.push(Diagnostic {
                object: None,
                code: "txt2mtxt.height_override_skipped".into(),
                message: format!(
                    "TXT2MTXT: source(s) {named} store a height different from the combined base \
                     height but their string has unbalanced '{{'/'}}', so the per-run \\H override \
                     was not emitted; the raw string and the base height were kept"
                ),
            });
        }
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

/// Build the replacement entities for `sources`.
fn plan_replacements(
    candidate: &mut DrawingDatabase,
    sources: &[DbEntity],
    settings: &Txt2MTextOptions,
) -> CadResult<Plan> {
    let first = sources
        .first()
        .ok_or_else(|| CadError::Invariant("TXT2MTXT lost its source set".into()))?;
    let mut replacements = Vec::new();
    let mut height_override_skipped = Vec::new();
    if settings.combine {
        let SemanticGeometry::Text {
            position,
            style,
            height,
            rotation,
            font,
            h_align,
            v_align,
            ..
        } = &first.geometry
        else {
            return Err(CadError::Invariant(
                "TXT2MTXT first source is not Text".into(),
            ));
        };
        let mut joined = String::new();
        for (index, source) in sources.iter().enumerate() {
            let SemanticGeometry::Text {
                text,
                height: source_height,
                ..
            } = &source.geometry
            else {
                return Err(CadError::Invariant("TXT2MTXT source is not Text".into()));
            };
            if index > 0 {
                joined.push_str("\\P");
            }
            // A differing stored height survives as an MTEXT per-run height
            // code around that source's own text; the raw string is never
            // rewritten. Wrapping needs balanced `{`/`}` in the source string,
            // otherwise the group would re-frame the following markup.
            match height_override_code(*source_height, *height) {
                Some(code) if balanced_braces(text) => {
                    joined.push('{');
                    joined.push_str(&code);
                    joined.push_str(text);
                    joined.push('}');
                }
                Some(_) => {
                    height_override_skipped.push(source.id);
                    joined.push_str(text);
                }
                None => joined.push_str(text),
            }
        }
        let geometry = SemanticGeometry::Text {
            text: joined,
            position: *position,
            style: *style,
            height: *height,
            rotation: *rotation,
            font: font.clone(),
            h_align: *h_align,
            v_align: *v_align,
            wrap_width: if settings.word_wrap {
                first_stored_wrap_width(sources)
            } else {
                None
            },
        };
        replacements.push(mtext_entity(candidate, first, geometry));
    } else {
        for source in sources {
            let mut geometry = source.geometry.clone();
            if let SemanticGeometry::Text { wrap_width, .. } = &mut geometry {
                if !settings.word_wrap {
                    *wrap_width = None;
                }
            }
            replacements.push(mtext_entity(candidate, source, geometry));
        }
    }
    Ok(Plan {
        sources: sources.to_vec(),
        replacements,
        height_override_skipped,
    })
}

/// The **first stored** wrap boundary in the ordered sources.
///
/// A plain `AcDbText` source has no boundary (`None`), so taking the first
/// `Some` keeps a declared MTEXT width instead of dropping it; when no source
/// declares one the result is `None` — no width is invented.
fn first_stored_wrap_width(sources: &[DbEntity]) -> Option<f64> {
    sources.iter().find_map(|source| match &source.geometry {
        SemanticGeometry::Text { wrap_width, .. } => *wrap_width,
        _ => None,
    })
}

/// The MTEXT relative height code (`\H<factor>x;`) that reproduces
/// `source_height` on top of the combined base `base_height`.
///
/// `None` when the two heights already match (within a relative `1e-9`), or
/// when either is not a usable positive finite number — the override is then
/// simply absent, never a fabricated value.
fn height_override_code(source_height: f64, base_height: f64) -> Option<String> {
    if !source_height.is_finite() || !base_height.is_finite() || base_height <= 0.0 {
        return None;
    }
    let factor = source_height / base_height;
    if !factor.is_finite() || factor <= 0.0 || (factor - 1.0).abs() <= 1e-9 {
        return None;
    }
    let mut number = format!("{factor:.6}");
    while number.ends_with('0') {
        number.pop();
    }
    if number.ends_with('.') {
        number.pop();
    }
    if number.is_empty() {
        return None;
    }
    Some(format!("\\H{number}x;"))
}

/// Whether every `{` in `text` has a matching `}` (no negative depth, depth 0 at
/// the end). Only a balanced string may be wrapped in the height-override group.
fn balanced_braces(text: &str) -> bool {
    let mut depth = 0i64;
    for character in text.chars() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// Create the new-row shell for one MTEXT replacement.
fn mtext_entity(
    candidate: &mut DrawingDatabase,
    source: &DbEntity,
    geometry: SemanticGeometry,
) -> DbEntity {
    let new_id = candidate.allocate_entity_id();
    DbEntity {
        object: cad_db::DbObject {
            id: ObjectId(new_id.0),
            type_key: text_policy::MTEXT_TYPE_KEY.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: new_id,
        layer: source.layer,
        space: source.space.clone(),
        geometry,
        draw_order: source.draw_order,
    }
}
