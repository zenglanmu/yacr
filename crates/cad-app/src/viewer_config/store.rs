use super::*;

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
        // A crosshair percentage outside 1..=100 is a refusal, never a clamp: the
        // overlay's own floor is a defence for direct callers, not a policy.
        let crosshair = self.view.crosshair.size_percent;
        if !(1..=100).contains(&crosshair) {
            return Err(ConfigError::new(
                "view.crosshair.sizePercent",
                "crosshair size must be an integer percentage from 1 to 100",
            ));
        }
        // Pickbox and aperture sizes are refusals outside AutoCAD's own option
        // ranges, never clamps; a fractional/negative/string value never reaches
        // here because the schema types them as integers.
        if !ViewPickbox::RANGE.contains(&self.view.pickbox.size_pixels) {
            return Err(ConfigError::new(
                "view.pickbox.sizePixels",
                "pickbox size must be an integer pixel count from 1 to 20",
            ));
        }
        if !ViewAperture::RANGE.contains(&self.view.aperture.size_pixels) {
            return Err(ConfigError::new(
                "view.aperture.sizePixels",
                "aperture size must be an integer pixel count from 1 to 50",
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
