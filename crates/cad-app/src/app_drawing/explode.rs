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
//!   surface. A member that is itself a block reference (a nested INSERT) **stays
//!   a block reference**: only the placement is composed into its transform, and
//!   its contents are produced only by exploding that reference again —
//!   AutoCAD's "remove one grouping level at a time". Because the walk never
//!   descends into a member's block definition, no cycle stack or depth cap is
//!   needed: a self-referential block exploded once simply yields the
//!   self-reference again. Members keep their **own block-definition layers**
//!   (AutoCAD consistent). Block attribute values are dropped: this project has
//!   no attribute model, which is the global rule for every INSERT.
//! * **Non-uniformly scaled INSERT → the deterministic subset.** AutoCAD warns
//!   that the result may be unexpected; instead of refusing the whole reference,
//!   every member with an **exact** image under the composed transform is placed.
//!   A `Circle`/`Arc`/`Ellipse` member whose image is an ellipse/elliptical arc
//!   is converted to the domain's exact `Ellipse` representation (see
//!   [`affine_conic_image`]). Any other member (a sheared `Text`, a polyline with
//!   bulge arcs, a `Shape`/`Opaque`/`Compound`) is a **named refusal that lists
//!   the member id and kind**, and the whole command writes nothing.
//! * **`Dimension` → its derived display children.** The dimension loses
//!   associativity and becomes real entities: extension/dimension lines as
//!   `LINE`, the angular arc as `ARC`, the arrow triangles as `MESH` (the domain
//!   has no 2D-solid variant; AutoCAD's exact arrow kind is **NOT modelled**),
//!   and the measurement text as `TEXT`. Each child keeps the dimension's
//!   **layer, space and draw order**, then the source is deleted. The text is not
//!   decomposed further.
//! * **`MultiLeader` → leader lines + text.** Straight path segments and the
//!   dogleg become `LINE`, a spline path stays the exact `SPLINE` display child,
//!   arrowheads stay their triangle `MESH`, and the annotation text becomes
//!   `MTEXT` (`AcDbMText`). Children keep the source's layer/space/draw order.
//! * **`Compound` → every child (one level).** A hatch imports as a `Compound`
//!   whose children are the boundary loops **plus** the fill mesh and pattern
//!   polylines, so decomposing to loops only would be a **silent loss**; all
//!   children are produced. This is a deliberate divergence from AutoCAD's hatch
//!   explosion (whose product is unverified).
//!
//! ## Refusals (explicit, nothing written)
//!
//! * A selection with nothing decomposable, or any selected entity that is not a
//!   decomposable kind.
//! * `TEXT` / `SPLINE` / `SHAPE` / `OPAQUE` as **members** of an INSERT or
//!   compound (they are atomic display pieces, not decomposable further).
//! * `IMAGE` / `MASK` by **policy**: AutoCAD does not explode raster images
//!   (wording unverified); both **are** transformable, so this is a policy
//!   refusal, not the transform layer.
//! * A `Compound` carrying the legacy `AcDbDimension`/`AcDbTable`/
//!   `AcDbMultiLeader` type key (a real dimension/multileader uses its own
//!   semantic variant). `MTable` has no explosion model.
//! * A **MINSERT** (imports as `Compound(Insert…)`) and an **XREF** block
//!   reference: AutoCAD cannot explode either.
//! * A non-uniform INSERT member with no exact representation (named by member
//!   id and kind).
//!
//! NOT IMPLEMENTED: exploding more than one level at once, `EXPLMODE`, colour /
//! linetype / lineweight changes, associative-array decomposition, attribute
//! semantics, and a 2D-solid arrow representation (arrows stay `MESH`).

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
    /// The draw order the child must inherit, when the source kind prescribes
    /// one (`DIMENSION`/`MULTILEADER` display children keep their source's
    /// order); otherwise the command allocates a fresh ascending order.
    draw_order: Option<i64>,
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
                draw_order: child.draw_order.unwrap_or(base_order + index as i64),
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
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                layout_patches: Vec::new(),
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
            point_report: None,
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
        SemanticGeometry::Compound(children) => explode_compound(source, children, out),
        SemanticGeometry::Image { .. } => Err(policy_refusal("IMAGE")),
        SemanticGeometry::Mask { .. } => Err(policy_refusal("MASK")),
        // A DIMENSION loses its associativity and becomes its derived display
        // children (lines/arcs, arrow meshes and the measurement text), each a
        // real entity on the dimension's layer/space/draw order (ADR 0017 A.8).
        SemanticGeometry::Dimension(dimension) => explode_dimension(source, dimension, out),
        // A HATCH keeps today's decomposition: its derived display is the
        // boundary loops plus the fill mesh and pattern polylines, exactly the
        // children the old `Compound` produced (ADR 0018 E). The `type_key`
        // guard still refuses it inside a block.
        SemanticGeometry::Hatch(hatch) => {
            refuse_compound_type_key(source)?;
            for child in hatch.display() {
                push_produced(source.layer, source.space.clone(), child.clone(), out)?;
            }
            Ok(())
        }
        // A MULTILEADER becomes its derived display: leader paths (their
        // straight segments as LINEs, a spline path as the SPLINE it already
        // is), arrowhead triangle meshes, doglegs and the annotation text as
        // MTEXT. Parts this build cannot represent are named refusals.
        SemanticGeometry::MultiLeader(leader) => explode_multileader(source, leader, out),
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
    for geometry in polyline_segments(points, bulges, closed)? {
        push_produced(source.layer, source.space.clone(), geometry, out)?;
    }
    Ok(())
}

