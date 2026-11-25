//! Startup config/preference loading and the one-time budget migration.
//!
//! Split out of `lib.rs` so both files stay inside the file-size gate. The
//! method body is unchanged: it owns the persisted-document read, the root-leaf
//! capture, the released-budget migration diagnostic (ADR-0026/0057) and the
//! startup seeds.

use super::*;

impl HostCore {
    /// Read and apply the durable config and user preference at startup.
    ///
    /// An absent file is normal; a malformed or unreadable one keeps the
    /// previous state and adds a human-readable diagnostic. Returns every
    /// diagnostic.
    pub fn load_startup(&self) -> Vec<String> {
        let mut diagnostics = Vec::new();
        if let Err(error) = self.update_config_json(HOST_ALLOWED_PATHS_JSON) {
            diagnostics.push(format!("host config defaults rejected: {error}"));
        }

        match self.config_store.load_config() {
            ConfigLoad::Absent => {}
            ConfigLoad::Text { path, text } => {
                if let Err(error) = self.set_config_json(&text) {
                    diagnostics.push(format!("config file {path} rejected: {error}"));
                }
            }
            ConfigLoad::Unreadable { path, reason } => {
                diagnostics.push(format!("config file {path} unreadable: {reason}"));
            }
        }

        match self.config_store.load_preference() {
            ConfigLoad::Absent => {}
            ConfigLoad::Text { path, text } => {
                // The renderer backend choice is a root preference leaf outside
                // the `ViewerConfig` schema, so the allowed-path projection above
                // cannot carry it: capture it here (unknown/absent -> `auto`) so
                // the settings snapshot reports it and every later write keeps it.
                *self.wgpu_backend.borrow_mut() = settings::wgpu_backend_from_preference(&text);
                // The five appearance/navigation leaves are root preferences
                // outside the `ViewerConfig` schema for the same reason: capture
                // them here (absent/malformed -> documented default) so the
                // settings snapshot reports them and every later write keeps them.
                *self.root_preferences.borrow_mut() =
                    settings_root::root_preferences_from_preference(&text);
                // Recent drawings are user data in the same document: the array
                // is captured before the strict allowed-path projection strips
                // it, so the list survives every later preference write.
                *self.recent_files.borrow_mut() = recent::recent_files_from_preference(&text);
                // The runtime alias set is the same kind of root user data: the
                // session validates each row against its command table when it
                // seeds its override layer.
                *self.command_aliases.borrow_mut() =
                    aliases::command_aliases_from_preference(&text)
                        .into_iter()
                        .collect();
                // The document is already durable: a store that cannot
                // *re-persist* its normalized projection must not make the
                // loaded values disappear (`load_user_preference` applies them
                // in memory; only runtime writes demand durability).
                //
                // A budget written by a release with the wider `10..=100` range
                // is migrated down to the current maximum **before** it is
                // validated, with a diagnostic naming the old and new value.
                // This is the read path only: a live write is still refused,
                // never clamped.
                let preference = recent::without_recent_files(&aliases::without_command_aliases(
                    &settings_root::without_root_preferences(&settings::without_wgpu_backend(
                        &text,
                    )),
                ));
                let (preference, migrated_budget) =
                    settings::migrate_persisted_gpu_budget_percent(&preference);
                if let Some(previous) = migrated_budget {
                    diagnostics.push(format!(
                        "preferences {path} migrated view.gpuBudget.percent {previous} -> {} (the accepted maximum)",
                        *cad_app::viewer_config::ViewGpuBudget::RANGE.end(),
                    ));
                }
                match self.load_user_preference(&preference) {
                    Ok(()) => {
                        if let Err(error) = self.persist_preference() {
                            diagnostics
                                .push(format!("preferences {path} not re-persisted: {error}"));
                        }
                    }
                    Err(error) => diagnostics.push(format!("preferences {path} rejected: {error}")),
                }
            }
            ConfigLoad::Unreadable { path, reason } => {
                diagnostics.push(format!("preferences {path} unreadable: {reason}"));
            }
        }
        // A persisted font directory that disappeared is diagnosed, not fatal.
        self.diagnose_fonts_dir(&mut diagnostics);
        // The loaded preference must seed the renderer policy the same way it
        // seeds the live interaction switches: the draw tier is chosen at the
        // next device attach from the engine handle's recorded choice.
        self.seed_gpu_storage();
        // The loaded preference must actually seed the live session, not just
        // sit in the config store: `CadStatus.snap_enabled`/`hover_preview` read
        // `SessionState`, whose constructor defaults both switches on.
        self.seed_session_interaction();
        // The newest durable recent entry also seeds the next open chooser's
        // starting directory. An empty list leaves the platform hint unset.
        self.seed_recent_target();
        // The pickbox/aperture are config-derived rather than session switches,
        // so the same startup pass installs the persisted policy on the live
        // session (the next interaction would derive it anyway; doing it here
        // keeps the first frame and the first pick consistent).
        self.seed_interaction_tolerance();

        *self.startup_diagnostics.borrow_mut() = diagnostics.clone();
        diagnostics
    }

    /// Write the store's allowed projection of the user preference.
    ///
    /// The renderer backend choice, the five appearance/navigation leaves and the
    /// recent-drawings array sit outside the `ViewerConfig` schema, so they are
    /// merged back into the projected document here — every preference write (a
    /// setting, a startup re-persist, a clear) therefore keeps exactly those root
    /// leaves and never drops them.
    pub(crate) fn persist_preference(&self) -> CadResult<()> {
        let projected = self.viewer_config.borrow().projected_user_preference_json();
        let merged = settings::merge_wgpu_backend_into_preference(
            &projected,
            self.wgpu_backend.borrow().as_deref(),
        )?;
        let merged = settings_root::merge_root_preferences_into_preference(
            &merged,
            &self.root_preferences.borrow(),
        )?;
        let merged =
            recent::merge_recent_files_into_preference(&merged, &self.recent_files.borrow())?;
        let merged = aliases::merge_command_aliases_into_preference(
            &merged,
            &self.command_aliases.borrow(),
        )?;
        self.config_store.persist_preference(&merged)
    }

    pub fn config_snapshot(&self) -> ConfigSnapshot {
        let store = self.viewer_config.borrow();
        ConfigSnapshot {
            effective: store.effective().clone(),
            revision: store.revision,
            preset: store.effective().ui.preset,
            effective_json: store.effective_json(),
        }
    }
}
