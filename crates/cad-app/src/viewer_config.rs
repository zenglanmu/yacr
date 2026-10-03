//! Host-owned, data-only viewer configuration and pure shell presentation.
//!
//! The protocol is defined by `docs/ui-spec/ui-desc.md` and must never contain
//! executable script or arbitrary callbacks: every field is plain data. Parsing
//! order is "built-in defaults → preset → host override → host-allowed user
//! preference"; missing fields inherit, explicit `false` survives, arrays are
//! replaced wholesale and user preferences may never re-enable a capability or
//! component the host/preset disabled (they clamp, they do not error).
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Stable command identifiers a host may reference from `ui.components.*` and
/// `ui.commandOverrides`. The namespace is dotted and independent of the Rust
/// [`cad_app::CommandId`] enum: several UI tool entries map onto one command
/// with a payload (for example `measure.distance` and `measure.area` both map to
/// `CommandId::Measure`). Unknown ids are rejected, never silently ignored.
pub const COMMAND_IDS: &[&str] = &[
    "file.open",
    "file.exportAnnotations",
    "file.importAnnotations",
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
    "measure.save",
    "annotation.text",
    "annotation.leader",
    "annotation.rectangle",
    "annotation.ellipse",
    "annotation.freehand",
    "annotation.cloud",
    "annotation.confirm",
    "annotation.cancel",
    "annotation.delete",
    "annotation.select",
    "annotation.visibility",
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
/// | `commands`          | yes    | yes       | no           |
///
/// `Minimal` is therefore "application frame plus canvas navigation, layout
/// tabs and dock panels, with only the ribbon/command/status *entry points*
/// hidden". Hiding an entry point never disables the command itself, so
/// `commands` stays `true`: minimal keeps command reachability (keyboard
/// shortcuts, navigation toolbar, panels) while dropping the top/bottom chrome.
///
/// `CanvasOnly` is strictly stronger than `Minimal`: every component minimal
/// keeps is also off, plus the application frame itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PresetComponents {
    ribbon: bool,
    command_bar: bool,
    status_bar: bool,
    navigation: bool,
    layout_tabs: bool,
    layer_panel: bool,
    properties_panel: bool,
    commands: bool,
}