/// One LINE per straight segment and one ARC per bulge segment (in segment
/// order, including the closing segment of a closed polyline).
///
/// A bulge segment whose chord or plane is degenerate (so no in-plane arc
/// exists) is an explicit refusal, never a silently straightened segment. The
/// polyline **width is not modelled**, so nothing is discarded here.
fn polyline_segments(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
) -> CadResult<Vec<SemanticGeometry>> {
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
    let mut out = Vec::with_capacity(segments);
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
        out.push(geometry);
    }
    Ok(out)
}

/// Explode one INSERT into its **direct** block members, baking the placement
/// transform into each (ADR 0016 decision G).
///
/// The walk is **exactly one level**: a member that is itself a block reference
/// (a nested INSERT) is emitted **as a block reference** with its own transform
/// composed with the placement, and its contents are produced only by exploding
/// that reference again — AutoCAD's "remove one grouping level at a time".
///
/// Because the walk never descends into a member's block definition, the export
/// layer's block-reference **cycle stack** and **depth cap are not needed here**:
/// a self-referential block exploded once simply yields the self-reference again,
/// so no infinite loop exists. (This deliberately differs from the export layer,
/// which flattens a block reference to leaf geometry.)
///
/// Members keep their own block-definition layer. The placement is baked through
/// the existing [`transform_geometry`] surface, which handles `Line`, a
/// straight-segment `Polyline`, `Point`, `Mesh`, `Spline`, `Image`, `Mask` and
/// the similarity-only conics exactly.
///
/// **Non-uniform scale.** AutoCAD warns that a non-uniformly scaled block's
/// exploded result may be unexpected; this build implements the deterministic
/// subset instead of refusing the whole reference:
///
/// * every member whose geometry has an exact image under the composed transform
///   is placed (the `transform_geometry` surface above);
/// * a `Circle`/`Arc`/`Ellipse` member whose exact image is an **ellipse** (or an
///   elliptical arc) is converted to one — the domain's `Ellipse { normal,
///   major_axis, ratio, start, sweep }` represents it exactly, so nothing is
///   approximated;
/// * any other member (a `Text` whose glyphs would be sheared, a polyline with
///   bulge arcs, a `Shape`/`Opaque`/`Compound`) is a **named refusal listing the
///   member id and kind**, and the whole command writes nothing.
///
/// A plain (non-nested) MINSERT never reaches this function: it imports as a
/// `Compound(Insert…)` and is refused by [`explode_compound`]. An XREF block
/// reference is refused here by name. Block attribute values are dropped (this
/// project has no attribute model), which is the global rule for every INSERT.
fn explode_insert(
    drawing: &DrawingDatabase,
    space: &SpaceId,
    block: BlockId,
    transform: &Transform3,
    out: &mut Vec<Produced>,
) -> CadResult<()> {
    // AutoCAD cannot explode an external reference, and this build has no
    // meaningful decomposition of one: refuse by name instead of emitting the
    // xref's cached members as ordinary geometry.
    if drawing.is_xref_block(block) {
        return Err(CadError::Unsupported(
            "EXPLODE refuses an XREF block reference: an external reference cannot be \
             exploded into this drawing's own entities"
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
    let uniform = transform.is_uniform_scale(INSERT_SIMILARITY_TOLERANCE);
    for member in &members {
        let child = drawing.entity(*member).ok_or_else(|| {
            CadError::Unsupported(format!(
                "EXPLODE: block member {} is not in the database",
                member.0
            ))
        })?;
        // An in-block MULTILEADER is refused explicitly by its stored type key,
        // not as a side effect of `transform_geometry`: the variant now has a
        // transform arm, so relying on that refusal would silently change the
        // behaviour (ADR 0019 decision G).
        if child.object.type_key == "AcDbMultiLeader" {
            return Err(CadError::Unsupported(format!(
                "EXPLODE refuses the in-block {} by type_key: a block member is not \
                 decomposed (only a top-level MULTILEADER becomes its display children)",
                child.object.type_key
            )));
        }
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
                let placed = match transform_geometry(other, transform) {
                    Ok(placed) => placed,
                    // A non-uniform scale has an exact affine image for every
                    // conic; the shared transform surface refuses it because it
                    // is a similarity-only API, so the deterministic subset is
                    // completed here.
                    Err(CadError::Unsupported(_)) if !uniform => {
                        affine_conic_image(other, transform).ok_or_else(|| {
                            CadError::Unsupported(format!(
                                "EXPLODE cannot place block member {} ({}): its exact image \
                                 under the non-uniform placement is not representable",
                                member.0,
                                geometry_kind(other)
                            ))
                        })?
                    }
                    Err(error) => {
                        return Err(CadError::Unsupported(format!(
                            "EXPLODE cannot place block member {} ({}): {error}",
                            member.0,
                            geometry_kind(other)
                        )))
                    }
                };
                push_produced(child.layer, space.clone(), placed, out)?;
            }
        }
    }
    Ok(())
}

/// The exact affine image of a `Circle`/`Arc`/`Ellipse` as an `Ellipse`, or
/// `None` when the image collapses (or the source kind is not a conic).
///
/// The source is written as `center + cos θ · p + sin θ · q` (the same
/// parameterisation the tessellator uses: the AutoCAD arbitrary-axis frame for a
/// circle/arc, the stored major/minor vectors for an ellipse). An affine map
/// sends that to `center' + cos θ · p' + sin θ · q'`, which is an ellipse with
/// conjugate semi-diameters `p'`/`q'`. The principal axes come from the 2×2
/// eigenproblem of `(p·p, p·q, q·q)`; `normal` is the oriented plane normal
/// `p' × q'`, so a mirror reverses it exactly like the similarity path does.
/// The parameter offset between `θ` and the ellipse's own `start` is read from
/// the transformed start point; the sweep magnitude and sign are preserved
/// (the parameter map has slope +1 in both directions).
fn affine_conic_image(
    geometry: &SemanticGeometry,
    transform: &Transform3,
) -> Option<SemanticGeometry> {
    let (center, _normal, p, q, start, sweep) = match geometry {
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => {
            let (u, v, _) = cad_geometry::arbitrary_axis(*normal);
            (
                *center,
                *normal,
                cad_geometry::scale(u, radius.abs()),
                cad_geometry::scale(v, radius.abs()),
                0.0,
                std::f64::consts::TAU,
            )
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let (u, v, _) = cad_geometry::arbitrary_axis(*normal);
            (
                *center,
                *normal,
                cad_geometry::scale(u, radius.abs()),
                cad_geometry::scale(v, radius.abs()),
                *start,
                *sweep,
            )
        }
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            let major_len = cad_geometry::length(*major_axis);
            if major_len < 1e-12 {
                return None;
            }
            let u = cad_geometry::scale(*major_axis, 1.0 / major_len);
            // `minor = cross(normal, major_unit)`, the domain's own rule.
            let minor_unit =
                cad_geometry::normalize(cad_geometry::cross(cad_geometry::normalize(*normal), u));
            (
                *center,
                *normal,
                *major_axis,
                cad_geometry::scale(minor_unit, major_len * ratio.abs()),
                *start,
                *sweep,
            )
        }
        _ => return None,
    };

    let p = transform.apply_vector(p);
    let q = transform.apply_vector(q);
    let a = cad_geometry::dot(p, p);
    let b = cad_geometry::dot(p, q);
    let c = cad_geometry::dot(q, q);
    let mean = (a + c) * 0.5;
    let spread = ((a - c) * 0.5).powi(2) + b * b;
    if !mean.is_finite() || !spread.is_finite() || spread < 0.0 {
        return None;
    }
    let root = spread.sqrt();
    let major_sq = mean + root;
    let minor_sq = mean - root;
    let major_len = major_sq.max(0.0).sqrt();
    let minor_len = minor_sq.max(0.0).sqrt();
    if !major_len.is_finite() || !minor_len.is_finite() || minor_len < 1e-12 {
        return None;
    }
    // Eigenvector of the major axis in the (p, q) parameter basis.
    let (w1, w2) = if b.abs() > 1e-12 {
        (b, major_sq - a)
    } else if a >= c {
        (1.0, 0.0)
    } else {
        (0.0, 1.0)
    };
    let norm = (w1 * w1 + w2 * w2).sqrt();
    if !norm.is_finite() || norm < 1e-12 {
        return None;
    }
    let (w1, w2) = (w1 / norm, w2 / norm);
    let major_dir = cad_geometry::normalize(cad_geometry::add(
        cad_geometry::scale(p, w1),
        cad_geometry::scale(q, w2),
    ));
    let normal_image = cad_geometry::cross(p, q);
    if cad_geometry::length(normal_image) < 1e-12 {
        return None;
    }
    let normal_image = cad_geometry::normalize(normal_image);
    let minor_dir = cad_geometry::normalize(cad_geometry::cross(normal_image, major_dir));
    if cad_geometry::length(minor_dir) < 1e-12 {
        return None;
    }
    // Read the ellipse parameter at the transformed start point.
    let start_offset = cad_geometry::add(
        cad_geometry::scale(p, start.cos()),
        cad_geometry::scale(q, start.sin()),
    );
    let new_start = (cad_geometry::dot(start_offset, minor_dir) / minor_len)
        .atan2(cad_geometry::dot(start_offset, major_dir) / major_len);
    let ratio = minor_len / major_len;
    if !new_start.is_finite() || !ratio.is_finite() || ratio <= 0.0 {
        return None;
    }
    Some(SemanticGeometry::Ellipse {
        center: transform.apply_point(center),
        normal: normal_image,
        major_axis: cad_geometry::scale(major_dir, major_len),
        ratio,
        start: new_start,
        sweep,
    })
}

