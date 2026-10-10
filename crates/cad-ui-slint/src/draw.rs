//! Shell-side drawing/editing support: state mapping, command sink and errors.
//!
//! The pure capture state machine lives in [`cad_app::draw_tool`]; this module
//! is the Slint boundary. It maps a [`cad_app::DrawPreview`] into the small
//! panel state the shell pushes (mirroring [`crate::MeasurementUiState`] and
//! [`crate::AnnotationPanelState`]), builds the catalog-driven kind labels, and
//! defines the [`DrawCommandSink`] a host installs to turn a confirmed
//! [`cad_app::DrawIntent`] into exactly one drawing command.
//!
//! ## Why a sink and not a direct command
//!
//! `docs/drawing-edit.md` §2 defines `CommandId::CreateLine`, `CreateCircle`,
//! `MoveEntities`, `TrimEntity`, but `Move`/`Trim` also need the session's
//! `SelectionRef`s. The shell holds the slice/selection counts, not the refs, so
//! it cannot invent a `Move`/`Trim` payload. The host that owns the session
//! installs a [`DrawCommandSink`] and maps the intent exactly as documented;
//! until it is installed, a confirm reports "not wired" in the status line
//! rather than fabricating a payload-free success (the same honest-degradation
//! pattern as [`crate::LayoutSwitchSink`] and [`crate::CanvasPickMapper`]).

use cad_app::{DrawIntent, DrawPreview, DrawToolKind};
use cad_domain::{CadError, CadResult};

use crate::i18n::MessageSource;

/// Receives a confirmed drawing/editing intent from the shell.
///
/// A host implements this alongside the shared `UiCommandSink`: `commit` builds
/// and sends the exact application command (`CreateLine`/`CreateCircle`/
/// `MoveEntities`/`TrimEntity`) through the single transaction/history path.
/// `begin` is an optional capture-start hook (a host may emit a begin command);
/// it defaults to a no-op because capture itself is session state.
pub trait DrawCommandSink: 'static {
    /// Start capture for `kind`. Defaults to a no-op.
    fn begin(&mut self, kind: DrawToolKind) {
        let _ = kind;
    }

    /// Commit exactly one transaction for `intent`.
    ///
    /// The mapping the host must apply (`docs/drawing-edit.md` §2):
    ///
    /// * `Line { start, end }` → `CommandId::CreateLine` with
    ///   `CommandPayload::Points([start, end])`;
    /// * `Circle { center, edge }` → `CommandId::CreateCircle` with
    ///   `CommandPayload::Points([center, edge])`;
    /// * `Move { delta }` → `CommandId::MoveEntities` with the session
    ///   selection refs plus `delta`;
    /// * `Copy { delta }` → `CommandId::CopyEntities` with the session
    ///   selection refs plus `delta`;
    /// * `Trim { target_pick, boundary_pick }` → `CommandId::TrimEntity` after
    ///   resolving the two picks to `SelectionRef`s.
    fn commit(&mut self, intent: DrawIntent) -> CadResult<()>;
}

/// Panel state pushed into the shell while a draw/edit tool is active.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawUiState {
    /// Whether a draw/edit tool is running.
    pub active: bool,
    /// Whether the confirm affordance is valid right now.
    pub can_confirm: bool,
    pub kind: DrawToolKind,
    /// Human-facing "picked N, need M" step text; empty when idle.
    pub step_label: String,
    /// Number of selected entities when the tool started (MOVE needs > 0).
    pub selection_count: usize,
}

impl Default for DrawUiState {
    fn default() -> Self {
        DrawUiState {
            active: false,
            can_confirm: false,
            kind: DrawToolKind::Line,
            step_label: String::new(),
            selection_count: 0,
        }
    }
}

impl DrawUiState {
    /// Derive the panel state from an optional active preview.
    pub fn from_preview(preview: Option<&DrawPreview>) -> Self {
        match preview {
            Some(preview) => DrawUiState {
                active: true,
                can_confirm: preview.can_confirm(),
                kind: preview.kind,
                step_label: preview.status_line(),
                selection_count: preview.selection_count,
            },
            None => DrawUiState::default(),
        }
    }

    /// Combobox/menu index for [`cad_app::DrawToolKind`].
    pub fn kind_index(&self) -> i32 {
        self.kind.index() as i32
    }
}

/// The catalog key for a stable draw kind (e.g. `line`).
pub fn draw_kind_key(kind_key: &str) -> String {
    format!("draw.kind.{kind_key}")
}

/// Every draw kind label, in `DrawToolKind::ALL` order.
///
/// Built from the shared `ALL` ordering, so a chosen row/label maps back to the
/// exact kind without a second hand-written list that could drift.
pub fn draw_kind_labels(messages: &MessageSource) -> Vec<String> {
    cad_app::DrawToolKind::ALL
        .iter()
        .map(|kind| messages.text(&draw_kind_key(kind.key()), &[]))
        .collect()
}