/// The explicit component set each [`Preset`] selects. See [`PresetComponents`]
/// for the truth table and the rationale behind `Minimal`.
fn preset_components(preset: Preset) -> PresetComponents {
    match preset {
        Preset::Full => PresetComponents {
            ribbon: true,
            command_bar: true,
            status_bar: true,
            navigation: true,
            layout_tabs: true,
            layer_panel: true,
            properties_panel: true,
            commands: true,
        },
        Preset::Minimal => PresetComponents {
            ribbon: false,
            command_bar: false,
            status_bar: false,
            navigation: true,
            layout_tabs: true,
            layer_panel: true,
            properties_panel: true,
            commands: true,
        },
        Preset::CanvasOnly => PresetComponents {
            ribbon: false,
            command_bar: false,
            status_bar: false,
            navigation: false,
            layout_tabs: false,
            layer_panel: false,
            properties_panel: false,
            commands: false,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayoutMode {
    #[default]
    Auto,
    Desktop,
    Compact,
    Mobile,
}

/// Dock/side placement of a panel or toolbar. `auto` lets the layout decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Placement {
    #[default]
    Auto,
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Breakpoints {
    pub compact_below: f64,
    pub mobile_below: f64,
}
impl Default for Breakpoints {
    fn default() -> Self {
        Self {
            compact_below: 1200.0,
            mobile_below: 720.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct LayoutConfig {
    pub mode: LayoutMode,
    pub breakpoints: Breakpoints,
}

/// One ribbon group: a stable id, an i18n catalog key label and command ids.
///
/// `label` is a catalog key (for example `"ribbon.review"`) resolved by the UI
/// through `MessageSource`; configuration must not carry literal user-facing
/// text (AGENTS i18n rule). `id` is a stable, non-user-visible identifier.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RibbonGroup {
    pub id: String,
    pub label: String,
    pub commands: Vec<String>,
    /// How this group's commands render. Defaults to `iconAndLabel`, which is
    /// exactly the pre-existing behavior.
    pub display: RibbonCommandDisplay,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RibbonTab {
    pub id: String,
    pub label: String,
    pub groups: Vec<RibbonGroup>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RibbonComponent {
    pub visible: bool,
    /// Empty means "use the shipped catalog ribbon". A non-empty list replaces
    /// the tab model wholesale (arrays replace, never merge).
    pub tabs: Vec<RibbonTab>,
}
impl Default for RibbonComponent {
    fn default() -> Self {
        Self {
            visible: true,
            tabs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelComponent {
    pub visible: bool,
    pub placement: Placement,
    /// Whether the panel starts expanded. Docked panels default to open so the
    /// shipped desktop arrangement is unchanged; the protocol example sets it
    /// explicitly to `false`.
    pub initially_open: bool,
}
impl Default for PanelComponent {
    fn default() -> Self {
        Self {
            visible: true,
            placement: Placement::Auto,
            initially_open: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct NavigationToolbarComponent {
    pub visible: bool,
    pub placement: Placement,
    pub commands: Vec<String>,
}
impl Default for NavigationToolbarComponent {
    fn default() -> Self {
        Self {
            visible: true,
            placement: Placement::Right,
            commands: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ToggleComponent {
    pub visible: bool,
}
impl Default for ToggleComponent {
    fn default() -> Self {
        Self { visible: true }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Components {
    pub ribbon: RibbonComponent,
    pub layer_panel: PanelComponent,
    pub properties_panel: PanelComponent,
    pub navigation_toolbar: NavigationToolbarComponent,
    pub command_bar: ToggleComponent,
    pub layout_tabs: ToggleComponent,
    pub status_bar: ToggleComponent,
}

/// Per-command override. Only `visible: false` is meaningful today; an omitted
/// override leaves the command's feature/preset visibility untouched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandOverride {
    pub visible: bool,
}
impl Default for CommandOverride {
    fn default() -> Self {
        Self { visible: true }
    }
}

/// Paths a host allows a user preference to touch. A path is a dotted JSON path
/// such as `ui.components.layerPanel.initiallyOpen`; a leaf is allowed when it
/// equals or is a descendant of one of these paths.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct UserCustomization {
    pub allowed_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct UiConfig {
    pub preset: Preset,
    pub layout: LayoutConfig,
    pub components: Components,
    pub command_overrides: BTreeMap<String, CommandOverride>,
    pub user_customization: UserCustomization,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct AnnotationFeatures {
    pub create: bool,
    pub update: bool,
    pub delete: bool,
    pub import: bool,
    pub export: bool,
}
impl Default for AnnotationFeatures {
    fn default() -> Self {
        Self {
            create: true,
            update: true,
            delete: true,
            import: true,
            export: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct FeaturesConfig {
    pub measure: bool,
    pub annotations: AnnotationFeatures,
}
impl Default for FeaturesConfig {
    fn default() -> Self {
        Self {
            measure: true,
            annotations: AnnotationFeatures::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewOverlays {
    pub axes: bool,
    pub grid: bool,
    pub selection_highlight: bool,
    pub snap_hints: bool,
    pub annotations: bool,
}
impl Default for ViewOverlays {
    fn default() -> Self {
        Self {
            axes: true,
            // AutoCAD starts with the grid off and the ui-spec example uses
            // `grid: false`; a host that wants it enables it explicitly.
            grid: false,
            selection_highlight: true,
            snap_hints: true,
            annotations: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewConfig {
    pub overlays: ViewOverlays,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct InteractionConfig {
    pub pointer: bool,
    pub touch: bool,
    pub keyboard_shortcuts: bool,
}
impl Default for InteractionConfig {
    fn default() -> Self {
        Self {
            pointer: true,
            touch: true,
            keyboard_shortcuts: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewerConfig {
    pub schema_version: u32,
    pub ui: UiConfig,
    pub features: FeaturesConfig,
    pub view: ViewConfig,
    pub interaction: InteractionConfig,
}
impl Default for ViewerConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            ui: UiConfig::default(),
            features: FeaturesConfig::default(),
            view: ViewConfig::default(),
            interaction: InteractionConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub path: String,
    pub reason: String,
}

impl ConfigError {
    fn new(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.reason)
    }
}

impl std::error::Error for ConfigError {}

fn join_path(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

/// The only map-typed field in the protocol; new keys are legal here.
fn is_map_path(path: &str) -> bool {
    path == "ui.commandOverrides"
}

/// Capabilities and component visibility a user preference may never re-enable.
fn is_clamped_path(path: &str) -> bool {
    if path == "features" || path.starts_with("features.") {
        return true;
    }
    // A command-override visibility leaf is capability-bearing too: a user may
    // hide a command but must never re-enable one a preset/feature disabled.
    if path == "ui.commandOverrides" || path.starts_with("ui.commandOverrides.") {
        return true;
    }
    // `ui.components.<name>.visible`
    let parts: Vec<&str> = path.split('.').collect();
    parts.len() == 4 && parts[0] == "ui" && parts[1] == "components" && parts[3] == "visible"
}

fn path_allowed(path: &str, allowed: &[String]) -> bool {
    allowed
        .iter()
        .any(|a| path == a || path.starts_with(&format!("{a}.")))
}

fn path_or_descendant_allowed(child: &str, allowed: &[String]) -> bool {
    allowed.iter().any(|a| {
        child == a || child.starts_with(&format!("{a}.")) || a.starts_with(&format!("{child}."))
    })
}

/// Project a preference patch onto the currently-allowed paths. Used by hosts
/// to persist only what a later load may legitimately re-apply.
fn project_allowed(patch: &Value, path: &str, allowed: &[String]) -> Value {
    match patch {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, value) in map {
                let child = join_path(path, key.as_str());
                let projected = project_allowed(value, &child, allowed);
                let keep = if projected.is_object() {
                    projected
                        .as_object()
                        .map(|m| !m.is_empty())
                        .unwrap_or(false)
                } else {
                    path_allowed(&child, allowed)
                };
                if keep {
                    out.insert(key.clone(), projected);
                }
            }
            Value::Object(out)
        }
        leaf => leaf.clone(),
    }
}

/// Reject `null` anywhere in a value tree (config has no null sentinel).
fn ensure_no_null(value: &Value, path: &str) -> Result<(), ConfigError> {
    match value {
        Value::Null => Err(ConfigError::new(path, "null is not a configuration value")),
        Value::Object(map) => {
            for (key, child) in map {
                ensure_no_null(child, &join_path(path, key))?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                ensure_no_null(child, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn type_error(path: &str) -> ConfigError {
    ConfigError::new(path, "configuration value has wrong type")
}

/// Deep-merge a preference patch, creating missing object nodes.
///
/// Unlike the schema-aware [`merge`], the preference accumulator has no fixed
/// shape: it only needs to remember what the host allowed. Arrays replace, null
/// is rejected; a type clash between the existing preference and the patch is
/// rejected rather than silently overwritten.
fn deep_merge_create(target: &mut Value, patch: &Value) -> Result<(), ConfigError> {
    match (&mut *target, patch) {
        (Value::Object(target), Value::Object(patch)) => {
            for (key, value) in patch {
                match target.get_mut(key) {
                    Some(slot) => deep_merge_create(slot, value)?,
                    None => {
                        ensure_no_null(value, key)?;
                        target.insert(key.clone(), value.clone());
                    }
                }
            }
            Ok(())
        }
        (_, Value::Null) => Err(ConfigError::new(
            "preference",
            "null is not a configuration value",
        )),
        (slot, leaf) => {
            ensure_no_null(leaf, "")?;
            let valid = matches!(
                (&*slot, leaf),
                (Value::Bool(_), Value::Bool(_))
                    | (Value::String(_), Value::String(_))
                    | (Value::Number(_), Value::Number(_))
                    | (Value::Array(_), Value::Array(_))
                    | (Value::Object(_), Value::Object(_))
            );
            if !valid {
                return Err(type_error("preference"));
            }
            *slot = leaf.clone();
            Ok(())
        }
    }
}

/// Structured merge for host updates: objects merge key-wise, arrays replace,
/// explicit `false` survives and `null` is rejected. New keys are only legal
/// inside the command-override map; everything else must exist already.
fn merge(target: &mut Value, patch: Value, path: &str) -> Result<(), ConfigError> {
    if let Value::Object(patch) = patch {
        let target = target
            .as_object_mut()
            .ok_or_else(|| ConfigError::new(path, "expected object"))?;
        for (key, value) in patch {
            let child = join_path(path, key.as_str());
            match target.get_mut(&key) {
                Some(slot) => merge(slot, value, &child)?,
                None if is_map_path(path) => {
                    ensure_no_null(&value, &child)?;
                    if !value.is_object() {
                        return Err(type_error(&child));
                    }
                    target.insert(key, value);
                }
                None => {
                    return Err(ConfigError::new(child, "unsupported configuration field"));
                }
            }
        }
        Ok(())
    } else {
        if patch.is_null() {
            return Err(ConfigError::new(path, "null is not a configuration value"));
        }
        let valid = matches!(
            (&*target, &patch),
            (Value::Bool(_), Value::Bool(_))
                | (Value::String(_), Value::String(_))
                | (Value::Number(_), Value::Number(_))
                | (Value::Array(_), Value::Array(_))
                | (Value::Object(_), Value::Object(_))
        );
        if !valid {
            return Err(type_error(path));
        }
        *target = patch;
        Ok(())
    }
}

/// Apply a host-allowed user preference onto a config value derived from the
/// host value. `strict` rejects disallowed paths (an explicit API call); the
/// lenient mode used when re-applying after `set_config` skips paths the host
/// has since revoked.
fn apply_preference(
    target: &mut Value,
    patch: &Value,
    path: &str,
    allowed: &[String],
    strict: bool,
) -> Result<(), ConfigError> {
    match patch {
        Value::Object(map) => {
            let target = target
                .as_object_mut()
                .ok_or_else(|| ConfigError::new(path, "expected object"))?;
            for (key, value) in map {
                let child = join_path(path, key.as_str());
                if let Some(slot) = target.get_mut(key) {
                    apply_preference(slot, value, &child, allowed, strict)?;
                } else if is_map_path(path) {
                    if path_or_descendant_allowed(&child, allowed) {
                        ensure_no_null(value, &child)?;
                        if !value.is_object() {
                            return Err(type_error(&child));
                        }
                        // A new command-override entry is capability-bearing: a
                        // user preference may create it but must never set its
                        // `visible` to true when the host has not enabled it.
                        let value = if let Value::Object(mut entry) = value.clone() {
                            if entry.contains_key("visible") {
                                entry.insert("visible".into(), Value::Bool(false));
                            }
                            Value::Object(entry)
                        } else {
                            value.clone()
                        };
                        target.insert(key.clone(), value);
                    } else if strict {
                        return Err(ConfigError::new(
                            child,
                            "user preference path is not allowed by host customization",
                        ));
                    }
                } else if strict {
                    return Err(ConfigError::new(child, "unsupported configuration field"));
                }
            }
            Ok(())
        }
        Value::Null => Err(ConfigError::new(path, "null is not a configuration value")),
        leaf => {
            if !path_allowed(path, allowed) {
                if strict {
                    return Err(ConfigError::new(
                        path,
                        "user preference path is not allowed by host customization",
                    ));
                }
                return Ok(());
            }
            let type_ok = matches!(
                (&*target, leaf),
                (Value::Bool(_), Value::Bool(_))
                    | (Value::String(_), Value::String(_))
                    | (Value::Number(_), Value::Number(_))
                    | (Value::Array(_), Value::Array(_))
                    | (Value::Object(_), Value::Object(_))
            );
            if !type_ok {
                return Err(type_error(path));
            }
            if is_clamped_path(path) {
                if let (Value::Bool(host), Value::Bool(pref)) = (&*target, leaf) {
                    // Clamp, never error: a user may turn a capability off, but
                    // may not switch on one the host/preset disabled.
                    *target = Value::Bool(*host && *pref);
                    return Ok(());
                }
            }
            *target = leaf.clone();
            Ok(())
        }
    }
}

/// Resolve a dotted path inside a serialized config value. Numeric segments
/// index arrays. Returns `None` when the path does not exist.
fn lookup_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    if path.is_empty() {
        return Some(current);
    }
    for segment in path.split('.') {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Validate one host-allowed path against the schema. A command-override path
/// names a whitelisted command id (which itself may contain dots).
fn allowed_path_is_known(config: &Value, path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if let Some(rest) = path.strip_prefix("ui.commandOverrides.") {
        return COMMAND_IDS
            .iter()
            .any(|id| rest == *id || rest.starts_with(&format!("{id}.")));
    }
    lookup_path(config, path).is_some()
}

impl ViewerConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != 1 {
            return Err(ConfigError::new(
                "schemaVersion",
                "unsupported schema version",
            ));
        }
        let b = &self.ui.layout.breakpoints;
        if !b.mobile_below.is_finite() || b.mobile_below <= 0.0 {
            return Err(ConfigError::new(
                "ui.layout.breakpoints.mobileBelow",
                "expected positive finite logical pixels",
            ));
        }
        if !b.compact_below.is_finite() || b.compact_below <= b.mobile_below {
            return Err(ConfigError::new(
                "ui.layout.breakpoints.compactBelow",
                "must exceed mobileBelow",
            ));
        }
        for key in self.ui.command_overrides.keys() {
            if !is_known_command(key) {
                return Err(ConfigError::new(
                    format!("ui.commandOverrides.{key}"),
                    "unknown command id",
                ));
            }
        }
        let mut seen_tabs: Vec<&str> = Vec::new();
        for (tab_index, tab) in self.ui.components.ribbon.tabs.iter().enumerate() {
            let tab_path = format!("ui.components.ribbon.tabs[{tab_index}]");
            if tab.id.is_empty() {
                return Err(ConfigError::new(
                    format!("{tab_path}.id"),
                    "ribbon tab id must not be empty",
                ));
            }
            if seen_tabs.contains(&tab.id.as_str()) {
                return Err(ConfigError::new(
                    format!("{tab_path}.id"),
                    "duplicate ribbon tab id",
                ));
            }
            seen_tabs.push(tab.id.as_str());
            let mut seen_groups: Vec<&str> = Vec::new();
            for (group_index, group) in tab.groups.iter().enumerate() {
                let group_path = format!("{tab_path}.groups[{group_index}]");
                if group.id.is_empty() {
                    return Err(ConfigError::new(
                        format!("{group_path}.id"),
                        "ribbon group id must not be empty",
                    ));
                }
                if seen_groups.contains(&group.id.as_str()) {
                    return Err(ConfigError::new(
                        format!("{group_path}.id"),
                        "duplicate ribbon group id",
                    ));
                }
                seen_groups.push(group.id.as_str());
                for (command_index, command) in group.commands.iter().enumerate() {
                    if !is_known_command(command) {
                        return Err(ConfigError::new(
                            format!("{group_path}.commands[{command_index}]"),
                            "unknown command id",
                        ));
                    }
                }
            }
        }
        for (index, command) in self
            .ui
            .components
            .navigation_toolbar
            .commands
            .iter()
            .enumerate()
        {
            if !is_known_command(command) {
                return Err(ConfigError::new(
                    format!("ui.components.navigationToolbar.commands[{index}]"),
                    "unknown command id",
                ));
            }
        }
        let value = serde_json::to_value(self).expect("serializable config");
        for (index, path) in self.ui.user_customization.allowed_paths.iter().enumerate() {
            if !allowed_path_is_known(&value, path) {
                return Err(ConfigError::new(
                    format!("ui.userCustomization.allowedPaths[{index}]"),
                    "unknown configuration path",
                ));
            }
        }
        Ok(())
    }
}

/// Receives an effective-config change. Data-driven Rust callbacks only; the
/// configuration JSON itself never carries script.
pub type ConfigObserver = Rc<dyn Fn(&ViewerConfig, u64)>;

/// Configuration store that never owns a CAD session.
///
/// It keeps the host config, the accumulated host-allowed user preference and
/// the resolved effective config. Failed updates are atomic: the previous
/// effective config and revision are preserved. Observers fire only when the
/// effective config really changed.
pub struct ViewerConfigStore {
    host: ViewerConfig,
    preference: Value,
    effective: ViewerConfig,
    pub revision: u64,
    observers: Vec<ConfigObserver>,
}

impl Default for ViewerConfigStore {
    fn default() -> Self {
        Self {
            host: ViewerConfig::default(),
            preference: Value::Object(Map::new()),
            effective: ViewerConfig::default(),
            revision: 0,
            observers: Vec::new(),
        }
    }
}

impl fmt::Debug for ViewerConfigStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewerConfigStore")
            .field("host", &self.host)
            .field("preference", &self.preference)
            .field("effective", &self.effective)
            .field("revision", &self.revision)
            .field("observers", &self.observers.len())
            .finish()
    }
}

impl ViewerConfigStore {
    /// The resolved config (host + allowed user preference, clamped).
    pub fn effective(&self) -> &ViewerConfig {
        &self.effective
    }
    /// The host-supplied config before user preference.
    pub fn host_config(&self) -> &ViewerConfig {
        &self.host
    }

    /// The accumulated user preference patch (possibly empty object).
    pub fn user_preference(&self) -> &Value {
        &self.preference
    }

    /// Subscribe to effective-config changes. Observers receive `(config, revision)`.
    pub fn subscribe(&mut self, observer: ConfigObserver) {
        self.observers.push(observer);
    }

    /// Parse and replace the whole host config from JSON. The stored user
    /// preference is re-applied and clamped. Atomic on any failure.
    pub fn set_config_json(&mut self, text: &str) -> Result<(), ConfigError> {
        let config: ViewerConfig = serde_json::from_str(text).map_err(|e| {
            ConfigError::new(
                "config",
                format!("configuration does not match schema: {e}"),
            )
        })?;
        self.set_config(config)
    }

    /// Parse a JSON patch and merge it into the host config.
    pub fn update_config_json(&mut self, text: &str) -> Result<(), ConfigError> {
        let patch: Value = serde_json::from_str(text)
            .map_err(|e| ConfigError::new("config", format!("invalid JSON: {e}")))?;
        self.update_config(patch)
    }

    /// Parse a JSON user-preference patch and merge it under the host's allowed
    /// paths. Atomic on any failure.
    pub fn apply_user_preference_json(&mut self, text: &str) -> Result<(), ConfigError> {
        let patch: Value = serde_json::from_str(text)
            .map_err(|e| ConfigError::new("preference", format!("invalid JSON: {e}")))?;
        self.apply_user_preference(patch)
    }

    /// The effective config as JSON (never fails for an in-memory config).
    pub fn effective_json(&self) -> String {
        serde_json::to_string(&self.effective).unwrap_or_else(|_| "{}".to_string())
    }

    /// The accumulated user preference patch as JSON.
    pub fn user_preference_json(&self) -> String {
        serde_json::to_string(&self.preference).unwrap_or_else(|_| "{}".to_string())
    }

    /// The persisted projection of the user preference: only leaves still under
    /// `allowedPaths`. What a host may safely write to durable storage.
    pub fn projected_user_preference_json(&self) -> String {
        let allowed = &self.effective.ui.user_customization.allowed_paths;
        let projected = project_allowed(&self.preference, "", allowed);
        serde_json::to_string(&projected).unwrap_or_else(|_| "{}".to_string())
    }

    /// Replace the host config; the stored user preference is re-applied and
    /// clamped against the new host allowances. Atomic on validation failure.
    pub fn set_config(&mut self, config: ViewerConfig) -> Result<(), ConfigError> {
        config.validate()?;
        let effective = self.resolve(&config, &self.preference)?;
        self.host = config;
        self.publish(effective);
        Ok(())
    }

    /// Structured merge into the host config (see [`ViewerConfigStore`]).
    pub fn update_config(&mut self, patch: Value) -> Result<(), ConfigError> {
        let mut candidate = serde_json::to_value(&self.host).expect("serializable config");
        merge(&mut candidate, patch, "")?;
        let host = serde_json::from_value(candidate).map_err(|e| {
            ConfigError::new(
                "config",
                format!("configuration does not match schema: {e}"),
            )
        })?;
        self.set_config(host)
    }

    /// Merge a user preference patch. Every leaf must be inside the host's
    /// `allowedPaths`; capabilities/components are clamped, never re-enabled.
    /// Atomic: a rejected or invalid preference leaves the store unchanged.
    pub fn apply_user_preference(&mut self, patch: Value) -> Result<(), ConfigError> {
        if !patch.is_object() {
            return Err(ConfigError::new(
                "preference",
                "user preference must be an object",
            ));
        }
        let allowed = self.host.ui.user_customization.allowed_paths.clone();
        // Validate strictly against the host allowances before mutating state.
        apply_preference(
            &mut serde_json::to_value(&self.host).expect("serializable config"),
            &patch,
            "",
            &allowed,
            true,
        )?;
        let mut preference = self.preference.clone();
        deep_merge_create(&mut preference, &patch)?;
        let effective = self.resolve(&self.host, &preference)?;
        self.preference = preference;
        self.publish(effective);
        Ok(())
    }

    /// Drop every stored user preference and recompute.
    pub fn clear_user_preference(&mut self) {
        self.preference = Value::Object(Map::new());
        if let Ok(effective) = self.resolve(&self.host, &self.preference) {
            self.publish(effective);
        }
    }

    /// Resolve `host` + `preference` into a validated effective config.
    fn resolve(
        &self,
        host: &ViewerConfig,
        preference: &Value,
    ) -> Result<ViewerConfig, ConfigError> {
        let mut candidate = serde_json::to_value(host).expect("serializable config");
        let allowed = host.ui.user_customization.allowed_paths.clone();
        apply_preference(&mut candidate, preference, "", &allowed, false)?;
        let effective: ViewerConfig = serde_json::from_value(candidate).map_err(|e| {
            ConfigError::new(
                "config",
                format!("configuration does not match schema: {e}"),
            )
        })?;
        effective.validate()?;
        Ok(effective)
    }

    /// Store the new effective config and notify observers iff it changed.
    fn publish(&mut self, effective: ViewerConfig) {
        if effective == self.effective {
            return;
        }
        self.effective = effective;
        self.revision += 1;
        let observers = self.observers.clone();
        for observer in &observers {
            observer(&self.effective, self.revision);
        }
    }
}

/// Overlay visibility resolved for the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayVisibility {
    pub axes: bool,
    pub grid: bool,
    pub selection_highlight: bool,
    pub snap_hints: bool,
    pub annotations: bool,
}

impl Default for OverlayVisibility {
    /// Every overlay is visible. `OverlayInputs::default()` therefore keeps the
    /// historical "nothing is hidden" behavior; only an explicit host push
    /// turns an overlay off.
    fn default() -> Self {
        Self {
            axes: true,
            grid: true,
            selection_highlight: true,
            snap_hints: true,
            annotations: true,
        }
    }
}

impl From<ViewOverlays> for OverlayVisibility {
    fn from(value: ViewOverlays) -> Self {
        Self {
            axes: value.axes,
            grid: value.grid,
            selection_highlight: value.selection_highlight,
            snap_hints: value.snap_hints,
            annotations: value.annotations,
        }
    }
}

/// Capability visibility resolved for the shell entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureVisibility {
    pub measure: bool,
    pub annotation_create: bool,
    pub annotation_update: bool,
    pub annotation_delete: bool,
    pub annotation_import: bool,
    pub annotation_export: bool,
}

/// The pure presentation model the shell renders. No CAD state is embedded.
#[derive(Debug, Clone, PartialEq)]
pub struct UiPresentationModel {
    pub layout: LayoutMode,
    pub application_ui: bool,
    pub ribbon: bool,
    pub layer_panel: bool,
    pub properties_panel: bool,
    pub layer_panel_initially_open: bool,
    pub properties_panel_initially_open: bool,
    pub navigation: bool,
    pub command_bar: bool,
    pub layout_tabs: bool,
    pub status_bar: bool,
    pub pointer: bool,
    pub touch: bool,
    pub keyboard_shortcuts: bool,
    pub overlays: OverlayVisibility,
    pub features: FeatureVisibility,
    /// Effective per-command visibility: application-UI × feature × override.
    pub command_visibility: BTreeMap<String, bool>,
    pub touch_target: f32,
    pub dock_width: f32,
}

impl UiPresentationModel {
    /// Whether any dock panel is visible (a convenience over the two flags).
    pub fn panels(&self) -> bool {
        self.layer_panel || self.properties_panel
    }

    /// Insets are top/right/bottom/left. Short landscape surfaces use compact chrome.
    pub fn resolve(config: &ViewerConfig, size: [f64; 2], insets: [f64; 4], touch: bool) -> Self {
        let width = (size[0] - insets[1] - insets[3]).max(0.0);
        let height = (size[1] - insets[0] - insets[2]).max(0.0);
        let layout = match config.ui.layout.mode {
            LayoutMode::Auto if width < config.ui.layout.breakpoints.mobile_below => {
                LayoutMode::Mobile
            }
            LayoutMode::Auto
                if width < config.ui.layout.breakpoints.compact_below || height < 540.0 =>
            {
                LayoutMode::Compact
            }
            LayoutMode::Auto => LayoutMode::Desktop,
            mode => mode,
        };
        let preset = preset_components(config.ui.preset);
        // The application frame is on for every preset except `CanvasOnly`,
        // which `preset_components` marks by turning off all chrome.
        let visible = config.ui.preset != Preset::CanvasOnly;
        let full = config.ui.preset == Preset::Full;
        let c = &config.ui.components;
        let layer_panel = preset.layer_panel && c.layer_panel.visible;
        let properties_panel = preset.properties_panel && c.properties_panel.visible;
        let features = FeatureVisibility {
            measure: config.features.measure,
            annotation_create: config.features.annotations.create,
            annotation_update: config.features.annotations.update,
            annotation_delete: config.features.annotations.delete,
            annotation_import: config.features.annotations.import,
            annotation_export: config.features.annotations.export,
        };
        let mut command_visibility = BTreeMap::new();
        for id in COMMAND_IDS {
            let mut entry_visible = preset.commands;
            if let Some(required) = required_feature(id) {
                entry_visible &= required(&features);
            }
            if let Some(over) = config.ui.command_overrides.get(*id) {
                entry_visible &= over.visible;
            }
            command_visibility.insert((*id).to_string(), entry_visible);
        }
        Self {
            layout,
            application_ui: visible,
            ribbon: preset.ribbon && c.ribbon.visible,
            layer_panel,
            properties_panel,
            // `initiallyOpen` is the desktop dock default. The compact/mobile
            // shell never auto-opens its drawer from a resize: there it is the
            // user's explicit toggle that opens the panel, so a breakpoint
            // change must not surprise the canvas with an overlay drawer.
            layer_panel_initially_open: layer_panel
                && c.layer_panel.initially_open
                && layout == LayoutMode::Desktop,
            properties_panel_initially_open: properties_panel
                && c.properties_panel.initially_open
                && layout == LayoutMode::Desktop,
            navigation: preset.navigation && c.navigation_toolbar.visible,
            command_bar: preset.command_bar && c.command_bar.visible,
            layout_tabs: preset.layout_tabs && c.layout_tabs.visible,
            status_bar: preset.status_bar && c.status_bar.visible,
            pointer: config.interaction.pointer,
            touch: config.interaction.touch,
            keyboard_shortcuts: config.interaction.keyboard_shortcuts,
            overlays: OverlayVisibility {
                axes: config.view.overlays.axes,
                grid: config.view.overlays.grid,
                selection_highlight: config.view.overlays.selection_highlight,
                snap_hints: config.view.overlays.snap_hints,
                annotations: config.view.overlays.annotations,
            },
            features,
            command_visibility,
            touch_target: if touch || layout == LayoutMode::Mobile {
                48.0
            } else {
                36.0
            },
            // Only the full desktop arrangement reserves a dock gutter. Minimal
            // keeps its panels togglable but never auto-reserves width, so it
            // reports `0.0` here.
            dock_width: if visible
                && full
                && layout == LayoutMode::Desktop
                && (layer_panel || properties_panel)
            {
                240.0
            } else {
                0.0
            },
        }
    }
}

/// Feature gate for a command id, if any.
fn required_feature(id: &str) -> Option<fn(&FeatureVisibility) -> bool> {
    if id.starts_with("measure.") {
        return Some(|f: &FeatureVisibility| f.measure);
    }
    match id {
        "file.importAnnotations" => Some(|f: &FeatureVisibility| f.annotation_import),
        "file.exportAnnotations" => Some(|f: &FeatureVisibility| f.annotation_export),
        "annotation.delete" => Some(|f: &FeatureVisibility| f.annotation_delete),
        id if id.starts_with("annotation.") => Some(|f: &FeatureVisibility| f.annotation_create),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The complete protocol from `docs/ui-spec/ui-desc.md` (labels are catalog
    /// keys in this codebase; the ui-desc example's literal strings are host data).
    const FULL_JSON: &str = r#"{
      "schemaVersion": 1,
      "ui": {
        "preset": "full",
        "layout": { "mode": "auto", "breakpoints": { "compactBelow": 1200, "mobileBelow": 720 } },
        "components": {
          "ribbon": {
            "visible": true,
            "tabs": [
              { "id": "review", "label": "ribbon.review", "groups": [
                { "id": "measure", "label": "ribbon.measure", "commands": ["measure.distance", "measure.angle", "measure.area"] },
                { "id": "annotate", "label": "ribbon.annotate", "commands": ["annotation.text", "annotation.leader", "annotation.cloud"] }
              ] }
            ]
          },
          "layerPanel": { "visible": true, "placement": "auto", "initiallyOpen": false },
          "propertiesPanel": { "visible": true, "placement": "auto", "initiallyOpen": false },
          "navigationToolbar": { "visible": true, "placement": "right", "commands": ["view.fit", "view.pan", "view.orbit"] },
          "commandBar": { "visible": false },
          "layoutTabs": { "visible": true },
          "statusBar": { "visible": true }
        },
        "commandOverrides": { "annotation.cloud": { "visible": false } },
        "userCustomization": { "allowedPaths": [
          "ui.components.layerPanel.initiallyOpen",
          "ui.components.propertiesPanel.initiallyOpen"
        ] }
      },
      "features": {
        "measure": true,
        "annotations": { "create": true, "update": true, "delete": false, "import": true, "export": true }
      },
      "view": { "overlays": { "axes": true, "grid": false, "selectionHighlight": true, "snapHints": true, "annotations": true } },
      "interaction": { "pointer": true, "touch": true, "keyboardShortcuts": true }
    }"#;

    fn store_with_user_paths() -> ViewerConfigStore {
        let mut store = ViewerConfigStore::default();
        store
            .update_config(json!({
                "ui": { "userCustomization": { "allowedPaths": [
                    "ui.components.layerPanel.initiallyOpen",
                    "ui.components.propertiesPanel.initiallyOpen",
                    "features.measure",
                    "ui.components.ribbon.visible",
                    "view.overlays.grid"
                ] } }
            }))
            .unwrap();
        store
    }

    #[test]
    fn defaults_roundtrip_and_breakpoints_use_available_surface() {
        let c = ViewerConfig::default();
        assert_eq!(
            serde_json::from_str::<ViewerConfig>(&serde_json::to_string(&c).unwrap()).unwrap(),
            c
        );
        for (size, expected) in [
            ([1280.0, 800.0], LayoutMode::Desktop),
            ([1000.0, 800.0], LayoutMode::Compact),
            ([390.0, 844.0], LayoutMode::Mobile),
            ([1280.0, 360.0], LayoutMode::Compact),
        ] {
            assert_eq!(
                UiPresentationModel::resolve(&c, size, [0.0; 4], false).layout,
                expected
            );
        }
        assert_eq!(
            UiPresentationModel::resolve(&c, [740.0, 800.0], [0.0, 20.0, 0.0, 20.0], true).layout,
            LayoutMode::Mobile
        );
    }

    #[test]
    fn full_protocol_json_roundtrips_and_validates() {
        let config: ViewerConfig = serde_json::from_str(FULL_JSON).unwrap();
        config.validate().unwrap();
        let roundtrip: ViewerConfig =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(roundtrip, config);
        assert_eq!(config.ui.components.ribbon.tabs.len(), 1);
        assert_eq!(config.ui.components.ribbon.tabs[0].groups[1].id, "annotate");
        assert!(!config.features.annotations.delete);
        assert_eq!(
            config.ui.user_customization.allowed_paths[0],
            "ui.components.layerPanel.initiallyOpen"
        );
    }

    #[test]
    fn unknown_fields_are_rejected_with_a_precise_path() {
        let error =
            serde_json::from_str::<ViewerConfig>(r#"{"ui":{"script":"alert(1)"}}"#).unwrap_err();
        assert!(error.to_string().contains("script"));
        let mut store = ViewerConfigStore::default();
        assert_eq!(
            store
                .update_config(json!({"ui":{"script":"alert(1)"}}))
                .unwrap_err()
                .path,
            "ui.script"
        );
    }

    #[test]
    fn unknown_command_override_key_is_rejected_by_validation_and_merge() {
        let error = serde_json::from_str::<ViewerConfig>(
            r#"{"ui":{"commandOverrides":{"mystery.command":{"visible":false}}}}"#,
        );
        // `commandOverrides` itself parses; validation is what rejects the key.
        let config = error.unwrap();
        assert_eq!(
            config.validate().unwrap_err().path,
            "ui.commandOverrides.mystery.command"
        );
        // Merge can add a new override entry, then validation rejects it.
        let mut store = ViewerConfigStore::default();
        assert_eq!(
            store
                .update_config(
                    json!({"ui":{"commandOverrides":{"mystery.command":{"visible":false}}}})
                )
                .unwrap_err()
                .path,
            "ui.commandOverrides.mystery.command"
        );
        // A whitelisted id round-trips through merge.
        store
            .update_config(json!({"ui":{"commandOverrides":{"view.fit":{"visible":false}}}}))
            .unwrap();
        assert!(!store.effective().ui.command_overrides["view.fit"].visible);
    }

    #[test]
    fn unknown_ribbon_command_and_navigation_command_are_rejected() {
        let config: ViewerConfig = serde_json::from_value(json!({
            "ui": { "components": { "ribbon": { "tabs": [
                { "id": "t", "label": "ribbon.review", "groups": [
                    { "id": "g", "label": "ribbon.measure", "commands": ["measure.distance", "nope"] }
                ] }
            ] } } }
        }))
        .unwrap();
        assert_eq!(
            config.validate().unwrap_err().path,
            "ui.components.ribbon.tabs[0].groups[0].commands[1]"
        );
        let config: ViewerConfig = serde_json::from_value(json!({
            "ui": { "components": { "navigationToolbar": { "commands": ["view.fit", "bogus"] } } }
        }))
        .unwrap();
        assert_eq!(
            config.validate().unwrap_err().path,
            "ui.components.navigationToolbar.commands[1]"
        );
    }

    #[test]
    fn duplicate_tab_and_group_ids_are_rejected() {
        let config: ViewerConfig = serde_json::from_value(json!({
            "ui": { "components": { "ribbon": { "tabs": [
                { "id": "a", "label": "ribbon.review", "groups": [] },
                { "id": "a", "label": "ribbon.measure", "groups": [] }
            ] } } }
        }))
        .unwrap();
        assert_eq!(
            config.validate().unwrap_err().path,
            "ui.components.ribbon.tabs[1].id"
        );
    }

    #[test]
    fn unknown_allowed_path_is_rejected() {
        let config: ViewerConfig = serde_json::from_value(json!({
            "ui": { "userCustomization": { "allowedPaths": ["ui.components.ribbon.bogus"] } }
        }))
        .unwrap();
        assert_eq!(
            config.validate().unwrap_err().path,
            "ui.userCustomization.allowedPaths[0]"
        );
        // A command-override path is accepted for a whitelisted command id.
        let config: ViewerConfig = serde_json::from_value(json!({
            "ui": {
                "commandOverrides": { "view.fit": { "visible": true } },
                "userCustomization": { "allowedPaths": ["ui.commandOverrides.view.fit.visible"] }
            }
        }))
        .unwrap();
        config.validate().unwrap();
    }

    /// A user preference may hide a command but must never re-enable one that a
    /// preset or feature capability disabled (spec: user preference cannot turn
    /// a forbidden capability back on). `ui.commandOverrides.<id>.visible` is a
    /// clamped path exactly like `features.*` and `ui.components.*.visible`.
    #[test]
    fn user_preference_cannot_re_enable_a_disabled_command_override() {
        let mut store = ViewerConfigStore::default();
        store
            .update_config(json!({
                "ui": {
                    "preset": "canvasOnly",
                    "userCustomization": { "allowedPaths": [
                        "ui.commandOverrides.view.fit.visible"
                    ] }
                }
            }))
            .unwrap();
        // A preference asking to show the command is clamped to the host's
        // disabled value, never switched on.
        store
            .apply_user_preference(json!({
                "ui": { "commandOverrides": { "view.fit": { "visible": true } } }
            }))
            .unwrap();
        assert!(
            !store.effective().ui.command_overrides["view.fit"].visible,
            "a user preference must not re-enable a preset-disabled command; got {:?}",
            store.effective().ui.command_overrides
        );
        assert!(
            !UiPresentationModel::resolve(store.effective(), [1280.0, 800.0], [0.0; 4], false)
                .command_visibility["view.fit"],
            "the resolved command visibility must stay disabled"
        );
        // Hiding is still honoured when the capability is otherwise on.
        let mut store = ViewerConfigStore::default();
        store
            .update_config(json!({
                "ui": { "userCustomization": { "allowedPaths": [
                    "ui.commandOverrides.view.fit.visible"
                ] } }
            }))
            .unwrap();
        store
            .apply_user_preference(json!({
                "ui": { "commandOverrides": { "view.fit": { "visible": false } } }
            }))
            .unwrap();
        assert!(!store.effective().ui.command_overrides["view.fit"].visible);
    }

    #[test]
    fn false_survives_merge_and_failure_is_atomic_with_field_path() {
        let mut store = ViewerConfigStore::default();
        store
            .update_config(json!({"ui":{"components":{"ribbon":{"visible":false}}}}))
            .unwrap();
        assert!(!store.effective().ui.components.ribbon.visible);
        let old = store.effective().clone();
        let rev = store.revision;
        let error = store
            .update_config(json!({"ui":{"layout":{"breakpoints":{"mobileBelow":2000}}}}))
            .unwrap_err();
        assert_eq!(error.path, "ui.layout.breakpoints.compactBelow");
        assert_eq!(store.effective(), &old);
        assert_eq!(store.revision, rev);
        assert_eq!(
            store
                .update_config(json!({"ui":{"script":"alert(1)"}}))
                .unwrap_err()
                .path,
            "ui.script"
        );
        // Null is never a configuration value.
        assert_eq!(
            store
                .update_config(json!({"ui":{"preset": null}}))
                .unwrap_err()
                .path,
            "ui.preset"
        );
    }

    #[test]
    fn arrays_are_replaced_wholesale_by_merge() {
        let mut store = ViewerConfigStore::default();
        store
            .update_config(
                json!({"ui":{"userCustomization":{"allowedPaths":["view.overlays.grid"]}}}),
            )
            .unwrap();
        assert_eq!(
            store.effective().ui.user_customization.allowed_paths,
            vec!["view.overlays.grid".to_string()]
        );
        store
            .update_config(json!({"ui":{"userCustomization":{"allowedPaths":["view.overlays.axes","view.overlays.snapHints"]}}}))
            .unwrap();
        assert_eq!(
            store.effective().ui.user_customization.allowed_paths.len(),
            2
        );
    }

    #[test]
    fn parse_order_host_then_allowed_user_preference_with_clamping() {
        let mut store = ViewerConfigStore::default();
        store
            .update_config(json!({
                "ui": { "userCustomization": { "allowedPaths": [
                    "features.measure",
                    "ui.components.ribbon.visible",
                    "ui.components.layerPanel.initiallyOpen",
                    "view.overlays.grid"
                ] },
                "components": { "ribbon": { "visible": false } } },
                "features": { "measure": false }
            }))
            .unwrap();
        // The host disabled measure and the ribbon; a user preference may not
        // switch them back on (clamp), but may disable or customize freely.
        store
            .apply_user_preference(json!({
                "features": { "measure": true },
                "ui": { "components": { "ribbon": { "visible": true },
                    "layerPanel": { "initiallyOpen": false } } },
                "view": { "overlays": { "grid": false } }
            }))
            .unwrap();
        let effective = store.effective();
        assert!(
            !effective.features.measure,
            "capability must stay clamped off"
        );
        assert!(
            !effective.ui.components.ribbon.visible,
            "component must stay clamped off"
        );
        assert!(!effective.ui.components.layer_panel.initially_open);
        assert!(!effective.view.overlays.grid);
    }

    #[test]
    fn user_preference_outside_allowed_paths_is_rejected_atomically() {
        let mut store = store_with_user_paths();
        let before = store.effective().clone();
        let rev = store.revision;
        let error = store
            .apply_user_preference(json!({"ui": {"preset": "canvasOnly"}}))
            .unwrap_err();
        assert_eq!(error.path, "ui.preset");
        assert_eq!(store.effective(), &before);
        assert_eq!(store.revision, rev);
        assert_eq!(store.user_preference(), &json!({}));
    }

    #[test]
    fn null_or_non_object_user_preference_is_rejected() {
        let mut store = store_with_user_paths();
        assert_eq!(
            store
                .apply_user_preference(json!({"view": {"overlays": {"grid": null}}}))
                .unwrap_err()
                .path,
            "view.overlays.grid"
        );
        assert_eq!(
            store.apply_user_preference(json!([1, 2])).unwrap_err().path,
            "preference"
        );
    }

    #[test]
    fn clear_user_preference_restores_host_values() {
        let mut store = store_with_user_paths();
        store
            .apply_user_preference(
                json!({"ui": {"components": {"layerPanel": {"initiallyOpen": false}}}}),
            )
            .unwrap();
        assert!(!store.effective().ui.components.layer_panel.initially_open);
        store.clear_user_preference();
        assert!(store.effective().ui.components.layer_panel.initially_open);
        assert_eq!(store.user_preference(), &json!({}));
    }

    #[test]
    fn set_config_reapplies_preference_and_drops_revoked_paths() {
        let mut store = store_with_user_paths();
        store
            .apply_user_preference(
                json!({"ui": {"components": {"layerPanel": {"initiallyOpen": false}}}}),
            )
            .unwrap();
        assert!(!store.effective().ui.components.layer_panel.initially_open);
        // New host config no longer allows that path; the stale preference is
        // ignored rather than erroring.
        let mut host = ViewerConfig::default();
        host.ui.user_customization.allowed_paths.clear();
        store.set_config(host).unwrap();
        assert!(store.effective().ui.components.layer_panel.initially_open);
    }

    #[test]
    fn per_command_visibility_combines_features_and_overrides() {
        let mut host = ViewerConfig::default();
        host.features.measure = false;
        host.features.annotations.delete = false;
        host.ui.command_overrides.insert(
            "annotation.cloud".into(),
            CommandOverride { visible: false },
        );
        host.validate().unwrap();
        let p = UiPresentationModel::resolve(&host, [1280.0, 800.0], [0.0; 4], false);
        assert!(!p.command_visibility["measure.distance"]);
        assert!(!p.command_visibility["measure.confirm"]);
        assert!(!p.command_visibility["annotation.delete"]);
        assert!(!p.command_visibility["annotation.cloud"]);
        assert!(p.command_visibility["annotation.text"]);
        assert!(p.command_visibility["view.fit"]);
        assert!(p.command_visibility["file.exportAnnotations"]);
    }

    /// The chrome component flags that every preset either permits or drops, in
    /// the order the truth table in [`PresetComponents`] documents them.
    fn component_flags(p: &UiPresentationModel) -> [bool; 8] {
        [
            p.ribbon,
            p.command_bar,
            p.status_bar,
            p.navigation,
            p.layout_tabs,
            p.layer_panel,
            p.properties_panel,
            p.command_visibility.values().all(|visible| *visible),
        ]
    }

    fn resolve_preset(preset: Preset) -> UiPresentationModel {
        let mut host = ViewerConfig::default();
        host.ui.preset = preset;
        host.validate().unwrap();
        // Full desktop surface so layout mode does not mask preset differences.
        UiPresentationModel::resolve(&host, [1280.0, 800.0], [0.0; 4], false)
    }

    #[test]
    fn minimal_preset_has_explicit_component_semantics() {
        let p = resolve_preset(Preset::Minimal);
        // Minimal keeps the application frame: it is not canvas-only.
        assert!(p.application_ui);
        // Entry points dropped by minimal.
        assert!(!p.ribbon, "minimal hides the ribbon");
        assert!(!p.command_bar, "minimal hides the command bar");
        assert!(!p.status_bar, "minimal hides the status bar");
        // Canvas navigation, layout tabs and both dock panels are kept.
        assert!(p.navigation, "minimal keeps navigation");
        assert!(p.layout_tabs, "minimal keeps the layout tabs");
        assert!(p.layer_panel, "minimal keeps the layer panel");
        assert!(p.properties_panel, "minimal keeps the properties panel");
        // Command reachability is untouched: hiding an entry point never
        // disables the command, so every catalog command stays visible.
        assert!(
            p.command_visibility.values().all(|visible| *visible),
            "minimal keeps command reachability"
        );
        // Minimal never auto-reserves a dock gutter; only full desktop docks.
        assert_eq!(
            p.dock_width, 0.0,
            "minimal never reserves a dock gutter; only full desktop docks"
        );
    }

    #[test]
    fn presets_are_monotonic_canvas_only_le_minimal_le_full() {
        let full = component_flags(&resolve_preset(Preset::Full));
        let minimal = component_flags(&resolve_preset(Preset::Minimal));
        let canvas_only = component_flags(&resolve_preset(Preset::CanvasOnly));
        for index in 0..full.len() {
            let (f, m, co) = (full[index], minimal[index], canvas_only[index]);
            assert!(
                !co || m,
                "canvasOnly shows component flag {index} that minimal hides"
            );
            assert!(
                !m || f,
                "minimal shows component flag {index} that full hides"
            );
        }
        // The presets are distinct, not aliases: each is strictly stronger than
        // the next on at least one field.
        assert!(full.iter().any(|&v| v) && minimal.iter().any(|&v| !v));
        assert!(minimal.iter().any(|&v| v) && canvas_only.iter().all(|&v| !v));
    }

    #[test]
    fn preset_canvas_only_hides_every_command_and_overlay_state_is_reported() {
        let mut host = ViewerConfig::default();
        host.ui.preset = Preset::CanvasOnly;
        host.features.measure = false;
        host.view.overlays.grid = false;
        host.interaction.touch = false;
        let p = UiPresentationModel::resolve(&host, [1280.0, 800.0], [0.0; 4], false);
        assert!(!p.application_ui);
        assert!(!p.layer_panel && !p.properties_panel && !p.ribbon && !p.navigation);
        assert!(!p.overlays.grid && p.overlays.axes);
        assert!(!p.features.measure);
        assert!(!p.touch && p.pointer);
        assert!(p.command_visibility.values().all(|visible| !visible));
        // Overlays are independent of the application UI (view controls the canvas).
        assert!(p.overlays.annotations);
        // CanvasOnly is strictly stronger than Minimal: it hides every chrome
        // component minimal keeps.
        let minimal = resolve_preset(Preset::Minimal);
        assert!(!p.ribbon && !p.command_bar && !p.status_bar && !p.navigation);
        assert!(!p.layout_tabs && !p.layer_panel && !p.properties_panel);
        assert!(minimal.navigation && minimal.layout_tabs && minimal.layer_panel);
        assert_ne!(p.application_ui, minimal.application_ui);
    }

    #[test]
    fn separate_panel_visibility_and_initially_open_are_resolved() {
        let config: ViewerConfig = serde_json::from_value(json!({
            "ui": { "components": {
                "layerPanel": { "visible": false, "initiallyOpen": true },
                "propertiesPanel": { "visible": true, "initiallyOpen": false }
            } }
        }))
        .unwrap();
        let p = UiPresentationModel::resolve(&config, [1280.0, 800.0], [0.0; 4], false);
        assert!(!p.layer_panel && p.properties_panel);
        assert!(!p.layer_panel_initially_open && !p.properties_panel_initially_open);
        assert_eq!(p.dock_width, 240.0);
        assert!(p.panels());
    }

    #[test]
    fn revision_and_observer_fire_once_per_real_change() {
        let mut store = ViewerConfigStore::default();
        let seen: Rc<std::cell::RefCell<Vec<(u64, bool)>>> =
            Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = seen.clone();
        store.subscribe(Rc::new(move |config, revision| {
            sink.borrow_mut()
                .push((revision, config.ui.components.ribbon.visible));
        }));
        assert_eq!(store.revision, 0);
        store
            .update_config(json!({"ui":{"components":{"ribbon":{"visible":false}}}}))
            .unwrap();
        assert_eq!(store.revision, 1);
        // A no-op update does not bump the revision or notify.
        store
            .update_config(json!({"ui":{"components":{"ribbon":{"visible":false}}}}))
            .unwrap();
        assert_eq!(store.revision, 1);
        assert_eq!(&*seen.borrow(), &[(1, false)]);
        // A rejected update does not notify either.
        let _ = store.update_config(json!({"ui":{"script":"x"}}));
        assert_eq!(store.revision, 1);
    }

    #[test]
    fn effective_config_json_query_round_trips() {
        let mut store = ViewerConfigStore::default();
        store
            .update_config(json!({"ui":{"preset":"canvasOnly"}}))
            .unwrap();
        let json = serde_json::to_string(store.effective()).unwrap();
        let parsed: ViewerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(&parsed, store.effective());
        assert_eq!(parsed.ui.preset, Preset::CanvasOnly);
    }

    fn ribbon_config_with_hidden_cloud() -> (ViewerConfig, BTreeMap<String, bool>) {
        let config: ViewerConfig = serde_json::from_value(json!({
            "ui": { "components": { "ribbon": { "tabs": [
                { "id": "review", "label": "ribbon.review", "groups": [
                    { "id": "measure", "label": "ribbon.measure",
                      "commands": ["measure.distance", "measure.area"] },
                    { "id": "annotate", "label": "ribbon.annotate",
                      "commands": ["annotation.text", "annotation.cloud"] }
                ] },
                { "id": "manage", "label": "ribbon.manage", "groups": [] }
            ] } } }
        }))
        .unwrap();
        let visibility = UiPresentationModel::resolve(&config, [1280.0, 800.0], [0.0; 4], false)
            .command_visibility;
        (config, visibility)
    }

    #[test]
    fn resolve_ribbon_preserves_order_and_label_keys() {
        let (config, visibility) = ribbon_config_with_hidden_cloud();
        let resolved = resolve_ribbon(&config, &visibility);
        assert_eq!(resolved.tabs.len(), 2);
        // Order is the config order; labels stay catalog keys, not literals.
        assert_eq!(resolved.tabs[0].id, "review");
        assert_eq!(resolved.tabs[0].label, "ribbon.review");
        assert_eq!(resolved.tabs[1].id, "manage");
        assert_eq!(
            resolved.tabs[0]
                .groups
                .iter()
                .map(|group| group.id.as_str())
                .collect::<Vec<_>>(),
            vec!["measure", "annotate"]
        );
        assert_eq!(resolved.tabs[0].groups[0].label, "ribbon.measure");
        assert_eq!(
            resolved.tabs[0].groups[0]
                .commands
                .iter()
                .map(|command| command.id.as_str())
                .collect::<Vec<_>>(),
            vec!["measure.distance", "measure.area"]
        );
        assert!(resolved.tabs[0].groups[0]
            .commands
            .iter()
            .all(|command| command.visible));
        // A tab with no groups is kept, not dropped.
        assert!(resolved.tabs[1].groups.is_empty());
    }

    #[test]
    fn resolve_ribbon_filters_hidden_commands() {
        let (mut config, _) = ribbon_config_with_hidden_cloud();
        config.ui.command_overrides.insert(
            "annotation.cloud".into(),
            CommandOverride { visible: false },
        );
        let visibility = UiPresentationModel::resolve(&config, [1280.0, 800.0], [0.0; 4], false)
            .command_visibility;
        let resolved = resolve_ribbon(&config, &visibility);
        let annotate = &resolved.tabs[0].groups[1];
        assert_eq!(
            annotate
                .commands
                .iter()
                .map(|command| command.id.as_str())
                .collect::<Vec<_>>(),
            vec!["annotation.text"]
        );
    }

    #[test]
    fn resolve_ribbon_empty_config_and_unknown_visibility_default_to_hidden() {
        let config = ViewerConfig::default();
        let visibility = BTreeMap::new();
        let resolved = resolve_ribbon(&config, &visibility);
        assert!(resolved.tabs.is_empty());
        assert_eq!(resolved, ResolvedRibbon::default());

        // A command absent from the map is treated as hidden, never visible.
        let (config, _) = ribbon_config_with_hidden_cloud();
        let resolved = resolve_ribbon(&config, &BTreeMap::new());
        assert!(resolved
            .tabs
            .iter()
            .flat_map(|tab| &tab.groups)
            .all(|group| group.commands.is_empty()));
    }

    #[test]
    fn ribbon_command_display_round_trips_and_reaches_the_resolved_model() {
        // `/display` is camelCase in JSON; the default must be `iconAndLabel`.
        for (json_display, expected) in [
            (serde_json::Value::Null, RibbonCommandDisplay::IconAndLabel),
            (json!("iconAndLabel"), RibbonCommandDisplay::IconAndLabel),
            (json!("iconOnly"), RibbonCommandDisplay::IconOnly),
            (json!("labelOnly"), RibbonCommandDisplay::LabelOnly),
        ] {
            let mut group = json!({
                "id": "measure", "label": "ribbon.measure",
                "commands": ["measure.distance", "measure.area"]
            });
            if !json_display.is_null() {
                group["display"] = json_display.clone();
            }
            let config: ViewerConfig = serde_json::from_value(json!({
                "ui": { "components": { "ribbon": { "tabs": [
                    { "id": "review", "label": "ribbon.review", "groups": [group] }
                ] } } }
            }))
            .unwrap();
            config.validate().unwrap();
            // The value survives a serialize/deserialize round trip.
            let roundtrip: ViewerConfig =
                serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
            assert_eq!(roundtrip, config);
            let resolved = resolve_ribbon(
                &config,
                &BTreeMap::from([
                    ("measure.distance".to_string(), true),
                    ("measure.area".to_string(), true),
                ]),
            );
            let commands = &resolved.tabs[0].groups[0].commands;
            assert_eq!(commands.len(), 2);
            assert!(
                commands.iter().all(|command| command.display == expected),
                "display {json_display} should resolve to {expected:?}"
            );
        }
        // An unknown display value is rejected by serde, never defaulted.
        assert!(serde_json::from_value::<ViewerConfig>(json!({
            "ui": { "components": { "ribbon": { "tabs": [
                { "id": "review", "label": "ribbon.review", "groups": [
                    { "id": "measure", "label": "ribbon.measure",
                      "commands": ["measure.distance"], "display": "big" }
                ] }
            ] } } }
        }))
        .is_err());
    }
}
