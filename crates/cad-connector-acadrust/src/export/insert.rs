//! BLOCK definitions, real `INSERT` records and the `XCLIP` object chain.
//!
//! The v1 exporter used to explode every block reference into its members,
//! which lost the block's identity, name and base point
//! (`export.converted.insert`) and left a stored `XCLIP` boundary with no block
//! reference to own its `ACAD_FILTER`/`SPATIAL`/`SPATIAL_FILTER` chain. This
//! module writes the real records instead:
//!
//! * **BLOCKS** — one `BLOCK_RECORD` (with its `BLOCK`/`ENDBLK` markers and its
//!   members, in block-local coordinates) for every definition a written
//!   reference reaches, recursively, each exactly once. A definition no written
//!   reference reaches is not written and is reported by `mod.rs`.
//! * **INSERT** — one record per representable reference, carrying the block
//!   name, insertion point, rotation, the three scale factors and the
//!   reference's own render attributes (layer, colour, linetype, lineweight,
//!   transparency, linetype scale). Non-uniform scale **is** representable
//!   (groups 41/42/43 are independent) and is written as such.
//! * **XCLIP** — the `INSERT → ACAD_XDICTIONARY → ACAD_FILTER → SPATIAL →
//!   SPATIAL_FILTER` chain, with the boundary in the reference's block-local
//!   space and the inverse block transform, so an imported clip round-trips.
//!
//! # Which references still explode (each declared, never silent)
//!
//! A reference cannot be expressed as one `INSERT` record when
//!
//! * the referenced definition has no registered table name/base point, so no
//!   `BLOCK_RECORD` can name it;
//! * the definition itself cannot be written (an external reference attachment,
//!   or a name the output document already owns);
//! * the placement is **mirrored** (`det < 0`) — a DXF `INSERT` carries scale
//!   and in-plane rotation only, and a mirror needs a negative scale factor
//!   this build refuses to emit;
//! * the placement **tilts** the block plane off the container's XY plane (a
//!   full 3D rotation: only an extrusion normal plus an in-plane rotation
//!   exists);
//! * the linear part has **shear**, or a zero/negative/non-finite scale factor;
//! * the reference closes a **block-definition cycle** (a DXF block definition
//!   cannot contain itself);
//! * the block nest is deeper than [`ExportLimits::max_explode_depth`].
//!
//! [`ExportLimits::max_explode_depth`]: super::report::ExportLimits::max_explode_depth
//!
//! Every such reference keeps the `export.converted.insert` entry with the
//! reason, and its members are still written (exploded), exactly as before.
//!
//! # Declared boundaries of the clip chain
//!
//! The written `SPATIAL_FILTER` carries the boundary ring, the boundary plane
//! normal (`+Z`), the display-enabled flag (`71`), the inverse block transform
//! and the front/back **clip distances** with their enabled flags (`40`/`72`,
//! `41`/`73`). Everything [`cad_domain::BlockClip`] stores round-trips; what the
//! object does not carry is declared rather than invented:
//!
//! * the boundary's local `origin` and `clip_bound_transform` are written as the
//!   neutral values (the boundary points are already in container space, which
//!   is what the reader maps back through the inverse block transform);
//! * a stored depth band is written as the WCS distance along `+Z` (the stored
//!   block-local value times the reference's `z` scale), and each `Some` side
//!   writes **its own flag plus distance** — a one-sided band cannot become a
//!   distance without a flag. A reference with no positive finite `z` scale has
//!   no such length: the boundary is still written and the depth-only loss is
//!   reported (`export.xclip_depth_dropped`), never invented;
//! * an **inverted** clip (`inside == false`) is refused by name: the format has
//!   no clip-mode field, so an inverted boundary cannot be told apart from an
//!   inside one;
//! * a reference that is exploded (any reason above) has nowhere to attach the
//!   chain, so its clip is reported with `export.xclip_clip_dropped`.

