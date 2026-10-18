//! Controlled application API for atomic entity edits and validated selection.

pub use crate::interaction::SelectionMode;
use crate::{
    app_drawing::{class_name, next_transaction_id},
    Application, Command, CommandId, CommandOutcome, SessionState,
};
use cad_db::{ChangeSet, DbEntity, DbObject, DrawingDatabase};
use cad_domain::{
    CadError, CadResult, DatabaseId, DocumentId, EntityId, LayerId, ObjectId, Point3, Revision,
    SelectionRef, SemanticGeometry, SpaceId, Transform3, ViewportId,
};
use cad_history::{drawing_patch, drawing_patch_deleted, DrawingUndoRecord};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiTarget {
    pub document: DocumentId,
    pub database: DatabaseId,
    pub viewport: ViewportId,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApiRequest {
    Commit {
        expected_revision: Revision,
        edits: Vec<EntityEdit>,
    },
    SelectByRefs {
        refs: Vec<SelectionRef>,
        mode: SelectionMode,
    },
    SelectByObjectIds {
        ids: Vec<ObjectId>,
        mode: SelectionMode,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum EntityEdit {
    Insert {
        layer: LayerId,
        space: SpaceId,
        entity: NewEntity,
    },
    Translate {
        reference: SelectionRef,
        delta: Point3,
    },
    Delete {
        reference: SelectionRef,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum NewEntity {
    Line {
        start: Point3,
        end: Point3,
    },
    Circle {
        center: Point3,
        radius: f64,
    },
    Arc {
        center: Point3,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiOutcome {
    pub created: Vec<ObjectId>,
    pub changes: Option<Arc<ChangeSet>>,
}

#[derive(Debug, Clone)]
pub struct RawCommandTrace {
    pub command: CommandId,
    pub result: Result<Arc<CommandOutcome>, CadError>,
}

#[derive(Debug, Clone)]
pub struct TracedCommandOutcome {
    pub result: Result<Arc<CommandOutcome>, CadError>,
    pub commands: Vec<RawCommandTrace>,
}

pub(crate) fn traced_command(
    command: CommandId,
    result: Result<Arc<CommandOutcome>, CadError>,
) -> TracedCommandOutcome {
    TracedCommandOutcome {
        result: result.clone(),
        commands: vec![RawCommandTrace { command, result }],
    }
}

impl Application {
    /// Execute one controlled API request against the identified application state.
    pub fn execute_api(
        &mut self,
        session: &mut SessionState,
        target: ApiTarget,
        request: ApiRequest,
    ) -> CadResult<ApiOutcome> {
        let active_space = session.active_space.clone();
        if matches!(&request, ApiRequest::Commit { .. }) {
            session.authorize(CommandId::CreateLine)?;
        }
        self.validate_api_target(session, target, &active_space)?;
        match request {
            ApiRequest::Commit {
                expected_revision,
                edits,
            } => self.commit_edits(target.document, expected_revision, &active_space, edits),
            ApiRequest::SelectByRefs { refs, mode } => {
                let resolved = refs
                    .iter()
                    .map(|reference| {
                        let drawing = &self.workspace.documents[&target.document].drawing;
                        validate_reference(
                            drawing.as_ref(),
                            target.document,
                            &active_space,
                            reference,
                        )
                        .map(|_| reference.clone())
                    })
                    .collect::<CadResult<Vec<_>>>()?;
                apply_selection(session, resolved, mode);
                Ok(ApiOutcome {
                    created: Vec::new(),
                    changes: None,
                })
            }
            ApiRequest::SelectByObjectIds { ids, mode } => {
                let resolved = ids
                    .iter()
                    .map(|object| {
                        let drawing = &self.workspace.documents[&target.document].drawing;
                        let id = EntityId(object.0);
                        let entity = drawing.entity(id).ok_or_else(|| {
                            CadError::InvalidInput(format!(
                                "object {} is not a top-level entity",
                                object.0
                            ))
                        })?;
                        if entity.space != active_space {
                            return Err(CadError::InvalidInput(format!(
                                "object {} is not in the active space",
                                object.0
                            )));
                        }
                        Ok(SelectionRef {
                            document: target.document,
                            entity: id,
                            instance: Default::default(),
                            sub_element: None,
                        })
                    })
                    .collect::<CadResult<Vec<_>>>()?;
                apply_selection(session, resolved, mode);
                Ok(ApiOutcome {
                    created: Vec::new(),
                    changes: None,
                })
            }
        }
    }

    fn validate_api_target(
        &self,
        session: &SessionState,
        target: ApiTarget,
        active_space: &SpaceId,
    ) -> CadResult<()> {
        if target.document != session.document {
            return Err(CadError::StaleResult);
        }
        let document = self
            .workspace
            .documents
            .get(&target.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        if document.drawing.id() != target.database {
            return Err(CadError::StaleResult);
        }
        let viewport = self
            .workspace
            .viewports
            .get(&target.viewport)
            .ok_or_else(|| CadError::InvalidInput("viewport not open".into()))?;
        if viewport.id != target.viewport || viewport.document != target.document {
            return Err(CadError::StaleResult);
        }
        crate::validate_space_id(document.drawing.as_ref(), active_space)?;
        Ok(())
    }

    fn commit_edits(
        &mut self,
        document_id: DocumentId,
        expected_revision: Revision,
        active_space: &SpaceId,
        edits: Vec<EntityEdit>,
    ) -> CadResult<ApiOutcome> {
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        if candidate.revision() != expected_revision {
            return Err(CadError::StaleResult);
        }
        if edits.is_empty() {
            return Ok(ApiOutcome {
                created: Vec::new(),
                changes: None,
            });
        }

        let inserts = edits
            .iter()
            .filter(|edit| matches!(edit, EntityEdit::Insert { .. }))
            .count();
        let next = candidate.next_entity_id().0;
        if next.checked_add(inserts as u128).is_none() {
            return Err(CadError::Invariant("entity id allocator exhausted".into()));
        }

        let mut targets = std::collections::BTreeSet::new();
        let mut effective = Vec::with_capacity(edits.len());
        for edit in edits {
            match &edit {
                EntityEdit::Insert {
                    layer,
                    space,
                    entity,
                } => {
                    if candidate.layer(*layer).is_none() {
                        return Err(CadError::InvalidInput(format!(
                            "layer {} does not exist",
                            layer.0
                        )));
                    }
                    crate::validate_space_id(&candidate, space)?;
                    if space != active_space {
                        return Err(CadError::InvalidInput(
                            "insert space must match the active space".into(),
                        ));
                    }
                    validate_new_entity(entity)?;
                    effective.push(edit);
                }
                EntityEdit::Translate { reference, delta } => {
                    let entity =
                        validate_reference(&candidate, document_id, active_space, reference)?;
                    validate_top_level_edit_reference(reference)?;
                    if !targets.insert(reference.entity) {
                        return Err(CadError::InvalidInput(
                            "an API batch cannot edit one entity more than once".into(),
                        ));
                    }
                    if !finite_point(*delta) {
                        return Err(CadError::InvalidInput(
                            "translation delta must be finite".into(),
                        ));
                    }
                    if !zero_point(*delta) {
                        cad_db::transform_geometry(
                            &entity.geometry,
                            &Transform3::translation(*delta),
                        )?;
                    }
                    effective.push(edit);
                }
                EntityEdit::Delete { reference } => {
                    validate_reference(&candidate, document_id, active_space, reference)?;
                    validate_top_level_edit_reference(reference)?;
                    if !targets.insert(reference.entity) {
                        return Err(CadError::InvalidInput(
                            "an API batch cannot edit one entity more than once".into(),
                        ));
                    }
                    effective.push(edit);
                }
            }
        }
        let effective: Vec<_> = effective
            .into_iter()
            .filter(
                |edit| !matches!(edit, EntityEdit::Translate { delta, .. } if zero_point(*delta)),
            )
            .collect();
        if effective.is_empty() {
            return Ok(ApiOutcome {
                created: Vec::new(),
                changes: None,
            });
        }

        // Allocate and stage only on the private candidate. If any later stage,
        // commit, or history validation fails, neither authoritative database nor
        // its allocator has been touched.
        let mut created = Vec::new();
        let mut planned = Vec::with_capacity(effective.len());
        let mut patches = Vec::with_capacity(effective.len());
        for edit in &effective {
            match edit {
                EntityEdit::Insert {
                    layer,
                    space,
                    entity,
                } => {
                    let geometry = new_geometry(entity);
                    let id = candidate.next_entity_id();
                    if candidate.entity(id).is_some() || id.0 == u128::MAX {
                        return Err(CadError::Invariant(
                            "entity id allocator is not available".into(),
                        ));
                    }
                    let prior_max =
                        candidate
                            .entities()
                            .filter(|existing| &existing.space == space)
                            .map(|existing| existing.draw_order)
                            .chain(planned.iter().filter_map(
                                |planned: &PlannedEdit| match planned {
                                    PlannedEdit::Insert(existing) if &existing.space == space => {
                                        Some(existing.draw_order)
                                    }
                                    _ => None,
                                },
                            ))
                            .max();
                    let draw_order = prior_max.map(|value| value.saturating_add(1)).unwrap_or(0);
                    let inserted = DbEntity {
                        object: DbObject {
                            id: ObjectId(id.0),
                            type_key: class_name(&geometry).to_string(),
                            revision: Revision(0),
                            source_handle: None,
                        },
                        id,
                        layer: *layer,
                        space: space.clone(),
                        geometry,
                        draw_order,
                    };
                    candidate.allocate_entity_id();
                    created.push(ObjectId(id.0));
                    patches.push(drawing_patch(id, None, Some(inserted.clone())));
                    planned.push(PlannedEdit::Insert(Box::new(inserted)));
                }
                EntityEdit::Translate { reference, delta } => {
                    let before = candidate
                        .entity(reference.entity)
                        .cloned()
                        .ok_or(CadError::StaleResult)?;
                    let after_geometry = cad_db::transform_geometry(
                        &before.geometry,
                        &Transform3::translation(*delta),
                    )?;
                    let mut after = before.clone();
                    after.geometry = after_geometry;
                    patches.push(drawing_patch(reference.entity, Some(before), Some(after)));
                    planned.push(PlannedEdit::Translate(reference.entity, *delta));
                }
                EntityEdit::Delete { reference } => {
                    let before = candidate
                        .entity(reference.entity)
                        .cloned()
                        .ok_or(CadError::StaleResult)?;
                    let display = candidate.entity_display_state(reference.entity);
                    patches.push(drawing_patch_deleted(reference.entity, before, display));
                    planned.push(PlannedEdit::Delete(reference.entity));
                }
            }
        }

        let transaction = next_transaction_id();
        let changes = {
            let mut tx =
                candidate.begin_drawing_transaction("controlled API edits", transaction)?;
            for edit in planned {
                match edit {
                    PlannedEdit::Insert(entity) => {
                        tx.insert_entity(*entity)?;
                    }
                    PlannedEdit::Translate(id, delta) => {
                        tx.transform_entity(id, &Transform3::translation(delta))?
                    }
                    PlannedEdit::Delete(id) => tx.delete_entity(id)?,
                }
            }
            tx.commit()?
        };

        let record = DrawingUndoRecord {
            transaction: changes.transaction,
            label: "controlled API edits".into(),
            patches,
            merge_key: None,
        };
        self.history
            .entry(document_id)
            .or_default()
            .record_drawing(record)?;
        document.drawing = Arc::new(candidate);
        Ok(ApiOutcome {
            created,
            changes: Some(Arc::new(changes)),
        })
    }
}

enum PlannedEdit {
    Insert(Box<DbEntity>),
    Translate(EntityId, Point3),
    Delete(EntityId),
}

impl Application {
    /// Trace one actual command entry. The owned API remains unchanged; the trace
    /// shares its Arc outcome rather than cloning the outcome's payload.
    pub fn execute_traced(
        &mut self,
        session: &mut SessionState,
        command: Command,
    ) -> CadResult<TracedCommandOutcome> {
        let id = command.id;
        let result = self.execute(session, command).map(Arc::new);
        Ok(traced_command(id, result))
    }
}

pub(crate) fn apply_selection(
    session: &mut SessionState,
    refs: Vec<SelectionRef>,
    mode: SelectionMode,
) {
    let unique = crate::SelectionSet::from_refs(refs).refs().to_vec();
    let mut selection = session.selection.clone();
    match mode {
        SelectionMode::Replace => selection.replace(unique),
        SelectionMode::Add => unique.into_iter().for_each(|reference| {
            selection.insert(reference);
        }),
        SelectionMode::Toggle => unique
            .into_iter()
            .for_each(|reference| selection.toggle(reference)),
        SelectionMode::Remove => unique.iter().for_each(|reference| {
            selection.remove(reference);
        }),
    }
    session.begin_selecting();
    session.selection = selection;
}

fn validate_reference<'a>(
    drawing: &'a DrawingDatabase,
    document: DocumentId,
    active_space: &SpaceId,
    reference: &SelectionRef,
) -> CadResult<&'a DbEntity> {
    if reference.document != document {
        return Err(CadError::StaleResult);
    }
    let mut expected_space = active_space.clone();
    if reference.instance.0.len() > 64 {
        return Err(CadError::InvalidInput(
            "selection instance path exceeds depth limit".into(),
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for instance_id in &reference.instance.0 {
        if !seen.insert(*instance_id) {
            return Err(CadError::InvalidInput(
                "selection instance path is cyclic".into(),
            ));
        }
        let insert = drawing.entity(*instance_id).ok_or_else(|| {
            CadError::InvalidInput(format!(
                "selection instance {} does not exist",
                instance_id.0
            ))
        })?;
        if insert.space != expected_space {
            return Err(CadError::InvalidInput(
                "selection instance path is not contiguous".into(),
            ));
        }
        let SemanticGeometry::Insert { block, .. } = &insert.geometry else {
            return Err(CadError::InvalidInput(
                "selection path entry is not an INSERT".into(),
            ));
        };
        if drawing.block(*block).is_none() {
            return Err(CadError::InvalidInput(
                "selection INSERT block is missing".into(),
            ));
        }
        expected_space = SpaceId::Block(*block);
    }
    let leaf = drawing.entity(reference.entity).ok_or_else(|| {
        CadError::InvalidInput(format!(
            "selection entity {} does not exist",
            reference.entity.0
        ))
    })?;
    if leaf.space != expected_space {
        return Err(CadError::InvalidInput(
            "selection leaf does not belong to the resolved space".into(),
        ));
    }
    if let Some(sub_element) = &reference.sub_element {
        let SemanticGeometry::Mesh(mesh) = &leaf.geometry else {
            return Err(CadError::InvalidInput(
                "sub-element selection requires mesh geometry".into(),
            ));
        };
        if !mesh
            .face_sources
            .iter()
            .flatten()
            .any(|candidate| candidate == sub_element)
        {
            return Err(CadError::StaleResult);
        }
    }
    Ok(leaf)
}

fn validate_top_level_edit_reference(reference: &SelectionRef) -> CadResult<()> {
    if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
        return Err(CadError::Unsupported(
            "API edits require a top-level whole-entity reference".into(),
        ));
    }
    Ok(())
}

fn finite_point(point: Point3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

fn zero_point(point: Point3) -> bool {
    point.x == 0.0 && point.y == 0.0 && point.z == 0.0
}

fn validate_new_entity(entity: &NewEntity) -> CadResult<()> {
    match entity {
        NewEntity::Line { start, end } => {
            if !finite_point(*start) || !finite_point(*end) {
                return Err(CadError::InvalidInput("line points must be finite".into()));
            }
            if start == end {
                return Err(CadError::InvalidInput("line endpoints must differ".into()));
            }
        }
        NewEntity::Circle { center, radius } => {
            if !finite_point(*center) || !radius.is_finite() || *radius <= 0.0 {
                return Err(CadError::InvalidInput(
                    "circle center and positive radius must be finite".into(),
                ));
            }
        }
        NewEntity::Arc {
            center,
            radius,
            start_angle,
            end_angle,
        } => {
            if !finite_point(*center)
                || !radius.is_finite()
                || *radius <= 0.0
                || !start_angle.is_finite()
                || !end_angle.is_finite()
            {
                return Err(CadError::InvalidInput(
                    "arc parameters must be finite with positive radius".into(),
                ));
            }
            let sweep = (end_angle - start_angle).rem_euclid(std::f64::consts::TAU);
            if !sweep.is_finite() || sweep <= f64::EPSILON {
                return Err(CadError::InvalidInput(
                    "arc start and end angles must define a non-degenerate arc".into(),
                ));
            }
        }
    }
    Ok(())
}

fn new_geometry(entity: &NewEntity) -> SemanticGeometry {
    let normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    match entity {
        NewEntity::Line { start, end } => SemanticGeometry::Line {
            start: *start,
            end: *end,
        },
        NewEntity::Circle { center, radius } => SemanticGeometry::Circle {
            center: *center,
            normal,
            radius: *radius,
        },
        NewEntity::Arc {
            center,
            radius,
            start_angle,
            end_angle,
        } => SemanticGeometry::Arc {
            center: *center,
            normal,
            radius: *radius,
            start: *start_angle,
            sweep: (end_angle - start_angle).rem_euclid(std::f64::consts::TAU),
        },
    }
}

#[cfg(test)]
mod tests;
