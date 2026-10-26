//! Read-only SELECTALL over the existing model-space pick-item catalogue.
//!
//! Selection is whole-entity, not mesh-face enumeration: the exact source ref
//! from each pick item is retained, including its full INSERT path and any
//! sub-element identity. The current builder emits `sub_element: None`.
//! Layer visibility and resolved dynamic-block states are the visibility data
//! available in the database; it has no independent source-entity hidden flag.
//! A block member is judged on its **effective** layer (a member on layer `0`
//! follows the containing `INSERT`'s layer, `cad_db::LayerScope`), through the
//! same walk that decides picking (`crate::picking::ref_visibility`).
//! No promise is made about visibility flags lost before database import.
//! Paper space, opaque geometry, nested compound INSERTs, unresolved dynamic
//! states and truncated/cyclic expansion are refused, never empty successes.

use crate::picking::ref_visibility;
use crate::{
    drawing_pick_items, Application, Command, CommandOutcome, CommandPayload, SelectionSet,
    SessionState,
};
use cad_db::{DbEntity, DrawingDatabase, LayerScope, MAX_INSTANCE_DEPTH};
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
        // One layer-table scan per command: the preflight walk and the selection
        // walk both carry it, so a block member is judged on its effective layer
        // everywhere and the table is never rescanned per entity.
        let scope = database.root_layer_scope();
        // The existing pick builder skips cycles/depth limits without a report.
        // Preflight the visible tree before trusting its catalogue or publishing.
        for entity in database.model_space() {
            validate_branch(database, session, entity, scope, &mut Vec::new())?;
        }
        let mut selection = SelectionSet::new();
        for item in drawing_pick_items(database, command.document) {
            let visible = ref_visibility(database, &session.layer_overrides, scope, &item.source)
                .ok_or(CadError::StaleResult)?;
            if visible {
                selection.insert(item.source);
            }
        }
        session.selection = selection;
        session.begin_selecting();
        Ok(CommandOutcome::none())
    }
}

pub(crate) fn validate_branch(
    database: &DrawingDatabase,
    session: &SessionState,
    entity: &DbEntity,
    scope: LayerScope,
    stack: &mut Vec<BlockId>,
) -> CadResult<()> {
    let effective = scope.effective_layer(entity.layer);
    if !session
        .layer_overrides
        .is_layer_visible(database, effective)
    {
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
        let child_scope = scope.child_scope(effective);
        for child in database.block_entities(*block) {
            validate_branch(database, session, child, child_scope, stack)?;
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
        // A dimension is selectable through its display children, exactly like
        // a compound (ADR 0017 A.8).
        SemanticGeometry::Dimension(dimension) => {
            for child in dimension.display() {
                validate_geometry(child)?;
            }
            if dimension.display().is_empty() {
                return Err(CadError::Unsupported(
                    "SelectAll cannot pick a dimension with no display geometry".into(),
                ));
            }
            Ok(())
        }
        // A hatch is validated through its display children (boundary loops plus
        // the fill), exactly like a compound (ADR 0018 E); the previous
        // wildcard silently skipped this check.
        SemanticGeometry::Hatch(hatch) => {
            for child in hatch.display() {
                validate_geometry(child)?;
            }
            if hatch.display().is_empty() {
                return Err(CadError::Unsupported(
                    "SelectAll cannot pick a hatch with no display geometry".into(),
                ));
            }
            Ok(())
        }
        // A multileader is validated through its display children, exactly like
        // the compound it used to be (ADR 0019 decision G).
        SemanticGeometry::MultiLeader(multileader) => {
            for child in multileader.display() {
                validate_geometry(child)?;
            }
            if multileader.display().is_empty() {
                return Err(CadError::Unsupported(
                    "SelectAll cannot pick a multileader with no display geometry".into(),
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