use acadrust::entities::Insert as DxfInsert;
use acadrust::objects::{Dictionary, ObjectType, SpatialFilter};
use acadrust::tables::BlockRecord;
use acadrust::types::{Handle, Matrix4, Vector2, Vector3};
use acadrust::{CadDocument, EntityType};
use cad_db::DbEntity;
use cad_domain::*;

use super::accumulator::{ExportAccumulator, WrittenBlock};
use super::common::{common_for, v3};
use super::entity::{emit, EntityContext};
use super::report::MappingOutcome;

/// Outcome of trying to write one block reference as a real `INSERT`.
pub(super) enum InsertEmission {
    /// One `INSERT` record was written (with its clip chain when it has a clip).
    Written,
    /// The reference is not representable as one record: the caller explodes it
    /// and declares `export.converted.insert` with this reason.
    Explode(String),
}

/// Why a referenced definition could not be written.
pub(super) enum BlockWriteError {
    /// A declared refusal: the reference converts (explodes) with this reason.
    Refused(String),
    /// A hard export failure (output limit, document add, ...).
    Failed(CadError),
}

/// The `INSERT` fields recovered from a stored reference placement.
struct Reference {
    insertion: Point3,
    rotation: f64,
    scale: [f64; 3],
}

/// Write one block reference as a real `INSERT` record when it is representable.
///
/// `placement` is the reference's transform in the space it is written into
/// (model space, or the containing block definition). On
/// [`InsertEmission::Explode`] the caller keeps the previous behaviour: expand
/// the members and record `export.converted.insert` with the returned reason;
/// nothing was written by this call.
pub(super) fn emit_insert(
    doc: &mut CadDocument,
    entity: &DbEntity,
    block: BlockId,
    placement: &Transform3,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
    stack: &mut Vec<BlockId>,
) -> CadResult<InsertEmission> {
    // The placement is checked before anything is written, so a reference that
    // cannot be one record never leaves an unreferenced definition behind.
    let Some(metadata) = ctx.database.block_metadata(block) else {
        return Ok(InsertEmission::Explode(format!(
            "block {block:?} has no registered table name and base point, so no BLOCK_RECORD can \
             name it"
        )));
    };
    let reference = match decompose(placement, metadata.base_point) {
        Ok(reference) => reference,
        Err(reason) => return Ok(InsertEmission::Explode(reason)),
    };
    let written = match ensure_block_written(doc, block, ctx, acc, stack) {
        Ok(written) => written,
        Err(BlockWriteError::Refused(reason)) => return Ok(InsertEmission::Explode(reason)),
        Err(BlockWriteError::Failed(error)) => return Err(error),
    };

    let (common, mut reasons) = common_for(doc, entity, ctx);
    let mut insert = DxfInsert::new(written.name.clone(), v3(reference.insertion));
    insert.set_x_scale(reference.scale[0]);
    insert.set_y_scale(reference.scale[1]);
    insert.set_z_scale(reference.scale[2]);
    insert.rotation = reference.rotation;
    // The exporter only writes plan placements; a tilted reference is exploded
    // above, so the extrusion normal is the world +Z.
    insert.normal = Vector3::UNIT_Z;
    insert.common = common;
    // The handle is allocated here (through the document's own allocator) because
    // the clip chain hangs off the reference's handle; `add_entity` keeps it.
    insert.common.handle = doc.allocate_handle();
    let handle = insert.common.handle;

    // ADR-0055: the reference's concrete attribute values are block-local, so
    // they attach to this `INSERT` unchanged. acadrust then writes group `66`,
    // the `ATTRIB` children and the terminating `SEQEND`.
    if let SemanticGeometry::Insert { attributes, .. } = &entity.geometry {
        let (written, attribute_reasons) = super::attribute::attributes_for(doc, attributes, ctx);
        insert.attributes = written;
        reasons.extend(attribute_reasons);
    }

    if let Some(clip) = ctx.database.entity_render_attributes(entity.id).clip {
        match write_clip(doc, handle, &clip, placement) {
            Ok((xdict, depth_loss)) => {
                insert.common.xdictionary_handle = Some(xdict);
                if let Some(reason) = depth_loss {
                    // The boundary itself was written; only the depth band could
                    // not be carried, and it says so under its own code.
                    acc.record_dropped_code(
                        Some(entity.object.id),
                        "export.xclip_depth_dropped",
                        vec![format!(
                            "the block reference's XCLIP boundary is written, but its depth band \
                             (block-local z {}..{}) is not: {reason}",
                            clip.front_clip
                                .map_or("-inf".to_string(), |value| value.to_string()),
                            clip.back_clip
                                .map_or("inf".to_string(), |value| value.to_string())
                        )],
                    );
                }
            }
            Err(reason) => acc.record_dropped_code(
                Some(entity.object.id),
                "export.xclip_clip_dropped",
                vec![format!(
                    "the block reference's XCLIP boundary ({} point(s), switched {}) is not \
                     written: {reason}",
                    clip.vertices.len(),
                    if clip.visible { "on" } else { "off" }
                )],
            ),
        }
    }
    if written.visibility_applied {
        reasons.push(
            "dynamic-block visibility applied: only the active state's members are written into \
             the block definition, so the other states' geometry is not in the file"
                .to_string(),
        );
    }

    acc.push(EntityType::Insert(insert), ctx)?;
    acc.written_inserts += 1;
    acc.record(
        Some(entity.object.id),
        "insert",
        if reasons.is_empty() {
            MappingOutcome::Exact
        } else {
            MappingOutcome::Converted(reasons)
        },
    );
    Ok(InsertEmission::Written)
}

