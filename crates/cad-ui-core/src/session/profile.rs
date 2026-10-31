//! The typed carrier description carried by `cad_status` (`hostProfile`).
//!
//! Historically the wire told the chrome which carrier it was talking to by the
//! **presence** of top-level keys (`surfaceMode`, `palettesAvailable`,
//! `standardViews`): the desktop `native-below` hosts omitted them and the
//! `dom-canvas` hosts added them. That made "absent" mean a different thing for
//! every key and forced every consumer to guess the carrier from a missing field.
//!
//! [`HostProfile`] replaces that with one **always-present** `hostProfile` object.
//! It is the single typed description of the carrier:
//!
//! ```json
//! "hostProfile": {
//!   "surfaceMode": "native-below",
//!   "documentOwner": "native",
//!   "palettesAvailable": true,
//!   "standardViews": null
//! }
//! ```
//!
//! `standardViews: null` means **not declared** (today's `native-below`
//! behaviour: every standard view is allowed); an empty array means **declared,
//! none available** (today's `dom-canvas` behaviour: the seven view buttons are
//! refused with their own reason). That distinction preserves the current
//! semantics instead of turning "undeclared" into "none".
//!
//! [`WireStatus`] is the envelope that keeps the frozen [`CadStatus`] field set
//! and **order** intact while appending the profile (and the optional
//! `canvasSession`): the flattened status is serialized first, then
//! `hostProfile`, then `canvasSession`. A `serde_json::Value` round-trip was
//! rejected on purpose — it would sort the keys and change the wire order.
//!
//! The envelope is generic over the `canvasSession` payload because the concrete
//! descriptor type lives in the host crates (`apps/app-linux`,
//! `apps/app-web`) in this phase. Both are plain `Serialize + Clone` payloads;
//! nothing about the descriptor is defined here, so `cad-ui-core` gains no host
//! dependency.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use super::status::CadStatus;

/// The carrier the host runs, as the wire spells it.
///
/// * [`HostSurfaceMode::NativeBelow`] — the native CAD session presents below the
///   WebView (Wayland production path, Windows/macOS hosts).
/// * [`HostSurfaceMode::DomCanvas`] — the wasm CAD session owns the pixels in the
///   WebView (`dom-canvas` runtime, ADR 0024).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostSurfaceMode {
    /// `"native-below"`.
    NativeBelow,
    /// `"dom-canvas"`.
    DomCanvas,
}

/// Which side owns the authoritative `cad-db` document.
///
/// Iron rule 8: on `dom-canvas` the wasm session is the only authority and the
/// native shell must not instantiate a second document. This field makes that
/// fact explicit instead of leaving it to be inferred from the carrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentOwner {
    /// The native host session owns the document.
    Native,
    /// The wasm session in the WebView owns the document.
    Wasm,
}

/// The typed carrier description carried by every `cad_status`.
///
/// Construct one of the two honest profiles with [`HostProfile::native_below`] /
/// [`HostProfile::dom_canvas`]; nothing else is produced by this repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostProfile {
    /// The installed carrier.
    pub surface_mode: HostSurfaceMode,
    /// The side that owns the authoritative document.
    pub document_owner: DocumentOwner,
    /// Whether the docked palettes (layers, selection rows, settings) are served
    /// by this carrier.
    pub palettes_available: bool,
    /// The declared standard-view capability list, or `null` when undeclared.
    ///
    /// `null` keeps every view enabled (the `native-below` answer, unchanged from
    /// the previous absence of the key); `[]` declares "none available" (the
    /// `dom-canvas` answer, unchanged from the previous top-level empty array).
    pub standard_views: Option<Vec<String>>,
}

