//! The durable runtime command-alias set (`ALIASEDIT ADD|REMOVE`).
//!
//! `commandAliases` is a **root-level** `preference.json` object mapping an
//! uppercase alias to its canonical command selector, exactly like the
//! `recentFiles` root array: it is user data outside the `ViewerConfig` schema,
//! is captured before the strict allowed-path projection, and is re-emitted on
//! every preference write so an unrelated setting change cannot drop it.
//!
//! A malformed document or a malformed row makes the **whole** set untrusted
//! and the host answers an empty set; a partly-salvaged alias table would be a
//! fabricated vocabulary. The command layer (`cad-ui-core`) is still the
//! authority that validates each row's target against the build's command
//! table at startup, so a row whose target disappeared is dropped there with a
//! startup diagnostic rather than guessed.

use super::*;

/// The root-level durable preference key holding the alias object.
pub const COMMAND_ALIASES_PREFERENCE_KEY: &str = "commandAliases";

/// Parse the durable alias object from a preference document.
///
/// An absent key, a non-object value, a non-string alias/target, or an empty
/// alias/target yields an empty set (never a partial, invented one).
pub fn command_aliases_from_preference(text: &str) -> Vec<(String, String)> {
    let Ok(document) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(value) = document.get(COMMAND_ALIASES_PREFERENCE_KEY) else {
        return Vec::new();
    };
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    let mut rows = Vec::with_capacity(object.len());
    for (alias, target) in object {
        let (Some(target), false) = (target.as_str(), alias.trim().is_empty()) else {
            return Vec::new();
        };
        let target = target.trim();
        if target.is_empty() {
            return Vec::new();
        }
        rows.push((alias.trim().to_ascii_uppercase(), target.to_string()));
    }
    rows.sort();
    rows.dedup();
    rows
}

/// Remove the alias leaf before the strict allowed-path projection is applied.
pub(crate) fn without_command_aliases(text: &str) -> String {
    let Ok(mut document) = serde_json::from_str::<serde_json::Value>(text) else {
        return text.to_string();
    };
    if let Some(object) = document.as_object_mut() {
        object.remove(COMMAND_ALIASES_PREFERENCE_KEY);
    }
    serde_json::to_string(&document).unwrap_or_else(|_| text.to_string())
}

/// Merge the alias object into the projected preference document.
///
/// An empty set removes the leaf (a real clear drops it). A projection that is
/// not a JSON object is an explicit error, never a silently rewritten file.
pub(crate) fn merge_command_aliases_into_preference(
    projected: &str,
    rows: &std::collections::BTreeMap<String, String>,
) -> CadResult<String> {
    let mut document: serde_json::Value = serde_json::from_str(projected).map_err(|error| {
        CadError::InvalidInput(format!("preference projection is not JSON: {error}"))
    })?;
    let object = document.as_object_mut().ok_or_else(|| {
        CadError::InvalidInput("preference projection is not a JSON object".into())
    })?;
    if rows.is_empty() {
        object.remove(COMMAND_ALIASES_PREFERENCE_KEY);
    } else {
        let value = serde_json::to_value(rows).map_err(|error| {
            CadError::InvalidInput(format!("command aliases are not JSON: {error}"))
        })?;
        object.insert(COMMAND_ALIASES_PREFERENCE_KEY.to_string(), value);
    }
    serde_json::to_string(&document)
        .map_err(|error| CadError::InvalidInput(format!("preference write is not JSON: {error}")))
}

impl HostCore {
    /// Append one startup diagnostic (a dropped persisted alias, for example).
    pub fn note_startup_diagnostic(&self, text: String) {
        self.startup_diagnostics.borrow_mut().push(text);
    }

    /// The durable runtime alias rows, sorted by alias (possibly empty).
    pub fn command_aliases(&self) -> Vec<(String, String)> {
        self.command_aliases
            .borrow()
            .iter()
            .map(|(alias, target)| (alias.clone(), target.clone()))
            .collect()
    }

    /// Replace and persist the runtime alias set.
    ///
    /// A persistence failure rolls the in-memory set back and returns the real
    /// error, exactly like a settings or recent-files write: an alias that the
    /// next start would lose must not look saved.
    pub fn replace_command_aliases(&self, rows: &[(String, String)]) -> CadResult<()> {
        let previous = self.command_aliases.borrow().clone();
        let next: std::collections::BTreeMap<String, String> = rows.iter().cloned().collect();
        *self.command_aliases.borrow_mut() = next;
        if let Err(error) = self.persist_preference() {
            *self.command_aliases.borrow_mut() = previous;
            return Err(crate::settings::preference_persist_refusal(&error));
        }
        Ok(())
    }
}
