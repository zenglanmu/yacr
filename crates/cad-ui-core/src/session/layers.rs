//! The layer-panel read projection (`cad_layers`) and the typed layer writes.
//!
//! The drawing database is the only authority for the layer table. Every row
//! published here is projected from the host controller's real
//! [`cad_app::layers::LayerRow`] list — the DWG layer flag plus the
//! **session-scoped temporary override set** — and nothing is invented: a
//! session with no document reports an empty list with no active layer, never a
//! synthetic `0` row.
//!
//! The three mutations are session-scoped and travel through the ordinary host
//! command funnel ([`CommandId::ToggleLayer`], [`CommandId::RestoreLayers`],
//! [`CommandId::SetActiveLayer`]) so the application layer performs the real
//! work: an override never writes the DWG layer table, never raises the database
//! revision and is never a second visibility state machine. A name supplied by
//! the panel is resolved against the projected rows here (exact match first,
//! then a **unique** case-insensitive one); an unknown or ambiguous name is
//! refused explicitly and changes nothing.

use cad_app::layers::LayerRow;
use cad_app::{CommandId, CommandPayload};
use cad_domain::LayerId;
use serde::Serialize;

use super::diagnostics::cad_error_text;
use super::CadSession;

/// One row of the [`CadLayers`] answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadLayer {
    /// Database layer name (the DWG layer-table authority).
    pub name: String,
    /// Visibility the scene honours right now (database flag + temporary override).
    pub visible: bool,
    /// The DWG layer-table flag, never modified by the UI.
    pub database_visible: bool,
    /// Whether a session-only temporary override is in effect for this layer.
    pub overridden: bool,
    /// Whether this is the active layer new entities are created on.
    pub current: bool,
}

/// The complete `cad_layers` answer.
///
/// The keys are always present (`layers`, `active`, `overridden`); `active` is
/// `null` when no drawing is open. This is the explicit IPC answer the React
/// layer panel consumes, deliberately **not** a `cad_status` field: the frozen
/// status contract is untouched.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadLayers {
    pub layers: Vec<CadLayer>,
    /// Name of the active layer, or `None` when no drawing is open.
    pub active: Option<String>,
    /// Number of layers carrying a temporary override (drives the panel's restore badge).
    pub overridden: usize,
}

impl CadLayers {
    /// The honest empty answer: no document, no rows, no active layer.
    pub fn empty() -> Self {
        Self {
            layers: Vec::new(),
            active: None,
            overridden: 0,
        }
    }
}

/// One queued layer mutation (the typed layer IPC channels).
///
/// The wire admission only carries names/booleans; whether a name resolves to a
/// real layer is live session state and is decided on the owner thread when this
/// write is applied ([`CadLayerWrite::apply`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CadLayerWrite {
    /// Set the temporary visibility of one layer, addressed by name.
    SetVisible { name: String, visible: bool },
    /// Drop every temporary override (restore the database flags).
    Restore,
    /// Make one layer the active layer new entities are created on.
    SetActive { name: String },
}

impl CadLayerWrite {
    /// Apply this write through the owner-thread session funnel.
    ///
    /// Every refusal is the session's real error (unknown/ambiguous layer name,
    /// no document, a host refusal), never a silent no-op.
    pub fn apply(&self, session: &CadSession) -> Result<(), String> {
        match self {
            Self::SetVisible { name, visible } => session.set_layer_visibility(name, *visible),
            Self::Restore => session.restore_layers(),
            Self::SetActive { name } => session.set_active_layer(name),
        }
    }
}

/// Stable refusal when a layer mutation is asked for without a drawing.
const LAYER_NO_DOCUMENT: &str = "layer.no_document: no drawing is open";

