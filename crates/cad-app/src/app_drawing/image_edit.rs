//! `IMAGEATTACH`, `IMAGECLIP` and `IMAGEADJUST`: real raster-image authoring on
//! the existing `SemanticGeometry::Image` pipeline.
//!
//! All three commands are **Work-only** and write through the normal drawing
//! transaction path: an attach inserts one entity through the shared
//! [`Application::commit_new_entity`] funnel (active layer, model space, one
//! transaction / one undo step), and a clip or adjustment updates one selected
//! image row in one transaction / one undo step. Every refusal happens before a
//! transaction, so a refused command writes nothing, bumps no revision and
//! records no history.
//!
//! `IMAGE` / `-IMAGE` are the read-only report and live in
//! `crate::app_extended_w16_reports`; the quality/editor names remain named
//! refusals in `crate::app_extended_w16_refusals`.
//!
//! Stable refusal codes:
//!
//! * `imageattach.payload` / `.unsafe_reference` / `.invalid_origin` /
//!   `.invalid_pixels` / `.invalid_units_per_pixel` / `.invalid_extent`;
//! * `imageclip.payload` / `.no_selection` / `.multiple` / `.not_image` /
//!   `.invalid_rect`;
//! * `imageadjust.payload` / `.no_selection` / `.multiple` / `.not_image` /
//!   `.invalid_value`.
//!
//! Texture resolution stays host-side: the stored entity carries only the
//! normalised logical key. When that key is not under the drawing directory the
//! representation layer emits the honest frame plus the existing
//! `image.unresolved` diagnostic — this command never fabricates a texture.

use super::*;
use crate::command_payload::{ImageAdjustCommand, ImageAttachCommand, ImageClipCommand};
use cad_domain::{DocumentId, ImageAdjust};

