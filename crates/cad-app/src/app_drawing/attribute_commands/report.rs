//! Read-only attribute reports: the payload-free `ATTDEF`, `BATTMAN`,
//! `BATTORDER` and the no-target `ATTSYNC` forms.
//!
//! These read the block table and the stored definitions/values directly. They
//! never open a transaction, bump the revision or record history, so they stay
//! Viewer-legal. A report never invents a definition or a value: a block with no
//! `ATTDEF` member is listed as having none.

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::document_drawing;

/// The payload-free `ATTDEF`: list every block's definitions and the typed form
/// that creates one.
pub(super) fn attdef_report(app: &Application, command: &Command) -> CadResult<CommandOutcome> {
    let drawing = document_drawing(app, command)?;
    let drawing = drawing.as_ref();
    let mut diagnostics = Vec::new();
    let mut definitions = 0usize;
    for block in drawing.blocks() {
        let name = drawing
            .block_name(block.id)
            .map(str::to_string)
            .unwrap_or_else(|| format!("block#{}", block.id.0));
        let mut block_defs = Vec::new();
        for member in &block.entities {
            let Some(entity) = drawing.entity(*member) else {
                continue;
            };
            if let SemanticGeometry::AttributeDefinition(attribute) = &entity.geometry {
                block_defs.push(attribute.as_ref().clone());
            }
        }
        if block_defs.is_empty() {
            continue;
        }
        block_defs.sort_by_key(|definition| definition.order);
        for definition in &block_defs {
            definitions += 1;
            diagnostics.push(diagnostic(
                "attdef.entry",
                format!(
                    "ATTDEF: block '{name}' tag '{}' order {} default '{}' prompt '{}' flags {}",
                    definition.tag,
                    definition.order,
                    definition.default_value,
                    definition.prompt,
                    flag_words(definition.flags)
                ),
            ));
        }
    }
    diagnostics.push(diagnostic(
        "attdef.summary",
        format!(
            "ATTDEF: {definitions} attribute definition(s) in this drawing; creating one is the \
             typed form `ATTDEF <block> <tag> <default> [PROMPT <text>] [POSITION <x> <y>] \
             [HEIGHT <h>] [ROTATION <deg>] [ORDER <n>] [CONSTANT] [INVISIBLE] [VERIFY] [PRESET] \
             [LOCK] [ANNOTATIVE]`"
        ),
    ));
    Ok(report_outcome(diagnostics))
}

/// `BATTMAN`: the complete block-attribute manager report.
pub(super) fn battman_report(app: &Application, command: &Command) -> CadResult<CommandOutcome> {
    let drawing = document_drawing(app, command)?;
    let drawing = drawing.as_ref();
    let mut diagnostics = Vec::new();
    let mut blocks_with_attributes = 0usize;
    let mut definition_count = 0usize;
    let mut stale_count = 0usize;
    for block in drawing.blocks() {
        let definitions = match definitions(drawing, block.id) {
            Ok(definitions) => definitions,
            Err(error) => {
                diagnostics.push(diagnostic("battman.duplicate_tag", error.to_string()));
                continue;
            }
        };
        if definitions.is_empty() {
            continue;
        }
        blocks_with_attributes += 1;
        let name = drawing
            .block_name(block.id)
            .map(str::to_string)
            .unwrap_or_else(|| format!("block#{}", block.id.0));
        let references = references(drawing, block.id);
        diagnostics.push(diagnostic(
            "battman.block",
            format!(
                "BATTMAN: block '{name}' id {} has {} attribute definition(s) and {} \
                 reference(s)",
                block.id.0,
                definitions.len(),
                references.len()
            ),
        ));
        for (_, definition) in &definitions {
            definition_count += 1;
            diagnostics.push(diagnostic(
                "battman.attribute",
                format!(
                    "BATTMAN: block '{name}' tag '{}' order {} default '{}' prompt '{}' \
                     position ({}, {}) height {} flags {}",
                    definition.tag,
                    definition.order,
                    definition.default_value,
                    definition.prompt,
                    definition.text.position.x,
                    definition.text.position.y,
                    definition.text.height,
                    flag_words(definition.flags)
                ),
            ));
        }
        for id in references {
            let Some(SemanticGeometry::Insert { attributes, .. }) =
                drawing.entity(id).map(|entity| &entity.geometry)
            else {
                continue;
            };
            let stale: Vec<&str> = attributes
                .iter()
                .filter(|value| {
                    !definitions
                        .iter()
                        .any(|(_, definition)| tags_equal(&definition.tag, &value.tag))
                })
                .map(|value| value.tag.as_str())
                .collect();
            stale_count += stale.len();
            if !stale.is_empty() {
                diagnostics.push(diagnostic(
                    "battman.stale_value",
                    format!(
                        "BATTMAN: reference {} of block '{name}' stores {} value(s) with no \
                         current definition: {}",
                        id.0,
                        stale.len(),
                        stale.join(", ")
                    ),
                ));
            }
        }
    }
    if blocks_with_attributes == 0 {
        diagnostics.push(diagnostic(
            "battman.no_attributes",
            "BATTMAN: no block in this drawing has an attribute definition; the manager has \
             nothing to edit"
                .into(),
        ));
    }
    diagnostics.push(diagnostic(
        "battman.summary",
        format!(
            "BATTMAN: {blocks_with_attributes} block(s) with {definition_count} definition(s) and \
             {stale_count} stale value tag(s); editing is the typed ATTDEF/ATTREDEF/ATTEDIT/\
             BATTORDER commands, not a dialog"
        ),
    ));
    Ok(report_outcome(diagnostics))
}

