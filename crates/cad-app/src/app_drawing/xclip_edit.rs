//! `XCLIP`: a real **display-only** block-reference clip boundary.
//!
//! The boundary is stored in the selected `INSERT`'s render attributes
//! (`cad_domain::BlockClip`), in the **block definition's local coordinate
//! space**. That is what makes it follow the reference through move/rotate/scale
//! — the representation layer maps the ring through the composed INSERT
//! transform at expansion time and cuts the expanded geometry there.
//!
//! # Typed grammar
//!
//! * `XCLIP RECT <minx> <miny> <maxx> <maxy>` — block-local rectangle
//!   (`minx < maxx`, `miny < maxy`). It is stored verbatim and rendered as the
//!   **quad its four corners span**: an axis-aligned world rectangle keeps the
//!   dedicated half-plane clippers (byte-identical output), and a reference
//!   rotated or sheared in world XY is cut by that rotated/sheared quad, never
//!   by its axis-aligned envelope. Only a boundary that maps off the world-XY
//!   plane or through a degenerate transform is refused (`xclip.non_xy_plane`,
//!   `xclip.degenerate_transform`), because a region needs a plane.
//! * `XCLIP POLY <x1> <y1> …` — at least three block-local points, validated as
//!   one **simple** ring (no repeated vertex, no self-intersection, non-zero
//!   area) and rendered as that polygon, concave included. A ring that fails
//!   validation is refused by name (`xclip.degenerate_polygon`,
//!   `xclip.self_intersecting`, `xclip.non_coplanar`) and nothing is written.
//! * `XCLIP ON|OFF` — switch the stored boundary on/off.
//! * `XCLIP DELETE` — remove the boundary.
//! * `XCLIP INVERT` — refused (`xclip.invert_not_renderable`), never stored
//!   invisibly: the region an inverted clip keeps is the unbounded complement of
//!   its boundary, which has no region polygon here.
//! * `XCLIP ?` — read-only report of the selected INSERT's stored boundary.
//!
//! # Write contract
//!
//! One selected, directly addressable **model-space** `INSERT`. The write is a
//! single `update_entity_display` stage (the entity row is unchanged), so it
//! bumps exactly one revision, carries `ChangeMask::STYLE` (the representation
//! is rebuilt) and records exactly one undo step whose patch carries the
//! before/after **display state** — an undo restores the previous clip. Every
//! refusal happens before the transaction, so it writes nothing, bumps no
//! revision and records no history.
//!
//! # Stable refusal codes
//!
//! `xclip.payload` / `.no_selection` / `.multiple` / `.not_insert` /
//! `.no_boundary` / `.degenerate_rect` / `.degenerate_polygon` /
//! `.non_coplanar` / `.self_intersecting` / `.degenerate_transform` /
//! `.non_xy_plane` / `.invert_not_renderable`, and the no-op `.unchanged`.
//!
//! # Boundaries (declared, not hidden)
//!
//! Picking, selection and bounds still see the unclipped geometry; the clip is a
//! **display** filter only. Nothing is persisted to DXF/DWG (import/export are
//! untouched).
//!
//! Text and `WIPEOUT` masks inside a clipped block have no exact region cut
//! (a mask is an erase region with its own fill rule), so they stay drawn and
//! are reported by the representation layer under
//! `representation.xclip_primitive_unsupported` — the clip is never claimed for
//! them.

use super::*;

use crate::command_payload::XClipCommand;
use cad_domain::{BlockClip, SpaceId};
use cad_representation::layout::WorldRectBoundary;
use cad_representation::{block_clip_boundary, BlockClipRejection};

impl Application {
    /// Dispatch one typed `XCLIP` command (Work-only).
    pub(crate) fn xclip_edit(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::XClip(payload) = &command.payload else {
            return Err(xclip_refusal(
                "xclip.payload",
                "XCLIP needs an XClip payload (`RECT <minx> <miny> <maxx> <maxy>`, \
                 `POLY <x1> <y1> <x2> <y2> <x3> <y3> …`, `ON`, `OFF`, `DELETE`, `INVERT` or `?`)",
            ));
        };
        match payload.as_ref() {
            XClipCommand::Report => self.xclip_report(session, command),
            request => self.xclip_write(session, command, request),
        }
    }

