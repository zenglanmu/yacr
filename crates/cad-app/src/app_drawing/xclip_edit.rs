//! `XCLIP`: a real **display-only** block-reference clip boundary.
//!
//! The boundary is stored in the selected `INSERT`'s render attributes
//! (`cad_domain::BlockClip`), in the **block definition's local coordinate
//! space**. That is what makes it follow the reference through move/rotate/scale
//! — the representation layer maps the corners through the composed INSERT
//! transform at expansion time and cuts the expanded geometry there.
//!
//! # Typed grammar
//!
//! * `XCLIP RECT <minx> <miny> <maxx> <maxy>` — block-local rectangle
//!   (`minx < maxx`, `miny < maxy`). The four corners must map to an
//!   **axis-aligned world-XY rectangle** for this INSERT (0/90/180/270 degree
//!   rotations pass); anything else is `xclip.rotation_not_renderable` and
//!   writes nothing, because the renderer only cuts axis-aligned world
//!   rectangles exactly.
//! * `XCLIP POLY <x1> <y1> …` — at least three block-local points. The N-gon is
//!   validated and stored verbatim, but there is no exact polygon clip path in
//!   this build, so the command reports `xclip.polygon_not_renderable` and never
//!   claims the boundary is drawn (the same declared boundary `VPCLIP` has).
//! * `XCLIP ON|OFF` — switch the stored boundary on/off.
//! * `XCLIP DELETE` — remove the boundary.
//! * `XCLIP INVERT` — refused (`xclip.invert_not_renderable`), never stored
//!   invisibly: an inverted region has no exact render path here.
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
//! `.non_xy_plane` / `.invert_not_renderable` / `.rotation_not_renderable`, and
//! the no-op `.unchanged`.
//!
//! # Boundaries (declared, not hidden)
//!
//! Picking, selection and bounds still see the unclipped geometry; the clip is a
//! **display** filter only. Nothing is persisted to DXF/DWG (import/export are
//! untouched), and a polygonal boundary is stored but not renderable.

use super::*;

use crate::command_payload::XClipCommand;
use cad_domain::{BlockClip, SpaceId};
use cad_representation::{block_clip_rect, BlockClipRejection};

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
                let world = match block_clip_rect(clip, transform) {
                    Ok(Some(rect)) => format!(
                        "world rect ({}, {})..({}, {})",
                        rect.min[0], rect.min[1], rect.max[0], rect.max[1]
                    ),
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
        // Bound the geometry before the selection is read: a malformed boundary
        // is the same refusal with or without a selected INSERT, and it never
        // opens a transaction.
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
            let world: Vec<Point3> = points
                .iter()
                .map(|p| Point3 {
                    x: p[0],
                    y: p[1],
                    z: 0.0,
                })
                .collect();
            // The shared validator owns the three polygon refusals; its
            // `xclip.` prefix makes them the documented codes.
            crate::app_layout_geometry::validate_clip_polygon(&world, "xclip")?;
        }

        let (document_id, mut candidate, before) =
            self.one_selected_insert(session, command, "XCLIP", "xclip")?;
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
                // Rectangles must stay axis-aligned in world XY; otherwise the
                // renderer would have to guess. `block_clip_rect` is the same
                // rule the representation layer applies, so the command cannot
                // accept a boundary that would then render unclipped.
                match block_clip_rect(&clip, transform) {
                    Ok(Some(_)) => Some(clip),
                    Ok(None) => unreachable!("a fresh ON/inside clip is always active"),
                    Err(rejection) => return Err(xclip_boundary_refusal(rejection)),
                }
            }
            XClipCommand::Polygon(points) => Some(BlockClip {
                vertices: points.clone(),
                inside: true,
                visible: true,
            }),
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
        if matches!(request, XClipCommand::Polygon(_)) {
            // The boundary is real stored data but has no exact render path.
            diagnostics.push(Diagnostic {
                object,
                code: "xclip.polygon_not_renderable".into(),
                message: format!(
                    "XCLIP: stored a block-local polygonal boundary on INSERT {}; this build has \
                     no exact polygon clip path, so the reference still renders unclipped and no \
                     draw is claimed here",
                    before.id.0
                ),
            });
        }
        let (code, message) = match &new_clip {
            Some(clip) if clip.is_rect() => (
                "xclip.applied",
                format!(
                    "XCLIP: INSERT {} has a block-local rectangular clip ({}, {})..({}, {}), {}; \
                     the representation layer maps it to a world rectangle at expansion time, so \
                     it follows the reference through move/rotate/scale",
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
                    "XCLIP: INSERT {} stored a {}-point block-local polygonal clip, {}",
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

/// The refusal for a boundary the render path cannot map exactly.
fn xclip_boundary_refusal(rejection: BlockClipRejection) -> CadError {
    match rejection {
        BlockClipRejection::Rotation => xclip_refusal(
            "xclip.rotation_not_renderable",
            "the block-local rectangle maps to a rotated or sheared world quad; only \
             0/90/180/270 degree rotations are renderable, so nothing was written",
        ),
        BlockClipRejection::NonXyPlane => xclip_refusal(
            "xclip.non_xy_plane",
            "the block-local rectangle does not map onto a world-XY plane, so nothing was \
             written",
        ),
        BlockClipRejection::Degenerate => xclip_refusal(
            "xclip.degenerate_transform",
            "the INSERT transform is singular/non-finite or the mapped rectangle has no extent, \
             so nothing was written",
        ),
        BlockClipRejection::Inverted => xclip_refusal(
            "xclip.invert_not_renderable",
            "an inverted clip region has no exact render path in this build, so nothing was \
             written",
        ),
        BlockClipRejection::Polygon => xclip_refusal(
            "xclip.polygon_not_renderable",
            "a polygonal boundary has no exact render path in this build, so nothing was written",
        ),
    }
}

/// One short note describing why a stored boundary is not rendered.
fn xclip_rejection_note(rejection: BlockClipRejection) -> &'static str {
    match rejection {
        BlockClipRejection::Polygon => "stored but not renderable (polygon)",
        BlockClipRejection::Rotation => "stored but not renderable (rotated world quad)",
        BlockClipRejection::NonXyPlane => "stored but not renderable (non-XY block plane)",
        BlockClipRejection::Degenerate => "stored but not renderable (degenerate transform)",
        BlockClipRejection::Inverted => "stored but not renderable (inverted)",
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