impl CadSession {
    /// The layer-panel projection of the open drawing.
    ///
    /// No document → [`CadLayers::empty`] (an explicit empty state, never a
    /// fabricated row). Otherwise the controller's real rows, with `current`
    /// comparing each row's id against the session's active layer and
    /// `overridden` counting the rows carrying a temporary override.
    pub fn layers(&self) -> Result<CadLayers, String> {
        if self.core.controller.borrow().drawing().is_none() {
            return Ok(CadLayers::empty());
        }
        let rows = self.layer_rows()?;
        let active_layer = self.core.controller.borrow().session.active_layer;
        let active = rows
            .iter()
            .find(|row| row.id == active_layer)
            .map(|row| row.name.clone());
        let overridden = rows.iter().filter(|row| row.is_overridden()).count();
        let layers = rows
            .into_iter()
            .map(|row| CadLayer {
                name: row.name,
                visible: row.effective_visible,
                database_visible: row.database_visible,
                overridden: row.override_visible.is_some(),
                current: row.id == active_layer,
            })
            .collect();
        Ok(CadLayers {
            layers,
            active,
            overridden,
        })
    }

    /// Apply a **temporary** visibility for the layer named `name`.
    ///
    /// The name is resolved to a real [`LayerId`] from the drawing's own layer
    /// rows; the write then goes through the ordinary command funnel
    /// ([`CommandId::ToggleLayer`] + [`CommandPayload::Layer`]) so the scene
    /// refresh, the override set and `cad_status.error` behave exactly as they do
    /// for the command line. The DWG layer table is never written.
    pub fn set_layer_visibility(&self, name: &str, visible: bool) -> Result<(), String> {
        let layer = self.resolve_layer(name)?;
        self.dispatch_payload(
            CommandId::ToggleLayer,
            CommandPayload::Layer(layer, visible),
        )
    }

    /// Drop every temporary layer override ([`CommandId::RestoreLayers`]).
    ///
    /// The drawing's own flags remain the authority and the database revision is
    /// untouched. Without a drawing this is an explicit refusal: an "accepted"
    /// restore that had nothing to restore would be a fake success.
    pub fn restore_layers(&self) -> Result<(), String> {
        self.require_layer_document()?;
        self.dispatch_command(CommandId::RestoreLayers)
    }

    /// Make the layer named `name` the active layer new entities are created on.
    ///
    /// Dispatched as [`CommandId::SetActiveLayer`] +
    /// [`CommandPayload::ActiveLayer`]; the name resolves against the real layer
    /// rows first, so an unknown name can never reach the session as a stale id.
    pub fn set_active_layer(&self, name: &str) -> Result<(), String> {
        let layer = self.resolve_layer(name)?;
        self.dispatch_payload(
            CommandId::SetActiveLayer,
            CommandPayload::ActiveLayer(layer),
        )
    }

    /// The controller's real layer rows (database flags + session overrides).
    fn layer_rows(&self) -> Result<Vec<LayerRow>, String> {
        self.require_layer_document()?;
        let controller = self.core.controller.borrow();
        controller
            .layer_rows()
            .map_err(|error| cad_error_text(&error))
    }

    /// Refuse explicitly when no drawing is open.
    fn require_layer_document(&self) -> Result<(), String> {
        if self.core.controller.borrow().drawing().is_some() {
            Ok(())
        } else {
            Err(LAYER_NO_DOCUMENT.to_string())
        }
    }

    /// Resolve a panel-supplied layer name to the drawing's own layer id.
    ///
    /// Exact name match wins; only when there is none is a **unique**
    /// case-insensitive match accepted (AutoCAD layer names are
    /// case-insensitive). An unknown name and a name matching several rows are
    /// distinct explicit refusals that name the offending layer, and neither
    /// changes any state.
    fn resolve_layer(&self, name: &str) -> Result<LayerId, String> {
        let rows = self.layer_rows()?;
        let mut exact = rows.iter().filter(|row| row.name == name);
        if let Some(row) = exact.next() {
            if exact.next().is_none() {
                return Ok(row.id);
            }
            return Err(format!(
                "layer.ambiguous_name: more than one drawing layer is named '{name}'"
            ));
        }
        // Same folding rule as the panel's own search (`filter_layer_rows`), so a
        // name the UI shows as a match resolves the same way here.
        let needle = name.to_lowercase();
        let matches: Vec<&LayerRow> = rows
            .iter()
            .filter(|row| row.name.to_lowercase() == needle)
            .collect();
        match matches.as_slice() {
            [row] => Ok(row.id),
            [] => Err(format!("layer.unknown: no drawing layer named '{name}'")),
            _ => Err(format!(
                "layer.ambiguous_name: {} drawing layers match '{name}' case-insensitively",
                matches.len()
            )),
        }
    }
}