/// `Compound` decomposition (one level).
///
/// A compound whose members are all `Insert` instances is the importer's
/// MINSERT representation (`rows × columns` cells). AutoCAD cannot explode a
/// MINSERT, so it is a named refusal rather than a silent fan-out into block
/// references.
fn explode_compound(
    source: &DbEntity,
    children: &[SemanticGeometry],
    out: &mut Vec<Produced>,
) -> CadResult<()> {
    if !children.is_empty()
        && children
            .iter()
            .all(|child| matches!(child, SemanticGeometry::Insert { .. }))
    {
        return Err(CadError::Unsupported(
            "EXPLODE refuses a MINSERT (an insert array imports as a Compound of Insert \
             instances): AutoCAD cannot explode a MINSERT"
                .into(),
        ));
    }
    refuse_compound_type_key(source)?;
    for child in children {
        push_produced(source.layer, source.space.clone(), child.clone(), out)?;
    }
    Ok(())
}

/// The exact `DIMENSION` decomposition: every derived display child becomes a
/// real entity on the dimension's layer, space and draw order, then the source
/// is deleted by the caller.
///
/// The display already holds the extension/dimension lines, the angular arc, the
/// arrowhead triangle meshes (the domain has no 2D-solid variant, so the arrow
/// stays a `MESH`; AutoCAD's exact output kind is NOT modelled) and the
/// measurement text. The text is produced as `TEXT`, not decomposed further.
/// Every child is exact display geometry; a child kind this build cannot stand
/// alone is a named refusal.
fn explode_dimension(
    source: &DbEntity,
    dimension: &cad_domain::DimensionGeometry,
    out: &mut Vec<Produced>,
) -> CadResult<()> {
    let display = dimension.display();
    if display.is_empty() {
        return Err(CadError::Unsupported(
            "EXPLODE cannot explode a DIMENSION with no derived display geometry".into(),
        ));
    }
    for child in display {
        push_display_child(
            source.layer,
            source.space.clone(),
            source.draw_order,
            None,
            child.clone(),
            out,
        )?;
    }
    Ok(())
}

