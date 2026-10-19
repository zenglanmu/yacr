//! EXPLODE edit command (ADR 0016 decision G).
//!
//! EXPLODE runs on the **current selection** with no extra prompt (AutoCAD asks
//! nothing else), carries no typed parameter and captures no point, Work-only,
//! and commits **one transaction with one undo step**: the sources are deleted
//! and their decomposed children are inserted. The whole command is
//! **all-or-nothing** — a refusal writes nothing and never partially succeeds.
//!
//! ## Decomposition (one level at a time)
//!
//! * **`Polyline` → straight segments + arcs.** Each segment is a LINE; each
//!   bulge segment is recovered as an ARC through
//!   [`cad_geometry::bulge_arc_geometry`] (the single arc-recovery
//!   implementation), in the polyline's own plane
//!   ([`cad_geometry::polyline_plane_normal`], the same normal the tessellator
//!   uses). Polyline **width is not modelled at all**, so nothing is discarded
//!   here (it was already lost at import).
//! * **`Insert` → its direct block members (exactly one level)**, with the
//!   placement transform **baked in** through the existing [`transform_geometry`]
//!   surface. A member that is itself a block reference (a nested INSERT or a
//!   MINSERT instance) **stays a block reference**: only the placement is
//!   composed into its transform, and its contents are produced only by
//!   exploding that reference again — AutoCAD's "remove one grouping level at a
//!   time". Because the walk never descends into a member's block definition, the
//!   export layer's block-reference **cycle stack** and **depth cap** are **not
//!   needed here** (a self-referential block exploded once simply yields the
//!   self-reference again, so no infinite loop exists); this deliberately differs
//!   from the export layer, which flattens a block reference to leaf geometry.
//!   Members keep their **own block-definition layers** (AutoCAD consistent).
//!   Dynamic-block member visibility follows the export layer's active-state rule
//!   ([`DrawingDatabase::block_visible_entities`]); AutoCAD's own dynamic-block
//!   EXPLODE behaviour is **unverified**, so this is our declared choice.
//! * **`Compound` → every child (one level).** A hatch imports as a `Compound`
//!   whose children are the boundary loops **plus** the fill mesh and pattern
//!   polylines, so decomposing to loops only would be a **silent loss**; all
//!   children are produced. This is a deliberate divergence from AutoCAD's hatch
//!   explosion (whose product is unverified).
//! * **MINSERT** imports as `Compound(Insert…)`, so the one-level rule handles it
//!   naturally: the first EXPLODE yields the Insert instances, the next yields
//!   their members.
//!
//! ## Refusals (explicit, nothing written)
//!
//! * A selection with nothing decomposable, or any selected entity that is not a
//!   decomposable kind (only LWPOLYLINE / INSERT / Compound are) — an explicit
//!   refusal, never a silent success.
//! * `TEXT` / `SPLINE` / `SHAPE` / `OPAQUE` and mirrored `Arc`/`Ellipse`
//!   children, via the existing `transform_geometry` refusal surface. A
//!   `Compound` member of an INSERT (for example an in-block MINSERT or hatch) is
//!   likewise untransformable and fails the whole command.
//! * `IMAGE` / `MASK` by **policy**: AutoCAD does not explode raster images (its
//!   wording is **unverified**). `Image`/`Mask` **are** transformable, so this is
//!   a policy refusal, **not** the transform layer.
//! * `Dimension` / `MTable` / `MLeader`-like compounds, refused explicitly by
//!   `type_key` (they import as `Compound`, so the general rule would otherwise
//!   decompose them; the project does not model the dimension-explosion
//!   semantics, and their children usually include `Text`).
//! * A non-uniformly scaled INSERT.
//! * An INSERT whose members include an untransformable kind — the whole command
//!   fails.
//!
//! NOT IMPLEMENTED: exploding more than one level at once, `EXPLMODE`, colour /
//! linetype / lineweight changes, Dimension decomposition, associative-array
//! decomposition, attribute semantics, xref semantics.

use super::*;

/// The similarity tolerance for the uniform-scale INSERT check: the same
/// judgement the transform surface uses (`cad_db::transform::SIMILARITY_TOLERANCE`).
const INSERT_SIMILARITY_TOLERANCE: f64 = 1e-9;

/// Below this a polyline bulge is a straight segment (the tessellator's rule).
const STRAIGHT_BULGE: f64 = 1e-12;