/// Recover the `INSERT` fields a stored reference placement encodes.
///
/// The database bakes the block base point into every reference transform
/// (`transform = T(insertion) · R(rotation) · S(scale) · T(−base)`), so the
/// insertion point is the placement applied to the base point and the linear
/// part has to be exactly `R · S` in the block's XY plane. Anything else is a
/// declared refusal, never a silently distorted placement.
fn decompose(placement: &Transform3, base: Point3) -> Result<Reference, String> {
    let m = &placement.matrix;
    if m[3] != [0.0, 0.0, 0.0, 1.0] || m.iter().flatten().any(|value| !value.is_finite()) {
        return Err(
            "the reference transform is not a finite affine map, so no INSERT placement can \
             express it"
                .into(),
        );
    }
    if placement.inverse().is_none() {
        return Err(
            "the reference transform is singular, and a DXF INSERT scale factor cannot be zero"
                .into(),
        );
    }
    if placement.determinant() < 0.0 {
        return Err(
            "the reference is mirrored (negative determinant); a DXF INSERT carries an insertion \
             point, an in-plane rotation and positive scale factors only, so a mirror has no \
             record"
                .into(),
        );
    }
    let z_axis = placement.apply_vector(Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    });
    let z_length = z_axis.x.hypot(z_axis.y).hypot(z_axis.z);
    let normal_tolerance = 1e-9 * z_length.max(1.0);
    if z_axis.x.abs() > normal_tolerance || z_axis.y.abs() > normal_tolerance || z_axis.z <= 0.0 {
        return Err(format!(
            "the reference tilts the block's +Z axis to ({}, {}, {}); a DXF INSERT places the \
             block plane parallel to the container's XY plane, with only an in-plane rotation",
            z_axis.x, z_axis.y, z_axis.z
        ));
    }

    let sx = m[0][0].hypot(m[1][0]);
    let sy = m[0][1].hypot(m[1][1]);
    let sz = m[2][2];
    let rotation = m[1][0].atan2(m[0][0]);
    let (sin, cos) = rotation.sin_cos();
    // Rebuild the only linear part an INSERT can express and compare it with the
    // stored one: shear (and a mis-signed in-plane axis) is refused here instead
    // of being written as a rotation × scale that does not match.
    let magnitude = sx.max(sy).max(sz).max(1.0);
    let close = |actual: f64, expected: f64| (actual - expected).abs() <= 1e-9 * magnitude;
    if !close(m[0][0], cos * sx)
        || !close(m[1][0], sin * sx)
        || !close(m[0][1], -sin * sy)
        || !close(m[1][1], cos * sy)
        || !close(m[0][2], 0.0)
        || !close(m[1][2], 0.0)
        || !close(m[2][0], 0.0)
        || !close(m[2][1], 0.0)
    {
        return Err(
            "the reference linear part is not rotation × scale (it has shear or a skew), which \
             no combination of INSERT rotation and scale factors can express"
                .into(),
        );
    }
    if sx <= 1e-12 || sy <= 1e-12 || sz <= 1e-12 {
        return Err(
            "the reference collapses one axis (a scale factor at or below zero), which a DXF \
             INSERT cannot express"
                .into(),
        );
    }
    let insertion = placement.apply_point(base);
    if !insertion.x.is_finite() || !insertion.y.is_finite() || !insertion.z.is_finite() {
        return Err(
            "the reference maps its block base point to a non-finite insertion point".into(),
        );
    }
    Ok(Reference {
        insertion,
        rotation,
        scale: [sx, sy, sz],
    })
}

