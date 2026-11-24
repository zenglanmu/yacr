//! `XPLODE`: the block-reference decomposition plus its property/attribute
//! controls.
//!
//! The decomposition and the single transaction live in
//! [`crate::app_drawing::xplode`], which reuses EXPLODE's one-level
//! [`crate::app_drawing::explode::explode_source`] walk. This module is the
//! command layer: it reads the typed [`XPlodeSettings`], refuses unsupported
//! controls **by name** before any write, and adds the option/attribute
//! diagnostics to the successful outcome.
//!
//! Refusals (nothing written, no transaction):
//!
//! * `xplode.payload` — a payload that is not the typed control set;
//! * `xplode.conflicting_options` — `Inherit from parent block` together with an
//!   explicit property list (the AutoCAD prompt replaces one with the other);
//! * `xplode.unsupported_property` — a key outside
//!   `layer`/`color`/`linetype`/`lineweight`, named in the message
//!   (`transparency` is not an XPLODE option; `ltscale`/`thickness`/`material`/
//!   plot style are not modelled);
//! * the property grammar's own refusals, re-coded under `xplode.*`;
//! * `xplode.selection` / `xplode.not_addressable` — an empty selection, a
//!   block-instance member, a sub-element or a paper-space row;
//! * `xplode.unsupported` — a selected entity that is not an INSERT reference
//!   (XPLODE's block-reference domain; `BURST` uses the same one-level block
//!   walk);
//! * `xplode.xref` — an INSERT of an external reference.
//!
//! Attribute values are not a control: XPLODE drops the reference's stored
//! values and emits the block definition's `ATTDEF` members as ordinary
//! entities (the AutoCAD `EXPLODE`/`XPLODE` result). `BURST` is the
//! value-preserving command. The outcome reports the dropped value count so the
//! behaviour is never silent.

use super::property_ops::prefixed_property_error;
use super::*;
use crate::app_drawing::xplode::XPlodeControls;
use crate::property_value::{self, PropertyValue};

/// The property keys XPLODE's option set can write.
const XPLODE_PROPERTY_KEYS: [&str; 4] = ["layer", "color", "linetype", "lineweight"];

/// The similarity tolerance for the uniform-scale INSERT check, matching the
/// shared transform surface.
const XPLODE_SIMILARITY_TOLERANCE: f64 = 1e-9;

impl Application {
    /// `XPLODE`: decompose each selected INSERT one level, apply the typed
    /// controls to the children, and commit once.
    pub(super) fn xplode(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let settings = match &command.payload {
            CommandPayload::None => XPlodeSettings::default(),
            CommandPayload::SelectionTool(payload) => match payload.as_ref() {
                SelectionToolPayload::XPlode(settings) => settings.clone(),
                _ => {
                    return Err(invalid(
                        "xplode",
                        "payload",
                        "XPLODE accepts only its own typed control set",
                    ))
                }
            },
            _ => {
                return Err(invalid(
                    "xplode",
                    "payload",
                    "XPLODE takes no payload other than its typed control set",
                ))
            }
        };
        if settings.inherit_from_parent && !settings.properties.is_empty() {
            return Err(unsupported(
                "xplode",
                "conflicting_options",
                "Inherit from parent block and an explicit property list are mutually exclusive; \
                 the AutoCAD prompt replaces one with the other, so choose exactly one",
            ));
        }

        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| invalid("xplode", "not_addressable", "document not open"))?
            .drawing
            .clone();
        let entities = resolve_direct(session, &drawing, command, "xplode", true)?;
        for entity in &entities {
            if !matches!(entity.geometry, SemanticGeometry::Insert { .. }) {
                return Err(unsupported(
                    "xplode",
                    "unsupported",
                    format!(
                        "entity {} is {}; XPLODE supports selected INSERT references only",
                        entity.id.0,
                        class_name(&entity.geometry)
                    ),
                ));
            }
            let SemanticGeometry::Insert {
                block, transform, ..
            } = &entity.geometry
            else {
                unreachable!("the INSERT guard above accepted only INSERT geometry");
            };
            if !transform.is_uniform_scale(XPLODE_SIMILARITY_TOLERANCE) {
                return Err(unsupported(
                    "xplode",
                    "non_uniform",
                    format!(
                        "entity {} has a non-uniform or degenerate placement; XPLODE does not \
                         explode a block reference with unequal absolute X/Y scales",
                        entity.id.0
                    ),
                ));
            }
            if drawing
                .layer(entity.layer)
                .is_some_and(|layer| !layer.visible)
            {
                return Err(unsupported(
                    "xplode",
                    "frozen_layer",
                    format!(
                        "entity {} is on a hidden/frozen layer; XPLODE does not operate on \
                         frozen-layer objects",
                        entity.id.0
                    ),
                ));
            }
            if drawing.is_xref_block(*block) {
                return Err(unsupported(
                    "xplode",
                    "xref",
                    format!(
                        "entity {} references an external reference; XPLODE cannot explode an \
                         XREF block into this drawing's own entities",
                        entity.id.0
                    ),
                ));
            }
        }

