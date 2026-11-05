//! Plugin-manager wire model: the carrier-neutral half of install/enable/
//! uninstall/read-entry.
//!
//! # Phase 1 contract
//!
//! * **One active decoder at a time** (radio semantics). `tch` is the bundled
//!   built-in and is active when no policy file exists.
//! * **Persist now, apply next launch.** Every mutation writes the policy
//!   immediately, but this module never mutates the running session; the status
//!   frame reports `restartRequired = activeDecoder != sessionActiveDecoder`.
//! * **Carrier honesty.** Only `native-below` supports the plugin manager. On
//!   `dom-canvas` the status says `supported: false` with the stable
//!   [`PLUGIN_UNSUPPORTED_CARRIER_REASON`] in `diagnostics`, mutations answer
//!   `accepted: false` with the same reason, and the bundled `tch` stays
//!   always-on (unchanged behaviour).
//!
//! This file is pure: it has no filesystem, Tauri or GPU access. The durable
//! bytes live behind `cad_platform::plugins::PluginStore`; the commands that
//! combine the two live in `cad_desktop_shell::plugins`. The one canonical id
//! grammar is re-exported from the platform store so both layers cannot drift.
//!
//! The wire shapes below are the frontend contract **exactly** (camelCase):
//! `{supported, carrier, plugins, activeDecoder, sessionActiveDecoder,
//! restartRequired, diagnostics}`; rows carry
//! `{id, name, version, abi, source, enabled, sessionActive, status}`; mutation
//! results carry `{accepted, reason?, restartRequired, status}`.

use serde::{Deserialize, Serialize};

pub use cad_platform::plugins::{
    is_safe_plugin_id, InstalledPluginRecord, PluginScan, MAX_PLUGIN_BYTES, MAX_PLUGIN_ID_BYTES,
    PLUGIN_ALREADY_INSTALLED_REASON, PLUGIN_ENTRY_FILE, PLUGIN_ENTRY_MISSING_REASON,
    PLUGIN_INVALID_BASE64_REASON, PLUGIN_INVALID_FILENAME_REASON, PLUGIN_INVALID_ID_REASON,
    PLUGIN_IO_REASON, PLUGIN_MANIFEST_INVALID_REASON, PLUGIN_MANIFEST_MISSING_REASON,
    PLUGIN_NOT_INSTALLED_REASON, PLUGIN_PAYLOAD_TOO_LARGE_REASON, PLUGIN_POLICY_INVALID_REASON,
    PLUGIN_READ_ENTRY_BUILTIN_REASON, PLUGIN_RESERVED_ID_REASON, PLUGIN_SCAN_FAILED_REASON,
    PLUGIN_STORE_UNAVAILABLE_REASON, PLUGIN_UNINSTALL_BUILTIN_REASON, PLUGIN_UNSAFE_PATH_REASON,
    PLUGIN_UNSUPPORTED_CARRIER_REASON,
};

use super::vendor::{decode_base64, encode_base64};

/// Version of the on-disk policy file.
pub const PLUGIN_POLICY_VERSION: u32 = 1;
/// Version of the synthesized manifest file.
pub const PLUGIN_MANIFEST_VERSION: u32 = 1;
/// ABI version understood by the bundled/installed decoder bundles.
pub const PLUGIN_ABI_VERSION: u32 = 1;
/// Id of the bundled Tianzheng decoder.
pub const BUILTIN_PLUGIN_ID: &str = "tch";
/// Display name of the bundled decoder.
pub const BUILTIN_PLUGIN_NAME: &str = "Tianzheng TCH";
/// Version of the bundled decoder.
pub const BUILTIN_PLUGIN_VERSION: &str = "0.1.0";
/// ABI of the bundled decoder.
pub const BUILTIN_PLUGIN_ABI: u32 = 1;
/// Entry file name inside every plugin directory.
pub const PLUGIN_ENTRY: &str = PLUGIN_ENTRY_FILE;

/// Metadata of the one bundled decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinPlugin {
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    pub abi: u32,
}

/// The bundled decoder metadata.
pub const BUILTIN_PLUGIN: BuiltinPlugin = BuiltinPlugin {
    id: BUILTIN_PLUGIN_ID,
    name: BUILTIN_PLUGIN_NAME,
    version: BUILTIN_PLUGIN_VERSION,
    abi: BUILTIN_PLUGIN_ABI,
};

/// Where one row came from (`source` on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginSource {
    Builtin,
    User,
}