/// Write the definition of `block` (once), returning the name it was written as.
///
/// The definition is written with its members in block-local coordinates, so a
/// nested `INSERT` member is placed exactly as it is stored. Members are drained
/// into the block record as they are emitted, which is what keeps the block's
/// output order and lets a member's own `INSERT`/clip resolve immediately.
pub(super) fn ensure_block_written(
    doc: &mut CadDocument,
    block: BlockId,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
    stack: &mut Vec<BlockId>,
) -> Result<WrittenBlock, BlockWriteError> {
    if let Some(written) = acc.written_blocks.get(&block) {
        return Ok(written.clone());
    }
    if let Some(reason) = acc.refused_blocks.get(&block) {
        return Err(BlockWriteError::Refused(reason.clone()));
    }
    if stack.contains(&block) {
        return Err(BlockWriteError::Refused(format!(
            "the reference closes a block-definition cycle at {block:?}; a DXF block definition \
             cannot contain itself"
        )));
    }
    let Some(metadata) = ctx.database.block_metadata(block) else {
        return Err(refuse(
            acc,
            block,
            format!(
                "block {block:?} has no registered table name and base point, so no BLOCK_RECORD \
                 can name it"
            ),
        ));
    };
    if crate::support::is_space_block_name(&metadata.name) {
        return Err(refuse(
            acc,
            block,
            format!(
                "block {block:?} is named '{}', which is a DXF space block name, not a block \
                 definition",
                metadata.name
            ),
        ));
    }
    if doc.block_records.get(&metadata.name).is_some() {
        return Err(refuse(
            acc,
            block,
            format!(
                "block {block:?} is named '{}', but the output document already owns that name",
                metadata.name
            ),
        ));
    }
    let Some(definition) = ctx.database.block(block) else {
        return Err(refuse(
            acc,
            block,
            format!("block {block:?} is not in the database"),
        ));
    };
    if let Some(reference) = ctx.database.external_reference_for_block(block) {
        return Err(refuse(
            acc,
            block,
            format!(
                "external reference '{}' (raw path '{}', status '{}') is not written: the v1 DXF \
                 exporter emits no xref attachment record or path, so its content cannot be \
                 round-tripped",
                reference.name,
                reference.raw_path,
                reference.status.as_str()
            ),
        ));
    }
    if stack.len() + 1 > ctx.limits.max_explode_depth {
        return Err(BlockWriteError::Refused(format!(
            "the block nest is deeper than the {}-level explode-depth limit",
            ctx.limits.max_explode_depth
        )));
    }

    // Dynamic-block visibility: when a descriptor exists, only the members the
    // active state makes visible are written into the definition (declared on the
    // reference, which is where the loss belongs).
    let (members, visibility_applied) = match ctx.database.block_visible_entities(block) {
        Some(visible) => (visible, true),
        None => (definition.entities.clone(), false),
    };

    let block_handle = doc.allocate_handle();
    let mut record = BlockRecord::new(metadata.name.clone());
    record.handle = block_handle;
    record.block_entity_handle = doc.allocate_handle();
    record.block_end_handle = doc.allocate_handle();
    // ADR-0055: group 70 bit 2 tells a reader the definition owns `ATTDEF`
    // members. It is derived from the members actually written, never set on a
    // block that has none.
    record.flags.has_attributes = members.iter().any(|id| {
        ctx.database.entity(*id).is_some_and(|member| {
            matches!(member.geometry, SemanticGeometry::AttributeDefinition(_))
        })
    });
    // The base point the importer bakes into every reference transform, written
    // back verbatim so a reference re-imports to the same placement.
    record.base_point = v3(metadata.base_point);
    doc.block_records.add_or_replace(record);

    let written = WrittenBlock {
        name: metadata.name.clone(),
        visibility_applied,
    };
    // Registered before the members are walked: a member may reference this same
    // definition again (a cycle), and must be refused by `stack`, not loop.
    acc.written_blocks.insert(block, written.clone());
    stack.push(block);
    let mut failure: Option<CadError> = None;
    for member in &members {
        let Some(child) = ctx.database.entity(*member) else {
            acc.record(
                None,
                "insert_member",
                MappingOutcome::Dropped(vec![format!(
                    "block member {member:?} is not in the database; skipped"
                )]),
            );
            continue;
        };
        let start = acc.entities.len();
        if let Err(error) = emit(
            doc,
            child,
            &Transform3::identity(),
            stack.len(),
            ctx,
            acc,
            stack,
        ) {
            failure = Some(error);
            break;
        }
        for mut member_entity in acc.entities.split_off(start) {
            // An emitted member normally has no owner yet (it is a plain
            // geometric record). A writer that already placed its own output —
            // the DIMENSION content-block children, for example — keeps that
            // owner: overwriting it would move those records out of their block.
            if member_entity.common().owner_handle.is_null() {
                member_entity.common_mut().owner_handle = block_handle;
            }
            doc.add_entity(member_entity).map_err(|error| {
                BlockWriteError::Failed(CadError::InvalidInput(format!(
                    "add block member failed: {error}"
                )))
            })?;
        }
    }
    stack.pop();
    if let Some(error) = failure {
        return Err(BlockWriteError::Failed(error));
    }
    Ok(written)
}