/// `BATTORDER` (payload-free): list each block's stored attribute order.
pub(super) fn battorder_report(app: &Application, command: &Command) -> CadResult<CommandOutcome> {
    let drawing = document_drawing(app, command)?;
    let drawing = drawing.as_ref();
    let mut diagnostics = Vec::new();
    let mut blocks = 0usize;
    for block in drawing.blocks() {
        let definitions = match definitions(drawing, block.id) {
            Ok(definitions) => definitions,
            Err(error) => {
                diagnostics.push(diagnostic("battorder.duplicate_tag", error.to_string()));
                continue;
            }
        };
        if definitions.is_empty() {
            continue;
        }
        blocks += 1;
        let name = drawing
            .block_name(block.id)
            .map(str::to_string)
            .unwrap_or_else(|| format!("block#{}", block.id.0));
        let order: Vec<String> = definitions
            .iter()
            .map(|(_, definition)| format!("{}:{}", definition.tag, definition.order))
            .collect();
        diagnostics.push(diagnostic(
            "battorder.entry",
            format!("BATTORDER: block '{name}' order {}", order.join(" > ")),
        ));
    }
    diagnostics.push(diagnostic(
        "battorder.summary",
        format!(
            "BATTORDER: {blocks} block(s) have attributes; the typed form `BATTORDER <block> \
             <tag>,<tag>,...` rewrites the order of every definition and reference value"
        ),
    ));
    Ok(report_outcome(diagnostics))
}

/// `ATTSYNC` without a target: report each block's reference mismatch count.
pub(super) fn attsync_report(app: &Application, command: &Command) -> CadResult<CommandOutcome> {
    let drawing = document_drawing(app, command)?;
    let drawing = drawing.as_ref();
    let mut diagnostics = Vec::new();
    let mut blocks = 0usize;
    let mut mismatched = 0usize;
    for block in drawing.blocks() {
        let definitions = match definitions(drawing, block.id) {
            Ok(definitions) => definitions,
            Err(error) => {
                diagnostics.push(diagnostic("attsync.duplicate_tag", error.to_string()));
                continue;
            }
        };
        if definitions.is_empty() {
            continue;
        }
        blocks += 1;
        let name = drawing
            .block_name(block.id)
            .map(str::to_string)
            .unwrap_or_else(|| format!("block#{}", block.id.0));
        let layers: Vec<LayerId> = definitions
            .iter()
            .map(|(id, _)| {
                drawing
                    .entity(*id)
                    .map(|entity| entity.layer)
                    .unwrap_or(LayerId(0))
            })
            .collect();
        let mut stale_refs = 0usize;
        for id in references(drawing, block.id) {
            let Some(SemanticGeometry::Insert { attributes, .. }) =
                drawing.entity(id).map(|entity| &entity.geometry)
            else {
                continue;
            };
            let synced = sync_values(&definitions, &layers, attributes);
            if synced != *attributes {
                stale_refs += 1;
            }
        }
        mismatched += stale_refs;
        diagnostics.push(diagnostic(
            "attsync.entry",
            format!(
                "ATTSYNC: block '{name}' has {} definition(s); {stale_refs} reference(s) would be \
                 rewritten",
                definitions.len()
            ),
        ));
    }
    diagnostics.push(diagnostic(
        "attsync.summary",
        format!(
            "ATTSYNC: {blocks} block(s) with attributes, {mismatched} reference(s) out of sync; \
             select one block reference (or type `ATTSYNC <block>`) to propagate the definitions"
        ),
    ));
    Ok(report_outcome(diagnostics))
}

/// The short flag words used by the reports.
fn flag_words(flags: cad_domain::AttributeFlags) -> String {
    let mut words = Vec::new();
    if flags.invisible {
        words.push("invisible");
    }
    if flags.constant {
        words.push("constant");
    }
    if flags.verify {
        words.push("verify");
    }
    if flags.preset {
        words.push("preset");
    }
    if flags.locked_position {
        words.push("lock-position");
    }
    if flags.annotative {
        words.push("annotative");
    }
    if words.is_empty() {
        "none".to_string()
    } else {
        words.join(",")
    }
}
