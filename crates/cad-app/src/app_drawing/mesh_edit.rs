//! P-MESH-2/3 command layer: mesh face move/press-pull, refine, smooth and hole
//! cap.
//!
//! These commands operate on the shared, kernel-free `cad-mesh-edit` layer. They
//! never call a modelling kernel: face move and press/pull are topology edits,
//! subdivision/smoothing are local mesh operations, and hole filling is a planar
//! boundary-loop patch. A product is still a non-exact `AcDbSubDMesh`.
//!
//! Face identity comes from the persisted `Mesh.face_sources` (`skp:face:<n>` on
//! SKP imports). A mesh without source ids refuses a face edit with
//! `mesh.face_identity_unavailable`; it is never rebuilt from triangle indices and
//! called a SketchUp face.
//!
//! Every command commits all edited rows in one transaction and one undo step.
//! Derived attributes (normals, vertex colours, UVs, texture, PBR) are dropped by
//! the edit layer because it does not recompute them; that loss is explicit.

use super::*;
use crate::app_drawing::MESH_TYPE_KEY;
use crate::app_utility::require_none_payload;
use cad_history::{drawing_patch, DrawingUndoRecord};

fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{code}: {detail}"))
}

/// The editable mesh of a row: a `Mesh` row, or an imported display-only body.
pub(crate) fn mesh_geometry(entity: &DbEntity) -> Option<&cad_domain::Mesh> {
    match &entity.geometry {
        SemanticGeometry::Mesh(mesh) if !mesh.triangles.is_empty() => Some(mesh),
        SemanticGeometry::Body3d(body)
            if body.kind == BodyKind::Unknown
                && body.exact_brep().is_none()
                && !body.display.mesh.triangles.is_empty() =>
        {
            Some(&body.display.mesh)
        }
        _ => None,
    }
}

/// Resolve the session selection to unique editable mesh rows.
///
/// Returns each row with the face keys selected for it (`None` means the whole
/// mesh). A mixed whole/face selection on one row is refused rather than guessed.
fn resolve_mesh_targets(
    drawing: &DrawingDatabase,
    document: DocumentId,
    refs: &[SelectionRef],
) -> CadResult<Vec<(DbEntity, Vec<String>)>> {
    if refs.is_empty() {
        return Err(invalid("mesh.edit_no_selection", "no mesh is selected"));
    }
    let mut targets: Vec<(DbEntity, Vec<String>)> = Vec::new();
    for reference in refs {
        if reference.document != document {
            return Err(invalid(
                "mesh.edit_other_document",
                "the selected mesh belongs to another document",
            ));
        }
        if !reference.instance.0.is_empty() {
            return Err(unsupported(
                "mesh.edit_instance_not_supported",
                "a mesh inside a block instance has no addressable edit identity; edit the block \
                 definition instead",
            ));
        }
        let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
            invalid(
                "mesh.edit_selection_changed",
                format!("entity {} no longer exists", reference.entity.0),
            )
        })?;
        if mesh_geometry(&entity).is_none() {
            return Err(unsupported(
                "mesh.edit_not_a_mesh",
                format!(
                    "entity {} is not a mesh body (only a Mesh row or a display-only imported \
                     body can be edited here)",
                    reference.entity.0
                ),
            ));
        }
        if let Some(existing) = targets.iter_mut().find(|(row, _)| row.id == entity.id) {
            if let Some(sub) = &reference.sub_element {
                if existing.1.is_empty() {
                    // The first ref was a whole-row ref: mixing is ambiguous.
                    return Err(invalid(
                        "mesh.edit_mixed_targets",
                        "a whole-mesh selection cannot be combined with a face selection on the \
                         same mesh",
                    ));
                }
                if !existing.1.contains(&sub.source_key) {
                    existing.1.push(sub.source_key.clone());
                }
            } else if !existing.1.is_empty() {
                return Err(invalid(
                    "mesh.edit_mixed_targets",
                    "a whole-mesh selection cannot be combined with a face selection on the same \
                     mesh",
                ));
            }
        } else {
            let keys = reference
                .sub_element
                .as_ref()
                .map(|sub| vec![sub.source_key.clone()])
                .unwrap_or_default();
            targets.push((entity, keys));
        }
    }
    Ok(targets)
}