/// The image authoring/editing commands (Work-only).
impl Application {
    /// Dispatch one raster-image edit command.
    pub(crate) fn image_edit(
        &mut self,
        session: &SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::ImageAttach => self.image_attach(session, command),
            ExtendedCommandId::ImageClip => self.image_clip(session, command),
            ExtendedCommandId::ImageAdjust | ExtendedCommandId::DashImageAdjust => {
                self.image_adjust(session, command)
            }
            other => Err(CadError::Invariant(format!(
                "not a raster-image edit command: {other:?}"
            ))),
        }
    }

    /// `IMAGEATTACH <file> <x> <y> <width_px> <height_px> <units_per_pixel>`.
    ///
    /// Builds one `SemanticGeometry::Image` on the `+Z` work plane:
    /// `origin` is the typed point, `u`/`v` are the per-pixel edge vectors
    /// `(units_per_pixel, 0, 0)` and `(0, units_per_pixel, 0)`, `pixels` is the
    /// typed `(width, height)` in pixels, `file` is the **normalised** logical key
    /// and the clip starts empty with the image visible.
    ///
    /// The command is inserted through the shared create funnel, so it lands on
    /// the active layer in model space in one transaction / one undo step. All
    /// four named refusals are checked before that transaction.
    fn image_attach(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ImageAttach(payload) = &command.payload else {
            return Err(image_refusal(
                "imageattach.payload",
                "IMAGEATTACH needs an ImageAttach payload (file, x, y, width_px, \
                 height_px, units_per_pixel)",
            ));
        };
        let ImageAttachCommand {
            file,
            origin,
            pixels,
            units_per_pixel,
        } = payload.as_ref();
        let Some(key) = cad_resources::image_resource_key(file) else {
            return Err(image_refusal(
                "imageattach.unsafe_reference",
                format!(
                    "`{}` is not a safe relative resource key (absolute paths, `..` \
                     traversal and URLs are refused)",
                    file.trim()
                ),
            ));
        };
        if !is_finite_point(*origin) {
            return Err(image_refusal(
                "imageattach.invalid_origin",
                format!("the image origin {origin:?} is not finite"),
            ));
        }
        if !pixels[0].is_finite() || !pixels[1].is_finite() || pixels[0] <= 0.0 || pixels[1] <= 0.0
        {
            return Err(image_refusal(
                "imageattach.invalid_pixels",
                format!(
                    "the pixel dimensions must be positive and finite, got {} x {}",
                    pixels[0], pixels[1]
                ),
            ));
        }
        if !units_per_pixel.is_finite() || *units_per_pixel <= 0.0 {
            return Err(image_refusal(
                "imageattach.invalid_units_per_pixel",
                format!(
                    "units per pixel must be finite and strictly positive, got \
                     {units_per_pixel}"
                ),
            ));
        }
        // The world-space extent is `pixels * units_per_pixel`; both products
        // must stay finite or the frame/basis has no representable extent.
        let width = pixels[0] * units_per_pixel;
        let height = pixels[1] * units_per_pixel;
        if !width.is_finite() || !height.is_finite() {
            return Err(image_refusal(
                "imageattach.invalid_extent",
                format!(
                    "the world-space extent {} x {} (pixels x units_per_pixel) is not \
                     finite; reduce the pixel size or units per pixel",
                    pixels[0], pixels[1]
                ),
            ));
        }
        let stored_key = key.0;
        let geometry = SemanticGeometry::Image {
            origin: *origin,
            u: Point3 {
                x: *units_per_pixel,
                y: 0.0,
                z: 0.0,
            },
            v: Point3 {
                x: 0.0,
                y: *units_per_pixel,
                z: 0.0,
            },
            pixels: *pixels,
            file: Some(stored_key.clone()),
            clip: None,
            visible: true,
            adjust: ImageAdjust::default(),
        };
        let mut outcome = self.commit_new_entity(session, command, geometry, "attach image")?;
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: "imageattach.created".into(),
            message: format!(
                "IMAGEATTACH: attached `{stored_key}` as {} x {} px at ({}, {}, {}); \
                 extent {} x {} world units ({} per pixel); the texture is resolved from \
                 the drawing directory by the host, which reloads image resources after \
                 this command (unresolved keys appear in the host image report)",
                pixels[0], pixels[1], origin.x, origin.y, origin.z, width, height, units_per_pixel
            ),
        });
        Ok(outcome)
    }

    /// `IMAGECLIP <minx> <miny> <maxx> <maxy>` / `IMAGECLIP OFF`.
    ///
    /// Pre-selection command: exactly one selected, directly addressable `Image`
    /// entity. The rectangle is in the image's **pixel coordinate space** and is
    /// stored as the two opposite corners with `inside: true` (the renderer reads
    /// two vertices as a rectangle). `OFF` clears the clip. An identical clip is
    /// `imageclip.unchanged` and opens no transaction.
    fn image_clip(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ImageClip(payload) = &command.payload else {
            return Err(image_refusal(
                "imageclip.payload",
                "IMAGECLIP needs an ImageClip payload (`<minx> <miny> <maxx> <maxy>` or \
                 `OFF`)",
            ));
        };
        let ImageClipCommand { rect } = payload.as_ref();
        let (document_id, mut candidate, before) =
            self.one_selected_image(session, command, "IMAGECLIP", "imageclip")?;
        let SemanticGeometry::Image { clip, .. } = &before.geometry else {
            return Err(image_refusal(
                "imageclip.not_image",
                format!(
                    "entity {} is a {}, not a raster image",
                    before.id.0,
                    class_name(&before.geometry)
                ),
            ));
        };
        let new_clip = match rect {
            None => None,
            Some([minx, miny, maxx, maxy]) => {
                if ![minx, miny, maxx, maxy]
                    .iter()
                    .all(|value| value.is_finite())
                    || minx >= maxx
                    || miny >= maxy
                {
                    return Err(image_refusal(
                        "imageclip.invalid_rect",
                        format!(
                            "the pixel-space rectangle must be finite with minx < maxx and \
                             miny < maxy, got [{minx}, {miny}] .. [{maxx}, {maxy}]"
                        ),
                    ));
                }
                Some(ImageClip {
                    vertices: vec![[*minx, *miny], [*maxx, *maxy]],
                    inside: true,
                })
            }
        };
        let object = Some(ObjectId(before.id.0));
        if *clip == new_clip {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object,
                    code: "imageclip.unchanged".into(),
                    message: format!(
                        "IMAGECLIP: image entity {} already has this pixel-space clip; \
                         nothing was written",
                        before.id.0
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }
        let mut after = before.clone();
        if let SemanticGeometry::Image { clip: slot, .. } = &mut after.geometry {
            *slot = new_clip.clone();
        }
        let label = if new_clip.is_some() {
            "image clip"
        } else {
            "image clip off"
        };
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction(label, transaction)?;
            staged.update_entity(after.clone())?;
            staged.commit()?
        };
        let stored = candidate.entity(after.id).cloned().ok_or_else(|| {
            CadError::Invariant("IMAGECLIP: the image vanished after commit".into())
        })?;
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.into(),
                patches: vec![drawing_patch(before.id, Some(before.clone()), Some(stored))],
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        let (code, message) = match &new_clip {
            Some(clip) => (
                "imageclip.applied",
                format!(
                    "IMAGECLIP: set a pixel-space clip on image entity {} from ({}, {}) to \
                     ({}, {}); 2-vertex rectangle (inside)",
                    before.id.0,
                    clip.vertices[0][0],
                    clip.vertices[0][1],
                    clip.vertices[1][0],
                    clip.vertices[1][1]
                ),
            ),
            None => (
                "imageclip.removed",
                format!(
                    "IMAGECLIP OFF: cleared the pixel-space clip on image entity {}",
                    before.id.0
                ),
            ),
        };
        Ok(CommandOutcome {
            objects: vec![ObjectId(before.id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object,
                code: code.into(),
                message,
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `IMAGEADJUST <brightness> <contrast> <fade> [MONO|COLOR]` /
    /// `IMAGEADJUST RESET`.
    ///
    /// Pre-selection command: exactly one selected, directly addressable `Image`
    /// entity. The three percentages are stored verbatim in the image's
    /// `adjust` field (`0..=100`, neutral `50`/`50`/`0`);
    /// `MONO`/`COLOR` are optional and an omitted word **keeps the image's
    /// current monochrome state** rather than resetting it. `RESET` restores the
    /// neutral 50/50/0/off transfer.
    ///
    /// A value above 100 is `imageadjust.invalid_value` (the payload is `u8`,
    /// whose type alone cannot express "at most 100"). An identical transfer is
    /// `imageadjust.unchanged` and opens no transaction, so a repeated command
    /// bumps neither the revision nor the undo stack.
    fn image_adjust(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ImageAdjust(payload) = &command.payload else {
            return Err(image_refusal(
                "imageadjust.payload",
                "IMAGEADJUST needs an ImageAdjust payload (`<brightness> <contrast> <fade> \
                 [MONO|COLOR]` or `RESET`)",
            ));
        };
        // The three percentages are validated **before** the selection is read:
        // a malformed transfer is the same refusal whether or not anything is
        // selected, and it never opens a transaction.
        if let ImageAdjustCommand::Set {
            brightness,
            contrast,
            fade,
            ..
        } = payload.as_ref()
        {
            if *brightness > 100 || *contrast > 100 || *fade > 100 {
                return Err(image_refusal(
                    "imageadjust.invalid_value",
                    format!(
                        "brightness, contrast and fade are percentages in 0..=100, got \
                         {brightness}/{contrast}/{fade}"
                    ),
                ));
            }
        }
        let (document_id, mut candidate, before) =
            self.one_selected_image(session, command, "IMAGEADJUST", "imageadjust")?;
        let current = match &before.geometry {
            SemanticGeometry::Image { adjust, .. } => *adjust,
            other => {
                return Err(image_refusal(
                    "imageadjust.not_image",
                    format!(
                        "entity {} is a {}, not a raster image",
                        before.id.0,
                        class_name(other)
                    ),
                ));
            }
        };
        let target = match payload.as_ref() {
            ImageAdjustCommand::Reset => ImageAdjust::default(),
            ImageAdjustCommand::Set {
                brightness,
                contrast,
                fade,
                monochrome,
            } => ImageAdjust {
                brightness: *brightness,
                contrast: *contrast,
                fade: *fade,
                // An omitted MONO/COLOR word keeps the stored state.
                monochrome: monochrome.unwrap_or(current.monochrome),
            },
        };
        let object = Some(ObjectId(before.id.0));
        let mode = |monochrome: bool| {
            if monochrome {
                "monochrome (MONO)"
            } else {
                "colour (COLOR)"
            }
        };
        if target == current {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object,
                    code: "imageadjust.unchanged".into(),
                    message: format!(
                        "IMAGEADJUST: image entity {} already has brightness {}, contrast {}, \
                         fade {}, {}; nothing was written",
                        before.id.0,
                        target.brightness,
                        target.contrast,
                        target.fade,
                        mode(target.monochrome)
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }
        let mut after = before.clone();
        if let SemanticGeometry::Image { adjust: slot, .. } = &mut after.geometry {
            *slot = target;
        }
        let label = "image adjust";
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction(label, transaction)?;
            staged.update_entity(after.clone())?;
            staged.commit()?
        };
        let stored = candidate.entity(after.id).cloned().ok_or_else(|| {
            CadError::Invariant("IMAGEADJUST: the image vanished after commit".into())
        })?;
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.into(),
                patches: vec![drawing_patch(before.id, Some(before.clone()), Some(stored))],
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects: vec![ObjectId(before.id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object,
                code: "imageadjust.applied".into(),
                message: format!(
                    "IMAGEADJUST: image entity {} set to brightness {}, contrast {}, fade {}, \
                     {}; the renderer applies these in the declared order monochrome -> \
                     contrast -> brightness -> fade toward the background",
                    before.id.0,
                    target.brightness,
                    target.contrast,
                    target.fade,
                    mode(target.monochrome)
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// The one directly addressable selected `Image` entity, plus a mutable
    /// drawing candidate cloned from the document.
    ///
    /// Shared by every pre-selection image edit so the selection refusals
    /// (`<code>.no_selection` / `.multiple` / `.not_image`) are worded and coded
    /// once. Nothing is written here: the caller stages its own transaction.
    fn one_selected_image(
        &self,
        session: &SessionState,
        command: &Command,
        name: &str,
        code: &str,
    ) -> CadResult<(DocumentId, cad_db::DrawingDatabase, DbEntity)> {
        let references = session.selection.refs();
        let reference = match references {
            [] => {
                return Err(image_refusal(
                    format!("{code}.no_selection"),
                    format!("{name} needs exactly one selected image; nothing is selected"),
                ));
            }
            [one] => one,
            many => {
                return Err(image_refusal(
                    format!("{code}.multiple"),
                    format!(
                        "{name} edits exactly one image; {} entities are selected",
                        many.len()
                    ),
                ));
            }
        };
        if reference.document != command.document
            || !reference.instance.0.is_empty()
            || reference.sub_element.is_some()
        {
            return Err(image_refusal(
                format!("{code}.not_image"),
                format!(
                    "entity {} is not a directly addressable image in this document",
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
            image_refusal(
                format!("{code}.not_image"),
                format!(
                    "entity {} does not exist in this drawing",
                    reference.entity.0
                ),
            )
        })?;
        Ok((document_id, candidate, before))
    }
}

/// One stable, code-prefixed refusal for an image authoring command.
fn image_refusal(code: impl std::fmt::Display, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}