impl HostProfile {
    /// The native session presents below the WebView: the document is native,
    /// the palettes are served, and the standard views are **undeclared** (the
    /// same "allow all" semantics the old missing key had).
    pub fn native_below() -> Self {
        Self {
            surface_mode: HostSurfaceMode::NativeBelow,
            document_owner: DocumentOwner::Native,
            palettes_available: true,
            // `None` = not declared, preserving the pre-profile semantics where
            // an absent key kept every standard view enabled.
            standard_views: None,
        }
    }

    /// The wasm session owns the pixels: the document is wasm-owned, the palettes
    /// are served by that same session, and the standard views are **declared
    /// empty** (the same refusal the old top-level `standardViews: []` carried).
    pub fn dom_canvas() -> Self {
        Self {
            surface_mode: HostSurfaceMode::DomCanvas,
            document_owner: DocumentOwner::Wasm,
            palettes_available: true,
            standard_views: Some(Vec::new()),
        }
    }
}

/// The `cad_status` envelope: frozen [`CadStatus`] fields, then `hostProfile`,
/// then the optional `canvasSession`.
///
/// The fields are [`Cow`]so one type serves both shapes without cloning on the
/// wasm path:
///
/// * the wasm path serializes borrowed values ([`WireStatus::new`], used by
///   [`wire_status_json`]);
/// * the Tauri command paths own their freshly built status/profile and return
///   [`WireStatus::owned`], because a command cannot return references to its own
///   locals.
///
/// `C` is the host-local `canvasSession` payload (`CanvasSessionDescriptor` on
/// `app-linux`, `CanvasSession` on `app-web`); it only has to be
/// `Serialize + Clone`. Both crates therefore share this envelope instead of
/// each keeping a look-alike wrapper.
#[derive(Debug, Serialize)]
pub struct WireStatus<'a, C: Serialize + Clone = serde_json::Value> {
    /// The frozen `CadStatus` field set, flattened **first** so its key order is
    /// unchanged.
    #[serde(flatten)]
    pub status: Cow<'a, CadStatus>,
    /// The typed carrier description; always present.
    #[serde(rename = "hostProfile")]
    pub profile: Cow<'a, HostProfile>,
    /// The wasm session's own descriptor, or absent before the first report.
    #[serde(rename = "canvasSession", skip_serializing_if = "Option::is_none")]
    pub canvas: Option<Cow<'a, C>>,
}

impl<'a, C: Serialize + Clone> WireStatus<'a, C> {
    /// Borrow every component (the wasm serialization path).
    pub fn new(status: &'a CadStatus, profile: &'a HostProfile, canvas: Option<&'a C>) -> Self {
        Self {
            status: Cow::Borrowed(status),
            profile: Cow::Borrowed(profile),
            canvas: canvas.map(Cow::Borrowed),
        }
    }

    /// Take ownership of the status, profile and descriptor (the Tauri command
    /// path, where the values are computed locally).
    pub fn owned(status: CadStatus, profile: HostProfile, canvas: Option<C>) -> Self {
        Self {
            status: Cow::Owned(status),
            profile: Cow::Owned(profile),
            canvas: canvas.map(Cow::Owned),
        }
    }
}