        // Resolve every value before the decomposition so a bad fourth value
        // cannot leave earlier values half-applied.
        let mut targets: Vec<PropertyValue> = Vec::with_capacity(settings.properties.len());
        for write in &settings.properties {
            if !XPLODE_PROPERTY_KEYS.contains(&write.key.as_str()) {
                return Err(unsupported(
                    "xplode",
                    "unsupported_property",
                    format!(
                        "'{}' cannot be written by XPLODE; supported properties: {} \
                         (XPLODE has no transparency option; ltscale, thickness, material and \
                         plot style are not modelled)",
                        write.key,
                        XPLODE_PROPERTY_KEYS.join(", ")
                    ),
                ));
            }
            let target = property_value::parse(&drawing, &write.key, &write.value)
                .map_err(|error| prefixed_property_error("xplode", error))?;
            targets.push(target);
        }

        let attribute_values: usize = entities
            .iter()
            .map(|entity| match &entity.geometry {
                SemanticGeometry::Insert { attributes, .. } => attributes.len(),
                _ => 0,
            })
            .sum();
        let definition_count: usize = entities
            .iter()
            .filter_map(|entity| match &entity.geometry {
                SemanticGeometry::Insert { block, .. } => Some(*block),
                _ => None,
            })
            .map(|block| {
                drawing
                    .block_entities(block)
                    .into_iter()
                    .filter(|member| {
                        matches!(member.geometry, SemanticGeometry::AttributeDefinition(_))
                    })
                    .count()
            })
            .sum();

        let mut outcome = self.xplode_sources(
            command,
            entities,
            &XPlodeControls {
                targets: &targets,
                inherit_from_parent: settings.inherit_from_parent,
            },
        )?;

        if !settings.properties.is_empty() {
            outcome.diagnostics.push(Diagnostic {
                object: None,
                code: "xplode.properties".into(),
                message: format!(
                    "XPLODE: applied {} to every produced child in one transaction",
                    settings
                        .properties
                        .iter()
                        .map(|write| write.key.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }
        if settings.inherit_from_parent {
            outcome.diagnostics.push(Diagnostic {
                object: None,
                code: "xplode.inherit".into(),
                message: "XPLODE: resolved layer-0 children to the reference's layer and ByBlock \
                          colour/linetype/lineweight against the exploded reference"
                    .into(),
            });
        }
        if attribute_values > 0 {
            outcome.diagnostics.push(Diagnostic {
                object: None,
                code: "xplode.attributes_reverted".into(),
                message: format!(
                    "XPLODE: dropped {attribute_values} stored attribute value(s) and emitted \
                     {definition_count} block-definition ATTDEF entity/entities instead; use BURST \
                     to keep the values as TEXT"
                ),
            });
        }
        Ok(outcome)
    }
}
