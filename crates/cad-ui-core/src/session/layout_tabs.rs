//! The paper-layout tab list and active drawing space on the frozen wire.
//!
//! Both are **derived projections** of the authoritative drawing/session: the
//! tab list comes from `cad_app::render_scene::layout_descriptors` (the same
//! descriptor list the representation layer uses), and the active space is the
//! session's real [`cad_domain::SpaceId`]. Nothing here invents a layout or a
//! "current" tab.

use cad_domain::SpaceId;
use serde::Serialize;

/// One paper-space layout tab of the frontend status.
///
/// `reason` is the descriptor's own stable, code-prefixed technical string
/// (e.g. `"viewport.complex_clip: …"`), never a localized UI guess. It is
/// `None` exactly when the layout is drawable, because the descriptor leaves
/// its reason empty in that case.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadLayoutTab {
    /// The authoritative `LayoutId` value.
    pub id: u128,
    /// The layout's display name from the drawing table.
    pub name: String,
    /// Whether this build can draw every viewport of the layout.
    pub supported: bool,
    /// The descriptor's stable technical reason when unsupported, else `None`.
    pub reason: Option<String>,
    /// The real number of viewports in the layout (0 for a paper sheet with no
    /// windows).
    pub viewport_count: u64,
}

impl CadLayoutTab {
    /// Project one authored layout descriptor; the caller has already filtered
    /// the synthetic model-space descriptor.
    pub(super) fn new(
        id: u128,
        name: String,
        supported: bool,
        reason: String,
        viewport_count: usize,
    ) -> Self {
        Self {
            id,
            name,
            supported,
            // An empty descriptor reason means "drawable": never fabricate a
            // technical code for a supported layout.
            reason: (!reason.is_empty()).then_some(reason),
            viewport_count: viewport_count as u64,
        }
    }
}

/// The session's active drawing space on the frozen wire.
///
/// Wire shape: `{kind:"model"}` or `{kind:"paper", id:<u128>}`. Model space is
/// its own tab, so a real paper layout is the only other selectable space.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CadActiveSpace {
    /// Model space.
    Model,
    /// One paper-space layout, named by its authoritative `LayoutId`.
    Paper { id: u128 },
}

impl CadActiveSpace {
    /// Project a session `SpaceId`.
    ///
    /// `SpaceId::Block` is block-definition space, which `cad-app` never lets a
    /// session select (`validate_space_id` refuses it), so it cannot be the
    /// active observation space. It is mapped to Model defensively rather than
    /// emitting a variant the frozen wire does not define.
    pub(super) fn from_space_id(space: &SpaceId) -> Self {
        match space {
            SpaceId::Model | SpaceId::Block(_) => Self::Model,
            SpaceId::Paper(id) => Self::Paper { id: id.0 },
        }
    }
}