/// One child entity produced by a decomposition, before it is given a fresh id.
struct Produced {
    layer: LayerId,
    space: SpaceId,
    type_key: String,
    geometry: SemanticGeometry,
}

impl Application {
    /// EXPLODE the current selection into its decomposed children (ADR 0016
    /// decision G).
    ///
    /// The selection is decomposed **before any write**; a refusal (an empty
    /// selection, a non-decomposable kind, an untransformable child, an
    /// image/mask policy refusal or a non-uniform INSERT) changes nothing. On
    /// success the sources are deleted and every child is inserted in **one
    /// transaction with one undo step**.
    pub(crate) fn explode_entities(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "ExplodeEntities takes no payload".into(),
            ));
        }
        let refs = session.selection.refs().to_vec();
        if refs.is_empty() {
            return Err(CadError::InvalidInput(
                "EXPLODE needs a non-empty selection".into(),
            ));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        let (sources, mut diagnostics) = Self::resolve_selection(
            drawing,
            document_id,
            &refs,
            "explode",
            "ExplodeEntities",
            // The action slot only reaches the pre-existing sub-element/instance
            // diagnostic templates, so it stays ASCII (the ADR fence forbids new
            // CJK literals on the Rust side).
            "explode",
            false,
        )?;

        // Capture the display state the delete will prune, so undo restores it.
        let display_states: Vec<_> = sources
            .iter()
            .map(|source| drawing.entity_display_state(source.id))
            .collect();

        // Decompose the whole selection before any write: EXPLODE is
        // all-or-nothing, so a single non-decomposable source fails the command
        // rather than being silently dropped.
        let mut produced: Vec<Produced> = Vec::new();
        for source in &sources {
            explode_source(drawing, source, &mut produced)?;
        }
        if produced.is_empty() {
            return Err(CadError::InvalidInput(
                "EXPLODE produced no geometry from the selection".into(),
            ));
        }

        // Fresh ids and draw orders for every child.
        let base_order = next_explode_draw_order(drawing);
        let mut children: Vec<(EntityId, DbEntity)> = Vec::with_capacity(produced.len());
        for (index, child) in produced.into_iter().enumerate() {
            let new_id = drawing.allocate_entity_id();
            let entity = DbEntity {
                object: cad_db::DbObject {
                    id: ObjectId(new_id.0),
                    type_key: child.type_key,
                    revision: Revision(0),
                    source_handle: None,
                },
                id: new_id,
                layer: child.layer,
                space: child.space,
                geometry: child.geometry,
                draw_order: base_order + index as i64,
            };
            children.push((new_id, entity));
        }

        // One transaction: delete every source, insert every child.
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("explode entities", transaction)?;
            for source in &sources {
                staged.delete_entity(source.id)?;
            }
            for (_, child) in &children {
                staged.insert_entity(child.clone())?;
            }
            staged.commit()?
        };

        // Delete patches (with the pruned display state) for the sources plus
        // insert patches for the children, so undo/redo restore both sides.
        let mut patches: Vec<DrawingPatch> = sources
            .iter()
            .zip(display_states)
            .map(|(source, state)| drawing_patch_deleted(source.id, source.clone(), state))
            .collect();
        patches.extend(
            children
                .iter()
                .map(|(id, child)| drawing_patch(*id, None, Some(child.clone()))),
        );

        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "explode entities".into(),
                patches,
                merge_key: None,
            },
        )?;

        diagnostics.push(Diagnostic {
            object: None,
            code: "drawing.explode".into(),
            message: format!(
                "EXPLODE: {} entities produced from {} sources (sources deleted)",
                children.len(),
                sources.len()
            ),
        });

        Ok(CommandOutcome {
            objects: children.iter().map(|(id, _)| ObjectId(id.0)).collect(),
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
        })
    }
}

/// The next free draw order above every model-space entity (the multi-entity
/// copy shape).
fn next_explode_draw_order(drawing: &DrawingDatabase) -> i64 {
    drawing
        .model_space()
        .iter()
        .map(|entity| entity.draw_order)
        .max()
        .map(|max| max.saturating_add(1))
        .unwrap_or(0)
}