    /// `XCLIP ?`: report the selected INSERT's stored boundary (writes nothing).
    fn xclip_report(&self, session: &SessionState, command: &Command) -> CadResult<CommandOutcome> {
        let (_, _, before) = self.one_selected_insert(session, command, "XCLIP", "xclip")?;
        let SemanticGeometry::Insert { transform, .. } = &before.geometry else {
            unreachable!("one_selected_insert verified the geometry");
        };
        let attributes = self
            .drawing(command.document)?
            .entity_render_attributes(before.id);
        let object = Some(ObjectId(before.id.0));
        let message = match attributes.clip.as_ref() {
            None => format!(
                "XCLIP ?: INSERT {} has no stored clip boundary (it draws every block member)",
                before.id.0
            ),
            Some(clip) => {
                let kind = if clip.is_rect() {
                    format!(
                        "rect ({}, {})..({}, {}) block-local",
                        clip.vertices[0][0],
                        clip.vertices[0][1],
                        clip.vertices[1][0],
                        clip.vertices[1][1]
                    )
                } else {
                    format!("{}-point polygon block-local", clip.vertices.len())
                };
                let world = match block_clip_boundary(clip, transform) {
                    Ok(Some(region)) => xclip_region_note(&region),
                    Ok(None) => "not applied (XCLIP OFF)".to_string(),
                    Err(rejection) => xclip_rejection_note(rejection).to_string(),
                };
                format!(
                    "XCLIP ?: INSERT {} has a {kind} clip ({}, inside), {world}",
                    before.id.0,
                    if clip.visible { "ON" } else { "OFF" }
                )
            }
        };
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object,
                code: "xclip.report".into(),
                message,
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// One boundary/switch write: one transaction, one undo step.
    fn xclip_write(
        &mut self,
        session: &SessionState,
        command: &Command,
        request: &XClipCommand,
    ) -> CadResult<CommandOutcome> {
        Self::validate_xclip_boundary(request)?;
        let (document_id, candidate, before) =
            self.one_selected_insert(session, command, "XCLIP", "xclip")?;
        self.write_block_clip(document_id, candidate, before, request)
    }

    /// The boundary-shape half of the pre-write validation: a malformed boundary
    /// is the same refusal with or without a selected INSERT, and it never opens
    /// a transaction.
    ///
    /// Shared with `CLIP`/`CLIPIT`, which resolve their own selection and then
    /// hand the mapped boundary to [`Self::write_block_clip`].
    pub(super) fn validate_xclip_boundary(request: &XClipCommand) -> CadResult<()> {
        if let XClipCommand::Rect([minx, miny, maxx, maxy]) = request {
            if ![minx, miny, maxx, maxy]
                .iter()
                .all(|value| value.is_finite())
                || minx >= maxx
                || miny >= maxy
            {
                return Err(xclip_refusal(
                    "xclip.degenerate_rect",
                    format!(
                        "the block-local rectangle must be finite with minx < maxx and \
                         miny < maxy, got [{minx}, {miny}] .. [{maxx}, {maxy}]"
                    ),
                ));
            }
        }
        if let XClipCommand::Polygon(points) = request {
            // The representation layer owns the one "is this a usable ring" rule
            // (`validate_simple_polygon`), so a ring it would refuse is refused
            // here, before a transaction, with this command's code prefix. That
            // covers `<3` points, a repeated vertex, zero area and a
            // self-intersection; the block plane is z = 0, so coplanarity is not
            // in question.
            if let Err(refusal) = cad_representation::layout::validate_simple_polygon(points) {
                return Err(xclip_refusal(
                    xclip_polygon_refusal_code(refusal.code),
                    refusal.message,
                ));
            }
        }
        Ok(())
    }

    /// The validated write half of [`Self::xclip_write`]: one INSERT display row,
    /// one transaction / one undo step.
    ///
    /// Split out so `CLIP`/`CLIPIT` reuse the block-clip model (`BlockClip`, the
    /// display-transaction path and its undo patch) instead of a second one.
    pub(super) fn write_block_clip(
        &mut self,
        document_id: DocumentId,
        mut candidate: cad_db::DrawingDatabase,
        before: DbEntity,
        request: &XClipCommand,
    ) -> CadResult<CommandOutcome> {
        let SemanticGeometry::Insert { transform, .. } = &before.geometry else {
            unreachable!("one_selected_insert verified the geometry");
        };
        xclip_plan_transform(transform)?;

        let current = candidate.entity_render_attributes(before.id).clip;
        let new_clip = match request {
            XClipCommand::Rect([minx, miny, maxx, maxy]) => {
                let clip = BlockClip {
                    vertices: vec![[*minx, *miny], [*maxx, *maxy]],
                    inside: true,
                    visible: true,
                };
                // The render path must accept the boundary, otherwise the
                // command would store a clip that never cuts. `block_clip_boundary`
                // is exactly the map the representation layer applies, so a
                // rotated/sheared rectangle passes (as the quad it is) and only a
                // boundary with no world-XY region is refused.
                match block_clip_boundary(&clip, transform) {
                    Ok(Some(_)) => Some(clip),
                    Ok(None) => unreachable!("a fresh ON/inside clip is always active"),
                    Err(rejection) => return Err(xclip_boundary_refusal(rejection)),
                }
            }
            XClipCommand::Polygon(points) => {
                let clip = BlockClip {
                    vertices: points.clone(),
                    inside: true,
                    visible: true,
                };
                // Same rule as the rectangle: the mapped region must exist. The
                // pre-write validator already refused a ring with no interior, so
                // this only catches the reference's own transform.
                match block_clip_boundary(&clip, transform) {
                    Ok(Some(_)) => Some(clip),
                    Ok(None) => unreachable!("a fresh ON/inside clip is always active"),
                    Err(rejection) => return Err(xclip_boundary_refusal(rejection)),
                }
            }
            XClipCommand::On => match current.as_ref() {
                None => return Err(xclip_no_boundary(before.id.0, "ON")),
                Some(clip) => Some(BlockClip {
                    visible: true,
                    ..clip.clone()
                }),
            },
            XClipCommand::Off => match current.as_ref() {
                None => return Err(xclip_no_boundary(before.id.0, "OFF")),
                Some(clip) => Some(BlockClip {
                    visible: false,
                    ..clip.clone()
                }),
            },
            XClipCommand::Delete => None,
            XClipCommand::Invert => {
                return Err(xclip_refusal(
                    "xclip.invert_not_renderable",
                    format!(
                        "XCLIP INVERT: an inverted clip region has no exact render path in this \
                         build, so entity {} was not modified (the boundary is not stored \
                         invisibly)",
                        before.id.0
                    ),
                ));
            }
            XClipCommand::Report => unreachable!("the report form is handled before the write"),
        };
        let object = Some(ObjectId(before.id.0));
        if new_clip == current {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object,
                    code: "xclip.unchanged".into(),
                    message: format!(
                        "XCLIP: INSERT {} already has this clip state; nothing was written",
                        before.id.0
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }

        let mut attributes = candidate.entity_render_attributes(before.id);
        attributes.clip = new_clip.clone();
        let label = if new_clip.is_some() {
            "xclip"
        } else {
            "xclip delete"
        };
        let transaction = next_transaction_id();
        let before_state = candidate.entity_display_state(before.id);
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction(label, transaction)?;
            staged.update_entity_display(before.id, attributes)?;
            staged.commit()?
        };
        let stored = candidate
            .entity(before.id)
            .cloned()
            .ok_or_else(|| CadError::Invariant("XCLIP: the INSERT vanished after commit".into()))?;
        let after_state = candidate.entity_display_state(before.id);
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.into(),
                patches: vec![DrawingPatch {
                    id: before.id,
                    before: Some(before.clone()),
                    after: Some(stored),
                    before_state: Some(before_state),
                    after_state: Some(after_state),
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

        let mut diagnostics = Vec::new();
        let (code, message) = match &new_clip {
            Some(clip) if clip.is_rect() => (
                "xclip.applied",
                format!(
                    "XCLIP: INSERT {} has a block-local rectangular clip ({}, {})..({}, {}), {}; \
                     the representation layer maps its four corners at expansion time (an \
                     axis-aligned world rectangle keeps the half-plane clippers, any other angle \
                     is cut as the quad it really is), so it follows the reference through \
                     move/rotate/scale",
                    before.id.0,
                    clip.vertices[0][0],
                    clip.vertices[0][1],
                    clip.vertices[1][0],
                    clip.vertices[1][1],
                    if clip.visible { "ON" } else { "OFF" }
                ),
            ),
            Some(clip) => (
                "xclip.applied",
                format!(
                    "XCLIP: INSERT {} stored a {}-point block-local polygonal clip, {}; the \
                     representation layer clips the expanded members to that ring (concave \
                     boundaries included) at expansion time",
                    before.id.0,
                    clip.vertices.len(),
                    if clip.visible { "ON" } else { "OFF" }
                ),
            ),
            None => (
                "xclip.removed",
                format!(
                    "XCLIP DELETE: cleared the clip boundary on INSERT {}",
                    before.id.0
                ),
            ),
        };
        diagnostics.push(Diagnostic {
            object,
            code: code.into(),
            message,
        });
        Ok(CommandOutcome {
            objects: vec![ObjectId(before.id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// The one directly addressable, model-space selected `INSERT`, plus a
    /// mutable drawing candidate cloned from the document.
    ///
    /// Shared by the report and every write so the selection refusals
    /// (`<code>.no_selection` / `.multiple` / `.not_insert`) are worded and coded
    /// once. A block member (an INSERT reached through another INSERT, or one
    /// living in block space) is refused: its clip would belong to the block
    /// definition, not to a visible reference.
    fn one_selected_insert(
        &self,
        session: &SessionState,
        command: &Command,
        name: &str,
        code: &str,
    ) -> CadResult<(DocumentId, cad_db::DrawingDatabase, DbEntity)> {
        let references = session.selection.refs();
        let reference = match references {
            [] => {
                return Err(xclip_refusal(
                    format!("{code}.no_selection"),
                    format!("{name} needs exactly one selected INSERT; nothing is selected"),
                ));
            }
            [one] => one,
            many => {
                return Err(xclip_refusal(
                    format!("{code}.multiple"),
                    format!(
                        "{name} edits exactly one INSERT; {} entities are selected",
                        many.len()
                    ),
                ));
            }
        };
        if reference.document != command.document
            || !reference.instance.0.is_empty()
            || reference.sub_element.is_some()
        {
            return Err(xclip_refusal(
                format!("{code}.not_insert"),
                format!(
                    "entity {} is not a directly addressable INSERT in this document (a block \
                     member has no clip of its own)",
                    reference.entity.0
                ),
            ));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let candidate = document.drawing.as_ref().clone();
        let before = candidate.entity(reference.entity).cloned().ok_or_else(|| {
            xclip_refusal(
                format!("{code}.not_insert"),
                format!(
                    "entity {} does not exist in this drawing",
                    reference.entity.0
                ),
            )
        })?;
        if before.space != SpaceId::Model {
            return Err(xclip_refusal(
                format!("{code}.not_insert"),
                format!(
                    "entity {} is a block member or paper-space reference, not a model-space \
                     INSERT",
                    before.id.0
                ),
            ));
        }
        if !matches!(before.geometry, SemanticGeometry::Insert { .. }) {
            return Err(xclip_refusal(
                format!("{code}.not_insert"),
                format!(
                    "entity {} is a {}, not an INSERT",
                    before.id.0,
                    class_name(&before.geometry)
                ),
            ));
        }
        Ok((document_id, candidate, before))
    }
}

/// One stable, code-prefixed refusal for `XCLIP`.
fn xclip_refusal(code: impl std::fmt::Display, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// The refusal for a boundary the render path cannot map to a world region.
///
/// `Polygon` and `Rotation` are the **axis-aligned oracle**'s answers, not
/// degradations: a polygon boundary and a rotated/sheared rectangle are both
/// rendered regions now, so [`block_clip_boundary`] never returns them and these
/// two arms exist only so the exhaustive match stays honest about that
/// distinction.
fn xclip_boundary_refusal(rejection: BlockClipRejection) -> CadError {
    match rejection {
        BlockClipRejection::Rotation => xclip_refusal(
            "xclip.rotation_unsupported_internal",
            "the block-local rectangle maps to a rotated or sheared world quad; that quad is a \
             renderable region, so this answer is not a refusal and no boundary was written",
        ),
        BlockClipRejection::Polygon => xclip_refusal(
            "xclip.polygon_unsupported_internal",
            "the boundary is a polygon; polygons are renderable regions, so this answer is not a \
             refusal and no boundary was written",
        ),
        BlockClipRejection::NonXyPlane => xclip_refusal(
            "xclip.non_xy_plane",
            "the block-local boundary does not map onto a plane parallel to world XY; the \
             representation clippers cut in world XY, so a tilted boundary has no region they \
             can express and nothing was written",
        ),
        BlockClipRejection::Degenerate => xclip_refusal(
            "xclip.degenerate_transform",
            "the INSERT transform is singular/non-finite or the mapped boundary has a repeated \
             vertex or no area, so nothing was written",
        ),
        BlockClipRejection::Inverted => xclip_refusal(
            "xclip.invert_not_renderable",
            "an inverted clip region keeps the unbounded outside of its boundary, which has no \
             region polygon in this build, so nothing was written",
        ),
        BlockClipRejection::Arity => xclip_refusal(
            "xclip.degenerate_polygon",
            "the block-local boundary has fewer than three points, so it is not a region and \
             nothing was written",
        ),
        BlockClipRejection::SelfIntersecting => xclip_refusal(
            "xclip.self_intersecting",
            "the block-local boundary crosses or touches itself, so it has no well-defined \
             interior and nothing was written",
        ),
    }
}

/// The `xclip.*` code one ring-validation refusal maps to.
///
/// The validator's own codes are the shared `viewport.*` family; this command's
/// contract keeps its `xclip.*` spelling, so the two stable code sets stay one
/// statement about the same rule.
fn xclip_polygon_refusal_code(code: &str) -> &'static str {
    match code {
        cad_representation::viewport_reason::POLYGON_SELF_INTERSECTING => "xclip.self_intersecting",
        cad_representation::viewport_reason::POLYGON_NON_PLANAR => "xclip.non_coplanar",
        _ => "xclip.degenerate_polygon",
    }
}

/// One short note describing why a stored boundary is not rendered.
fn xclip_rejection_note(rejection: BlockClipRejection) -> &'static str {
    match rejection {
        BlockClipRejection::Polygon => "rendered as a polygonal region (oracle answer only)",
        BlockClipRejection::Rotation => "rendered as the rotated quad (oracle answer only)",
        BlockClipRejection::NonXyPlane => "stored but not renderable (non-XY block plane)",
        BlockClipRejection::Degenerate => "stored but not renderable (degenerate transform)",
        BlockClipRejection::Inverted => "stored but not renderable (inverted)",
        BlockClipRejection::Arity => "stored but not renderable (fewer than three points)",
        BlockClipRejection::SelfIntersecting => "stored but not renderable (self-intersecting)",
    }
}

/// The world-space region a stored boundary maps to, as one report phrase.
///
/// A rectangle keeps its historical `world rect (…)..(…)` wording so an existing
/// reader cannot be surprised; every other shape names the ring it really is.
fn xclip_region_note(region: &WorldRectBoundary) -> String {
    if let Some((min, max)) = region.as_rect() {
        return format!(
            "world rect ({}, {})..({}, {})",
            min[0], min[1], max[0], max[1]
        );
    }
    match region.pieces() {
        [only] => format!(
            "world {}-point clip ring (concave/rotated boundaries included)",
            only.boundary().len()
        ),
        pieces => format!("world clip region of {} disjoint pieces", pieces.len()),
    }
}

/// Refuse an `ON`/`OFF` that has no stored boundary to switch.
fn xclip_no_boundary(entity: u128, option: &str) -> CadError {
    xclip_refusal(
        "xclip.no_boundary",
        format!(
            "XCLIP {option}: INSERT {entity} has no stored clip boundary; use \
             `XCLIP RECT …` or `XCLIP POLY …` first"
        ),
    )
}

/// Require the INSERT transform to be a finite, invertible **plan** transform.
///
/// A block clipped in its own XY plane is only meaningful in world space when
/// the reference maps that plane onto a plane parallel to world XY; a tilted
/// (3D-rotated) reference has no world rectangle to clip against. This is the
/// same rule [`block_clip_rect`] applies to the boundary itself.
fn xclip_plan_transform(transform: &Transform3) -> CadResult<()> {
    if transform.inverse().is_none() {
        return Err(xclip_refusal(
            "xclip.degenerate_transform",
            "the INSERT transform is singular or non-finite, so a block-local clip cannot be \
             mapped to world space; nothing was written",
        ));
    }
    let normal = transform.apply_vector(Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    });
    let length = normal.x.hypot(normal.y).hypot(normal.z);
    if !length.is_finite() || length <= 1e-12 {
        return Err(xclip_refusal(
            "xclip.degenerate_transform",
            "the INSERT transform maps the block's Z axis to zero length; nothing was written",
        ));
    }
    let tolerance = 1e-9 * length.max(1.0);
    if normal.x.abs() > tolerance || normal.y.abs() > tolerance {
        return Err(xclip_refusal(
            "xclip.non_xy_plane",
            format!(
                "the INSERT transform tilts the block plane (its Z axis maps to ({}, {}, {})); a \
                 block-local clip needs a plane parallel to world XY, so nothing was written",
                normal.x, normal.y, normal.z
            ),
        ));
    }
    Ok(())
}
