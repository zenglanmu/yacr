//! `CHPROP` / `CHANGE` typed property writes.
//!
//! Split out of `app_extended_edit4.rs` to keep every file inside the 1000-line
//! code gate. Both commands used to mean "move to the current layer and reset
//! colour/linetype/lineweight to ByLayer"; they now apply the caller's **typed
//! property list** through the same [`PropertyValue`] grammar and
//! [`plan_entity`] applicator `SETENTITYPROPERTY` uses, so there is exactly one
//! property-write rule in the crate.
//!
//! ## All-or-nothing
//!
//! Every value is resolved against the drawing's own tables and every selected
//! ref is validated **before** the transaction opens:
//!
//! * `chprop.no_properties` / `change.no_properties` — the payload carries no
//!   property at all (this build has no interactive property prompt, so an empty
//!   request asks for nothing and is refused rather than silently resetting to
//!   ByLayer);
//! * `chprop.payload` / `change.payload` — a payload other than the typed
//!   [`crate::ChangePropertiesRequest`] list;
//! * `chprop.unsupported_property` / `change.unsupported_property` — a key
//!   outside [`crate::property_value::SUPPORTED_KEYS`], named in the message
//!   (`text` has its own `TEXTEDIT` path; thickness/material/plot style are not
//!   modelled; linetype scale has no property key);
//! * the property grammar's own refusals, re-coded under the command prefix
//!   (`chprop.unknown_layer`, `chprop.invalid_value`, …);
//! * `*.selection` / `*.not_addressable` — an empty selection, a block-instance
//!   member, a sub-element or a block-definition row.
//!
//! A refusal writes nothing: no revision bump, no `ChangeSet`, no undo entry.
//!
//! ## The write
//!
//! All requested values are applied to every selected entity through one
//! [`plan_entity`] call, which yields a single before/after pair per entity.
//! Entities that already hold every value are not staged, so undoing restores
//! exactly what changed. Both state layers (the entity row's layer and the
//! display attributes) are patched together in **one transaction / one undo
//! step**. Repeating a key in the list is allowed and the **last** value wins,
//! matching the order an interactive property prompt would apply them in.

use super::*;
use crate::property_value::{self, PropertyValue};
use cad_history::DrawingUndoRecord;

impl Application {
    /// `CHPROP` / `CHANGE`: apply a typed property list to the selection.
    ///
    /// Paper-space rows are accepted (layer/colour are space-independent); a
    /// block-definition member is refused by name, because editing it would
    /// silently change every reference of that block.
    pub(super) fn change_properties(
        &mut self,
        session: &SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        let (prefix, label) = if ext == ExtendedCommandId::ChProp {
            ("chprop", "CHPROP")
        } else {
            ("change", "CHANGE")
        };
        let request = match &command.payload {
            CommandPayload::SelectionTool(payload) => match payload.as_ref() {
                SelectionToolPayload::ChangeProperties(request) => request,
                _ => {
                    return Err(invalid(
                        prefix,
                        "payload",
                        format!("{label} accepts only its own typed property list"),
                    ))
                }
            },
            CommandPayload::None => {
                return Err(invalid(
                    prefix,
                    "no_properties",
                    format!(
                        "{label} needs at least one typed property write; this build has no \
                         interactive property prompt"
                    ),
                ))
            }
            _ => {
                return Err(invalid(
                    prefix,
                    "payload",
                    format!(
                        "{label} takes no payload other than its typed property list \
                         (layer/color/linetype/lineweight/transparency)"
                    ),
                ))
            }
        };
        if request.properties.is_empty() {
            return Err(invalid(
                prefix,
                "no_properties",
                format!(
                    "{label} needs at least one typed property write; an empty list would be a \
                     silent no-op"
                ),
            ));
        }

        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| invalid(prefix, "not_addressable", "document not open"))?
            .drawing
            .clone();

        // Resolve every value once, before any ref or write is staged: a bad
        // value in the fourth write must not leave the first three half-applied.
        let mut targets: Vec<PropertyValue> = Vec::with_capacity(request.properties.len());
        for write in &request.properties {
            if !property_value::is_supported_key(&write.key) {
                return Err(unsupported(
                    prefix,
                    "unsupported_property",
                    format!(
                        "'{}' cannot be written by {label}; supported properties: {} \
                         (text has its own TEXTEDIT path; thickness, material and plot style \
                         are not modelled; linetype scale has no property key)",
                        write.key,
                        property_value::SUPPORTED_KEYS.join(", ")
                    ),
                ));
            }
            let target = property_value::parse(&drawing, &write.key, &write.value)
                .map_err(|error| prefixed_property_error(prefix, error))?;
            targets.push(target);
        }

        let entities = resolve_direct(session, &drawing, command, prefix, false)?;
        let refs: Vec<&PropertyValue> = targets.iter().collect();
        let mut staged_changes = Vec::new();
        for before in entities {
            if let Some(change) = plan_entity(&drawing, before, &refs) {
                staged_changes.push(change);
            }
        }
        if staged_changes.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: format!("{prefix}.unchanged"),
                    message: format!(
                        "{label}: every selected entity already holds the requested property \
                         values"
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }

        let mut candidate = drawing.as_ref().clone();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction(prefix, transaction)?;
            for change in &staged_changes {
                if change.after != change.before {
                    staged.update_entity(change.after.clone())?;
                }
                if let Some(attributes) = &change.after_attributes {
                    staged.update_entity_display(change.id, attributes.clone())?;
                }
            }
            staged.commit()?
        };
        for change in &mut staged_changes {
            change.after = candidate
                .entity(change.id)
                .cloned()
                .ok_or_else(|| CadError::Invariant(format!("{label}: entity vanished")))?;
            if change.after_attributes.is_some() {
                change.after_attributes = Some(candidate.entity_render_attributes(change.id));
            }
        }
        let patches: Vec<_> = staged_changes
            .iter()
            .map(|change| DrawingPatch {
                id: change.id,
                before: Some(change.before.clone()),
                after: Some(change.after.clone()),
                before_state: Some(drawing.entity_display_state(change.id)),
                after_state: Some(candidate.entity_display_state(change.id)),
            })
            .collect();
        let objects: Vec<ObjectId> = staged_changes
            .iter()
            .map(|change| ObjectId(change.id.0))
            .collect();
        let count = objects.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: prefix.into(),
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
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: format!("{prefix}.applied"),
                message: format!(
                    "{label}: wrote {} on {count} entity/entities in one transaction",
                    request
                        .properties
                        .iter()
                        .map(|write| write.key.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Re-code a [`property_value`] refusal under the calling command's prefix.
///
/// The grammar and its reason stay the same (`property.invalid_value` becomes
/// `chprop.invalid_value`), so a caller sees which command refused while the
/// value rule has exactly one authority.
pub(super) fn prefixed_property_error(prefix: &str, error: CadError) -> CadError {
    let recode = |message: String| match message.strip_prefix("property.") {
        Some(rest) => format!("{prefix}.{rest}"),
        None => format!("{prefix}.property: {message}"),
    };
    match error {
        CadError::InvalidInput(message) => CadError::InvalidInput(recode(message)),
        CadError::Unsupported(message) => CadError::Unsupported(recode(message)),
        other => other,
    }
}