/// The `MULTILEADER` decomposition: leader paths, arrowhead meshes, doglegs and
/// the annotation text.
///
/// A `Polyline` path is exploded into its straight `LINE` segments (the display
/// only ever builds a straight path as a polyline), a `Spline` path stays the
/// `SPLINE` it already is, and the annotation text is produced as `MTEXT`
/// (`AcDbMText`, the source class of a leader's annotation). Arrowheads are the
/// derived triangle `MESH` (there is no 2D-solid variant). Parts with no
/// semantic form (`Shape`/`Opaque` and any nested compound) are named refusals.
fn explode_multileader(
    source: &DbEntity,
    leader: &cad_domain::MultiLeaderGeometry,
    out: &mut Vec<Produced>,
) -> CadResult<()> {
    let display = leader.display();
    if display.is_empty() {
        return Err(CadError::Unsupported(
            "EXPLODE cannot explode a MULTILEADER with no derived display geometry".into(),
        ));
    }
    for child in display {
        match child {
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
            } => {
                for geometry in polyline_segments(points, bulges, *closed)? {
                    push_display_child(
                        source.layer,
                        source.space.clone(),
                        source.draw_order,
                        None,
                        geometry,
                        out,
                    )?;
                }
            }
            SemanticGeometry::Text { .. } => push_display_child(
                source.layer,
                source.space.clone(),
                source.draw_order,
                Some(text_policy::MTEXT_TYPE_KEY),
                child.clone(),
                out,
            )?,
            other => push_display_child(
                source.layer,
                source.space.clone(),
                source.draw_order,
                None,
                other.clone(),
                out,
            )?,
        }
    }
    Ok(())
}