/// The manifest verdict of one row (`status` on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRowStatus {
    Ok,
    ManifestMissing,
    ManifestInvalid,
}

impl BuiltinPlugin {
    /// The row this built-in contributes to the status list.
    pub fn row(&self, enabled: bool, session_active: bool) -> PluginRow {
        PluginRow {
            id: self.id.to_string(),
            name: self.name.to_string(),
            version: self.version.to_string(),
            abi: self.abi,
            source: PluginSource::Builtin,
            enabled,
            session_active,
            status: PluginRowStatus::Ok,
            installed_at: None,
        }
    }
}

/// Whether `id` is a valid plugin id (`[a-z0-9][a-z0-9_-]{0,63}`).
pub fn is_valid_plugin_id(id: &str) -> bool {
    is_safe_plugin_id(id)
}

/// Whether `id` is reserved for the bundled decoder.
pub fn is_reserved_plugin_id(id: &str) -> bool {
    id == BUILTIN_PLUGIN_ID
}

/// The on-disk policy: one active decoder, or `null` when all are disabled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPolicy {
    /// Policy schema version ([`PLUGIN_POLICY_VERSION`]).
    pub version: u32,
    /// The one active decoder id; `None` disables every decoder.
    pub active_decoder: Option<String>,
}

impl Default for PluginPolicy {
    fn default() -> Self {
        Self::builtin_default()
    }
}

impl PluginPolicy {
    /// The first-run policy: the bundled `tch` decoder is active.
    pub fn builtin_default() -> Self {
        Self {
            version: PLUGIN_POLICY_VERSION,
            active_decoder: Some(BUILTIN_PLUGIN_ID.to_string()),
        }
    }

    /// Parse and validate the policy file.
    ///
    /// `activeDecoder` must be present (the value `null` is the explicit
    /// "all disabled" state); a missing key is a malformed policy rather than a
    /// silent disable.
    pub fn parse(text: &str) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_str(text)
            .map_err(|error| format!("{PLUGIN_POLICY_INVALID_REASON}: {error}"))?;
        if value.get("activeDecoder").is_none() {
            return Err(format!(
                "{PLUGIN_POLICY_INVALID_REASON}: missing activeDecoder"
            ));
        }
        let policy: Self = serde_json::from_value(value)
            .map_err(|error| format!("{PLUGIN_POLICY_INVALID_REASON}: {error}"))?;
        if policy.version != PLUGIN_POLICY_VERSION {
            return Err(format!(
                "{PLUGIN_POLICY_INVALID_REASON}: unsupported version {} (expected {})",
                policy.version, PLUGIN_POLICY_VERSION
            ));
        }
        Ok(policy)
    }

    /// Serialize the policy (including explicit `activeDecoder: null`).
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self)
            .map_err(|error| format!("{PLUGIN_IO_REASON}: policy serialization: {error}"))
    }
}

/// The synthesized manifest of one installed plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub manifest_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub abi: u32,
    pub source_filename: String,
    pub entry: String,
    /// UTC install time, RFC 3339 (`2026-11-05T10:00:00Z`).
    pub installed_at: String,
}

impl PluginManifest {
    /// Synthesize the manifest the host writes for an installed bundle.
    pub fn synthesize(
        id: &str,
        name: &str,
        version: &str,
        source_filename: &str,
        installed_at: String,
    ) -> Self {
        Self {
            manifest_version: PLUGIN_MANIFEST_VERSION,
            id: id.to_string(),
            name: name.to_string(),
            version: version.to_string(),
            abi: PLUGIN_ABI_VERSION,
            source_filename: source_filename.to_string(),
            entry: PLUGIN_ENTRY_FILE.to_string(),
            installed_at,
        }
    }