/// Serialize the `cad_status` wire object for the wasm carrier.
///
/// This is the only serializer the wasm path needs; the desktop hosts return
/// [`WireStatus`] itself and let Tauri serialize it, so there is exactly one
/// field-name/order definition. Key order is the flattened [`CadStatus`] fields
/// first, then `hostProfile`, then `canvasSession`.
pub fn wire_status_json<C: Serialize + Clone>(
    status: &CadStatus,
    profile: &HostProfile,
    canvas: Option<&C>,
) -> String {
    let wire = WireStatus::new(status, profile, canvas);
    serde_json::to_string(&wire).expect("CadStatus/HostProfile are plain JSON values")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::unsupported_status;

    fn status_has_profile(json: &str, profile: &HostProfile) {
        let value: serde_json::Value = serde_json::from_str(json).expect("valid JSON");
        assert_eq!(
            value.get("hostProfile"),
            Some(&serde_json::to_value(profile).expect("profile serializes")),
            "{json}"
        );
    }

    #[test]
    fn native_below_and_dom_canvas_carry_the_documented_profiles() {
        let native = HostProfile::native_below();
        assert_eq!(native.surface_mode, HostSurfaceMode::NativeBelow);
        assert_eq!(native.document_owner, DocumentOwner::Native);
        assert!(native.palettes_available);
        assert_eq!(
            native.standard_views, None,
            "native leaves views undeclared"
        );

        let canvas = HostProfile::dom_canvas();
        assert_eq!(canvas.surface_mode, HostSurfaceMode::DomCanvas);
        assert_eq!(canvas.document_owner, DocumentOwner::Wasm);
        assert!(canvas.palettes_available);
        assert_eq!(
            canvas.standard_views,
            Some(Vec::new()),
            "dom-canvas declares none, it does not leave them undeclared"
        );
    }

    #[test]
    fn the_profile_wire_spelling_is_frozen_and_round_trips() {
        let native = HostProfile::native_below();
        assert_eq!(
            serde_json::to_value(&native).expect("serializes"),
            serde_json::json!({
                "surfaceMode": "native-below",
                "documentOwner": "native",
                "palettesAvailable": true,
                "standardViews": null,
            })
        );
        let canvas = HostProfile::dom_canvas();
        assert_eq!(
            serde_json::to_value(&canvas).expect("serializes"),
            serde_json::json!({
                "surfaceMode": "dom-canvas",
                "documentOwner": "wasm",
                "palettesAvailable": true,
                "standardViews": [],
            })
        );
        let decoded: HostProfile =
            serde_json::from_value(serde_json::to_value(&native).expect("serializes"))
                .expect("profile decodes");
        assert_eq!(decoded, native);
    }

    #[test]
    fn the_flat_status_keys_keep_their_order_before_the_profile() {
        // `to_string` (not a `Value` round-trip) must emit the frozen CadStatus
        // fields first, then `hostProfile`, then `canvasSession`.
        let status = unsupported_status(None);
        let json = wire_status_json::<serde_json::Value>(&status, &HostProfile::dom_canvas(), None);
        let first = json.find("\"documentLabel\"").expect("first field");
        let last_status = json.find("\"gripDrag\"").expect("last frozen field");
        let profile = json.find("\"hostProfile\"").expect("profile key");
        assert!(first < last_status, "frozen field order changed: {json}");
        assert!(
            last_status < profile,
            "hostProfile must follow the flattened status: {json}"
        );
        // The profile's own keys keep their declaration order.
        let surface = json.find("\"surfaceMode\"").expect("surfaceMode");
        let owner = json.find("\"documentOwner\"").expect("documentOwner");
        let palettes = json
            .find("\"palettesAvailable\"")
            .expect("palettesAvailable");
        let views = json.find("\"standardViews\"").expect("standardViews");
        assert!(profile < surface && surface < owner && owner < palettes && palettes < views);
        // No descriptor reported yet: the key is absent, not `null`.
        assert!(!json.contains("\"canvasSession\""), "{json}");
        status_has_profile(&json, &HostProfile::dom_canvas());
    }

    #[test]
    fn a_canvas_descriptor_is_appended_after_the_profile() {
        let status = unsupported_status(None);
        let canvas = serde_json::json!({"label": "plan.dwg", "batches": 2, "draws": 5});
        let json = wire_status_json(&status, &HostProfile::dom_canvas(), Some(&canvas));
        let profile = json.find("\"hostProfile\"").expect("profile key");
        let session = json.find("\"canvasSession\"").expect("descriptor key");
        assert!(profile < session, "canvasSession must come last: {json}");
        assert!(json.ends_with("}"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(
            value.get("canvasSession"),
            Some(&canvas),
            "the descriptor is relayed unchanged"
        );
        status_has_profile(&json, &HostProfile::dom_canvas());
    }
}
