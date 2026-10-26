//! The layer-panel read projection (`cad_layers`) and the typed layer writes.
//!
//! The drawing database is the only authority for the layer table. Every row
//! published here is projected from the host controller's real
//! [`cad_app::layers::LayerRow`] list — the DWG layer flag plus the
//! **session-scoped temporary override set** — and nothing is invented: a
//! session with no document reports an empty list with no active layer, never a
//! synthetic `0` row.
//!
//! The three session-scoped mutations travel through the ordinary host command
//! funnel ([`CommandId::ToggleLayer`], [`CommandId::RestoreLayers`],
//! [`CommandId::SetActiveLayer`]) so the application layer performs the real
//! work: an override never writes the DWG layer table, never raises the database
//! revision and is never a second visibility state machine. A name supplied by
//! the panel is resolved against the projected rows here (exact match first,
//! then a **unique** case-insensitive one); an unknown or ambiguous name is
//! refused explicitly and changes nothing.
//!
//! The three **structural table** writes ([`CadSession::create_layer`],
//! [`CadSession::rename_layer`], [`CadSession::delete_layer`] →
//! [`CommandId::CreateLayer`] / `RenameLayer` / `DeleteLayer`) and the attribute
//! write ([`CadSession::set_layer_property`] → [`CommandId::SetLayerProperty`])
//! are different: they are real database writes with one revision bump each,
//! they carry a name selector instead of a resolved id (the application layer
//! applies the same resolution rule), and they are **not undoable** —
//! `cad-history` stores entity patches only, so a layer patch kind is a separate
//! lane.
//!
//! Every [`CadLayer`] row also carries the layer's stored attributes (colour,
//! linetype, lineweight, transparency and the off/frozen/locked/plottable flags),
//! formatted with the **write grammar's** own canonical projections
//! ([`cad_app::layer_property`]) so a value a row shows can be sent straight back
//! to `cad_set_layer_property`. They are display/edit inputs, **not** behaviour:
//! only `off`/`frozen` are reflected in `visible` (through the database's own
//! `visible == !(off || frozen)` rule), and `locked`/`plottable`/colour/linetype/
//! lineweight/transparency still have no effect anywhere in the build.

use cad_app::layer_property;
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
    /// The layer's stored colour: the ACI identity (when the source had one) plus
    /// the resolved sRGB value.
    ///
    /// Both halves are sent because they are different facts (see
    /// [`cad_db::LayerColor`]): `aci` is what a re-export writes and `rgb` is what
    /// a UI can paint. `aci` is `null` for a true-colour layer.
    pub color: CadLayerColor,
    /// The layer's linetype **name** (a table entry, never a dangling reference).
    pub linetype: String,
    /// The layer's lineweight in [`cad_app::layer_property`]'s own vocabulary:
    /// `"ByLayer"`, `"Default"`, or the DXF `370` integer as a string in
    /// **1/100 mm** (`"35"` = 0.35 mm). Documented explicitly because it is *not*
    /// the entity property grammar's millimetre string.
    pub lineweight: String,
    /// Opacity as **percent transparent** (`"0"` = opaque), the write grammar's
    /// own unit, so the value can be sent back unchanged. A stored opacity
    /// outside `[0, 1]` reads `"opacity:<raw>"` and is not writable back.
    pub transparency: String,
    /// Layer is locked (stored only: no behaviour yet).
    pub locked: bool,
    /// Layer is frozen (hides the layer through the existing `visible` path).
    pub frozen: bool,
    /// Layer is off (hides the layer through the existing `visible` path).
    pub off: bool,
    /// Whether the layer plots (stored only: plotting is not implemented).
    pub plottable: bool,
}