    /// Parse and validate an installed manifest.
    pub fn parse(text: &str) -> Result<Self, String> {
        let manifest: Self = serde_json::from_str(text)
            .map_err(|error| format!("{PLUGIN_MANIFEST_INVALID_REASON}: {error}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validate the frozen manifest fields.
    pub fn validate(&self) -> Result<(), String> {
        if self.manifest_version != PLUGIN_MANIFEST_VERSION {
            return Err(format!(
                "{PLUGIN_MANIFEST_INVALID_REASON}: unsupported manifestVersion {}",
                self.manifest_version
            ));
        }
        if self.abi != PLUGIN_ABI_VERSION {
            return Err(format!(
                "{PLUGIN_MANIFEST_INVALID_REASON}: unsupported abi {}",
                self.abi
            ));
        }
        if self.entry != PLUGIN_ENTRY_FILE {
            return Err(format!(
                "{PLUGIN_MANIFEST_INVALID_REASON}: unsupported entry {}",
                self.entry
            ));
        }
        if !is_valid_plugin_id(&self.id) {
            return Err(format!(
                "{PLUGIN_MANIFEST_INVALID_REASON}: invalid id {}",
                self.id
            ));
        }
        if self.name.trim().is_empty() {
            return Err(format!("{PLUGIN_MANIFEST_INVALID_REASON}: empty name"));
        }
        if self.version.trim().is_empty() {
            return Err(format!("{PLUGIN_MANIFEST_INVALID_REASON}: empty version"));
        }
        Ok(())
    }

    /// Serialize the manifest.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self)
            .map_err(|error| format!("{PLUGIN_IO_REASON}: manifest serialization: {error}"))
    }
}

/// Derive the id slug from an install filename.
///
/// The extension must be `.js`/`.mjs` (case-insensitive). Characters outside
/// `[a-z0-9_-]` become `-`, repeated dashes collapse, and the result is trimmed
/// and truncated to [`MAX_PLUGIN_ID_BYTES`]. `None` when no usable id remains.
pub fn plugin_slug(filename: &str) -> Option<String> {
    let (stem, extension) = filename.rsplit_once('.')?;
    if !extension.eq_ignore_ascii_case("js") && !extension.eq_ignore_ascii_case("mjs") {
        return None;
    }
    let mut slug = String::new();
    let mut last_dash = false;
    for character in stem.chars() {
        let lower = character.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() || lower == '_' {
            slug.push(lower);
            last_dash = false;
        } else if !slug.is_empty() && !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    while slug.ends_with(['-', '_']) {
        slug.pop();
    }
    while slug.starts_with(['-', '_']) {
        slug.remove(0);
    }
    if slug.len() > MAX_PLUGIN_ID_BYTES {
        slug.truncate(MAX_PLUGIN_ID_BYTES);
        while slug.ends_with(['-', '_']) {
            slug.pop();
        }
    }
    if !is_valid_plugin_id(&slug) {
        return None;
    }
    Some(slug)
}

/// Validate an install filename and return the id it maps to.
///
/// The id must be derived from a basename with a `.js`/`.mjs` extension and must
/// not collide with the reserved built-in id.
pub fn plugin_id_for_filename(filename: &str) -> Result<String, String> {
    if filename.trim().is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains('\0')
    {
        return Err(format!("{PLUGIN_INVALID_FILENAME_REASON}: {filename}"));
    }
    let Some(id) = plugin_slug(filename) else {
        return Err(format!("{PLUGIN_INVALID_FILENAME_REASON}: {filename}"));
    };
    if is_reserved_plugin_id(&id) {
        return Err(format!("{PLUGIN_RESERVED_ID_REASON}: {id}"));
    }
    Ok(id)
}

/// Decode the base64 install payload under the frozen size limit.
pub fn decode_plugin_payload(payload: &str) -> Result<Vec<u8>, String> {
    // Bound the encoded length before allocating: standard base64 uses 4
    // characters per 3 bytes, plus padding.
    let max_encoded = MAX_PLUGIN_BYTES.div_ceil(3) * 4 + 4;
    if payload.len() > max_encoded {
        return Err(PLUGIN_PAYLOAD_TOO_LARGE_REASON.to_string());
    }
    let bytes = decode_base64(payload)
        .map_err(|error| format!("{PLUGIN_INVALID_BASE64_REASON}: {error}"))?;
    if bytes.len() > MAX_PLUGIN_BYTES {
        return Err(PLUGIN_PAYLOAD_TOO_LARGE_REASON.to_string());
    }
    Ok(bytes)
}

/// Encode entry bytes for the `read_entry` wire frame.
pub fn encode_plugin_entry(bytes: &[u8]) -> String {
    encode_base64(bytes)
}

/// Format unix epoch seconds as a UTC RFC 3339 timestamp.
///
/// A tiny civil-date conversion keeps the manifest timestamp stable without a
/// date crate. Out-of-range values clamp to the epoch.
pub fn format_installed_at(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let seconds_of_day = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard Hinnant's `civil_from_days` (days since 1970-01-01).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = (month_prime + if month_prime < 10 { 3 } else { -9 }) as u32;
    let year = year + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// One row of the plugin list (`cad_plugin_status().plugins[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRow {
    pub id: String,
    pub name: String,
    pub version: String,
    pub abi: u32,
    pub source: PluginSource,
    pub enabled: bool,
    pub session_active: bool,
    pub status: PluginRowStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_at: Option<String>,
}

impl PluginRow {
    /// Project a validated manifest into a healthy status row.
    pub fn from_manifest(manifest: &PluginManifest, enabled: bool, session_active: bool) -> Self {
        Self {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            abi: manifest.abi,
            source: PluginSource::User,
            enabled,
            session_active,
            status: PluginRowStatus::Ok,
            installed_at: Some(manifest.installed_at.clone()),
        }
    }

    /// A row whose manifest could not be read or parsed.
    ///
    /// Only facts the directory itself provides are shown (`id`); version/abi
    /// stay empty/zero and the non-`ok` status is the host's verdict.
    pub fn invalid(id: &str, status: PluginRowStatus, enabled: bool, session_active: bool) -> Self {
        Self {
            id: id.to_string(),
            name: id.to_string(),
            version: String::new(),
            abi: 0,
            source: PluginSource::User,
            enabled,
            session_active,
            status,
            installed_at: None,
        }
    }
}

/// The full plugin-manager status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginStatusFrame {
    /// Whether this carrier serves the plugin manager.
    pub supported: bool,
    /// The carrier wire name (`native-below` / `dom-canvas`).
    pub carrier: String,
    /// Built-in first, then installed plugins in id order.
    pub plugins: Vec<PluginRow>,
    /// The decoder the persisted policy selects.
    pub active_decoder: Option<String>,
    /// The decoder this process actually enabled at launch.
    pub session_active_decoder: Option<String>,
    /// Whether a restart is needed for `activeDecoder` to take effect.
    pub restart_required: bool,
    /// Non-fatal problems; an unsupported carrier names its reason here.
    pub diagnostics: Vec<String>,
}

impl PluginStatusFrame {
    /// Assemble a status frame, computing `restartRequired` from the two decoder
    /// ids (the one persisted now vs the one enabled at launch).
    pub fn new(
        supported: bool,
        carrier: &str,
        active_decoder: Option<String>,
        session_active_decoder: Option<String>,
        plugins: Vec<PluginRow>,
        diagnostics: Vec<String>,
    ) -> Self {
        let restart_required = active_decoder != session_active_decoder;
        Self {
            supported,
            carrier: carrier.to_string(),
            plugins,
            active_decoder,
            session_active_decoder,
            restart_required,
            diagnostics,
        }
    }

    /// The honest `dom-canvas` answer: unsupported, with the bundled decoder
    /// still listed as the always-on one.
    pub fn unsupported() -> Self {
        Self::new(
            false,
            "dom-canvas",
            Some(BUILTIN_PLUGIN_ID.to_string()),
            Some(BUILTIN_PLUGIN_ID.to_string()),
            vec![BUILTIN_PLUGIN.row(true, true)],
            vec![PLUGIN_UNSUPPORTED_CARRIER_REASON.to_string()],
        )
    }
}

/// Result of one plugin mutation (`accepted` + the registry after it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginMutationResult {
    pub accepted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub restart_required: bool,
    pub status: PluginStatusFrame,
}

impl PluginMutationResult {
    /// Build the result of a persisted mutation.
    pub fn persisted(status: PluginStatusFrame) -> Self {
        Self {
            accepted: true,
            reason: None,
            restart_required: status.restart_required,
            status,
        }
    }

