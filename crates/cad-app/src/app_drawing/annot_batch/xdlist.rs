//! `XDLIST`: read-only report of the selected entities' preserved extended data.
//!
//! `cad-domain`/`cad-db` retain no xdata (`ExtendedEntityData`/`object_xdata`
//! exists only in the importer's source report), so every selected entity is
//! reported with the named `xdlist.not_retained` diagnostic. Nothing is invented
//! and nothing is written.

use super::*;

impl Application {
    /// `XDLIST`: report whether each selected entity retains xdata.
    pub(crate) fn xd_list(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(invalid(
                "xdlist.payload",
                "XDLIST takes no payload; the selection is its input",
            ));
        }
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(invalid(
                "xdlist.selection",
                "XDLIST needs at least one selected entity",
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
        let mut diagnostics = Vec::with_capacity(references.len() + 1);
        let mut seen = std::collections::BTreeSet::new();
        for reference in &references {
            if reference.document != document_id
                || !reference.instance.0.is_empty()
                || reference.sub_element.is_some()
            {
                return Err(invalid(
                    "xdlist.not_addressable",
                    format!(
                        "entity {} is not a directly addressable entity of this document",
                        reference.entity.0
                    ),
                ));
            }
            if !seen.insert(reference.entity) {
                continue;
            }
            let entity = drawing.entity(reference.entity).ok_or_else(|| {
                invalid(
                    "xdlist.not_addressable",
                    format!("entity {} no longer exists", reference.entity.0),
                )
            })?;
            diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "xdlist.not_retained".into(),
                message: format!(
                    "XDLIST: entity {} ({}) retains no extended data in this build: \
                     cad-domain/cad-db do not store xdata after import (no application names or \
                     values to list)",
                    entity.id.0,
                    class_name(&entity.geometry)
                ),
            });
        }
        diagnostics.push(Diagnostic {
            object: None,
            code: "xdlist.summary".into(),
            message: format!(
                "XDLIST: {} selected entity/entities checked; 0 retained an xdata record \
                 (read-only, nothing changed)",
                seen.len()
            ),
        });
        Ok(report_outcome(diagnostics))
    }
}
