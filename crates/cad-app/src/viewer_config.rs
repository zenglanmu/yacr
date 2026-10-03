//! Host-owned, data-only viewer configuration and pure shell presentation.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Preset {
    #[default]
    Full,
    Minimal,
    CanvasOnly,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Component {
    pub visible: bool,
    pub initially_open: bool,
}
impl Default for Component {
    fn default() -> Self {
        Self {
            visible: true,
            initially_open: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Components {
    pub ribbon: Component,
    pub layer_panel: Component,
    pub properties_panel: Component,
    pub navigation_toolbar: Component,
    pub command_bar: Component,
    pub layout_tabs: Component,
    pub status_bar: Component,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct UiConfig {
    pub preset: Preset,
    pub layout: LayoutConfig,
    pub components: Components,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewerConfig {
    pub schema_version: u32,
    pub ui: UiConfig,
}
impl Default for ViewerConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            ui: UiConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub path: String,
    pub reason: String,
}

impl ViewerConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != 1 {
            return Err(ConfigError {
                path: "schemaVersion".into(),
                reason: "unsupported schema version".into(),
            });
        }
        let b = &self.ui.layout.breakpoints;
        if !b.mobile_below.is_finite() || b.mobile_below <= 0.0 {
            return Err(ConfigError {
                path: "ui.layout.breakpoints.mobileBelow".into(),
                reason: "expected positive finite logical pixels".into(),
            });
        }
        if !b.compact_below.is_finite() || b.compact_below <= b.mobile_below {
            return Err(ConfigError {
                path: "ui.layout.breakpoints.compactBelow".into(),
                reason: "must exceed mobileBelow".into(),
            });
        }
        Ok(())
    }
}

/// Configuration never owns a CAD session. Failed updates leave the old value intact.
#[derive(Debug, Clone, Default)]
pub struct ViewerConfigStore {
    effective: ViewerConfig,
    pub revision: u64,
}
impl ViewerConfigStore {
    pub fn effective(&self) -> &ViewerConfig {
        &self.effective
    }
    pub fn set_config(&mut self, config: ViewerConfig) -> Result<(), ConfigError> {
        config.validate()?;
        if config != self.effective {
            self.effective = config;
            self.revision += 1;
        }
        Ok(())
    }
    /// Structured object merge. Arrays replace, explicit false survives; null is rejected.
    pub fn update_config(&mut self, patch: Value) -> Result<(), ConfigError> {
        let mut candidate = serde_json::to_value(&self.effective).expect("serializable config");
        merge(&mut candidate, patch, "")?;
        let config = serde_json::from_value(candidate).map_err(|e| ConfigError {
            path: "config".into(),
            reason: e.to_string(),
        })?;
        self.set_config(config)
    }
}
fn merge(target: &mut Value, patch: Value, path: &str) -> Result<(), ConfigError> {
    if let Value::Object(patch) = patch {
        let target = target.as_object_mut().ok_or_else(|| ConfigError {
            path: path.into(),
            reason: "expected object".into(),
        })?;
        for (key, value) in patch {
            let child = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            let slot = target.get_mut(&key).ok_or_else(|| ConfigError {
                path: child.clone(),
                reason: "unsupported configuration field".into(),
            })?;
            merge(slot, value, &child)?;
        }
    } else {
        if patch.is_null() {
            return Err(ConfigError {
                path: path.into(),
                reason: "null is not a configuration value".into(),
            });
        }
        let valid = matches!(
            (&*target, &patch),
            (Value::Bool(_), Value::Bool(_))
                | (Value::String(_), Value::String(_))
                | (Value::Number(_), Value::Number(_))
                | (Value::Array(_), Value::Array(_))
        );
        if !valid {
            return Err(ConfigError {
                path: path.into(),
                reason: "configuration value has wrong type".into(),
            });
        }
        *target = patch;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiPresentationModel {
    pub layout: LayoutMode,
    pub application_ui: bool,
    pub ribbon: bool,
    pub panels: bool,
    pub navigation: bool,
    pub command_bar: bool,
    pub layout_tabs: bool,
    pub status_bar: bool,
    pub touch_target: f32,
    pub dock_width: f32,
}
impl UiPresentationModel {
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
        let full = config.ui.preset == Preset::Full;
        let visible = config.ui.preset != Preset::CanvasOnly;
        let c = &config.ui.components;
        Self {
            layout,
            application_ui: visible,
            ribbon: visible && full && c.ribbon.visible,
            panels: visible && (c.layer_panel.visible || c.properties_panel.visible),
            navigation: visible && c.navigation_toolbar.visible,
            command_bar: visible && full && c.command_bar.visible,
            layout_tabs: visible && c.layout_tabs.visible,
            status_bar: visible && full && c.status_bar.visible,
            touch_target: if touch || layout == LayoutMode::Mobile {
                48.0
            } else {
                36.0
            },
            dock_width: if visible && full && layout == LayoutMode::Desktop && c.layer_panel.visible
            {
                240.0
            } else {
                0.0
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
    }
    #[test]
    fn canvas_only_forces_zero_chrome_and_restores_component_preferences() {
        let mut store = ViewerConfigStore::default();
        store
            .update_config(json!({"ui":{"preset":"canvasOnly"}}))
            .unwrap();
        let p = UiPresentationModel::resolve(store.effective(), [1280.0, 800.0], [0.0; 4], false);
        assert!(
            !p.application_ui
                && !p.ribbon
                && !p.panels
                && !p.navigation
                && !p.command_bar
                && !p.layout_tabs
                && !p.status_bar
        );
        assert_eq!(p.dock_width, 0.0);
        store
            .update_config(json!({"ui":{"preset":"full"}}))
            .unwrap();
        assert!(
            UiPresentationModel::resolve(store.effective(), [1280.0, 800.0], [0.0; 4], false)
                .ribbon
        );
    }
}
