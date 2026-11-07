//! Long-form notes for the two selection-based **layer visibility** commands
//! [`crate::CommandId::IsolateLayers`] (`LAYISO`) and
//! [`crate::CommandId::LayOff`] (`LAYOFF`).
//!
//! Split out of `command_id.rs` to keep that catalogue inside the code-size
//! limit; the two variants keep compact docs and link here. Nothing is
//! implemented in this module — the write paths live in
//! `crate::layers_isolate` / `crate::layers`.

/// `LAYISO`: hide every layer that is not on a selected entity or is not the
/// current layer, using **session visibility overrides only**.
///
/// Payload is [`crate::CommandPayload::None`]; the selection is the input. The
/// keep-set is `{ layers of the selected entities } ∪ { current layer }` — the
/// current layer is included unconditionally because this build already refuses
/// to make an invisible layer current and refuses to hide the current layer
/// (`layer.current_off` / `layer.current_frozen`); see
/// [`crate::layers_isolate`]. When the current layer carried no selected entity
/// a `layers.isolate.current_layer` diagnostic names it.
///
/// The hides are published as **one atomic batch** through the same validated
/// path as [`crate::CommandId::SetLayerVisibilities`], so a refusal changes
/// nothing. No database write, no revision bump, no history entry and no
/// persisted preference: the drawing's own layer flags stay the authority. When
/// every layer is already in the keep-set the command writes no override at all
/// and reports `layers.isolate.noop`.
///
/// `LAYUNISO` and `LAYON` are **aliases of
/// [`crate::CommandId::RestoreLayers`]** (they clear the overrides); there is no
/// second restore path. `LAYERP` is **not** an alias: it is
/// [`crate::CommandId::LayerPrevious`], which restores the previous layer state.
/// Allowed in Viewer and Work — it is a session view operation, not an edit. An
/// empty selection is an explicit refusal (`layers.isolate.needs_selection`):
/// isolating nothing would hide every layer with no way back from the canvas.
pub struct IsolateLayersCommand;

/// `LAYOFF`: turn **off** every layer the selected entities live on, except the
/// current layer, using **session visibility overrides only**.
///
/// Payload is [`crate::CommandPayload::None`]; the selection is the input (at
/// least one entity, refused by name otherwise). It is the sibling of
/// [`crate::CommandId::IsolateLayers`] and the second half of AutoCAD's layer
/// surgery: where `LAYISO` keeps the selection's layers and hides the rest,
/// `LAYOFF` hides exactly the selection's layers.
///
/// The **current layer is never turned off**: this build already refuses to
/// make an invisible layer current and refuses to turn the current layer off
/// (`layer.current_off`, see `app_drawing::layer_edit`), and a hidden current
/// layer would silently receive new geometry. When the selection has entities
/// *only* on the current layer the command changes nothing and reports
/// `layers.off.current_layer`.
///
/// The hides are published as **one atomic batch** through the same validated
/// path as [`crate::CommandId::SetLayerVisibilities`] (session overrides only),
/// so a refusal changes nothing. No database write, no revision bump, no
/// history entry. Allowed in Viewer and Work — it is a session view operation,
/// not an edit.
///
/// `LAYON` is an **alias of [`crate::CommandId::RestoreLayers`]** (it clears the
/// overrides, which is what turning the layers back on means here); it carries
/// no `CommandId` of its own. `LAYERP` is a real command
/// ([`crate::CommandId::LayerPrevious`]), never an alias of this restore.
pub struct LayOffCommand;
