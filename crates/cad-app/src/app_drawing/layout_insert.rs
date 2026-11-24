//! The one-entity insert helper for a **paper layout**.
//!
//! The shared create funnel ([`Application::insert_new_entities`]) deliberately
//! bases a new row's `draw_order` on the model-space maximum, a contract pinned
//! by `tests::drawing::create_draw_order` (an OFFSET copy that keeps its source
//! space still takes the model base). A command that **creates** an entity in a
//! paper layout needs that layout's own draw-order sequence instead, or the new
//! row collides with the rows already on the sheet.
//!
//! This module is that layout entity write path: one validated transaction, one
//! undo step, the active layer, the session `COLOR`, the session `ELEV` offset
//! and the next draw order of the target `LayoutId`. It is deliberately narrow
//! (exactly one entity) so the model-space funnel is not changed for callers
//! that rely on its pinned behaviour.

use super::*;

use super::create_inputs::new_entity_color;

impl Application {
    /// Insert exactly one new entity into paper `layout` in one transaction /
    /// one undo step.
    ///
    /// The layout must exist (a stale `active_space` is the caller's named
    /// refusal, checked before this helper) and the active layer must exist.
    /// `ELEV` is applied through the shared elevation rule, the session `COLOR`
    /// token is staged on the new row exactly like the model-space funnel, and
    /// the draw order is one past the highest row **of that layout**.
    pub(super) fn commit_new_layout_entity(
        &mut self,
        session: &SessionState,
        command: &Command,
        geometry: SemanticGeometry,
        layout: cad_domain::LayoutId,
        label: &str,
    ) -> CadResult<CommandOutcome> {
        let geometry = Self::elevate_new_geometry(geometry, session.drafting_extras.elev())?;
        let color = new_entity_color(session)?;
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;
        if drawing.layout(layout).is_none() {
            return Err(CadError::InvalidInput(format!(
                "{label}.unknown_layout: this drawing has no layout with id {}",
                layout.0
            )));
        }
        if drawing.layer(session.active_layer).is_none() {
            return Err(CadError::InvalidInput(format!(
                "layer {} does not exist",
                session.active_layer.0
            )));
        }
        let space = SpaceId::Paper(layout);
        let base_order = drawing
            .entities()
            .filter(|entity| entity.space == space)
            .map(|entity| entity.draw_order)
            .max()
            .map(|order| order.saturating_add(1))
            .unwrap_or(0);
        let entity_id = drawing.allocate_entity_id();
        let entity = DbEntity {
            object: cad_db::DbObject {
                id: ObjectId(entity_id.0),
                type_key: class_name(&geometry).to_string(),
                revision: Revision(0),
                source_handle: None,
            },
            id: entity_id,
            layer: session.active_layer,
            space,
            geometry,
            draw_order: base_order,
        };
        let display_state = color.map(|color| {
            let mut state = cad_db::EntityDisplayState::default();
            state.attributes.color = color;
            state
        });
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction(label, transaction)?;
            staged.insert_entity(entity.clone())?;
            if let Some(state) = &display_state {
                staged.update_entity_display_state(entity.id, state)?;
            }
            staged.commit()?
        };
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.to_string(),
                patches: vec![DrawingPatch {
                    id: entity.id,
                    before: None,
                    after: Some(entity.clone()),
                    before_state: None,
                    after_state: display_state,
                }],
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
            objects: vec![ObjectId(entity.id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: Vec::new(),
            measurement: None,
            point_report: None,
        })
    }
}