/// Refuse a definition by name, remembering the reason for later references.
fn refuse(acc: &mut ExportAccumulator, block: BlockId, reason: String) -> BlockWriteError {
    acc.refused_blocks.insert(block, reason.clone());
    BlockWriteError::Refused(reason)
}

/// Write `INSERT → ACAD_FILTER → SPATIAL → SPATIAL_FILTER` for one reference.
///
/// `insert` is the handle of the just-built `INSERT`; the returned handle is its
/// extension dictionary, which the caller stores on the entity's common data
/// (that group is what makes the chain reachable from the record). Boundary
/// points are written in the reference's container space — the space the
/// inverse block transform maps back from — so the reader recovers exactly the
/// stored block-local ring.
///
/// The second return value is a **depth-only** loss: the boundary was written,
/// but `BlockClip`'s front/back distances could not be expressed as WCS lengths
/// (the reference has no positive finite `z` scale). The caller records it as
/// `export.xclip_depth_dropped`, so a depth-only loss never reads as "no clip".
fn write_clip(
    doc: &mut CadDocument,
    insert: Handle,
    clip: &BlockClip,
    placement: &Transform3,
) -> Result<(Handle, Option<String>), String> {
    if !clip.inside {
        return Err(
            "the stored clip is inverted (XCLIP INVERT), and AcDbSpatialFilter as modelled and \
             written (groups 70/10/20/210/11/71/72/73/40/41) has no clip-mode field, so an \
             inverted boundary cannot be told apart from an inside one"
                .into(),
        );
    }
    if clip.vertices.len() < 2 {
        return Err(format!(
            "the stored boundary has {} point(s); a rectangle needs two opposite corners and a \
             polygon at least three",
            clip.vertices.len()
        ));
    }
    let inverse = placement.inverse().ok_or_else(|| {
        "the reference transform is singular, so the WCS → block transform the filter stores does \
         not exist"
            .to_string()
    })?;
    let mut boundary = Vec::with_capacity(clip.vertices.len());
    for vertex in &clip.vertices {
        let point = placement.apply_point(Point3 {
            x: vertex[0],
            y: vertex[1],
            z: 0.0,
        });
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(
                "a stored boundary point maps to a non-finite point under the reference transform"
                    .into(),
            );
        }
        boundary.push(Vector2::new(point.x, point.y));
    }

    let xdict_handle = doc.allocate_handle();
    let filter_handle = doc.allocate_handle();
    let spatial_handle = doc.allocate_handle();

    let mut xdict = Dictionary::new();
    xdict.handle = xdict_handle;
    xdict.owner = insert;
    xdict.add_entry("ACAD_FILTER", filter_handle);

    let mut filter = Dictionary::new();
    filter.handle = filter_handle;
    filter.owner = xdict_handle;
    filter.add_entry("SPATIAL", spatial_handle);

    let mut spatial = SpatialFilter::new();
    spatial.handle = spatial_handle;
    spatial.owner = filter_handle;
    spatial.boundary_points = boundary;
    spatial.normal = Vector3::UNIT_Z;
    spatial.display_enabled = clip.visible;
    spatial.inverse_block_transform = Matrix4 { m: inverse.matrix };
    // Front/back clip distances (groups 72/40 and 73/41). `BlockClip` stores them
    // as a block-local z band, so each present side becomes the WCS length the
    // filter carries (the reference's positive z scale) **and** its enabled flag:
    // acadrust writes flag `72`/`73` from `Some`, which is exactly the
    // "distance only with its flag" rule the format requires — a one-sided band
    // sets one flag and one distance, never a distance without a flag.
    let mut depth_loss = None;
    match depth_world(placement, clip) {
        Ok((front, back)) => {
            spatial.front_clip = front;
            spatial.back_clip = back;
        }
        Err(reason) => depth_loss = Some(reason),
    }

    doc.objects
        .insert(xdict_handle, ObjectType::Dictionary(xdict));
    doc.objects
        .insert(filter_handle, ObjectType::Dictionary(filter));
    doc.objects
        .insert(spatial_handle, ObjectType::SpatialFilter(spatial));
    Ok((xdict_handle, depth_loss))
}