/// The wire form of a layer's colour: the ACI identity (or `null`) plus
/// `#rrggbb`.
///
/// `rgb` is always present (it is the display value), `aci` is `null` for a
/// true-colour layer. Writing `aci:N` stores both halves; writing `#RRGGBB`
/// stores `aci: null`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadLayerColor {
    /// The source ACI index, or `null` for a true colour.
    pub aci: Option<u8>,
    /// Resolved sRGB value, lowercase `#rrggbb`.
    pub rgb: String,
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
/// The wire admission only carries names/booleans/strings; whether a name
/// resolves to a real layer is live session state and is decided on the owner
/// thread when this write is applied ([`CadLayerWrite::apply`]).
///
/// Despite the module's visibility emphasis this is the **layer mutation**
/// channel, not a visibility channel: it also carries the structural table writes
/// and the attribute write. A parallel enum would have to duplicate the same
/// bounded queue and the same popup/probe-mode gates for no gain, so the one
/// queue stays the single admission point. The table variants are real database
/// writes (one revision bump each, not undoable); the visibility/active variants
/// stay session state only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CadLayerWrite {
    /// Set the temporary visibility of one layer, addressed by name.
    SetVisible { name: String, visible: bool },
    /// Drop every temporary override (restore the database flags).
    Restore,
    /// Make one layer the active layer new entities are created on.
    SetActive { name: String },
    /// Create a new layer table entry (`CREATELAYER`) with this name.
    ///
    /// The name is validated by the database on the owner thread (trimmed,
    /// non-empty, legal characters, unique case-insensitively); a refusal is
    /// returned, never repaired.
    Create { name: String },
    /// Rename the layer addressed by `layer` to `name` (`RENAMELAYER`).
    Rename { layer: String, name: String },
    /// Delete the layer addressed by `layer` (`DELETELAYER`).
    ///
    /// The database refuses the default layer `0` and any layer an entity still
    /// references; the command layer then reports the real reason.
    Delete { layer: String },
    /// Write one attribute of the layer addressed by `layer`
    /// (`SETLAYERPROPERTY`).
    ///
    /// `key` is one of [`cad_app::layer_property::SUPPORTED_KEYS`] and `value`
    /// follows that key's documented grammar (one shared grammar — the layer's
    /// own units: `lineweight` is 1/100 mm, `transparency` is percent
    /// transparent). The application layer resolves the name, parses the value
    /// against the drawing's tables and writes through
    /// [`cad_db::DrawingDatabase::set_layer_attributes`] in one revision bump.
    /// Not undoable, like the other layer-table writes.
    SetProperty {
        layer: String,
        key: String,
        value: String,
    },
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
            Self::Create { name } => session.create_layer(name),
            Self::Rename { layer, name } => session.rename_layer(layer, name),
            Self::Delete { layer } => session.delete_layer(layer),
            Self::SetProperty { layer, key, value } => {
                session.set_layer_property(layer, key, value)
            }
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
            .map(|row| {
                let attributes = &row.attributes;
                CadLayer {
                    name: row.name,
                    visible: row.effective_visible,
                    database_visible: row.database_visible,
                    overridden: row.override_visible.is_some(),
                    current: row.id == active_layer,
                    // The stored attributes, projected with the write grammar's
                    // own canonical forms (one grammar for read and write, so a
                    // value a row shows can be sent back unchanged).
                    color: CadLayerColor {
                        aci: attributes.color.aci,
                        rgb: layer_property::color_hex(attributes.color.rgb),
                    },
                    linetype: attributes.linetype.clone(),
                    lineweight: layer_property::lineweight_value(attributes.lineweight),
                    transparency: layer_property::transparency_value(attributes.opacity),
                    locked: attributes.locked,
                    frozen: attributes.frozen,
                    off: attributes.off,
                    plottable: attributes.plottable,
                }
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

    /// Restore the **previous** layer state ([`CommandId::LayerPrevious`],
    /// `LAYERP`).
    ///
    /// Pops the newest snapshot of the session's bounded layer-override history
    /// and applies it, so the panel can offer the same one-step "back" the
    /// `LAYERP` selector does. Unlike [`Self::restore_layers`] it is **not** a
    /// clear-all: only the most recent change is undone, and it never pushes (so
    /// repeated calls walk back monotonically).
    ///
    /// An empty history is **not** an error: the host reports the
    /// `layers.previous.empty` diagnostic and nothing changes — the same "nothing
    /// to restore" shape `RESTORELAYERS` uses. Without a drawing it is the same
    /// explicit `layer.no_document` refusal as every other layer mutation.
    pub fn layer_previous(&self) -> Result<(), String> {
        self.require_layer_document()?;
        self.dispatch_command(CommandId::LayerPrevious)
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

    /// Create a new layer table entry (`CREATELAYER`).
    ///
    /// A **real database write**: the application layer validates the name (trim,
    /// empty/whitespace-only, the AutoCAD-illegal character set, unique
    /// **case-insensitively**) and commits one revision bump. Nothing is repaired
    /// here — a bad name comes back as the database's own explicit refusal, which
    /// is recorded for `cad_status.error`. Layer edits are **not undoable**
    /// (history has no layer patch kind).
    ///
    /// The answer is the ordinary queued ack; the panel re-reads [`Self::layers`],
    /// whose rows are projected from the drawing's own table.
    pub fn create_layer(&self, name: &str) -> Result<(), String> {
        self.require_layer_document()?;
        self.dispatch_payload(
            CommandId::CreateLayer,
            CommandPayload::CreateLayer(name.to_string()),
        )
    }

    /// Rename the layer currently named `layer` to `name` (`RENAMELAYER`).
    ///
    /// `layer` is resolved by the application layer with the **same** rule this
    /// module applies to the panel's names (exact match first, then a unique
    /// case-insensitive one), including its stable `layer.unknown` /
    /// `layer.ambiguous_name` reasons, so a row the panel selected always resolves
    /// the same way. Entity references are by id, so the rename retargets nothing.
    /// Not undoable.
    pub fn rename_layer(&self, layer: &str, name: &str) -> Result<(), String> {
        self.require_layer_document()?;
        self.dispatch_payload(
            CommandId::RenameLayer,
            CommandPayload::RenameLayer {
                layer: layer.to_string(),
                name: name.to_string(),
            },
        )
    }

    /// Delete the layer currently named `layer` (`DELETELAYER`).
    ///
    /// The application layer refuses the default layer `0` and any layer an entity
    /// still references (naming the count), so no `Entity.layer` is ever left
    /// dangling. When the deleted layer is the session's active layer the active
    /// layer falls back to `0`; the panel sees that in the next [`Self::layers`]
    /// read (`active`), never as a stale id. Not undoable.
    pub fn delete_layer(&self, layer: &str) -> Result<(), String> {
        self.require_layer_document()?;
        self.dispatch_payload(
            CommandId::DeleteLayer,
            CommandPayload::DeleteLayer {
                layer: layer.to_string(),
            },
        )
    }

    /// Write one attribute of the layer currently named `layer`
    /// (`SETLAYERPROPERTY`).
    ///
    /// A **real database write**: the application layer resolves `layer` with the
    /// same exact-then-unique-case-insensitive rule as [`Self::rename_layer`],
    /// parses `value` with the one layer grammar
    /// ([`cad_app::layer_property`]) — which checks a linetype name against the
    /// drawing's own table — and applies it to a **candidate** attribute set
    /// before publishing. One revision bump per real change, **no** undo entry
    /// (layer edits are not undoable).
    ///
    /// `off` / `frozen` are the only keys with behaviour: the database recomputes
    /// `Layer::visible` from the pair in the same mutation, so the scene hides a
    /// frozen layer through the existing visibility path. `locked`, `plottable`,
    /// colour, linetype, lineweight and transparency are stored only.
    ///
    /// The answer is the ordinary queued ack; the panel re-reads [`Self::layers`],
    /// whose rows carry the stored attributes in the write grammar's own units.
    pub fn set_layer_property(&self, layer: &str, key: &str, value: &str) -> Result<(), String> {
        self.require_layer_document()?;
        self.dispatch_payload(
            CommandId::SetLayerProperty,
            CommandPayload::SetLayerProperty {
                layer: layer.to_string(),
                key: key.to_string(),
                value: value.to_string(),
            },
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