/// Decompose one selected entity (one level), appending its children to `out`.
fn explode_source(
    drawing: &DrawingDatabase,
    source: &DbEntity,
    out: &mut Vec<Produced>,
) -> CadResult<()> {
    match &source.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => explode_polyline(source, points, bulges, *closed, out),
        SemanticGeometry::Insert { block, transform } => {
            explode_insert(drawing, &source.space, *block, transform, out)
        }
        SemanticGeometry::Compound(children) => {
            refuse_compound_type_key(source)?;
            for child in children {
                push_produced(source.layer, source.space.clone(), child.clone(), out)?;
            }
            Ok(())
        }
        SemanticGeometry::Image { .. } => Err(policy_refusal("IMAGE")),
        SemanticGeometry::Mask { .. } => Err(policy_refusal("MASK")),
        other => Err(CadError::Unsupported(format!(
            "EXPLODE cannot explode {}: only LWPOLYLINE, INSERT and Compound are \
             decomposable",
            geometry_kind(other)
        ))),
    }
}

/// Explode a polyline into one LINE per straight segment and one ARC per bulge
/// segment (ADR 0016 decision G).
///
/// The closing segment of a closed polyline is produced too. A bulge segment
/// whose chord or plane is degenerate (so no in-plane arc exists) is an explicit
/// refusal, never a silently straightened segment. The polyline **width is not
/// modelled**, so nothing is discarded here.
fn explode_polyline(
    source: &DbEntity,
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    out: &mut Vec<Produced>,
) -> CadResult<()> {
    if points.len() < 2 {
        return Err(CadError::InvalidInput(
            "EXPLODE cannot explode a polyline with fewer than two vertices".into(),
        ));
    }
    // The bulge arcs lie in the polyline's own plane; reuse the tessellator's
    // normal so the recovered arc is exactly the drawn arc.
    let plane_normal = cad_geometry::polyline_plane_normal(points);
    let count = points.len();
    let segments = if closed { count } else { count - 1 };
    for index in 0..segments {
        let a = points[index];
        let b = points[(index + 1) % count];
        let bulge = bulges.get(index).copied().unwrap_or(0.0);
        let geometry = if bulge.abs() <= STRAIGHT_BULGE {
            SemanticGeometry::Line { start: a, end: b }
        } else {
            cad_geometry::bulge_arc_geometry(a, b, bulge, plane_normal).ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "EXPLODE cannot recover the arc of polyline bulge segment {index} \
                     (degenerate chord or plane)"
                ))
            })?
        };
        push_produced(source.layer, source.space.clone(), geometry, out)?;
    }
    Ok(())
}

/// Explode one INSERT into its **direct** block members, baking the placement
/// transform into each (ADR 0016 decision G).
///
/// The walk is **exactly one level**: a member that is itself a block reference
/// (a nested INSERT or a MINSERT instance) is emitted **as a block reference**
/// with its own transform composed with the placement, and its contents are
/// produced only by exploding that reference again — AutoCAD's "remove one
/// grouping level at a time".
///
/// Because the walk never descends into a member's block definition, the export
/// layer's block-reference **cycle stack** and **depth cap are not needed here**:
/// a self-referential block exploded once simply yields the self-reference again,
/// so no infinite loop exists. (This deliberately differs from the export layer,
/// which flattens a block reference to leaf geometry.)
///
/// Members keep their own block-definition layer; the placement is baked through
/// the existing [`transform_geometry`] surface, so a member that the surface
/// cannot place (a `Text`/`Spline`/`Shape`/`Opaque`/`Compound` member, e.g. an
/// in-block MINSERT/hatch) fails the whole command.
fn explode_insert(
    drawing: &DrawingDatabase,
    space: &SpaceId,
    block: BlockId,
    transform: &Transform3,
    out: &mut Vec<Produced>,
) -> CadResult<()> {
    // A non-uniform scale cannot be baked into the members exactly; refuse rather
    // than shear them (ADR 0016 decision G).
    if !transform.is_uniform_scale(INSERT_SIMILARITY_TOLERANCE) {
        return Err(CadError::Unsupported(
            "EXPLODE refuses a non-uniformly scaled INSERT (its members cannot be \
             represented exactly)"
                .into(),
        ));
    }
    let definition = drawing.block(block).ok_or_else(|| {
        CadError::Unsupported(format!("EXPLODE: block {block:?} is not in the database"))
    })?;
    // Dynamic-block visibility follows the export layer's active-state rule; a
    // block without a descriptor exposes every member.
    let members: Vec<EntityId> = match drawing.block_visible_entities(block) {
        Some(visible) => visible,
        None => definition.entities.clone(),
    };
    for member in &members {
        let child = drawing.entity(*member).ok_or_else(|| {
            CadError::Unsupported(format!(
                "EXPLODE: block member {} is not in the database",
                member.0
            ))
        })?;
        match &child.geometry {
            // A nested block reference stays a reference: only the placement is
            // composed into its transform, and its contents await another EXPLODE.
            SemanticGeometry::Insert {
                block: nested,
                transform: nested_transform,
            } => {
                let geometry = SemanticGeometry::Insert {
                    block: *nested,
                    transform: transform.matrix_mul(nested_transform),
                };
                push_produced(child.layer, space.clone(), geometry, out)?;
            }
            other => {
                let placed = transform_geometry(other, transform).map_err(|error| {
                    CadError::Unsupported(format!(
                        "EXPLODE cannot place block member {} ({}): {error}",
                        member.0,
                        geometry_kind(other)
                    ))
                })?;
                push_produced(child.layer, space.clone(), placed, out)?;
            }
        }
    }
    Ok(())
}

