//! `IMAGEATTACH` and `IMAGECLIP`: real raster-image authoring on the existing
//! `SemanticGeometry::Image` pipeline.
//!
//! Both commands are **Work-only** and write through the normal drawing
//! transaction path: an attach inserts one entity through the shared
//! [`Application::commit_new_entity`] funnel (active layer, model space, one
//! transaction / one undo step), and a clip updates one selected image row in one
//! transaction / one undo step. Every refusal happens before a transaction, so a
//! refused command writes nothing, bumps no revision and records no history.
//!
//! `IMAGE` / `-IMAGE` are the read-only report and live in
//! `crate::app_extended_w16_reports`; the adjustment/quality/editor names remain
//! named refusals in `crate::app_extended_w16_refusals`.
//!
//! Stable refusal codes:
//!
//! * `imageattach.payload` / `.unsafe_reference` / `.invalid_origin` /
//!   `.invalid_pixels` / `.invalid_units_per_pixel`;
//! * `imageclip.payload` / `.no_selection` / `.multiple` / `.not_image` /
//!   `.invalid_rect`.
//!
//! Texture resolution stays host-side: the stored entity carries only the
//! normalised logical key. When that key is not under the drawing directory the
//! representation layer emits the honest frame plus the existing
//! `image.unresolved` diagnostic — this command never fabricates a texture.

use super::*;
use crate::command_payload::{ImageAttachCommand, ImageClipCommand};

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
        };
        let width = pixels[0] * units_per_pixel;
        let height = pixels[1] * units_per_pixel;
        let mut outcome = self.commit_new_entity(session, command, geometry, "attach image")?;
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: "imageattach.created".into(),
            message: format!(
                "IMAGEATTACH: attached `{stored_key}` as {} x {} px at ({}, {}, {}); \
                 extent {} x {} world units ({} per pixel); the texture resolves host-side \
                 from the drawing directory",
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
        let references = session.selection.refs();
        let reference = match references {
            [] => {
                return Err(image_refusal(
                    "imageclip.no_selection",
                    "IMAGECLIP needs exactly one selected image; nothing is selected",
                ));
            }
            [one] => one,
            many => {
                return Err(image_refusal(
                    "imageclip.multiple",
                    format!(
                        "IMAGECLIP edits exactly one image; {} entities are selected",
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
                "imageclip.not_image",
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
        let mut candidate = document.drawing.as_ref().clone();
        let before = candidate.entity(reference.entity).cloned().ok_or_else(|| {
            image_refusal(
                "imageclip.not_image",
                format!(
                    "entity {} does not exist in this drawing",
                    reference.entity.0
                ),
            )
        })?;
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
}

/// One stable, code-prefixed refusal for an image authoring command.
fn image_refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}