/// Append one **derived display** child (`DIMENSION`/`MULTILEADER`) as a real
/// entity with the source's layer, space and draw order.
///
/// Unlike [`push_produced`] this accepts `Text` and `Spline`: they are not being
/// decomposed further, they are already the exact display entity and can stand
/// alone. `type_key` overrides the stored class (`MULTILEADER` text is MTEXT).
fn push_display_child(
    layer: LayerId,
    space: SpaceId,
    draw_order: i64,
    type_key: Option<&str>,
    geometry: SemanticGeometry,
    out: &mut Vec<Produced>,
) -> CadResult<()> {
    match &geometry {
        SemanticGeometry::Image { .. }
        | SemanticGeometry::Mask { .. }
        | SemanticGeometry::Shape { .. }
        | SemanticGeometry::Opaque { .. }
        | SemanticGeometry::Compound(_)
        | SemanticGeometry::Dimension(_)
        | SemanticGeometry::Hatch(_)
        | SemanticGeometry::MultiLeader(_) => Err(child_refusal(&geometry)),
        _ => {
            out.push(Produced {
                layer,
                space,
                type_key: type_key
                    .unwrap_or_else(|| class_name(&geometry))
                    .to_string(),
                geometry,
                draw_order: Some(draw_order),
            });
            Ok(())
        }
    }
}

