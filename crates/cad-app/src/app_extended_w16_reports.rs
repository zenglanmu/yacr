//! Wave 16 read-only **raster-image report** (`IMAGE` / `-IMAGE`).
//!
//! Both spellings list every `SemanticGeometry::Image` entity the drawing really
//! stores, in every space (model, paper layouts and block definitions), with its
//! entity id, space, logical file key, pixel size, visibility, clip summary and
//! origin. Nothing is opened, written, created or deleted: there is no
//! transaction, no revision bump, no `cad-history` entry and no `ChangeSet`, so
//! both are Viewer-legal.
//!
//! Diagnostic codes: `image.entry` per image plus the final `image.report`
//! summary; an empty drawing is a successful `image.none` report (not an error).
//! The `-IMAGE` spelling uses the `-image.*` codes, the same convention `XREF` /
//! `-XREF` follow.
//!
//! Attaching, clipping and the display adjustment are the separate, Work-only
//! commands in [`crate::app_drawing::image_edit`].

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::{document_drawing, require_none_payload};

impl Application {
    /// Dispatch one Wave 16 read-only report.
    pub(crate) fn extended_w16_reports(
        &self,
        _session: &SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::Image => self.image_report(command, ImageReport::Image),
            ExtendedCommandId::DashImage => self.image_report(command, ImageReport::DashImage),
            other => Err(CadError::Invariant(format!(
                "not a Wave 16 report: {other:?}"
            ))),
        }
    }

    /// `IMAGE` / `-IMAGE`: read-only listing of the stored raster images.
    fn image_report(&self, command: &Command, report: ImageReport) -> CadResult<CommandOutcome> {
        require_none_payload(command, report.selector())?;
        let drawing = document_drawing(self, command)?;
        let mut diagnostics = Vec::new();
        let mut count = 0usize;
        let mut model = 0usize;
        let mut paper = 0usize;
        let mut block = 0usize;
        for entity in drawing.entities() {
            let SemanticGeometry::Image {
                origin,
                pixels,
                file,
                clip,
                visible,
                ..
            } = &entity.geometry
            else {
                continue;
            };
            count += 1;
            match entity.space {
                SpaceId::Model => model += 1,
                SpaceId::Paper(_) => paper += 1,
                SpaceId::Block(_) => block += 1,
            }
            diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: report.entry_code().into(),
                message: format!(
                    "{}: entity {} in {}; file {}; pixels {} x {}; visible {}; clip {}; \
                     origin ({}, {}, {})",
                    report.selector(),
                    entity.id.0,
                    space_label(&entity.space),
                    file.as_deref().unwrap_or("<none>"),
                    pixels[0],
                    pixels[1],
                    if *visible { "true" } else { "false" },
                    clip_summary(clip.as_ref()),
                    origin.x,
                    origin.y,
                    origin.z
                ),
            });
        }
        if count == 0 {
            diagnostics.push(diagnostic(
                report.none_code(),
                format!(
                    "{}: the drawing has no raster image entity; nothing was written",
                    report.selector()
                ),
            ));
        } else {
            diagnostics.push(diagnostic(
                report.summary_code(),
                format!(
                    "{}: {count} raster image(s) reported (model {model}, paper {paper}, \
                     block {block}); this is a read-only report and nothing was written",
                    report.selector()
                ),
            ));
        }
        Ok(report_outcome(diagnostics))
    }
}

/// Which `IMAGE` spelling is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageReport {
    /// `IMAGE`: `image.entry` / `image.report` / `image.none`.
    Image,
    /// `-IMAGE`: `-image.entry` / `-image.report` / `-image.none`.
    DashImage,
}

impl ImageReport {
    fn selector(self) -> &'static str {
        match self {
            Self::Image => "IMAGE",
            Self::DashImage => "-IMAGE",
        }
    }

    fn entry_code(self) -> &'static str {
        match self {
            Self::Image => "image.entry",
            Self::DashImage => "-image.entry",
        }
    }

    fn summary_code(self) -> &'static str {
        match self {
            Self::Image => "image.report",
            Self::DashImage => "-image.report",
        }
    }

    fn none_code(self) -> &'static str {
        match self {
            Self::Image => "image.none",
            Self::DashImage => "-image.none",
        }
    }
}

/// A human-readable space label: `Model`, `Paper(<id>)` or `Block(<id>)`.
fn space_label(space: &SpaceId) -> String {
    match space {
        SpaceId::Model => "Model".into(),
        SpaceId::Paper(layout) => format!("Paper({})", layout.0),
        SpaceId::Block(block) => format!("Block({})", block.0),
    }
}

/// A human-readable clip summary: `none`, `N vertices (inside|outside)`, or
/// `2-vertex rectangle (inside|outside)`.
fn clip_summary(clip: Option<&ImageClip>) -> String {
    let Some(clip) = clip else {
        return "none".into();
    };
    let side = if clip.inside { "inside" } else { "outside" };
    if clip.vertices.len() == 2 {
        format!("2-vertex rectangle ({side})")
    } else {
        format!("{} vertices ({side})", clip.vertices.len())
    }
}
