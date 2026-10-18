use super::*;

/// Stable command identifiers a host may reference from `ui.components.*` and
/// `ui.commandOverrides`. The namespace is dotted and independent of the Rust
/// [`cad_app::CommandId`] enum: several UI tool entries map onto one command
/// with a payload (for example `measure.distance` and `measure.area` both map to
/// `CommandId::Measure`). Unknown ids are rejected, never silently ignored.
pub const COMMAND_IDS: &[&str] = &[
    "file.open",
    "edit.undo",
    "edit.redo",
    "view.fit",
    "view.pan",
    "view.orbit",
    "view.reset",
    "view.standard",
    "view.projection",
    "view.switch2d3d",
    "measure.distance",
    "measure.polyline",
    "measure.angle",
    "measure.area",
    "measure.confirm",
    "measure.cancel",
    "layer.toggle",
    "layer.restore",
    "layout.switch",
    "draw.line",
    "draw.circle",
    "draw.move",
    "draw.trim",
    "backend.switch",
    "diagnostics.open",
    "mode.toggle",
];

/// Whether `id` is a command id this build understands.
pub fn is_known_command(id: &str) -> bool {
    COMMAND_IDS.contains(&id)
}

/// The full stable command id list (for hosts and presentation consumers).
pub fn command_ids() -> &'static [&'static str] {
    COMMAND_IDS
}

/// How a configured ribbon command renders its icon and label.
///
/// `IconAndLabel` is the historical behavior and the default; a host opts into
/// `IconOnly` or `LabelOnly` per ribbon group. The value is pure presentation
/// data and never changes which command a click dispatches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RibbonCommandDisplay {
    #[default]
    IconAndLabel,
    IconOnly,
    LabelOnly,
}

/// A configured ribbon command after visibility resolution.
///
/// Only commands whose effective `command_visibility` is true are present:
/// hiding an entry removes the button but never disables the command itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRibbonCommand {
    pub id: String,
    pub visible: bool,
    /// Render mode inherited from the owning group.
    pub display: RibbonCommandDisplay,
}

/// A configured ribbon group after visibility resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRibbonGroup {
    pub id: String,
    /// Catalog key from the host config; the UI resolves the display text.
    pub label: String,
    pub commands: Vec<ResolvedRibbonCommand>,
}

/// A configured ribbon tab after visibility resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRibbonTab {
    pub id: String,
    /// Catalog key from the host config; the UI resolves the display text.
    pub label: String,
    pub groups: Vec<ResolvedRibbonGroup>,
}

/// The resolved configured ribbon. Empty `tabs` means "use the shipped catalog
/// ribbon" (see [`RibbonComponent`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedRibbon {
    pub tabs: Vec<ResolvedRibbonTab>,
}