/// Validate and append one produced child geometry.
///
/// `Image`/`Mask` are refused by **policy** (they are transformable, so this is
/// deliberately not the transform layer); every other non-decomposable child is
/// refused by an **explicit named reason** in [`child_refusal`] — including
/// `Text`, whose transform is now supported (ADR 0019 decision B) and therefore
/// can no longer be refused as a side effect of `transform_geometry`.
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
        | SemanticGeometry::Dimension(_)
        | SemanticGeometry::Compound(_)
        | SemanticGeometry::Hatch(_)
        | SemanticGeometry::MultiLeader(_) => Err(child_refusal(&geometry)),
        _ => {
            let type_key = class_name(&geometry).to_string();
            out.push(Produced {
                layer,
                space,
                type_key,
                geometry,
                draw_order: None,
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

/// Refuse a produced child with its **own named reason** (never a transform
/// side effect).
///
/// ADR 0019 decision B changed the old mechanism: `Text` used to be refused by
/// `transform_geometry`, so an INSERT with a Text member failed implicitly. Text
/// is now transformable, so the refusal is made explicit here — the command still
/// fails as a whole (the behaviour is unchanged), but for a stated reason.
fn child_refusal(geometry: &SemanticGeometry) -> CadError {
    let reason = match geometry {
        SemanticGeometry::Text { .. } => {
            "EXPLODE does not decompose TEXT: a text run is one atomic glyph layout and \
             AutoCAD does not document it as an exploding kind"
        }
        SemanticGeometry::Spline { .. } => {
            "EXPLODE does not decompose SPLINE: the domain keeps control points/knots only, \
             so the exploded segments would not be the source curve"
        }
        SemanticGeometry::Shape { .. } => {
            "EXPLODE does not decompose a SHAPE: its glyph needs the compiled shape file"
        }
        SemanticGeometry::Opaque { .. } => {
            "EXPLODE does not decompose opaque/proxy geometry: it has no semantic form"
        }
        SemanticGeometry::Dimension(_) => {
            "EXPLODE does not decompose a DIMENSION member inside a compound in this build"
        }
        SemanticGeometry::Compound(_) => {
            "EXPLODE does not decompose a nested Compound member (for example an in-block \
             MINSERT or hatch) in this build"
        }
        SemanticGeometry::Hatch(_) => {
            "EXPLODE does not decompose a HATCH member inside a compound in this build"
        }
        SemanticGeometry::MultiLeader(_) => {
            "EXPLODE does not decompose a MULTILEADER member inside a compound in this build"
        }
        _ => "EXPLODE does not decompose this kind",
    };
    CadError::Unsupported(format!(
        "EXPLODE cannot explode {}: {reason}",
        geometry_kind(geometry)
    ))
}

/// Refuse a legacy `Compound` carrying a dimension/table/mleader `type_key`.
///
/// A real `DIMENSION`/`MULTILEADER` is its own [`SemanticGeometry`] variant and
/// is decomposed by [`explode_dimension`]/[`explode_multileader`]. A `Compound`
/// that merely carries the old key is a legacy/ambiguous representation: the
/// general "Compound → all children" rule would silently change its meaning, so
/// it stays an explicit `type_key` refusal. `AcDbTable` (MTable) has no
/// explosion model in this build at all.
fn refuse_compound_type_key(source: &DbEntity) -> CadResult<()> {
    match source.object.type_key.as_str() {
        "AcDbDimension" | "AcDbTable" | "AcDbMultiLeader" => Err(CadError::Unsupported(format!(
            "EXPLODE refuses a {} compound by type_key: a real {} entity uses its own \
             semantic variant; a Compound carrying this key is not decomposed",
            source.object.type_key, source.object.type_key
        ))),
        _ => Ok(()),
    }
}