/// Convert a stored block-local depth band into the WCS distances the filter
/// carries, or name why the format cannot carry them.
///
/// The filter's distances run along the boundary's normal (written as `+Z`) from
/// the boundary plane, which is the reference's local `z = 0`, so the length is
/// the reference's `z` scale times the stored distance. A reference whose `z`
/// axis is not a positive finite scale (a degenerate or mirrored placement) has
/// no such length; the caller keeps the written boundary and reports the depth
/// band by name instead of inventing a value.
pub(super) fn depth_world(
    placement: &Transform3,
    clip: &BlockClip,
) -> Result<(Option<f64>, Option<f64>), String> {
    if clip.front_clip.is_none() && clip.back_clip.is_none() {
        return Ok((None, None));
    }
    let origin = placement.apply_point(Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
    let unit = placement.apply_point(Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    });
    let scale = unit.z - origin.z;
    if !scale.is_finite() || scale <= 0.0 {
        return Err(format!(
            "the reference maps the block's +Z to a {} z scale, so the stored front/back \
             distances ({}..{}) have no WCS length the SPATIAL_FILTER groups 40/41 can carry",
            if scale.is_finite() {
                scale.to_string()
            } else {
                "non-finite".to_string()
            },
            clip.front_clip
                .map_or("-inf".to_string(), |value| value.to_string()),
            clip.back_clip
                .map_or("inf".to_string(), |value| value.to_string())
        ));
    }
    let map = |distance: Option<f64>| distance.map(|value| value * scale);
    Ok((map(clip.front_clip), map(clip.back_clip)))
}
