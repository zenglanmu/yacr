//! `TXT2MTXT` — combine selected TEXT/MTEXT into one or more MTEXT objects.
//!
//! ## Settings
//!
//! * **Combine into a single mtext object** (default on): one `AcDbMText` holds
//!   every source, joined with raw `\P` paragraph breaks. The first source in
//!   the chosen order supplies position/style/font/height/rotation/alignment,
//!   layer, draw order and (when word-wrap is on) `wrap_width`; per-source font
//!   and height differences cannot survive a single object and are declared.
//! * **Combine off** (`one per source`): every source becomes one new
//!   `AcDbMText` at its own position, carrying its own style/font/height/
//!   rotation/alignment/layer/draw order. This is the declared non-combined
//!   rule; an MTEXT source is re-created as MTEXT, not left with a stale id.
//! * **Text ordering**: `TopDown` sorts descending Y then ascending X, stably;
//!   `Selection` keeps the session selection order exactly.
//! * **Word-wrap**: uses the existing `wrap_width` field. On preserves each
//!   source's boundary (including `None` = no wrap); off stores `None`. No width
//!   is invented.
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
                    "txt2mtxt.unsupported: {} is not a Text/MText entity; the whole command is \
                     refused and nothing is converted",
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
                merge_key: None,
            },
        )?;
        let count = plan.sources.len();
        let made = objects.len();
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
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
            }],
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
    if settings.combine {
        let SemanticGeometry::Text {
            position,
            style,
            height,
            rotation,
            font,
            h_align,
            v_align,
            wrap_width,
            ..
        } = &first.geometry
        else {
            return Err(CadError::Invariant(
                "TXT2MTXT first source is not Text".into(),
            ));
        };
        let mut joined = String::new();
        for (index, source) in sources.iter().enumerate() {
            let SemanticGeometry::Text { text, .. } = &source.geometry else {
                return Err(CadError::Invariant("TXT2MTXT source is not Text".into()));
            };
            if index > 0 {
                joined.push_str("\\P");
            }
            joined.push_str(text);
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
                *wrap_width
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
    })
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