impl Application {
    /// Commit one or more edited meshes in a single transaction/undo step.
    fn commit_mesh_edits(
        &mut self,
        document: DocumentId,
        edits: Vec<(EntityId, cad_domain::Mesh)>,
        label: &str,
        mut diagnostics: Vec<Diagnostic>,
    ) -> CadResult<CommandOutcome> {
        if edits.is_empty() {
            return Err(invalid("mesh.edit_no_selection", "no mesh was edited"));
        }
        let mut candidate = {
            let document = self
                .workspace
                .documents
                .get(&document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            document.drawing.as_ref().clone()
        };
        let (change_set, patches, objects) = {
            let befores: Vec<(EntityId, DbEntity)> = edits
                .iter()
                .map(|(id, _)| {
                    candidate
                        .entity(*id)
                        .cloned()
                        .map(|before| (*id, before))
                        .ok_or_else(|| {
                            invalid(
                                "mesh.edit_selection_changed",
                                format!("entity {} no longer exists", id.0),
                            )
                        })
                })
                .collect::<CadResult<Vec<_>>>()?;
            let transaction = crate::app_drawing::next_transaction_id();
            let mut objects = Vec::with_capacity(edits.len());
            let mut staged = candidate.begin_drawing_transaction(label, transaction)?;
            for ((id, mesh), (_, before)) in edits.iter().zip(befores.iter()) {
                let mut after = before.clone();
                // A mesh edit product is a mesh row, never exact B-rep geometry and
                // never the source's preview class key.
                after.object.type_key = MESH_TYPE_KEY.to_string();
                after.geometry = SemanticGeometry::Mesh(mesh.clone());
                staged.update_entity(after)?;
                objects.push(ObjectId(id.0));
            }
            let change_set = staged.commit()?;
            let patches = befores
                .into_iter()
                .map(|(id, before)| {
                    let after = candidate.entity(id).cloned().ok_or_else(|| {
                        CadError::Invariant(format!("edited mesh {} vanished", id.0))
                    })?;
                    Ok(drawing_patch(id, Some(before), Some(after)))
                })
                .collect::<CadResult<Vec<_>>>()?;
            (change_set, patches, objects)
        };
        self.publish_candidate(
            document,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.into(),
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
        diagnostics.push(Diagnostic {
            object: None,
            code: "mesh.edit_applied".into(),
            message: format!(
                "{label}: committed in one transaction; the product is a non-exact mesh"
            ),
        });
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// `MOVE` on mesh face/edge refs; `None` keeps the ordinary whole-entity path.
    pub(crate) fn move_mesh_faces(
        &mut self,
        command: &Command,
    ) -> CadResult<Option<CommandOutcome>> {
        let CommandPayload::Move { refs, delta } = &command.payload else {
            return Ok(None);
        };
        let Some(drawing) = self
            .workspace
            .documents
            .get(&command.document)
            .map(|document| document.drawing.as_ref())
        else {
            return Ok(None);
        };
        let mut mesh_refs = 0usize;
        let mut non_mesh_sub = false;
        for reference in refs {
            if reference.sub_element.is_some() {
                match drawing.entity(reference.entity) {
                    Some(entity) if mesh_geometry(entity).is_some() => mesh_refs += 1,
                    _ => non_mesh_sub = true,
                }
            }
        }
        if mesh_refs == 0 {
            return Ok(None);
        }
        if non_mesh_sub || mesh_refs != refs.len() {
            return Err(invalid(
                "mesh.edit_mixed_targets",
                "MOVE cannot combine mesh faces with whole entities or non-mesh sub-elements in \
                 one command",
            ));
        }
        let targets = resolve_mesh_targets(drawing, command.document, refs)?;
        let Some((entity, keys)) = targets.first() else {
            return Ok(None);
        };
        if targets.len() > 1 {
            return Err(unsupported(
                "mesh.edit_multiple_entities",
                "MOVE on mesh faces supports one mesh entity per command",
            ));
        }
        if keys.is_empty() {
            return Ok(None);
        }
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        let mesh = mesh_geometry(entity).expect("checked above");
        let edge_keys = keys
            .iter()
            .filter(|key| key.starts_with("mesh:edge:"))
            .count();
        if edge_keys != 0 && edge_keys != keys.len() {
            return Err(invalid(
                "mesh.edit_mixed_targets",
                "MOVE cannot mix mesh edge and mesh face sub-elements in one command",
            ));
        }
        let (product, code, what) = if edge_keys == keys.len() {
            (
                cad_mesh_edit::move_edges(mesh, &keys, *delta)?,
                "mesh.move_edges",
                format!("{} mesh edge(s)", keys.len()),
            )
        } else {
            (
                cad_mesh_edit::move_faces(mesh, &keys, *delta)?,
                "mesh.move_faces",
                format!("{} source face(s)", keys.len()),
            )
        };
        let diagnostics = vec![Diagnostic {
            object: Some(ObjectId(entity.id.0)),
            code: code.into(),
            message: format!("moved {what} by ({}, {}, {})", delta.x, delta.y, delta.z),
        }];
        Ok(Some(self.commit_mesh_edits(
            command.document,
            vec![(entity.id, product)],
            "move mesh faces",
            diagnostics,
        )?))
    }

    /// `PRESSPULL` on a mesh face; `None` keeps the exact-solid path.
    pub(crate) fn press_pull_mesh_face(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<Option<CommandOutcome>> {
        let CommandPayload::PressPullFace { body, .. } = &command.payload else {
            return Ok(None);
        };
        let Some(key) = body.sub_element.as_ref().map(|sub| sub.source_key.clone()) else {
            return Ok(None);
        };
        let Some(drawing) = self
            .workspace
            .documents
            .get(&command.document)
            .map(|document| document.drawing.as_ref())
        else {
            return Ok(None);
        };
        let Some(entity) = drawing.entity(body.entity).cloned() else {
            return Err(invalid(
                "presspull.selection_changed",
                format!("entity {} no longer exists", body.entity.0),
            ));
        };
        let Some(mesh) = mesh_geometry(&entity) else {
            return Ok(None);
        };
        let distance = crate::app_press_pull::validate_press_pull_distance(
            session
                .tool_parameter(ToolParameter::PressPullDistance)
                .number()
                .unwrap_or(0.0),
        )?;
        let product = cad_mesh_edit::extrude_faces(mesh, &[key.as_str()], distance)?;
        let diagnostics = vec![Diagnostic {
            object: Some(ObjectId(entity.id.0)),
            code: "mesh.presspull".into(),
            message: format!(
                "press/pulled source face {key:?} by {distance} along its outward normal"
            ),
        }];
        Ok(Some(self.commit_mesh_edits(
            command.document,
            vec![(entity.id, product)],
            "press/pull mesh face",
            diagnostics,
        )?))
    }

    /// `MESHREFINE`: subdivide the selected meshes (or their selected faces).
    pub(crate) fn refine_meshes(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "MESHREFINE")?;
        let refs = session.selection.refs();
        let drawing = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref();
        let targets = resolve_mesh_targets(drawing, command.document, refs)?;
        let mut edits = Vec::with_capacity(targets.len());
        let mut diagnostics = Vec::new();
        for (entity, keys) in targets {
            let mesh = mesh_geometry(&entity).expect("checked");
            let keys = if keys.is_empty() {
                None
            } else {
                Some(keys.iter().map(String::as_str).collect::<Vec<_>>())
            };
            let product = cad_mesh_edit::subdivide(mesh, keys.as_deref())?;
            diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "mesh.refine".into(),
                message: "subdivided every selected triangle into four".to_string(),
            });
            edits.push((entity.id, product));
        }
        self.commit_mesh_edits(command.document, edits, "refine mesh", diagnostics)
    }

    /// `MESHSMOOTH`: one Laplacian smoothing pass over each selected mesh.
    pub(crate) fn smooth_meshes(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "MESHSMOOTH")?;
        let refs = session.selection.refs();
        if refs.iter().any(|reference| reference.sub_element.is_some()) {
            return Err(unsupported(
                "meshsmooth.sub_element_not_supported",
                "MESHSMOOTH smooths a whole mesh; select the mesh row without a face sub-element",
            ));
        }
        let drawing = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref();
        let targets = resolve_mesh_targets(drawing, command.document, refs)?;
        let mut edits = Vec::with_capacity(targets.len());
        let mut diagnostics = Vec::new();
        for (entity, _) in targets {
            let mesh = mesh_geometry(&entity).expect("checked");
            let product = cad_mesh_edit::smooth(mesh, 1, 0.5)?;
            diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "mesh.smooth".into(),
                message: "smoothing pass completed; boundary vertices stay pinned".into(),
            });
            edits.push((entity.id, product));
        }
        self.commit_mesh_edits(command.document, edits, "smooth mesh", diagnostics)
    }

    /// `MESHCAP`: fill simple planar boundary loops of the one selected mesh.
    pub(crate) fn cap_mesh(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "MESHCAP")?;
        let refs = session.selection.refs();
        if refs.iter().any(|reference| reference.sub_element.is_some()) {
            return Err(unsupported(
                "meshcap.sub_element_not_supported",
                "MESHCAP fills whole boundary loops; select the mesh row without a face sub-element",
            ));
        }
        let drawing = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref();
        let targets = resolve_mesh_targets(drawing, command.document, refs)?;
        if targets.len() != 1 {
            return Err(invalid(
                "meshcap.needs_one",
                format!("MESHCAP fills one mesh at a time, got {}", targets.len()),
            ));
        }
        let (entity, _) = targets.into_iter().next().expect("one target");
        let mesh = mesh_geometry(&entity).expect("checked");
        let result = cad_mesh_edit::fill_holes(mesh)?;
        cad_mesh_edit::require_filled(&result)?;
        let mut diagnostics = Vec::new();
        for (index, hole) in result.holes.iter().enumerate() {
            diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: if hole.filled {
                    "mesh.hole_filled".into()
                } else {
                    "mesh.hole_refused".into()
                },
                message: format!(
                    "hole {}: {} vertex/vertices, {} triangle(s): {}{}",
                    index + 1,
                    hole.boundary_vertices,
                    hole.triangles,
                    hole.detail,
                    hole.code
                        .map(|code| format!(" [{code}]"))
                        .unwrap_or_default()
                ),
            });
        }
        diagnostics.push(Diagnostic {
            object: Some(ObjectId(entity.id.0)),
            code: if result.topology.class.is_closed() {
                "mesh.cap_closed".into()
            } else {
                "mesh.cap_partial".into()
            },
            message: format!(
                "filled {} hole(s), refused {}; product is {}",
                result.filled,
                result.refused,
                result.topology.class.as_str()
            ),
        });
        self.commit_mesh_edits(
            command.document,
            vec![(entity.id, result.mesh)],
            "cap mesh holes",
            diagnostics,
        )
    }
}