/// Resolve the host-configured ribbon against effective command visibility.
///
/// Tab, group and command order is preserved. A command is kept only when
/// `command_visibility[id]` is true; hidden commands are absent, matching the
/// rule that hiding an entry never disables the command. Labels stay catalog
/// keys because `cad-app` owns no message catalog; the UI resolves them.
pub fn resolve_ribbon(
    config: &ViewerConfig,
    command_visibility: &BTreeMap<String, bool>,
) -> ResolvedRibbon {
    let tabs = config
        .ui
        .components
        .ribbon
        .tabs
        .iter()
        .map(|tab| ResolvedRibbonTab {
            id: tab.id.clone(),
            label: tab.label.clone(),
            groups: tab
                .groups
                .iter()
                .map(|group| ResolvedRibbonGroup {
                    id: group.id.clone(),
                    label: group.label.clone(),
                    commands: group
                        .commands
                        .iter()
                        .filter(|id| command_visibility.get(*id).copied().unwrap_or(false))
                        .map(|id| ResolvedRibbonCommand {
                            id: id.clone(),
                            visible: true,
                            display: group.display,
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();
    ResolvedRibbon { tabs }
}

/// UI chrome preset. `canvasOnly` forces every application-UI entry hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Preset {
    #[default]
    Full,
    Minimal,
    CanvasOnly,
}

/// Which chrome components a [`Preset`] permits, *before* the per-component
/// `visible` flags and the layout mode are applied.
///
/// This is the single, readable table of the preset policy; [`UiPresentationModel::resolve`]
/// consumes it instead of scattering preset checks through the field
/// assignments. The booleans are named after the resolved fields they gate:
///
/// | field               | `Full` | `Minimal` | `CanvasOnly` |
/// |---------------------|--------|-----------|--------------|
/// | `ribbon`            | yes    | no        | no           |
/// | `command_bar`       | yes    | no        | no           |
/// | `status_bar`        | yes    | no        | no           |
/// | `navigation`        | yes    | yes       | no           |
/// | `layout_tabs`       | yes    | yes       | no           |
/// | `layer_panel`       | yes    | yes       | no           |
/// | `properties_panel`  | yes    | yes       | no           |
///
/// `Minimal` is therefore "application frame plus canvas navigation, layout
/// tabs and dock panels, with the ribbon/command/status *entry points* hidden".
/// Which commands remain reachable is a separate, explicit decision made by
/// [`preset_command_visible`]: hiding an entry point never disables a command,
/// so the command surface is narrowed by a documented whitelist rather than by
/// the component table above.
///
/// `CanvasOnly` is strictly stronger than `Minimal`: every component minimal
/// keeps is also off, plus the application frame itself and every command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PresetComponents {
    pub(super) ribbon: bool,
    pub(super) command_bar: bool,
    pub(super) status_bar: bool,
    pub(super) navigation: bool,
    pub(super) layout_tabs: bool,
    pub(super) layer_panel: bool,
    pub(super) properties_panel: bool,
}

/// The explicit component set each [`Preset`] selects. See [`PresetComponents`]
/// for the truth table and the rationale behind `Minimal`.
pub(super) fn preset_components(preset: Preset) -> PresetComponents {
    match preset {
        Preset::Full => PresetComponents {
            ribbon: true,
            command_bar: true,
            status_bar: true,
            navigation: true,
            layout_tabs: true,
            layer_panel: true,
            properties_panel: true,
        },
        Preset::Minimal => PresetComponents {
            ribbon: false,
            command_bar: false,
            status_bar: false,
            navigation: true,
            layout_tabs: true,
            layer_panel: true,
            properties_panel: true,
        },
        Preset::CanvasOnly => PresetComponents {
            ribbon: false,
            command_bar: false,
            status_bar: false,
            navigation: false,
            layout_tabs: false,
            layer_panel: false,
            properties_panel: false,
        },
    }
}

/// The commands [`Preset::Minimal`] keeps reachable.
///
/// Minimal is a *review/measure* surface, not a stripped-down editing surface:
/// it keeps reading and markup capabilities (measure, annotate, view, layers,
/// layouts, open/export/import and diagnostics) and hides CAD authoring
/// commands (undo/redo, draw and move/trim) plus host-mode switches. This is
/// the Microsoft/AutoCAD-style "reviewer" reduction: a reviewer can open a
/// drawing, measure it, annotate it, navigate and inspect layer/layout state,
/// but cannot edit geometry.
///
/// The list is authoritative for the whitelisted surface: a `Minimal` command
/// is shown only when it appears here and is not in [`MINIMAL_HIDDEN_COMMANDS`]
/// (feature gates and `commandOverrides` still apply on top). `full` keeps
/// every command and `canvasOnly` hides every command, so
/// `canvasOnly ⊆ minimal ⊆ full`.
pub(super) const MINIMAL_VISIBLE_COMMANDS: &[&str] = &[
    // Measurement tools and their confirm/cancel/save lifecycle.
    "measure.distance",
    "measure.polyline",
    "measure.angle",
    "measure.area",
    "measure.confirm",
    "measure.cancel",
    // Canvas view navigation and projection/standard-view controls.
    "view.fit",
    "view.pan",
    "view.orbit",
    "view.reset",
    "view.standard",
    "view.projection",
    "view.switch2d3d",
    // Document open.
    "file.open",
    // Layer and layout inspection.
    "layer.toggle",
    "layer.restore",
    "layout.switch",
    // Diagnostics remain available for troubleshooting the reduced surface.
    "diagnostics.open",
];

/// The commands [`Preset::Minimal`] hides explicitly.
///
/// These are CAD authoring/editing commands (`edit.undo`, `edit.redo`,
/// `draw.line`, `draw.circle`, `draw.move`, `draw.trim`) and host-mode switches
/// (`mode.toggle`, `backend.switch`); none are part of a review-only surface.
/// A command is reachable under `Minimal` only when it is whitelisted and not
/// blacklisted; the test `minimal_preset_command_set_is_explicit` asserts the
/// two lists are disjoint and partition `COMMAND_IDS` exactly, so adding a new
/// command id without classifying it fails the suite.
pub(super) const MINIMAL_HIDDEN_COMMANDS: &[&str] = &[
    "edit.undo",
    "edit.redo",
    "draw.line",
    "draw.circle",
    "draw.move",
    "draw.trim",
    "mode.toggle",
    "backend.switch",
];

/// Whether command `id` is reachable under `preset`, before feature gates and
/// `commandOverrides` are applied.
///
/// `Full` keeps every command, `CanvasOnly` hides every command and `Minimal`
/// keeps exactly the commands in [`MINIMAL_VISIBLE_COMMANDS`] that are not also
/// in [`MINIMAL_HIDDEN_COMMANDS`]. `id` is expected to be a known command id
/// (the caller iterates `COMMAND_IDS`); an unknown id is hidden, which is the
/// safe default.
pub(super) fn preset_command_visible(preset: Preset, id: &str) -> bool {
    match preset {
        Preset::Full => true,
        Preset::CanvasOnly => false,
        Preset::Minimal => {
            MINIMAL_VISIBLE_COMMANDS.contains(&id) && !MINIMAL_HIDDEN_COMMANDS.contains(&id)
        }
    }
}
