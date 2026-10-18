use cad_db::DrawingDatabase;
use cad_domain::LayoutId;

use super::view::{viewport_transform, ViewportTransform};

/// A precise, stable reason a primitive could not be clipped exactly.
///
/// The caller keeps the unclipped primitive and reports `Partial`; it must not
/// drop it silently and must not claim it was clipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipRefusal {
    /// Stable machine code, one of [`clip_reason`].
    pub reason: &'static str,
    /// Human-readable explanation. Never empty.
    pub message: String,
}

impl ClipRefusal {
    pub fn new(reason: &'static str, message: impl Into<String>) -> Self {
        ClipRefusal {
            reason,
            message: message.into(),
        }
    }
}

/// A user-facing choice of drawing space: model space or one paper-space layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceSelection {
    Model,
    Paper(LayoutId),
}

impl SpaceSelection {
    /// The layout a paper selection refers to, if any.
    pub fn layout(&self) -> Option<LayoutId> {
        match self {
            SpaceSelection::Model => None,
            SpaceSelection::Paper(id) => Some(*id),
        }
    }

    /// Whether this selection is model space.
    pub fn is_model(&self) -> bool {
        matches!(self, SpaceSelection::Model)
    }
}

/// A layout as exposed to a caller that wants to switch spaces.
///
/// `supported` is false when at least one of the layout's viewports cannot be
/// drawn correctly; `reason` then names the first blocking cause so the caller
/// can report it instead of showing a wrong or empty sheet. The reason is
/// prefixed with a stable [`viewport_reason`] code and then `: ` for
/// machine-readable branching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutDescriptor {
    pub id: LayoutId,
    pub name: String,
    /// True when every viewport in the layout is drawable by this build.
    pub supported: bool,
    /// Empty when `supported`; otherwise `"<code>: <message>"`.
    pub reason: String,
    /// Number of viewports in the layout (0 for a paper sheet with no windows).
    pub viewport_count: usize,
}

/// Build the ordered layout list a space-switch affordance offers.
///
/// The order is the database's own (a `BTreeMap`, so deterministic by id).
/// Model space is *not* included: it is always available and is represented by
/// [`SpaceSelection::Model`]. A layout with no viewports is still `supported`
/// (it simply draws its own paper geometry).
pub fn enumerate_layouts(database: &DrawingDatabase) -> Vec<LayoutDescriptor> {
    database
        .layouts()
        .map(|layout| {
            let mut reason = String::new();
            let mut viewport_count = 0usize;
            for viewport in &layout.viewports {
                viewport_count += 1;
                if reason.is_empty() {
                    if let ViewportState::Unsupported(why) = viewport_transform(viewport) {
                        reason = why.to_string();
                    }
                }
            }
            LayoutDescriptor {
                id: layout.id,
                name: layout.name.clone(),
                supported: reason.is_empty(),
                reason,
                viewport_count,
            }
        })
        .collect()
}

/// A concrete, stable reason a viewport cannot be drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportUnsupported {
    /// Stable machine code, one of [`viewport_reason`].
    pub code: &'static str,
    /// Human-readable explanation. Never empty.
    pub message: String,
}

impl ViewportUnsupported {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        ViewportUnsupported {
            code,
            message: message.into(),
        }
    }

    /// The human-readable explanation (without the code prefix).
    pub fn reason(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ViewportUnsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// The result of interpreting one stored [`cad_db::PaperViewport`].
#[derive(Debug, Clone, PartialEq)]
pub enum ViewportState {
    /// A drawable rectangular viewport with an exact transform. Boxed so the
    /// enum stays small; the unsupported reason is payload-light.
    Supported(Box<ViewportTransform>),
    /// The viewport cannot be drawn correctly by this build. Never drawn at a
    /// guessed ratio; the geometry is simply omitted and the reason reported.
    Unsupported(ViewportUnsupported),
}
