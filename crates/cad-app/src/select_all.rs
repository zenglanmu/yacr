//! Read-only SELECTALL over the existing model-space pick-item catalogue.
//!
//! Selection is whole-entity, not mesh-face enumeration: the exact source ref
//! from each pick item is retained, including its full INSERT path and any
//! sub-element identity. The current builder emits `sub_element: None`.
//! Layer visibility and resolved dynamic-block states are the visibility data
//! available in the database; it has no independent source-entity hidden flag.
//! No promise is made about visibility flags lost before database import.
//! Paper space, opaque geometry, nested compound INSERTs, unresolved dynamic
//! states and truncated/cyclic expansion are refused, never empty successes.

use crate::{
    drawing_pick_items, Application, Command, CommandOutcome, CommandPayload, SelectionSet,
    SessionState, ToolState,
};
use cad_db::{DbEntity, DrawingDatabase, MAX_INSTANCE_DEPTH};
use cad_domain::{BlockId, CadError, CadResult, SemanticGeometry, SpaceId};

impl Application {
    pub(crate) fn select_all(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(&command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput("SelectAll needs no payload".into()));
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
                "SelectAll currently supports model space only".into(),
            ));
        }
        let database = &document.drawing;
        // The existing pick builder skips cycles/depth limits without a report.
        // Preflight the visible tree before trusting its catalogue or publishing.
        for entity in database.model_space() {
            validate_branch(database, session, entity, &mut Vec::new())?;
        }
        let mut selection = SelectionSet::new();
        for item in drawing_pick_items(database, command.document) {
            let mut visible = true;
            for id in item
                .source
                .instance
                .0
                .iter()
                .chain(std::iter::once(&item.source.entity))
            {
                let entity = database.entity(*id).ok_or(CadError::StaleResult)?;
                visible &= session.layer_overrides.is_entity_visible(database, entity);
            }
            if visible {
                selection.insert(item.source);
            }
        }
        session.selection = selection;
        session.tool = ToolState::Selecting;
        Ok(CommandOutcome::none())
    }
}

fn validate_branch(
    database: &DrawingDatabase,
    session: &SessionState,
    entity: &DbEntity,
    stack: &mut Vec<BlockId>,
) -> CadResult<()> {
    if !session.layer_overrides.is_entity_visible(database, entity) {
        return Ok(());
    }
    if let SemanticGeometry::Insert { block, .. } = &entity.geometry {
        if stack.len() >= MAX_INSTANCE_DEPTH || stack.contains(block) {
            return Err(CadError::Unsupported(
                "SelectAll cannot enumerate cyclic or depth-limited INSERTs".into(),
            ));
        }
        let definition = database.block(*block).ok_or_else(|| {
            CadError::Unsupported("SelectAll cannot enumerate a missing block".into())
        })?;
        if definition
            .dynamic_visibility
            .as_ref()
            .is_some_and(|visibility| visibility.active_entities().is_none())
        {
            return Err(CadError::Unsupported(
                "SelectAll cannot resolve dynamic-block visibility".into(),
            ));
        }
        for id in &definition.entities {
            if database.entity(*id).is_none() {
                return Err(CadError::Unsupported(
                    "SelectAll cannot enumerate missing block members".into(),
                ));
            }
        }
        stack.push(*block);
        for child in database.block_entities(*block) {
            validate_branch(database, session, child, stack)?;
        }
        stack.pop();
        Ok(())
    } else {
        validate_geometry(&entity.geometry)
    }
}

fn validate_geometry(geometry: &SemanticGeometry) -> CadResult<()> {
    match geometry {
        SemanticGeometry::Mesh(mesh) if mesh.triangles.is_empty() => Err(CadError::Unsupported(
            "SelectAll cannot pick an empty mesh".into(),
        )),
        SemanticGeometry::Polyline { points, .. } if points.len() < 2 => Err(
            CadError::Unsupported("SelectAll cannot pick an empty polyline".into()),
        ),
        SemanticGeometry::Spline { control_points, .. } if control_points.len() < 2 => Err(
            CadError::Unsupported("SelectAll cannot pick an empty spline".into()),
        ),
        SemanticGeometry::Opaque { .. } | SemanticGeometry::Insert { .. } => {
            Err(CadError::Unsupported(
                "SelectAll cannot enumerate opaque geometry or compound INSERTs".into(),
            ))
        }
        SemanticGeometry::Compound(children) => {
            for child in children {
                validate_geometry(child)?;
            }
            if children.is_empty() {
                return Err(CadError::Unsupported(
                    "SelectAll cannot pick an empty compound".into(),
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
