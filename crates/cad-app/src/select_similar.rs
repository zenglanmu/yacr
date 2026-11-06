//! Read-only `SELECTSIMILAR` over the existing model-space pick-item catalogue.
//!
//! v1 is **type-based only**: two items are "similar" when their
//! [`SemanticGeometry`] discriminants agree. Attribute similarity (layer, colour,
//! linetype, lineweight, transparency) is deliberately **NOT IMPLEMENTED** here
//! and must not be inferred from this command.
//!
//! The catalogue is the same one `SELECTALL` uses: model space only, the shared
//! [`validate_branch_with_vendor`] preflight (cycles, depth limits, unresolved
//! dynamic visibility, missing block members and unsupported opaque geometry are
//! explicit refusals) and the same [`ref_visibility`] filter. Every currently
//! selected ref must resolve to a visible catalogue item; a ref that does not is
//! [`CadError::StaleResult`] rather than a silent no-op.
//!
//! The result is a **union** with the current selection (order preserved), never
//! a replacement. No database write, revision bump, history entry or `ChangeSet`
//! is produced, so the command is legal in Viewer and Work.

use crate::picking::{drawing_pick_items_with_vendor, ref_visibility};
use crate::select_all::validate_branch_with_vendor;
use crate::{Application, Command, CommandOutcome, CommandPayload, SelectionSet, SessionState};
use cad_domain::{CadError, CadResult, Diagnostic, SemanticGeometry, SpaceId};
use std::mem::Discriminant;

impl Application {
    pub(crate) fn select_similar(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(&command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "SelectSimilar needs no payload".into(),
            ));
        }
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        if document.id != command.document {
            return Err(CadError::StaleResult);
        }
        if session.active_space != SpaceId::Model {
            return Err(CadError::Unsupported(
                "SelectSimilar currently supports model space only".into(),
            ));
        }
        if session.selection.is_empty() {
            return Err(CadError::InvalidInput(
                "selectsimilar.selection_required: select at least one object first".into(),
            ));
        }

        let database = &document.drawing;
        // Same catalogue source as SELECTALL, including the vendor stand-in for
        // an opaque entity (ADR-0028 D2).
        let vendor = self.vendor_representations.as_deref();
        // One layer-table scan per command, shared by the preflight and the
        // selection walk (a block member on layer 0 follows its INSERT).
        let scope = database.root_layer_scope();
        // The pick builder skips cycles/depth limits without a report; refuse any
        // branch it cannot enumerate rather than publish a truncated union.
        for entity in database.model_space() {
            validate_branch_with_vendor(database, session, entity, scope, &mut Vec::new(), vendor)?;
        }

        // The visible catalogue, exactly SELECTALL's item set.
        let mut items = Vec::new();
        for item in drawing_pick_items_with_vendor(database, command.document, vendor) {
            let visible = ref_visibility(database, &session.layer_overrides, scope, &item.source)
                .ok_or(CadError::StaleResult)?;
            if visible {
                items.push(item);
            }
        }

        // Resolve every current selection into its geometry kind. A ref that is
        // not in the visible catalogue (deleted, foreign, hidden or otherwise
        // unresolved) cannot be the input of a real similarity match.
        let mut kinds: Vec<Discriminant<SemanticGeometry>> = Vec::new();
        for reference in session.selection.refs() {
            let item = items
                .iter()
                .find(|item| SelectionSet::same_object(&item.source, reference))
                .ok_or(CadError::StaleResult)?;
            let kind = std::mem::discriminant(&item.geometry);
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }

        let before = session.selection.len();
        for item in &items {
            if kinds.contains(&std::mem::discriminant(&item.geometry)) {
                session.selection.insert(item.source.clone());
            }
        }
        let added = session.selection.len() - before;
        session.begin_selecting();
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "selectsimilar.result".into(),
                message: format!("SELECTSIMILAR: added {added} similar object(s) to the selection"),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