    /// Build a refusal that still carries the real current registry snapshot.
    pub fn refused(reason: impl Into<String>, status: PluginStatusFrame) -> Self {
        Self {
            accepted: false,
            reason: Some(reason.into()),
            restart_required: status.restart_required,
            status,
        }
    }
}

/// The base64 bundle returned by `cad_plugin_read_entry`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginEntryFrame {
    /// Standard base64 of `plugin.js`.
    pub payload: String,
    /// The entry file name (`plugin.js`).
    pub entry: String,
}

impl PluginEntryFrame {
    /// Encode one installed entry.
    pub fn new(bytes: &[u8]) -> Self {
        Self {
            payload: encode_plugin_entry(bytes),
            entry: PLUGIN_ENTRY_FILE.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_metadata_is_frozen() {
        assert_eq!(BUILTIN_PLUGIN.id, "tch");
        assert_eq!(BUILTIN_PLUGIN.name, "Tianzheng TCH");
        assert_eq!(BUILTIN_PLUGIN.version, "0.1.0");
        assert_eq!(BUILTIN_PLUGIN.abi, 1);
        let row = BUILTIN_PLUGIN.row(true, true);
        assert_eq!(row.source, PluginSource::Builtin);
        assert!(row.enabled);
        assert!(row.session_active);
        assert_eq!(row.status, PluginRowStatus::Ok);
        assert_eq!(row.id, "tch");
    }

    #[test]
    fn slug_derivation_is_deterministic_and_bounded() {
        assert_eq!(plugin_slug("demo.js").as_deref(), Some("demo"));
        assert_eq!(plugin_slug("Demo Tools.mjs").as_deref(), Some("demo-tools"));
        assert_eq!(plugin_slug("my_plugin.JS").as_deref(), Some("my_plugin"));
        assert_eq!(plugin_slug("a.b.c.js").as_deref(), Some("a-b-c"));
        assert_eq!(plugin_slug("---.js"), None);
        assert_eq!(plugin_slug("demo.txt"), None);
        assert_eq!(plugin_slug("demo"), None);
        assert_eq!(plugin_slug(".js"), None);
        let long = format!("{}.js", "x".repeat(MAX_PLUGIN_ID_BYTES + 20));
        assert_eq!(
            plugin_slug(&long).as_deref().map(str::len),
            Some(MAX_PLUGIN_ID_BYTES)
        );
    }

    #[test]
    fn install_filenames_are_basenames_with_a_js_extension() {
        assert_eq!(
            plugin_id_for_filename("My Tools.js").as_deref(),
            Ok("my-tools")
        );
        for filename in ["tch.js", "TCH.mjs"] {
            let error = plugin_id_for_filename(filename).expect_err("reserved");
            assert!(error.starts_with(PLUGIN_RESERVED_ID_REASON), "{error}");
        }
        for filename in ["", "..", "a/b.js", "a\\b.js", "demo.txt", "demo"] {
            let error = plugin_id_for_filename(filename).expect_err("invalid");
            assert!(
                error.starts_with(PLUGIN_INVALID_FILENAME_REASON),
                "{filename}: {error}"
            );
        }
    }

    #[test]
    fn id_validation_and_reservation_are_exact() {
        for id in ["a", "tch", "plugin_1-x", "0abc"] {
            assert!(is_valid_plugin_id(id), "{id}");
        }
        for id in ["", "_a", "-a", "A", "a/b", "..", "a b"] {
            assert!(!is_valid_plugin_id(id), "{id}");
        }
        assert!(is_reserved_plugin_id("tch"));
        assert!(!is_reserved_plugin_id("tch2"));
    }

    #[test]
    fn policy_round_trips_with_explicit_null_and_rejects_wrong_version() {
        let default = PluginPolicy::builtin_default();
        assert_eq!(default.active_decoder.as_deref(), Some("tch"));
        assert_eq!(
            PluginPolicy::parse(&default.to_json().expect("json")),
            Ok(default)
        );

        let disabled = PluginPolicy {
            version: 1,
            active_decoder: None,
        };
        let json = disabled.to_json().expect("json");
        assert!(json.contains("\"activeDecoder\":null"), "{json}");
        assert_eq!(PluginPolicy::parse(&json), Ok(disabled));

        let error = PluginPolicy::parse(r#"{"version":2,"activeDecoder":null}"#)
            .expect_err("version mismatch");
        assert!(error.starts_with(PLUGIN_POLICY_INVALID_REASON), "{error}");
        let error = PluginPolicy::parse(r#"{"version":1}"#).expect_err("missing field");
        assert!(error.starts_with(PLUGIN_POLICY_INVALID_REASON), "{error}");
    }

    #[test]
    fn manifest_round_trips_with_the_frozen_wire_names() {
        let manifest = PluginManifest::synthesize(
            "demo",
            "Demo",
            "1.2.3",
            "Demo.js",
            "2026-11-05T10:00:00Z".to_string(),
        );
        let value = serde_json::to_value(&manifest).expect("value");
        assert_eq!(value["manifestVersion"], 1);
        assert_eq!(value["sourceFilename"], "Demo.js");
        assert_eq!(value["installedAt"], "2026-11-05T10:00:00Z");
        assert_eq!(value["entry"], "plugin.js");
        let json = manifest.to_json().expect("json");
        assert_eq!(PluginManifest::parse(&json), Ok(manifest));

        let mut invalid = PluginManifest::synthesize(
            "demo",
            "Demo",
            "1.2.3",
            "Demo.js",
            "2026-11-05T10:00:00Z".to_string(),
        );
        invalid.entry = "index.js".to_string();
        assert!(invalid
            .validate()
            .expect_err("entry mismatch")
            .starts_with(PLUGIN_MANIFEST_INVALID_REASON));
        invalid.entry = "plugin.js".to_string();
        invalid.abi = 2;
        assert!(invalid
            .validate()
            .expect_err("abi mismatch")
            .starts_with(PLUGIN_MANIFEST_INVALID_REASON));
    }

    #[test]
    fn installed_timestamps_are_utc_rfc3339() {
        assert_eq!(format_installed_at(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_installed_at(1_760_000_000), "2025-10-09T08:53:20Z");
    }

    #[test]
    fn status_restart_required_tracks_actual_and_session_active() {
        let plugins = vec![BUILTIN_PLUGIN.row(true, true)];
        let current = PluginStatusFrame::new(
            true,
            "native-below",
            Some("tch".into()),
            Some("tch".into()),
            plugins.clone(),
            Vec::new(),
        );
        assert!(!current.restart_required);

        let pending = PluginStatusFrame::new(
            true,
            "native-below",
            Some("demo".into()),
            Some("tch".into()),
            plugins,
            vec!["plugin.manifest_invalid: demo".into()],
        );
        assert!(pending.restart_required);

        let value = serde_json::to_value(&pending).expect("value");
        assert_eq!(value["activeDecoder"], "demo");
        assert_eq!(value["sessionActiveDecoder"], "tch");
        assert_eq!(value["restartRequired"], true);
        assert_eq!(value["supported"], true);
        assert!(value.get("reason").is_none());
        assert!(value.get("rows").is_none());
        assert_eq!(value["plugins"][0]["source"], "builtin");
        assert_eq!(value["plugins"][0]["status"], "ok");
        assert!(value["plugins"][0].get("builtin").is_none());
        assert!(value["plugins"][0].get("active").is_none());
    }

    #[test]
    fn unsupported_frame_keeps_the_builtin_always_on() {
        let frame = PluginStatusFrame::unsupported();
        assert!(!frame.supported);
        assert_eq!(frame.carrier, "dom-canvas");
        assert_eq!(frame.diagnostics, vec![PLUGIN_UNSUPPORTED_CARRIER_REASON]);
        assert_eq!(frame.active_decoder.as_deref(), Some("tch"));
        assert_eq!(frame.session_active_decoder.as_deref(), Some("tch"));
        assert!(!frame.restart_required);
        assert_eq!(frame.plugins.len(), 1);
        assert_eq!(frame.plugins[0].source, PluginSource::Builtin);
        assert!(frame.plugins[0].enabled && frame.plugins[0].session_active);
    }

    #[test]
    fn mutation_result_matches_the_frontend_shape() {
        let status = PluginStatusFrame::unsupported();
        let refused =
            PluginMutationResult::refused(PLUGIN_UNSUPPORTED_CARRIER_REASON, status.clone());
        let value = serde_json::to_value(&refused).expect("value");
        assert_eq!(value["accepted"], false);
        assert_eq!(value["reason"], PLUGIN_UNSUPPORTED_CARRIER_REASON);
        assert_eq!(value["restartRequired"], false);
        assert!(value["status"].is_object());
        assert!(value.get("ok").is_none());

        let persisted = PluginMutationResult::persisted(status);
        let value = serde_json::to_value(&persisted).expect("value");
        assert_eq!(value["accepted"], true);
        assert!(value.get("reason").is_none());
    }

    #[test]
    fn payload_decode_enforces_base64_and_the_size_limit() {
        let bytes = b"export function decode(buffer) { return buffer; }";
        assert_eq!(
            decode_plugin_payload(&encode_plugin_entry(bytes)).expect("decodes"),
            bytes
        );
        assert!(decode_plugin_payload("!!!!")
            .expect_err("invalid base64")
            .starts_with(PLUGIN_INVALID_BASE64_REASON));
        let oversized = "A".repeat(MAX_PLUGIN_BYTES.div_ceil(3) * 4 + 8);
        assert!(decode_plugin_payload(&oversized)
            .expect_err("too large")
            .starts_with(PLUGIN_PAYLOAD_TOO_LARGE_REASON));
        let frame = PluginEntryFrame::new(bytes);
        assert_eq!(frame.entry, PLUGIN_ENTRY_FILE);
        assert_eq!(decode_base64(&frame.payload).expect("round trip"), bytes);
    }
}