/// Map a localized label back to a draw kind.
pub fn draw_kind_from_label(messages: &MessageSource, label: &str) -> Option<DrawToolKind> {
    draw_kind_labels(messages)
        .iter()
        .position(|candidate| candidate == label)
        .and_then(|index| DrawToolKind::from_index(index as i32))
}

/// Receives the in-progress draw preview so a host can push it into the CAD
/// overlay (`CadView::set_draw_preview`).
///
/// Kept separate from [`DrawCommandSink`] because rendering lives in the
/// `bridge`/`CadView` half of the crate while command emission lives in the
/// adapter half; a host wires the two together (the same split as
/// [`crate::ViewInput`]).
pub trait DrawPreviewSink: 'static {
    /// Store (`Some`) or clear (`None`) the in-progress preview.
    ///
    /// Takes `&self` so a host can install it as an `Rc<dyn DrawPreviewSink>`
    /// (mirroring [`crate::ViewInput`]); use interior mutability for any state.
    fn set_preview(&self, preview: Option<DrawPreview>);
}

/// Localize the error a [`DrawCommandSink`]/command layer returned.
///
/// Permission is the Viewer refusal (`docs/drawing-edit.md` §2); a TRIM with no
/// intersection is reported as `InvalidInput("... no intersection ...")` by the
/// command layer and gets the dedicated "no intersection" text; an unsupported
/// geometry is `Unsupported`. Everything else keeps the real reason, never a
/// fake success.
pub fn draw_error_text(messages: &MessageSource, error: &CadError) -> String {
    match error {
        CadError::PermissionDenied => messages.text("draw.error.read_only", &[]),
        CadError::Unsupported(reason) => {
            messages.text("draw.error.unsupported", &[("reason", reason)])
        }
        CadError::InvalidInput(reason) => {
            // The command layer's no-intersection refusal is a distinct,
            // user-facing case; detect it by its stable phrase so the shell does
            // not genericise it away.
            if reason.to_ascii_lowercase().contains("no intersection") {
                messages.text("draw.error.no_intersection", &[])
            } else {
                messages.text("draw.error.invalid", &[("reason", reason)])
            }
        }
        other => messages.text("draw.error.failed", &[("reason", &other.to_string())]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Locale;
    use cad_domain::Point3;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn zh() -> MessageSource {
        MessageSource::for_locale(Locale::ZhCn)
    }

    fn en() -> MessageSource {
        MessageSource::for_locale(Locale::En)
    }

    #[test]
    fn labels_follow_the_shared_order_and_map_back() {
        for messages in [zh(), en()] {
            let labels = draw_kind_labels(&messages);
            assert_eq!(labels.len(), DrawToolKind::ALL.len());
            for (index, kind) in DrawToolKind::ALL.iter().copied().enumerate() {
                assert!(!labels[index].is_empty());
                assert_eq!(draw_kind_from_label(&messages, &labels[index]), Some(kind));
            }
            // A machine key is not a translated label.
            assert_eq!(draw_kind_from_label(&messages, "line"), None);
        }
    }

    #[test]
    fn ui_state_tracks_the_preview_and_selection() {
        let idle = DrawUiState::from_preview(None);
        assert!(!idle.active);
        assert!(!idle.can_confirm);
        assert_eq!(idle.step_label, "");
        assert_eq!(idle.kind_index(), DrawToolKind::Line.index() as i32);

        let mut tool = cad_app::DrawTool::new(DrawToolKind::Move, 0);
        tool.set_selection_count(3);
        tool.push_point(p(0.0, 0.0)).unwrap();
        let capturing = DrawUiState::from_preview(Some(&tool.preview()));
        assert!(capturing.active);
        assert!(!capturing.can_confirm);
        assert_eq!(capturing.kind, DrawToolKind::Move);
        assert_eq!(capturing.selection_count, 3);

        tool.push_point(p(2.0, 2.0)).unwrap();
        let ready = DrawUiState::from_preview(Some(&tool.preview()));
        assert!(ready.can_confirm);
    }

    #[test]
    fn error_text_distinguishes_read_only_no_intersection_and_unsupported() {
        for messages in [zh(), en()] {
            let read_only = draw_error_text(&messages, &CadError::PermissionDenied);
            let no_intersection = draw_error_text(
                &messages,
                &CadError::InvalidInput("no intersection with boundary".into()),
            );
            let unsupported =
                draw_error_text(&messages, &CadError::Unsupported("SPLINE target".into()));
            assert!(!read_only.is_empty());
            assert!(!no_intersection.is_empty());
            assert!(!unsupported.is_empty());
            assert_ne!(read_only, no_intersection);
            assert_ne!(no_intersection, unsupported);
            // The concrete reason survives for an unsupported kind.
            assert!(unsupported.contains("SPLINE"));
        }
    }
}
