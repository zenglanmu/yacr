//! Small wire value types shared by the command, orbit and view channels.
//!
//! Split out of `wire.rs` for the code-size gate. They carry no policy: the
//! gates and parsers live in `admission.rs`, and `wire.rs` re-exports these so
//! every existing `wire::…` path is unchanged.
/// Admission result of a queued/applied command: **accepted**, not completed.
///
/// `{accepted:true}` is the whole promise: on the desktop it means *queued* onto
/// the owner thread, and on a synchronous sink it means *applied*. The real
/// outcome surfaces through `cad_status.error`. It lives here (rather than in the
/// Tauri layer) so both hosts answer the identical wire shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct CadCommandAck {
    /// Always `true` for an admitted command; it never reports completion.
    pub accepted: bool,
}

impl CadCommandAck {
    /// The only acknowledgement this layer produces.
    pub fn accepted() -> Self {
        Self { accepted: true }
    }
}

/// One AutoCAD-style abbreviation the router accepts, as the chrome consumes it.
///
/// `command` is the canonical selector the alias normalizes onto (never another
/// alias), so a frontend can display the target without re-resolving anything.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadCommandAlias {
    /// The typed abbreviation, e.g. `L`.
    pub alias: String,
    /// The canonical selector it resolves to, e.g. `LINE`.
    pub command: String,
}

/// The stable refusal of a `cad_orbit` request whose angles are not finite.
///
/// Shared by the wire parser ([`super::admission::parse_orbit`]) and every host, so
/// a NaN/infinite delta is refused by name instead of corrupting the camera.
pub const ORBIT_ANGLES_NOT_FINITE: &str =
    "orbit.angles_not_finite: cad_orbit needs finite yaw and pitch";

/// One typed orbit request: yaw/pitch deltas in radians, both finite.
///
/// Reached from the dedicated `cad_orbit` wire command (the WebView ViewCube), never
/// parsed out of the name-only `cad_command` funnel. It is carried verbatim — no
/// clamp and no default — because the application's orbit is a relative gesture and
/// a clamped delta would silently change the requested camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrbitDelta {
    /// Rotation about world +Z, radians.
    pub yaw: f64,
    /// Rotation about the view right axis, radians.
    pub pitch: f64,
}