/// Validate and append one produced child geometry.
///
/// `Image`/`Mask` are refused by **policy** (they are transformable, so this is
/// deliberately not the transform layer); a `Text`/`Spline`/`Shape`/`Opaque`/
/// `Compound` child is refused through the existing transform surface, reusing
/// its exact reason.
fn push_produced(
    layer: LayerId,
    space: SpaceId,
    geometry: SemanticGeometry,
    out: &mut Vec<Produced>,
) -> CadResult<()> {
    match &geometry {
        SemanticGeometry::Image { .. } => Err(policy_refusal("IMAGE")),
        SemanticGeometry::Mask { .. } => Err(policy_refusal("MASK")),
        SemanticGeometry::Text { .. }
        | SemanticGeometry::Spline { .. }
        | SemanticGeometry::Shape { .. }
        | SemanticGeometry::Opaque { .. }
        | SemanticGeometry::Compound(_) => Err(child_refusal(&geometry)),
        _ => {
            let type_key = class_name(&geometry).to_string();
            out.push(Produced {
                layer,
                space,
                type_key,
                geometry,
            });
            Ok(())
        }
    }
}

/// The `IMAGE`/`MASK` policy refusal: AutoCAD does not explode raster images,
/// but the official wording is **unverified**, and both kinds **are**
/// transformable, so this is a stated policy choice rather than a transform-layer
/// refusal.
fn policy_refusal(kind: &str) -> CadError {
    CadError::Unsupported(format!(
        "EXPLODE refuses {kind} by policy (AutoCAD does not explode raster images; its \
         wording is unverified). {kind} is transformable, so this is a policy refusal, \
         not the transform layer"
    ))
}

/// Refuse a produced child through the existing transform surface, reusing its
/// exact reason so the EXPLODE refusal never invents a different one.
fn child_refusal(geometry: &SemanticGeometry) -> CadError {
    match transform_geometry(geometry, &Transform3::identity()) {
        Err(CadError::Unsupported(reason)) => CadError::Unsupported(format!(
            "EXPLODE cannot explode {}: {reason}",
            geometry_kind(geometry)
        )),
        Err(other) => other,
        // Defensive: the matched kinds are all transform-refused, so a success
        // here is unreachable. Fall back to a stable, honest message.
        Ok(_) => CadError::Unsupported(format!(
            "EXPLODE cannot explode {}",
            geometry_kind(geometry)
        )),
    }
}

/// Refuse a `Dimension`/`MTable`/`MLeader`-like compound by its stored
/// `type_key`.
///
/// These import as a `Compound` (their `type_key` is `AcDbDimension` /
/// `AcDbTable` / `AcDbMultiLeader`), so the general "Compound → all children"
/// rule would otherwise decompose them. The project does not model the
/// dimension-explosion semantics (associativity loss + 2D solid/text output), and
/// their children usually include `Text` (which the transform surface would
/// refuse anyway), so an explicit `type_key` refusal is clearer than the generic
/// failure.
fn refuse_compound_type_key(source: &DbEntity) -> CadResult<()> {
    match source.object.type_key.as_str() {
        "AcDbDimension" | "AcDbTable" | "AcDbMultiLeader" => Err(CadError::Unsupported(format!(
            "EXPLODE refuses a {} compound by type_key: this project does not model the \
             dimension/table/mleader explosion semantics (and its children usually include \
             Text)",
            source.object.type_key
        ))),
        _ => Ok(()),
    }
}
