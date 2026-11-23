//! Which prepared-scene diagnostics may become the host-visible error.
//!
//! `cad_status.error` is a single, ungraded string and the React shell labels it
//! "host-reported error". An overlay diagnostic therefore only belongs there when
//! the scene is genuinely [`Completeness::Missing`] — something could not be
//! resolved at all (an unmatched selection reference), not merely drawn
//! differently.
//!
//! A [`Completeness::Partial`] verdict is a **display approximation**: the
//! `$PDMODE 0` dot drawn as a plus because the marker topology is `Lines`-only,
//! a clamped `$PDSIZE`, a marker cap. Those stay in
//! [`PreparedScene::highlight_diagnostics`] for any host that wants to inspect
//! them; they are not a red banner on an otherwise healthy drawing (ADR 0015,
//! 2026-11-22 增补). Promotion of every `Partial` diagnostic was exactly how a
//! QCAD-exported flange with the DWG-default `$PDMODE 0` opened "with a host
//! error" — the drawing had opened correctly.

use cad_app::render_scene::PreparedScene;
use cad_domain::Completeness;

/// The one host-facing error a prepared scene may contribute.
///
/// `None` for `Complete`, `Partial` and `Unverified`; for `Missing` the message
/// is the diagnostic whose text is one of the missing reasons (falling back to
/// the first reason when a producer recorded a reason without a diagnostic).
pub fn highlight_failure_message(ready: &PreparedScene) -> Option<String> {
    let Completeness::Missing(reasons) = &ready.highlight_completeness else {
        return None;
    };
    ready
        .highlight_diagnostics
        .iter()
        .find(|diagnostic| reasons.iter().any(|reason| reason == &diagnostic.message))
        .map(|diagnostic| diagnostic.message.clone())
        .or_else(|| reasons.first().cloned())
}
