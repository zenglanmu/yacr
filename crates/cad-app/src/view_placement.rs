//! Typed camera placement for the `cad_set_view` wire channel.
//!
//! Two forms, both **view state only** (no database write, revision bump,
//! history entry or `ChangeSet`):
//!
//! * [`ViewPlacement::Preset`] — a *fitted* observation the host resolves against
//!   the active space bounds. `overview` is the isometric 3D overview, `plan` is
//!   the 2D top view, and the four elevations are 3D side views. Each is framed
//!   from the real document bounds, which the chrome does not know; that is why
//!   the preset is resolved on the host instead of composed in the UI.
//! * [`ViewPlacement::Camera`] — one absolute camera (eye/target/up + projection)
//!   plus the 2D/3D observation mode. This is the positioning API: a caller that
//!   knows the pose it wants (a saved orientation, a scan viewpoint, a test)
//!   does not have to orbit its way there.
//!
//! The type itself carries no behaviour; [`crate::Application::set_view`] owns
//! every rule and refusal.
use crate::{Camera, StandardView};

/// One host-fitted observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewPreset {
    /// Isometric 3D overview framed on the active space.
    Overview,
    /// 2D top view framed on the active space.
    Plan,
    /// 3D elevation looking from world `-Y`.
    Front,
    /// 3D elevation looking from world `+Y`.
    Back,
    /// 3D elevation looking from world `+X`.
    Left,
    /// 3D elevation looking from world `-X`.
    Right,
}

impl ViewPreset {
    /// Every preset, in the order the wire/UI lists them.
    pub const ALL: [ViewPreset; 6] = [
        ViewPreset::Overview,
        ViewPreset::Plan,
        ViewPreset::Front,
        ViewPreset::Back,
        ViewPreset::Left,
        ViewPreset::Right,
    ];

    /// The stable wire key (`cad_set_view { preset }`).
    pub fn key(self) -> &'static str {
        match self {
            ViewPreset::Overview => "overview",
            ViewPreset::Plan => "plan",
            ViewPreset::Front => "front",
            ViewPreset::Back => "back",
            ViewPreset::Left => "left",
            ViewPreset::Right => "right",
        }
    }

    /// Parse one case-insensitive wire key; an unknown name is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let name = text.trim().to_ascii_lowercase();
        Self::ALL
            .iter()
            .copied()
            .find(|preset| preset.key() == name)
    }

    /// The standard direction this preset shares, if it is a direction preset.
    ///
    /// `plan` is the standard `Top` observation; `overview` is `Isometric`; the
    /// four elevations are `Front`/`Back`/`Left`/`Right`. `Plan` is `Some(Top)`
    /// even though the placement is 2D: the direction is still the top view.
    pub fn standard_view(self) -> StandardView {
        match self {
            ViewPreset::Overview => StandardView::Isometric,
            ViewPreset::Plan => StandardView::Top,
            ViewPreset::Front => StandardView::Front,
            ViewPreset::Back => StandardView::Back,
            ViewPreset::Left => StandardView::Left,
            ViewPreset::Right => StandardView::Right,
        }
    }

    /// Whether the preset is a 3D observation (everything but `plan`).
    pub fn is_3d(self) -> bool {
        !matches!(self, ViewPreset::Plan)
    }
}

/// The observation mode an absolute placement asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewModeRequest {
    /// Plan/2D observation. Requires an orthographic camera.
    Plan,
    /// 3D observation. Requires a perspective camera (the 3D renderer is
    /// perspective-only).
    ThreeD,
}

/// One accepted `cad_set_view` request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewPlacement {
    /// A host-fitted preset.
    Preset(ViewPreset),
    /// One absolute camera and the observation mode it belongs to.
    Camera {
        camera: Camera,
        mode: ViewModeRequest,
    },
}
